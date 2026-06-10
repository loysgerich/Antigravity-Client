pub mod db;
pub mod local_proxy;
pub mod dns;
pub mod tunnel;
pub mod utils {
    pub mod protobuf;
}

use std::sync::Mutex;
use tokio::sync::watch;

// Global proxy shutdown handle
static PROXY_SHUTDOWN: Mutex<Option<watch::Sender<bool>>> = Mutex::new(None);
static PROXY_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static PROXY_SESSION_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[tauri::command]
async fn inject_token_and_start_ide(
    token: String, 
    proxy_url: String,
    ide_type: String,
    custom_exe_path: Option<String>,
    custom_db_path: Option<String>,
) -> Result<String, String> {
    // 1. Kill any running instance of the selected IDE first
    kill_running_antigravity(&ide_type);

    // 2. Strip /v1 suffix if present since the proxy needs the base URL
    let base_url = proxy_url.trim_end_matches("/v1").to_string();

    // 3. Stop existing proxy first to avoid port conflict and configuration reuse
    stop_existing_proxy();
    let _session_id = PROXY_SESSION_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;

    // 4. Start local proxy on port 8047
    eprintln!("[Client] Starting local proxy on :8047 -> {}", base_url);

    let config = local_proxy::ProxyConfig {
        listen_port: 8047,
        target_url: base_url.clone(),
        bearer_token: token.clone(),
    };

    let shutdown_tx = local_proxy::start_proxy(config)
        .await
        .map_err(|e| format!("Failed to start local proxy: {}", e))?;

    // Store the shutdown handle
    if let Ok(mut guard) = PROXY_SHUTDOWN.lock() {
        *guard = Some(shutdown_tx);
    }
    PROXY_RUNNING.store(true, std::sync::atomic::Ordering::SeqCst);
    eprintln!("[Client] Local proxy started successfully");

    eprintln!("[Client] Injecting proxy token into IDE keyring...");
    db::inject_real_token(
        &token,
        "proxy_managed_refresh_token",
        4070908800i64,
        "http://127.0.0.1:8047/v1", // Route IDE traffic to our local proxy
        &ide_type,
        custom_db_path.as_deref(),
    )?;

    eprintln!("[Client] Token injected successfully");

    // 5. Start Reverse Tunnel for API requests
    eprintln!("[Client] Starting reverse tunnel connection to Manager...");
    crate::tunnel::start_tunnel_worker(base_url.clone(), token.clone()).await;

    // 6. Patch IDE main.js to route hardcoded Google API URLs through our local proxy
    eprintln!("[Client] Patching IDE main.js to redirect API traffic through local proxy...");
    match patch_ide_main_js(&ide_type, custom_exe_path.as_deref()) {
        Ok(patched) => {
            if patched {
                eprintln!("[Client] IDE main.js patched successfully");
            } else {
                eprintln!("[Client] IDE main.js already patched or not found");
            }
        }
        Err(e) => eprintln!("[Client] Warning: Failed to patch IDE main.js: {}", e),
    }

    // 6b. Patch language server binary to route Google API URLs through our local proxy
    eprintln!("[Client] Patching IDE language server binary to redirect API traffic through local proxy...");
    match patch_ide_language_server(&ide_type, custom_exe_path.as_deref()) {
        Ok(patched) => {
            if patched {
                eprintln!("[Client] IDE language_server patched successfully");
            } else {
                eprintln!("[Client] IDE language_server already patched or not found");
            }
        }
        Err(e) => eprintln!("[Client] Warning: Failed to patch IDE language_server: {}", e),
    }

    // 7. Start Antigravity IDE
    start_antigravity_ide(&ide_type, custom_exe_path.as_deref())?;

    Ok("Proxy started and IDE launched successfully".to_string())
}

#[tauri::command]
fn stop_proxy() -> Result<String, String> {
    stop_existing_proxy();
    Ok("Proxy stopped".to_string())
}

#[tauri::command]
fn get_proxy_status() -> bool {
    PROXY_RUNNING.load(std::sync::atomic::Ordering::SeqCst)
}

fn stop_existing_proxy() {
    if let Ok(mut guard) = PROXY_SHUTDOWN.lock() {
        if let Some(tx) = guard.take() {
            let _ = tx.send(true);
            eprintln!("[Client] Sent proxy shutdown signal");
        }
    }
    PROXY_RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
}

fn stop_proxy_for_session(session_id: u64) {
    let current_session = PROXY_SESSION_ID.load(std::sync::atomic::Ordering::SeqCst);
    if current_session == session_id {
        eprintln!("[Client] Stopping proxy for session {}", session_id);
        stop_existing_proxy();
    } else {
        eprintln!("[Client] Ignoring stop_proxy for old session {} (current is {})", session_id, current_session);
    }
}

/// Kill running Antigravity processes for the selected IDE type
fn kill_running_antigravity(ide_type: &str) {
    #[cfg(target_os = "macos")]
    {
        let app_name = if ide_type == "Antigravity 2.0" { "Antigravity" } else { "Antigravity IDE" };
        let _ = std::process::Command::new("pkill")
            .args(["-f", app_name])
            .output();
    }
    #[cfg(target_os = "windows")]
    {
        let exe_name = if ide_type == "Antigravity 2.0" { "Antigravity.exe" } else { "Antigravity IDE.exe" };
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/IM", exe_name])
            .output();
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/IM", "language_server.exe"])
            .output();
    }
    #[cfg(target_os = "linux")]
    {
        let bin_name = if ide_type == "Antigravity 2.0" { "antigravity" } else { "antigravity-ide" };
        let _ = std::process::Command::new("pkill")
            .args(["-x", bin_name])
            .output();
        let _ = std::process::Command::new("pkill")
            .args(["-f", "language_server"])
            .output();
    }
}

/// Find the IDE's main.js file path based on ide_type and optional custom exe path
/// Find the IDE's resources directory based on ide_type and optional custom exe path
fn get_ide_resources_path(ide_type: &str, custom_exe_path: Option<&str>) -> Option<std::path::PathBuf> {
    if let Some(exe) = custom_exe_path {
        if !exe.is_empty() {
            let exe_path = std::path::Path::new(exe);
            if let Some(parent) = exe_path.parent() {
                let resources = parent.join("resources");
                if resources.exists() {
                    return Some(resources);
                }
            }
        }
    }

    // Default paths by OS
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("LOCALAPPDATA").unwrap_or_default();
        let subfolder = if ide_type == "Antigravity 2.0" { "Antigravity" } else { "Antigravity IDE" };
        let path = std::path::PathBuf::from(&appdata)
            .join("Programs")
            .join(subfolder)
            .join("resources");
        if path.exists() {
            return Some(path);
        }
        // Fallback for case-insensitive check
        let path_lower = std::path::PathBuf::from(&appdata)
            .join("Programs")
            .join(if ide_type == "Antigravity 2.0" { "antigravity" } else { "Antigravity IDE" })
            .join("resources");
        if path_lower.exists() {
            return Some(path_lower);
        }
    }

    #[cfg(target_os = "macos")]
    {
        let app_name = if ide_type == "Antigravity 2.0" { "Antigravity" } else { "Antigravity IDE" };
        let path = std::path::PathBuf::from(format!(
            "/Applications/{}.app/Contents/Resources", app_name
        ));
        if path.exists() {
            return Some(path);
        }
    }

    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir().unwrap_or_default();
        let bin_name = if ide_type == "Antigravity 2.0" { "antigravity" } else { "antigravity-ide" };
        let path = std::path::PathBuf::from(format!("/usr/share/{}/resources", bin_name));
        if path.exists() {
            return Some(path);
        }
        let path2 = home.join(format!(".local/share/{}/resources", bin_name));
        if path2.exists() {
            return Some(path2);
        }
    }

    None
}

/// Patch the IDE's main.js or app.asar to redirect hardcoded Google API URLs through local proxy.
/// Returns Ok(true) if patched, Ok(false) if already patched or file not found.
fn patch_ide_main_js(ide_type: &str, custom_exe_path: Option<&str>) -> Result<bool, String> {
    let resources_dir = match get_ide_resources_path(ide_type, custom_exe_path) {
        Some(p) => p,
        None => {
            eprintln!("[Client] Could not find IDE resources directory to patch");
            return Ok(false);
        }
    };

    let app_asar = resources_dir.join("app.asar");
    let main_js = resources_dir.join("app").join("out").join("main.js");

    if app_asar.exists() {
        eprintln!("[Client] Found IDE app.asar at: {:?}", app_asar);
        patch_asar_file(&app_asar)
    } else if main_js.exists() {
        eprintln!("[Client] Found IDE main.js at: {:?}", main_js);
        patch_js_file(&main_js)
    } else {
        eprintln!("[Client] Neither app.asar nor app/out/main.js found in resources");
        Ok(false)
    }
}

/// Find the language server binary path and patch it.
fn patch_ide_language_server(ide_type: &str, custom_exe_path: Option<&str>) -> Result<bool, String> {
    let resources_dir = match get_ide_resources_path(ide_type, custom_exe_path) {
        Some(p) => p,
        None => {
            eprintln!("[Client] Could not find IDE resources directory to patch language server");
            return Ok(false);
        }
    };

    let bin_dir = resources_dir.join("bin");
    if !bin_dir.exists() {
        return Ok(false);
    }

    let binary_names = ["language_server.exe", "language_server"];
    let mut patched = false;

    for name in &binary_names {
        let bin_path = bin_dir.join(name);
        if bin_path.exists() {
            eprintln!("[Client] Found language server binary at: {:?}", bin_path);
            if patch_binary_file(&bin_path)? {
                patched = true;
            }
        }
    }

    Ok(patched)
}

/// Binary exact-length patching for language server binaries
fn patch_binary_file(path: &std::path::Path) -> Result<bool, String> {
    let mut content = std::fs::read(path)
        .map_err(|e| format!("Failed to read binary file {:?}: {}", path, e))?;

    let replacements = [
        (
            b"https://cloudcode-pa.googleapis.com".to_vec(),
            b"http://127.0.0.1:8047/v1internal///".to_vec()
        ),
        (
            b"https://autopush-cloudcode-pa.sandbox.googleapis.com".to_vec(),
            b"http://127.0.0.1:8047/v1internal////////////////////".to_vec()
        ),
        (
            b"https://preprod-daily-cloudcode-pa.sandbox.googleapis.com".to_vec(),
            b"http://127.0.0.1:8047/v1internal/////////////////////////".to_vec()
        ),
        (
            b"https://daily-cloudcode-pa.googleapis.com".to_vec(),
            b"http://127.0.0.1:8047/v1internal/////////".to_vec()
        ),
        (
            b"https://www.googleapis.com/oauth2/v2/userinfo".to_vec(),
            b"http://127.0.0.1:8047/userinfo///////////////".to_vec()
        ),
        (
            b"https://play.googleapis.com/log".to_vec(),
            b"http://127.0.0.1:8047/log//////".to_vec()
        ),
        (
            b"https://www.googleapis.com".to_vec(),
            b"http://127.0.0.1:8047/////".to_vec()
        ),
        (
            b"https://oauth2.googleapis.com".to_vec(),
            b"http://127.0.0.1:8047////////".to_vec()
        ),
    ];

    let mut patched_any = false;
    for (from, to) in &replacements {
        assert_eq!(from.len(), to.len(), "Replacement lengths must match exactly!");
        
        let mut i = 0;
        while i + from.len() <= content.len() {
            if content[i..i + from.len()] == **from {
                content[i..i + from.len()].copy_from_slice(to);
                patched_any = true;
                i += from.len();
            } else {
                i += 1;
            }
        }
    }

    if patched_any {
        std::fs::write(path, content)
            .map_err(|e| format!("Failed to write patched binary {:?}: {}", path, e))?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Text patching for standard unpacked main.js
fn patch_js_file(main_js_path: &std::path::Path) -> Result<bool, String> {
    let content = std::fs::read_to_string(main_js_path)
        .map_err(|e| format!("Failed to read main.js: {}", e))?;

    if content.contains("http://127.0.0.1:8047/v1internal") {
        eprintln!("[Client] main.js is already patched, skipping");
        return Ok(false);
    }

    let patched = content
        .replace(
            "https://cloudcode-pa.googleapis.com",
            "http://127.0.0.1:8047/v1internal",
        )
        .replace(
            "https://autopush-cloudcode-pa.sandbox.googleapis.com",
            "http://127.0.0.1:8047/v1internal",
        )
        .replace(
            "https://preprod-daily-cloudcode-pa.sandbox.googleapis.com",
            "http://127.0.0.1:8047/v1internal",
        )
        .replace(
            "https://daily-cloudcode-pa.googleapis.com",
            "http://127.0.0.1:8047/v1internal",
        )
        .replace(
            "https://www.googleapis.com/oauth2/v2/userinfo",
            "http://127.0.0.1:8047/userinfo",
        )
        .replace(
            "https://play.googleapis.com/log",
            "http://127.0.0.1:8047/telemetry-noop",
        )
        .replace(
            "https://www.googleapis.com",
            "http://127.0.0.1:8047/////",
        )
        .replace(
            "https://oauth2.googleapis.com",
            "http://127.0.0.1:8047////////",
        );

    std::fs::write(main_js_path, patched)
        .map_err(|e| format!("Failed to write patched main.js: {}", e))?;

    Ok(true)
}

/// Binary exact-length patching for packed app.asar
fn patch_asar_file(asar_path: &std::path::Path) -> Result<bool, String> {
    let mut content = std::fs::read(asar_path)
        .map_err(|e| format!("Failed to read app.asar: {}", e))?;

    let needle = b"http://127.0.0.1:8047/v1internal";
    if content.windows(needle.len()).any(|window| window == needle) {
        eprintln!("[Client] app.asar is already patched, skipping");
        return Ok(false);
    }

    let replacements = [
        (
            b"https://cloudcode-pa.googleapis.com".to_vec(),
            b"http://127.0.0.1:8047/v1internal///".to_vec()
        ),
        (
            b"https://autopush-cloudcode-pa.sandbox.googleapis.com".to_vec(),
            b"http://127.0.0.1:8047/v1internal////////////////////".to_vec()
        ),
        (
            b"https://preprod-daily-cloudcode-pa.sandbox.googleapis.com".to_vec(),
            b"http://127.0.0.1:8047/v1internal/////////////////////////".to_vec()
        ),
        (
            b"https://daily-cloudcode-pa.googleapis.com".to_vec(),
            b"http://127.0.0.1:8047/v1internal/////////".to_vec()
        ),
        (
            b"https://www.googleapis.com/oauth2/v2/userinfo".to_vec(),
            b"http://127.0.0.1:8047/userinfo///////////////".to_vec()
        ),
        (
            b"https://play.googleapis.com/log".to_vec(),
            b"http://127.0.0.1:8047/log//////".to_vec()
        ),
        (
            b"https://www.googleapis.com".to_vec(),
            b"http://127.0.0.1:8047/////".to_vec()
        ),
        (
            b"https://oauth2.googleapis.com".to_vec(),
            b"http://127.0.0.1:8047////////".to_vec()
        ),
    ];

    let mut patched_any = false;
    for (from, to) in &replacements {
        assert_eq!(from.len(), to.len(), "Replacement lengths must match exactly!");
        
        let mut i = 0;
        while i + from.len() <= content.len() {
            if content[i..i + from.len()] == **from {
                content[i..i + from.len()].copy_from_slice(to);
                patched_any = true;
                i += from.len();
            } else {
                i += 1;
            }
        }
    }

    if patched_any {
        std::fs::write(asar_path, content)
            .map_err(|e| format!("Failed to write patched app.asar: {}", e))?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Start Antigravity IDE
fn start_antigravity_ide(ide_type: &str, custom_exe_path: Option<&str>) -> Result<(), String> {
    let mut child_opt = None;

    if let Some(path) = custom_exe_path {
        if !path.is_empty() {
            child_opt = std::process::Command::new(path).spawn().ok();
        }
    }

    if child_opt.is_none() {
        #[cfg(target_os = "macos")]
        {
            let app_name = if ide_type == "Antigravity 2.0" { "Antigravity" } else { "Antigravity IDE" };
            child_opt = std::process::Command::new("open")
                .args(["-n", "-a", app_name])
                .spawn().ok();
        }
        #[cfg(target_os = "windows")]
        {
            let appdata = std::env::var("LOCALAPPDATA").unwrap_or_default();
            let path = if ide_type == "Antigravity 2.0" {
                format!(r"{}\Programs\antigravity\Antigravity.exe", appdata)
            } else {
                format!(r"{}\Programs\Antigravity IDE\Antigravity IDE.exe", appdata)
            };
            
            child_opt = std::process::Command::new(path).spawn().ok();
        }
        #[cfg(target_os = "linux")]
        {
            // Inject settings.json before launching
            let _ = crate::db::inject_to_settings(
                "http://127.0.0.1:8047/v1",
                ide_type
            );

            let bin_name = if ide_type == "Antigravity 2.0" { 
                "antigravity" 
            } else { 
                "/home/yaaaa/projects/Antigravity IDE Linux/antigravity-ide" 
            };
            child_opt = std::process::Command::new(bin_name)
                .env("DONT_PROMPT_WSL_INSTALL", "1")
                .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus")
                .spawn().ok();
        }
    }

    let ide_type_clone = ide_type.to_string();
    let session_id = PROXY_SESSION_ID.load(std::sync::atomic::Ordering::SeqCst);
    std::thread::spawn(move || {
        // Wait for the spawned process to exit to reap zombies
        if let Some(mut child) = child_opt {
            let _ = child.wait();
        }

        #[cfg(target_os = "windows")]
        let exe_name = if ide_type_clone == "Antigravity 2.0" { "Antigravity.exe" } else { "Antigravity IDE.exe" };
        #[cfg(target_os = "macos")]
        let exe_name = if ide_type_clone == "Antigravity 2.0" { "Antigravity" } else { "Antigravity IDE" };
        #[cfg(target_os = "linux")]
        let exe_name = if ide_type_clone == "Antigravity 2.0" { "antigravity" } else { "antigravity-ide" };

        loop {
            let is_running = {
                #[cfg(target_os = "windows")]
                {
                    if let Ok(output) = std::process::Command::new("tasklist").args(&["/FI", &format!("IMAGENAME eq {}", exe_name), "/NH"]).output() {
                        String::from_utf8_lossy(&output.stdout).to_lowercase().contains(&exe_name.to_lowercase())
                    } else {
                        true // fallback if tasklist fails
                    }
                }
                #[cfg(not(target_os = "windows"))]
                {
                    // Use pgrep, but exclude zombies (-z) if possible, or just standard match since we already reaped our child
                    if let Ok(output) = std::process::Command::new("pgrep").arg("-x").arg(exe_name).output() {
                        output.status.success()
                    } else {
                        true // fallback if pgrep fails
                    }
                }
            };

            // Also check if proxy was stopped manually by user (PROXY_RUNNING is false)
            let proxy_running = crate::PROXY_RUNNING.load(std::sync::atomic::Ordering::SeqCst);
            let current_session = crate::PROXY_SESSION_ID.load(std::sync::atomic::Ordering::SeqCst);

            if !is_running || !proxy_running || current_session != session_id {
                if proxy_running && current_session == session_id {
                    eprintln!("[Client] IDE process exited. Stopping proxy.");
                    crate::stop_proxy_for_session(session_id);
                }
                break;
            }

            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    });

    Ok(())
}




#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            inject_token_and_start_ide,
            stop_proxy,
            get_proxy_status
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

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
fn get_ide_main_js_path(ide_type: &str, custom_exe_path: Option<&str>) -> Option<std::path::PathBuf> {
    // If user provided a custom exe path, look for main.js relative to it
    if let Some(exe) = custom_exe_path {
        if !exe.is_empty() {
            let exe_path = std::path::Path::new(exe);
            if let Some(parent) = exe_path.parent() {
                let main_js = parent.join("resources").join("app").join("out").join("main.js");
                if main_js.exists() {
                    return Some(main_js);
                }
            }
        }
    }

    // Default paths by OS
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("LOCALAPPDATA").unwrap_or_default();
        let subfolder = if ide_type == "Antigravity 2.0" { "antigravity" } else { "Antigravity IDE" };
        let path = std::path::PathBuf::from(&appdata)
            .join("Programs")
            .join(subfolder)
            .join("resources")
            .join("app")
            .join("out")
            .join("main.js");
        if path.exists() {
            return Some(path);
        }
    }

    #[cfg(target_os = "macos")]
    {
        let app_name = if ide_type == "Antigravity 2.0" { "Antigravity" } else { "Antigravity IDE" };
        let path = std::path::PathBuf::from(format!(
            "/Applications/{}.app/Contents/Resources/app/out/main.js", app_name
        ));
        if path.exists() {
            return Some(path);
        }
    }

    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir().unwrap_or_default();
        // Try standard install location
        let bin_name = if ide_type == "Antigravity 2.0" { "antigravity" } else { "antigravity-ide" };
        let path = std::path::PathBuf::from(format!("/usr/share/{}/resources/app/out/main.js", bin_name));
        if path.exists() {
            return Some(path);
        }
        // Try snap / flatpak / custom
        let path2 = home.join(format!(".local/share/{}/resources/app/out/main.js", bin_name));
        if path2.exists() {
            return Some(path2);
        }
    }

    None
}

/// Patch the IDE's main.js to redirect hardcoded Google API URLs through local proxy.
/// Returns Ok(true) if patched, Ok(false) if already patched or file not found.
fn patch_ide_main_js(ide_type: &str, custom_exe_path: Option<&str>) -> Result<bool, String> {
    let main_js_path = match get_ide_main_js_path(ide_type, custom_exe_path) {
        Some(p) => p,
        None => {
            eprintln!("[Client] Could not find IDE main.js to patch");
            return Ok(false);
        }
    };

    eprintln!("[Client] Found IDE main.js at: {:?}", main_js_path);

    let content = std::fs::read_to_string(&main_js_path)
        .map_err(|e| format!("Failed to read main.js: {}", e))?;

    // Check if already patched (proxy URL already present)
    if content.contains("http://127.0.0.1:8047/v1internal") {
        eprintln!("[Client] main.js is already patched, skipping");
        return Ok(false);
    }

    // Replace hardcoded Google API endpoints with local proxy
    let patched = content
        // Main API endpoint (production)
        .replace(
            "https://cloudcode-pa.googleapis.com",
            "http://127.0.0.1:8047/v1internal",
        )
        // Staging/daily endpoints
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
        // OAuth userinfo endpoint
        .replace(
            "https://www.googleapis.com/oauth2/v2/userinfo",
            "http://127.0.0.1:8047/userinfo",
        )
        // Telemetry endpoint (blocks 401 crashes)
        .replace(
            "https://play.googleapis.com/log",
            "http://127.0.0.1:8047/telemetry-noop",
        );

    // Write back
    std::fs::write(&main_js_path, patched)
        .map_err(|e| format!("Failed to write patched main.js: {}", e))?;

    Ok(true)
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
                        String::from_utf8_lossy(&output.stdout).contains(exe_name)
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

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

    // 6c. Bypass macOS signature protection if running on macOS
    #[cfg(target_os = "macos")]
    {
        eprintln!("[Client] Running on macOS. Applying signature bypass for IDE...");
        let app_path = get_app_bundle_path(&ide_type, custom_exe_path.as_deref());
        bypass_macos_signature_protection(&app_path);
    }

    // 7. Start Antigravity IDE
    start_antigravity_ide(&ide_type, custom_exe_path.as_deref())?;

    Ok("Proxy started and IDE launched successfully".to_string())
}

#[tauri::command]
fn stop_proxy() -> Result<String, String> {
    stop_existing_proxy();
    restore_original_state_all();
    Ok("Proxy stopped and original state restored".to_string())
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
            .args(["-x", app_name])
            .output();
        let _ = std::process::Command::new("pkill")
            .args(["-x", "language_server"])
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

#[cfg(target_os = "macos")]
fn get_app_bundle_path(ide_type: &str, custom_exe_path: Option<&str>) -> std::path::PathBuf {
    if let Some(exe) = custom_exe_path {
        if !exe.is_empty() {
            let exe_path = std::path::Path::new(exe);
            for ancestor in exe_path.ancestors() {
                if ancestor.extension().and_then(|ext| ext.to_str()) == Some("app") {
                    return ancestor.to_path_buf();
                }
            }
        }
    }
    
    let app_name = if ide_type == "Antigravity 2.0" { "Antigravity" } else { "Antigravity IDE" };
    if let Some(home) = dirs::home_dir() {
        let user_path = home.join("Applications").join(format!("{}.app", app_name));
        if user_path.exists() {
            return user_path;
        }
    }
    std::path::PathBuf::from(format!("/Applications/{}.app", app_name))
}

#[cfg(target_os = "macos")]
fn bypass_macos_signature_protection(app_path: &std::path::Path) {
    eprintln!("[Client] Bypassing macOS signature protection for: {:?}", app_path);
    if !app_path.exists() {
        eprintln!("[Client] Warning: App bundle path does not exist, skipping signature bypass: {:?}", app_path);
        return;
    }
    
    // 1. Remove quarantine flag from the outer bundle itself (non-recursively) to bypass Gatekeeper.
    // This succeeds even without Full Disk Access as long as the user owns the bundle directory.
    let _ = std::process::Command::new("xattr")
        .arg("-d")
        .arg("com.apple.quarantine")
        .arg(app_path)
        .status();

    // Try recursive xattr cleanup but ignore errors if some system frameworks are read-only
    let _ = std::process::Command::new("xattr")
        .arg("-cr")
        .arg(app_path)
        .status();

    // 2. Sign all language_server binary variations if they exist
    let bin_dirs = [
        app_path.join("Contents/Resources/bin"),
        app_path.join("Contents/Resources/app/extensions/antigravity/bin"),
    ];

    let binary_names = [
        "language_server.exe",
        "language_server",
        "language_server_macos_arm",
        "language_server_macos_x64",
        "language_server_linux_x64",
    ];

    for bin_dir in &bin_dirs {
        if bin_dir.exists() {
            for name in &binary_names {
                let bin_path = bin_dir.join(name);
                if bin_path.exists() {
                    let _ = std::process::Command::new("codesign")
                        .args(&["--force", "--sign", "-", &bin_path.to_string_lossy()])
                        .status();
                }
            }
        }
    }

    // 3. Sign the main app bundle itself WITHOUT --deep to avoid "Operation not permitted"
    // on unmodified system frameworks (e.g. Squirrel.framework) while successfully re-signing app.asar and main executable.
    let status_codesign = std::process::Command::new("codesign")
        .args(&["--force", "--sign", "-", &app_path.to_string_lossy()])
        .status();
    match status_codesign {
        Ok(status) => eprintln!("[Client] codesign exited with status: {}", status),
        Err(e) => eprintln!("[Client] Failed to run codesign: {}", e),
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
        if let Some(home) = dirs::home_dir() {
            let path = home.join("Applications").join(format!("{}.app/Contents/Resources", app_name));
            if path.exists() {
                return Some(path);
            }
        }
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

    let mut patched = false;

    if app_asar.exists() {
        eprintln!("[Client] Found IDE app.asar at: {:?}", app_asar);
        if patch_asar_file(&app_asar)? {
            patched = true;
        }
        match patch_unpacked_js_files(&resources_dir) {
            Ok(p) => { if p { patched = true; } }
            Err(e) => eprintln!("[Client] Warning: Failed to patch unpacked JS files: {}", e),
        }
    } else if main_js.exists() {
        eprintln!("[Client] Found IDE main.js at: {:?}", main_js);
        if patch_js_file(&main_js)? {
            patched = true;
        }
    } else {
        eprintln!("[Client] Neither app.asar nor app/out/main.js found in resources");
    }

    Ok(patched)
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

    let bin_dirs = [
        resources_dir.join("bin"),
        resources_dir.join("app/extensions/antigravity/bin"),
    ];

    let binary_names = [
        "language_server.exe",
        "language_server",
        "language_server_macos_arm",
        "language_server_macos_x64",
        "language_server_linux_x64",
    ];

    let mut patched = false;

    for bin_dir in &bin_dirs {
        if bin_dir.exists() {
            for name in &binary_names {
                let bin_path = bin_dir.join(name);
                if bin_path.exists() {
                    eprintln!("[Client] Found language server binary at: {:?}", bin_path);
                    if patch_binary_file(&bin_path)? {
                        patched = true;
                    }
                }
            }
        }
    }

    Ok(patched)
}

/// Binary exact-length patching for language server binaries
fn patch_binary_file(path: &std::path::Path) -> Result<bool, String> {
    let mut backup_filename = path.file_name().unwrap_or_default().to_os_string();
    backup_filename.push(".bak");
    let backup_path = path.with_file_name(backup_filename);

    if backup_path.exists() {
        let _ = std::fs::remove_file(path);
        std::fs::copy(&backup_path, path)
            .map_err(|e| format!("Failed to restore clean binary for patching: {}", e))?;
    }

    let mut content = std::fs::read(path)
        .map_err(|e| format!("Failed to read binary file {:?}: {}", path, e))?;

    if !backup_path.exists() {
        std::fs::copy(path, &backup_path)
            .map_err(|e| format!("Failed to create backup copy of binary: {}", e))?;
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
        (
            b"https://generativelanguage.googleapis.com".to_vec(),
            b"http://127.0.0.1:8047/v1internal/////////".to_vec()
        ),
        (
            b"aicode.googleapis.com".to_vec(),
            b"aaaa.127.0.0.1.nip.io".to_vec()
        ),
    ];

    let mut patched_any = false;
    for (from, to) in &replacements {
        assert_eq!(from.len(), to.len(), "Replacement lengths must match exactly!");
        if from.is_empty() {
            continue;
        }
        let first_byte = from[0];
        let limit = content.len().saturating_sub(from.len());
        let mut i = 0;
        while i <= limit {
            if content[i] == first_byte {
                if content[i..i + from.len()] == **from {
                    content[i..i + from.len()].copy_from_slice(to);
                    patched_any = true;
                    i += from.len();
                    continue;
                }
            }
            i += 1;
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
    let mut backup_filename = main_js_path.file_name().unwrap_or_default().to_os_string();
    backup_filename.push(".bak");
    let backup_path = main_js_path.with_file_name(backup_filename);

    if backup_path.exists() {
        let _ = std::fs::remove_file(main_js_path);
        std::fs::copy(&backup_path, main_js_path)
            .map_err(|e| format!("Failed to restore clean main.js for patching: {}", e))?;
    }

    let content = std::fs::read_to_string(main_js_path)
        .map_err(|e| format!("Failed to read main.js: {}", e))?;

    if !backup_path.exists() {
        std::fs::copy(main_js_path, &backup_path)
            .map_err(|e| format!("Failed to create backup copy of main.js: {}", e))?;
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
        )
        .replace(
            "https://generativelanguage.googleapis.com",
            "http://127.0.0.1:8047/v1internal",
        )
        .replace(
            "aicode.googleapis.com",
            "aaaa.127.0.0.1.nip.io",
        );

    std::fs::write(main_js_path, patched)
        .map_err(|e| format!("Failed to write patched main.js: {}", e))?;

    Ok(true)
}

/// Binary exact-length patching for packed app.asar
fn patch_asar_file(asar_path: &std::path::Path) -> Result<bool, String> {
    let mut backup_filename = asar_path.file_name().unwrap_or_default().to_os_string();
    backup_filename.push(".bak");
    let backup_path = asar_path.with_file_name(backup_filename);

    if backup_path.exists() {
        let _ = std::fs::remove_file(asar_path);
        std::fs::copy(&backup_path, asar_path)
            .map_err(|e| format!("Failed to restore clean app.asar for patching: {}", e))?;
    }

    let mut content = std::fs::read(asar_path)
        .map_err(|e| format!("Failed to read app.asar: {}", e))?;

    if !backup_path.exists() {
        std::fs::copy(asar_path, &backup_path)
            .map_err(|e| format!("Failed to create backup copy of app.asar: {}", e))?;
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
        (
            b"https://generativelanguage.googleapis.com".to_vec(),
            b"http://127.0.0.1:8047/v1internal/////////".to_vec()
        ),
        (
            b"aicode.googleapis.com".to_vec(),
            b"aaaa.127.0.0.1.nip.io".to_vec()
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
            let app_path = get_app_bundle_path(ide_type, custom_exe_path);
            child_opt = std::process::Command::new("open")
                .args(["-n", &app_path.to_string_lossy()])
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
                    // Use pgrep with -f to match against the full command line path, which is much more robust
                    // since some Electron versions run with the generic process name 'Electron' but their path contains the app name.
                    if let Ok(output) = std::process::Command::new("pgrep").arg("-f").arg(exe_name).output() {
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

fn restore_files(ide_type: &str) -> Result<(), String> {
    let resources_dir = match get_ide_resources_path(ide_type, None) {
        Some(p) => p,
        None => {
            eprintln!("[Client] Could not find IDE resources directory to restore for {}", ide_type);
            return Ok(());
        }
    };

    let app_asar = resources_dir.join("app.asar");
    let main_js = resources_dir.join("app").join("out").join("main.js");

    let mut paths_to_restore = Vec::new();
    if app_asar.exists() {
        paths_to_restore.push(app_asar);
    }
    if main_js.exists() {
        paths_to_restore.push(main_js);
    }

    let unpacked_dir = resources_dir.join("app.asar.unpacked").join("node_modules").join("chrome-devtools-mcp").join("build").join("src");
    if unpacked_dir.exists() {
        let files = [
            unpacked_dir.join("telemetry").join("watchdog").join("ClearcutSender.js"),
            unpacked_dir.join("third_party").join("index.js"),
            unpacked_dir.join("third_party").join("lighthouse-devtools-mcp-bundle.js"),
            unpacked_dir.join("tools").join("performance.js"),
        ];
        for f in &files {
            if f.exists() {
                paths_to_restore.push(f.clone());
            }
        }
    }

    let bin_dirs = [
        resources_dir.join("bin"),
        resources_dir.join("app/extensions/antigravity/bin"),
    ];

    let binary_names = [
        "language_server.exe",
        "language_server",
        "language_server_macos_arm",
        "language_server_macos_x64",
        "language_server_linux_x64",
    ];

    for bin_dir in &bin_dirs {
        if bin_dir.exists() {
            for name in &binary_names {
                let bin_path = bin_dir.join(name);
                if bin_path.exists() {
                    paths_to_restore.push(bin_path);
                }
            }
        }
    }

    for path in paths_to_restore {
        let mut backup_filename = path.file_name().unwrap_or_default().to_os_string();
        backup_filename.push(".bak");
        let backup_path = path.with_file_name(backup_filename);

        if backup_path.exists() {
            eprintln!("[Client] Restoring original file from backup: {:?}", backup_path);
            let _ = std::fs::remove_file(&path);
            if let Err(e) = std::fs::rename(&backup_path, &path) {
                eprintln!("[Client] Warning: Failed to rename backup file {:?} to {:?}: {}. Trying copy...", backup_path, path, e);
                if let Err(copy_err) = std::fs::copy(&backup_path, &path) {
                    eprintln!("[Client] Error: Failed to copy backup back to {:?}: {}", path, copy_err);
                } else {
                    let _ = std::fs::remove_file(&backup_path);
                }
            }
        }
    }

    Ok(())
}

fn patch_unpacked_js_files(resources_dir: &std::path::Path) -> Result<bool, String> {
    let unpacked_dir = resources_dir.join("app.asar.unpacked").join("node_modules").join("chrome-devtools-mcp").join("build").join("src");
    if !unpacked_dir.exists() {
        return Ok(false);
    }

    let files_to_patch = [
        unpacked_dir.join("telemetry").join("watchdog").join("ClearcutSender.js"),
        unpacked_dir.join("third_party").join("index.js"),
        unpacked_dir.join("third_party").join("lighthouse-devtools-mcp-bundle.js"),
        unpacked_dir.join("tools").join("performance.js"),
    ];

    let mut patched_any = false;
    for path in &files_to_patch {
        if path.exists() {
            let mut backup_filename = path.file_name().unwrap_or_default().to_os_string();
            backup_filename.push(".bak");
            let backup_path = path.with_file_name(backup_filename);

            if backup_path.exists() {
                let _ = std::fs::remove_file(path);
                std::fs::copy(&backup_path, path)
                    .map_err(|e| format!("Failed to restore clean unpacked file {:?}: {}", path, e))?;
            }

            let content = std::fs::read_to_string(path)
                .map_err(|e| format!("Failed to read unpacked JS file {:?}: {}", path, e))?;

            if !backup_path.exists() {
                std::fs::copy(path, &backup_path)
                    .map_err(|e| format!("Failed to create backup copy of unpacked JS: {}", e))?;
            }

            let patched = content
                .replace("https://play.googleapis.com/log", "http://127.0.0.1:8047/telemetry-noop")
                .replace("https://chromeuxreport.googleapis.com", "http://127.0.0.1:8047/telemetry-noop");

            std::fs::write(path, patched)
                .map_err(|e| format!("Failed to write patched unpacked JS file {:?}: {}", path, e))?;
            patched_any = true;
            eprintln!("[Client] Patched unpacked JS file: {:?}", path);
        }
    }

    Ok(patched_any)
}

fn restore_original_state_all() {
    eprintln!("[Client] Restoring original state (clearing settings, restoring files, clearing keyring)...");
    
    // Kill processes to ensure we can modify their files
    kill_running_antigravity("Antigravity IDE");
    kill_running_antigravity("Antigravity 2.0");

    if let Err(e) = db::clear_keyring_credentials() {
        eprintln!("[Client] Warning: Failed to clear keyring: {}", e);
    }

    for ide in &["Antigravity IDE", "Antigravity 2.0"] {
        if let Err(e) = db::clear_proxy_settings(ide) {
            eprintln!("[Client] Warning: Failed to clear proxy settings for {}: {}", ide, e);
        }
        if let Err(e) = restore_files(ide) {
            eprintln!("[Client] Warning: Failed to restore files for {}: {}", ide, e);
        }
    }
}

#[tauri::command]
async fn install_client_update(app_handle: tauri::AppHandle, download_url: String) -> Result<(), String> {
    eprintln!("[Client] Starting update installation. Download URL: {}", download_url);
    
    let temp_dir = std::env::temp_dir();
    #[cfg(target_os = "windows")]
    let msi_path = temp_dir.join("AntigravityClient_setup.msi");
    #[cfg(target_os = "macos")]
    let msi_path = temp_dir.join("AntigravityClient_setup.dmg");
    #[cfg(target_os = "linux")]
    let msi_path = temp_dir.join("AntigravityClient_setup.AppImage");
    
    // 1. Download the setup file
    let client = reqwest::Client::new();
    let resp = client.get(&download_url)
        .send()
        .await
        .map_err(|e| format!("Failed to send download request: {}", e))?;
        
    if !resp.status().is_success() {
        return Err(format!("Download request failed with status: {}", resp.status()));
    }
    
    // Write body to file
    let bytes = resp.bytes().await.map_err(|e| format!("Failed to read response bytes: {}", e))?;
    std::fs::write(&msi_path, bytes).map_err(|e| format!("Failed to write setup file: {}", e))?;
    
    eprintln!("[Client] Setup file downloaded to {:?}", msi_path);
    
    // 2. Get current executable path
    let current_exe = std::env::current_exe()
        .map_err(|e| format!("Failed to get current executable path: {}", e))?;
        
    eprintln!("[Client] Current executable path: {:?}", current_exe);
    
    // 3. Spawn background script and exit based on OS
    #[cfg(target_os = "windows")]
    {
        let ps_command = format!(
            "Start-Sleep -Seconds 2; Start-Process msiexec.exe -ArgumentList '/i', '{}', '/passive', '/norestart' -Wait; Start-Process '{}'",
            msi_path.to_string_lossy(),
            current_exe.to_string_lossy()
        );
        
        eprintln!("[Client] Spawning PowerShell command: {}", ps_command);
        
        std::process::Command::new("powershell")
            .arg("-NoProfile")
            .arg("-WindowStyle")
            .arg("Hidden")
            .arg("-Command")
            .arg(ps_command)
            .spawn()
            .map_err(|e| format!("Failed to spawn PowerShell installer script: {}", e))?;
            
        // Gracefully exit current Tauri app
        app_handle.exit(0);
    }
    
    #[cfg(target_os = "macos")]
    {
        let parent_dir = current_exe.parent().ok_or("No parent directory found")?;
        let sh_command = format!(
            "sleep 2; hdiutil attach -nobrowse -mountpoint /tmp/ag_mount '{}'; cp -R '/tmp/ag_mount/Antigravity Client.app' '{}'; hdiutil detach /tmp/ag_mount; open -n '{}'",
            msi_path.to_string_lossy(),
            parent_dir.to_string_lossy(),
            current_exe.to_string_lossy()
        );
        
        eprintln!("[Client] Spawning macOS shell command: {}", sh_command);
        
        std::process::Command::new("sh")
            .arg("-c")
            .arg(sh_command)
            .spawn()
            .map_err(|e| format!("Failed to spawn macOS shell installer: {}", e))?;
            
        app_handle.exit(0);
    }
    
    #[cfg(target_os = "linux")]
    {
        let sh_command = format!(
            "sleep 2; chmod +x '{}'; mv '{}' '{}'; '{}' &",
            msi_path.to_string_lossy(),
            msi_path.to_string_lossy(),
            current_exe.to_string_lossy(),
            current_exe.to_string_lossy()
        );
        
        eprintln!("[Client] Spawning Linux shell command: {}", sh_command);
        
        std::process::Command::new("sh")
            .arg("-c")
            .arg(sh_command)
            .spawn()
            .map_err(|e| format!("Failed to spawn Linux shell installer: {}", e))?;
            
        app_handle.exit(0);
    }
    
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|_app| {
            eprintln!("[Client] Application startup. Running cleanup to ensure clean state...");
            restore_original_state_all();
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            inject_token_and_start_ide,
            stop_proxy,
            get_proxy_status,
            install_client_update
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, event| {
            if let tauri::RunEvent::Exit = event {
                eprintln!("[Client] Tauri application is exiting. Running final cleanup...");
                restore_original_state_all();
            }
        });
}

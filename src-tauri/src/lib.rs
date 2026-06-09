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

    // 2. Stop any existing proxy
    stop_existing_proxy();

    // 3. Small delay to ensure processes are fully terminated
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // 4. Start the local proxy on port 8047
    // This proxy intercepts all IDE requests and forwards them to the Manager
    eprintln!("[Client] Starting local proxy on :8047 -> {}", proxy_url);

    // Strip /v1 suffix if present since the proxy needs the base URL
    let base_url = proxy_url.trim_end_matches("/v1").to_string();

    let config = local_proxy::ProxyConfig {
        listen_port: 8047,
        target_url: base_url,
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
    crate::tunnel::start_tunnel_worker(proxy_url.clone(), token.clone()).await;

    // 6. Start Antigravity IDE
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

/// Start Antigravity IDE
fn start_antigravity_ide(ide_type: &str, custom_exe_path: Option<&str>) -> Result<(), String> {
    if let Some(path) = custom_exe_path {
        if !path.is_empty() {
            let _ = std::process::Command::new(path).spawn();
            return Ok(());
        }
    }

    #[cfg(target_os = "macos")]
    {
        let app_name = if ide_type == "Antigravity 2.0" { "Antigravity" } else { "Antigravity IDE" };
        let _ = std::process::Command::new("open")
            .args(["-a", app_name])
            .spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("LOCALAPPDATA").unwrap_or_default();
        let path = if ide_type == "Antigravity 2.0" {
            format!("{}\\Programs\\antigravity\\Antigravity.exe", appdata)
        } else {
            format!("{}\\Programs\\Antigravity IDE\\Antigravity IDE.exe", appdata)
        };
        
        let _ = std::process::Command::new(path).spawn();
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
        let _ = std::process::Command::new(bin_name)
            .env("DONT_PROMPT_WSL_INSTALL", "1")
            .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus")
            .spawn();
    }

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

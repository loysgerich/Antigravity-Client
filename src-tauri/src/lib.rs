pub mod db;
pub mod local_proxy;
pub mod utils {
    pub mod protobuf;
}

use std::sync::Mutex;
use tokio::sync::watch;

// Global proxy shutdown handle
static PROXY_SHUTDOWN: Mutex<Option<watch::Sender<bool>>> = Mutex::new(None);
static PROXY_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[tauri::command]
async fn inject_token_and_start_ide(token: String, proxy_url: String) -> Result<String, String> {
    // 1. Kill any running Antigravity IDE first
    kill_running_antigravity();

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

    // 5. Write a fake ya29.* token to keyring so IDE's internal auth check passes.
    // The actual authentication is handled by our proxy → Manager pipeline.
    eprintln!("[Client] Injecting proxy token into IDE keyring...");
    db::inject_real_token(
        "ya29.proxy_managed_token_do_not_use",
        "proxy_managed_refresh_token",
        4070908800i64,
        "http://127.0.0.1:8047/v1", // Route IDE traffic to our local proxy
    )?;

    eprintln!("[Client] Token injected successfully");

    // 6. Start Antigravity IDE
    start_antigravity_ide()?;

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

/// Kill any running Antigravity IDE processes
fn kill_running_antigravity() {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("pkill")
            .args(["-f", "Antigravity"])
            .output();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/IM", "Antigravity IDE.exe"])
            .output();
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/IM", "Antigravity.exe"])
            .output();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("pkill")
            .args(["-x", "antigravity"])
            .output();
        let _ = std::process::Command::new("pkill")
            .args(["-f", "language_server"])
            .output();
    }
}

/// Start Antigravity IDE
fn start_antigravity_ide() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .args(["-a", "Antigravity IDE"])
            .spawn()
            .or_else(|_| {
                std::process::Command::new("open")
                    .args(["-a", "Antigravity"])
                    .spawn()
            });
    }
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("LOCALAPPDATA").unwrap_or_default();
        let path_ide = format!("{}\\Programs\\Antigravity IDE\\Antigravity IDE.exe", appdata);
        if std::path::Path::new(&path_ide).exists() {
            let _ = std::process::Command::new(path_ide).spawn();
        } else {
            let path_old = format!("{}\\Programs\\Antigravity\\Antigravity.exe", appdata);
            let _ = std::process::Command::new(path_old).spawn();
        }
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("antigravity-ide")
            .env("DONT_PROMPT_WSL_INSTALL", "1")
            .spawn()
            .or_else(|_| {
                std::process::Command::new("antigravity")
                    .env("DONT_PROMPT_WSL_INSTALL", "1")
                    .spawn()
            });
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

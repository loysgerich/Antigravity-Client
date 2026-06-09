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

            if !is_running || !proxy_running {
                if proxy_running {
                    eprintln!("[Client] IDE process exited. Stopping proxy.");
                    crate::stop_existing_proxy();
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

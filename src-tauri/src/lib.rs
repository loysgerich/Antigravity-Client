pub mod db;
pub mod utils {
    pub mod protobuf;
}

#[tauri::command]
fn inject_token_and_start_ide(token: &str, proxy_url: &str) -> Result<String, String> {
    // 1. Kill any running Antigravity IDE first
    kill_running_antigravity();
    
    // 2. Small delay to ensure the process is fully terminated
    std::thread::sleep(std::time::Duration::from_millis(500));
    
    // 3. Fetch real OAuth token from Manager API
    let real_token = fetch_real_token_from_manager(proxy_url)
        .unwrap_or_else(|e| {
            eprintln!("[Client] Failed to get real token from Manager: {}, falling back to provided token", e);
            None
        });
    
    let (access_token, refresh_token, expiry) = if let Some((at, rt, exp)) = real_token {
        eprintln!("[Client] Got real OAuth token from Manager (ya29...)");
        (at, rt, exp)
    } else {
        eprintln!("[Client] Using provided token as-is");
        (token.to_string(), token.to_string(), 4070908800i64)
    };
    
    // 4. Write token to keyring and database
    db::inject_real_token(&access_token, &refresh_token, expiry, proxy_url)?;
    
    // 5. Start Antigravity IDE
    start_antigravity_ide(proxy_url)?;
    
    Ok("Injection successful".to_string())
}

/// Fetch the real OAuth access_token from Manager's account API
fn fetch_real_token_from_manager(proxy_url: &str) -> Result<Option<(String, String, i64)>, String> {
    // Try to read from Manager's account file directly
    let home = dirs::home_dir().ok_or("No home dir")?;
    let accounts_dir = home.join(".antigravity_tools/accounts");
    
    if !accounts_dir.exists() {
        return Ok(None);
    }
    
    // Find account files
    let entries = std::fs::read_dir(&accounts_dir)
        .map_err(|e| format!("Failed to read accounts dir: {}", e))?;
    
    for entry in entries {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let path = entry.path();
        if path.extension().map_or(false, |e| e == "json") {
            let content = std::fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
            let account: serde_json::Value = serde_json::from_str(&content)
                .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))?;
            
            if let Some(token_obj) = account.get("token") {
                let access_token = token_obj.get("access_token")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let refresh_token = token_obj.get("refresh_token")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let expiry = token_obj.get("expiry_timestamp")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(4070908800);
                
                if !access_token.is_empty() && access_token.starts_with("ya29.") {
                    return Ok(Some((access_token, refresh_token, expiry)));
                }
            }
        }
    }
    
    Ok(None)
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
        // Use pkill to terminate specific IDE processes, not all processes containing "antigravity"
        let _ = std::process::Command::new("pkill")
            .args(["-x", "antigravity"])
            .output();
        let _ = std::process::Command::new("pkill")
            .args(["-f", "language_server"])
            .output();
    }
}

/// Start Antigravity IDE
fn start_antigravity_ide(proxy_url: &str) -> Result<(), String> {
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
            .spawn()
            .or_else(|_| std::process::Command::new("antigravity").spawn());
    }
    
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![inject_token_and_start_ide])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

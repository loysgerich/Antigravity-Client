use rusqlite::Connection;
use std::path::PathBuf;
use crate::utils::protobuf;

#[cfg(target_os = "linux")]
fn get_wsl_windows_appdata() -> Option<PathBuf> {
    if std::env::var("WSL_DISTRO_NAME").is_err() {
        return None;
    }
    let output = std::process::Command::new("cmd.exe")
        .args(&["/c", "echo %APPDATA%"])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let appdata = stdout.trim();
    if appdata.is_empty() || appdata == "%APPDATA%" {
        return None;
    }
    let wslpath_output = std::process::Command::new("wslpath")
        .args(&["-u", appdata])
        .output()
        .ok()?;
    let wsl_path = String::from_utf8_lossy(&wslpath_output.stdout);
    let wsl_path = wsl_path.trim();
    if wsl_path.is_empty() {
        return None;
    }
    Some(PathBuf::from(wsl_path))
}

pub fn get_db_path(ide_type: &str, custom_db_path: Option<&str>) -> Result<PathBuf, String> {
    if let Some(path) = custom_db_path {
        if !path.is_empty() {
            let pb = PathBuf::from(path);
            if pb.exists() {
                return Ok(pb);
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().ok_or("Failed to get home directory")?;
        let subfolder = if ide_type == "Antigravity 2.0" { "Antigravity" } else { "Antigravity IDE" };
        let new_path = home.join(format!("Library/Application Support/{}/User/globalStorage/state.vscdb", subfolder));
        if new_path.exists() {
            return Ok(new_path);
        }
        Ok(home.join("Library/Application Support/Antigravity/User/globalStorage/state.vscdb"))
    }
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").map_err(|_| "Failed to get APPDATA environment variable".to_string())?;
        let subfolder = if ide_type == "Antigravity 2.0" { "antigravity" } else { "Antigravity IDE" };
        let new_path = PathBuf::from(&appdata).join(format!("{}\\User\\globalStorage\\state.vscdb", subfolder));
        if new_path.exists() {
            return Ok(new_path);
        }
        Ok(PathBuf::from(appdata).join("Antigravity\\User\\globalStorage\\state.vscdb"))
    }
    #[cfg(target_os = "linux")]
    {
        let subfolder = if ide_type == "Antigravity 2.0" { "Antigravity" } else { "Antigravity IDE" };

        if ide_type == "Antigravity IDE" {
            if let Some(wsl_appdata) = get_wsl_windows_appdata() {
                let win_path = wsl_appdata.join(subfolder).join("User/globalStorage/state.vscdb");
                if win_path.exists() {
                    return Ok(win_path);
                }
            }
        }

        let home = dirs::home_dir().ok_or("Failed to get home directory")?;
        let new_path = home.join(format!(".config/{}/User/globalStorage/state.vscdb", subfolder));
        if new_path.exists() {
            return Ok(new_path);
        }
        Ok(home.join(".config/Antigravity/User/globalStorage/state.vscdb"))
    }
}

/// Inject token using BOTH methods for maximum compatibility:
/// 1. System Keyring (secret-tool on Linux, security on macOS, cmdkey on Windows)
///    — required for Antigravity >= 2.0.0
/// 2. SQLite database injection — required for Antigravity < 2.0.0
pub fn inject_token_and_proxy(token: &str, proxy_url: &str, ide_type: &str, custom_db_path: Option<&str>) -> Result<String, String> {
    let email = "proxy_user@antigravity";
    let expiry: i64 = 4070908800; // 2099 year

    // ===== Method 1: System Keyring (for Antigravity >= 2.0.0) =====
    let keyring_result = write_to_system_keyring(token, expiry);
    match &keyring_result {
        Ok(_) => eprintln!("[Client] Successfully wrote token to system keyring"),
        Err(e) => eprintln!("[Client] Keyring write failed (may be OK for older versions): {}", e),
    }

    // ===== Method 2: SQLite database injection (for Antigravity < 2.0.0) =====
    let db_result = inject_to_sqlite(token, proxy_url, email, expiry, ide_type, custom_db_path);
    match &db_result {
        Ok(_) => eprintln!("[Client] Successfully wrote token to SQLite database"),
        Err(e) => eprintln!("[Client] SQLite write failed (may be OK for newer versions): {}", e),
    }

    // Success if either method worked
    if keyring_result.is_ok() || db_result.is_ok() {
        Ok("Token injection successful".to_string())
    } else {
        Err(format!(
            "Both injection methods failed. Keyring: {:?}, SQLite: {:?}",
            keyring_result.err(),
            db_result.err()
        ))
    }
}

/// Inject a real OAuth token (with separate access/refresh tokens) for IDE v2.0+
pub fn inject_real_token(access_token: &str, refresh_token: &str, expiry: i64, proxy_url: &str, ide_type: &str, custom_db_path: Option<&str>) -> Result<String, String> {
    let email = "proxy_user@antigravity";

    // ===== Method 1: System Keyring (for Antigravity >= 2.0.0) =====
    let keyring_result = write_real_token_to_keyring(access_token, refresh_token, expiry);
    match &keyring_result {
        Ok(_) => eprintln!("[Client] Successfully wrote real token to system keyring"),
        Err(e) => eprintln!("[Client] Keyring write failed: {}", e),
    }

    // ===== Method 2: SQLite (for older versions) =====
    let db_result = inject_to_sqlite(access_token, proxy_url, email, expiry, ide_type, custom_db_path);
    match &db_result {
        Ok(_) => eprintln!("[Client] Successfully wrote token to SQLite database"),
        Err(e) => eprintln!("[Client] SQLite write failed: {}", e),
    }

    // ===== Method 3: settings.json =====
    let settings_result = inject_to_settings(proxy_url, ide_type);
    match &settings_result {
        Ok(_) => eprintln!("[Client] Successfully wrote proxyBaseUrl to settings.json"),
        Err(e) => eprintln!("[Client] settings.json write failed: {}", e),
    }

    if keyring_result.is_ok() || db_result.is_ok() {
        Ok("Token injection successful".to_string())
    } else {
        Err(format!(
            "Both injection methods failed. Keyring: {:?}, SQLite: {:?}",
            keyring_result.err(),
            db_result.err()
        ))
    }
}

/// Inject proxyBaseUrl into settings.json
pub fn inject_to_settings(proxy_url: &str, ide_type: &str) -> Result<(), String> {
    let mut settings_path = std::path::PathBuf::new();

    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().ok_or("Failed to get home directory")?;
        let subfolder = if ide_type == "Antigravity 2.0" { "Antigravity" } else { "Antigravity IDE" };
        settings_path = home.join(format!("Library/Application Support/{}/User/settings.json", subfolder));
    }

    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").map_err(|_| "Failed to get APPDATA environment variable".to_string())?;
        let subfolder = if ide_type == "Antigravity 2.0" { "antigravity" } else { "Antigravity IDE" };
        settings_path = std::path::PathBuf::from(appdata).join(format!("{}\\User\\settings.json", subfolder));
    }

    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir().ok_or("Failed to get home directory")?;
        let subfolder = if ide_type == "Antigravity 2.0" { "Antigravity" } else { "Antigravity IDE" };
        settings_path = home.join(format!(".config/{}/User/settings.json", subfolder));
    }

    let mut settings: serde_json::Value = if settings_path.exists() {
        let content = std::fs::read_to_string(&settings_path).unwrap_or_default();
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        if let Some(parent) = settings_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        serde_json::json!({})
    };

    if let Some(obj) = settings.as_object_mut() {
        // Force the URL to include /v1 since the IDE proxy configuration usually requires it
        let final_url = if proxy_url.ends_with("/v1") { proxy_url.to_string() } else { format!("{}/v1", proxy_url) };
        obj.insert("antigravity.proxyBaseUrl".to_string(), serde_json::json!(final_url));
        
        // Disable telemetry to prevent 401 crashes with play.googleapis.com
        obj.insert("telemetry.telemetryLevel".to_string(), serde_json::json!("off"));
        obj.insert("telemetry.enableCrashReporter".to_string(), serde_json::json!(false));
        obj.insert("telemetry.enableTelemetry".to_string(), serde_json::json!(false));
    }

    let content = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    std::fs::write(&settings_path, content).map_err(|e| format!("Failed to write to {:?}: {}", settings_path, e))?;

    Ok(())
}

/// Write a real OAuth token to system keyring as raw JSON
fn write_real_token_to_keyring(access_token: &str, refresh_token: &str, expiry: i64) -> Result<(), String> {
    use std::process::Command;

    let expiry_str = format_timestamp_rfc3339(expiry);

    let payload = serde_json::json!({
        "token": {
            "access_token": access_token,
            "token_type": "Bearer",
            "refresh_token": refresh_token,
            "expiry": expiry_str
        },
        "auth_method": "consumer"
    });

    let payload_json = serde_json::to_string(&payload)
        .map_err(|e| format!("Failed to serialize keyring JSON: {}", e))?;

    #[cfg(target_os = "linux")]
    {
        use std::io::Write;

        // Delete old credential
        let _ = Command::new("secret-tool")
            .args(["clear", "service", "gemini", "username", "antigravity"])
            .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus")
            .output();

        // Write new credential
        let mut child = Command::new("secret-tool")
            .args(["store", "--label=gemini", "service", "gemini", "username", "antigravity"])
            .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to spawn secret-tool: {}", e))?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(payload_json.as_bytes())
                .map_err(|e| format!("Failed to write to secret-tool stdin: {}", e))?;
        }

        let output = child.wait_with_output()
            .map_err(|e| format!("Failed to wait for secret-tool: {}", e))?;

        if !output.status.success() {
            let err_msg = String::from_utf8_lossy(&output.stderr);
            return Err(format!("secret-tool failed: {}", err_msg.trim()));
        }

        if let Ok(_) = std::env::var("WSL_DISTRO_NAME") {
            let _ = Command::new("cmdkey.exe")
                .args(["/delete:gemini:antigravity"])
                .output();
            let _ = Command::new("cmdkey.exe")
                .args(["/generic:gemini:antigravity", "/user:antigravity", &format!("/pass:{}", payload_json)])
                .output();
            eprintln!("[Client] Also injected credential into Windows Credential Manager via WSL");
        }
    }

    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("security")
            .args(["delete-generic-password", "-s", "gemini", "-a", "antigravity"])
            .output();
        let output = Command::new("security")
            .args(["add-generic-password", "-s", "gemini", "-a", "antigravity", "-w", &payload_json, "-A"])
            .output()
            .map_err(|e| format!("Failed to execute security command: {}", e))?;
        if !output.status.success() {
            let err_msg = String::from_utf8_lossy(&output.stderr);
            return Err(format!("macOS security command failed: {}", err_msg.trim()));
        }
    }

    #[cfg(target_os = "windows")]
    {
        use std::ptr;
        use std::os::windows::ffi::OsStrExt;

        #[repr(C)]
        struct FILETIME {
            dw_low_date_time: u32,
            dw_high_date_time: u32,
        }

        #[repr(C)]
        struct CREDENTIALW {
            flags: u32,
            cred_type: u32,
            target_name: *const u16,
            comment: *const u16,
            last_written: FILETIME,
            credential_blob_size: u32,
            credential_blob: *const u8,
            persist: u32,
            attribute_count: u32,
            attributes: *const std::ffi::c_void,
            target_alias: *const u16,
            user_name: *const u16,
        }

        #[link(name = "advapi32")]
        extern "system" {
            fn CredWriteW(credential: *const CREDENTIALW, flags: u32) -> i32;
            fn CredDeleteW(target_name: *const u16, type_: u32, flags: u32) -> i32;
        }

        let target = "gemini:antigravity";
        let user = "antigravity";
        // Language server expects raw UTF-8 JSON string on Windows
        let secret_bytes = payload_json.as_bytes();
        let secret_size = secret_bytes.len() as u32;
        let secret_ptr = secret_bytes.as_ptr();

        let target_wide: Vec<u16> = std::ffi::OsStr::new(target)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let user_wide: Vec<u16> = std::ffi::OsStr::new(user)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let cred = CREDENTIALW {
            flags: 0,
            cred_type: 1, // CRED_TYPE_GENERIC
            target_name: target_wide.as_ptr(),
            comment: ptr::null(),
            last_written: FILETIME { dw_low_date_time: 0, dw_high_date_time: 0 },
            credential_blob_size: secret_size,
            credential_blob: secret_ptr,
            persist: 2, // CRED_PERSIST_LOCAL_MACHINE
            attribute_count: 0,
            attributes: ptr::null(),
            target_alias: ptr::null(),
            user_name: user_wide.as_ptr(),
        };

        unsafe {
            let _ = CredDeleteW(target_wide.as_ptr(), 1, 0);
            let res = CredWriteW(&cred, 0);
            if res == 0 {
                let err = std::io::Error::last_os_error();
                return Err(format!("Windows CredWriteW failed: {}", err));
            }
        }
    }

    Ok(())
}

/// Write token to system credential store (matches Manager's write_to_system_keyring)
fn write_to_system_keyring(token: &str, expiry: i64) -> Result<(), String> {
    use base64::{engine::general_purpose, Engine as _};
    use std::process::Command;

    // Build the exact same JSON payload format as the Manager
    let expiry_secs = expiry;
    // Format expiry as RFC3339 with microseconds
    // 4070908800 = 2099-01-01T00:00:00.000000Z
    let expiry_str = format_timestamp_rfc3339(expiry_secs);

    let payload = serde_json::json!({
        "token": {
            "access_token": token,
            "token_type": "Bearer",
            "refresh_token": token,
            "expiry": expiry_str
        },
        "auth_method": "consumer"
    });

    let payload_json = serde_json::to_string(&payload)
        .map_err(|e| format!("Failed to serialize keyring JSON: {}", e))?;

    // Language server v2.x expects raw JSON in keyring (not go-keyring-base64 encoded)
    let full_keyring_value = payload_json;

    #[cfg(target_os = "linux")]
    {
        use std::io::Write;

        // Ensure gnome-keyring-daemon is running (critical for WSL)
        // This uses a wrapper script that starts dbus + keyring if needed
        let _ = Command::new("bash")
            .arg("-c")
            .arg("if [ -z \"$DBUS_SESSION_BUS_ADDRESS\" ]; then eval $(dbus-launch --sh-syntax); export DBUS_SESSION_BUS_ADDRESS; fi; killall -0 gnome-keyring-daemon || (rm -rf ~/.local/share/keyrings && mkdir -p ~/.local/share/keyrings && eval $(echo '' | gnome-keyring-daemon --unlock --components=secrets))")
            .output();
        
        // Delete old credential (ignore errors)
        let _ = Command::new("secret-tool")
            .args(["clear", "service", "gemini", "username", "antigravity"])
            .output();

        // Write new credential
        let mut child = Command::new("secret-tool")
            .args(["store", "--label=gemini", "service", "gemini", "username", "antigravity"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to spawn secret-tool: {}", e))?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(full_keyring_value.as_bytes())
                .map_err(|e| format!("Failed to write to secret-tool stdin: {}", e))?;
        }

        let output = child.wait_with_output()
            .map_err(|e| format!("Failed to wait for secret-tool: {}", e))?;

        if !output.status.success() {
            let err_msg = String::from_utf8_lossy(&output.stderr);
            return Err(format!("secret-tool failed: {}", err_msg.trim()));
        }

        if let Ok(_) = std::env::var("WSL_DISTRO_NAME") {
            let _ = Command::new("cmdkey.exe")
                .args(["/delete:gemini:antigravity"])
                .output();
            let _ = Command::new("cmdkey.exe")
                .args(["/generic:gemini:antigravity", "/user:antigravity", &format!("/pass:{}", full_keyring_value)])
                .output();
            eprintln!("[Client] Also injected old-format credential into Windows Credential Manager via WSL");
        }
    }

    #[cfg(target_os = "macos")]
    {
        // Delete old
        let _ = Command::new("security")
            .args(["delete-generic-password", "-s", "gemini", "-a", "antigravity"])
            .output();

        // Write new
        let output = Command::new("security")
            .args(["add-generic-password", "-s", "gemini", "-a", "antigravity", "-w", &full_keyring_value, "-A"])
            .output()
            .map_err(|e| format!("Failed to execute security command: {}", e))?;

        if !output.status.success() {
            let err_msg = String::from_utf8_lossy(&output.stderr);
            return Err(format!("macOS security command failed: {}", err_msg.trim()));
        }
    }

    #[cfg(target_os = "windows")]
    {
        use std::ptr;
        use std::os::windows::ffi::OsStrExt;

        #[repr(C)]
        struct FILETIME {
            dw_low_date_time: u32,
            dw_high_date_time: u32,
        }

        #[repr(C)]
        struct CREDENTIALW {
            flags: u32,
            cred_type: u32,
            target_name: *const u16,
            comment: *const u16,
            last_written: FILETIME,
            credential_blob_size: u32,
            credential_blob: *const u8,
            persist: u32,
            attribute_count: u32,
            attributes: *const std::ffi::c_void,
            target_alias: *const u16,
            user_name: *const u16,
        }

        #[link(name = "advapi32")]
        extern "system" {
            fn CredWriteW(credential: *const CREDENTIALW, flags: u32) -> i32;
            fn CredDeleteW(target_name: *const u16, type_: u32, flags: u32) -> i32;
        }

        let target = "gemini:antigravity";
        let user = "antigravity";
        // Language server expects raw UTF-8 JSON string on Windows
        let secret_bytes = full_keyring_value.as_bytes();
        let secret_size = secret_bytes.len() as u32;
        let secret_ptr = secret_bytes.as_ptr();

        let target_wide: Vec<u16> = std::ffi::OsStr::new(target)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let user_wide: Vec<u16> = std::ffi::OsStr::new(user)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let cred = CREDENTIALW {
            flags: 0,
            cred_type: 1, // CRED_TYPE_GENERIC
            target_name: target_wide.as_ptr(),
            comment: ptr::null(),
            last_written: FILETIME { dw_low_date_time: 0, dw_high_date_time: 0 },
            credential_blob_size: secret_size,
            credential_blob: secret_ptr,
            persist: 2, // CRED_PERSIST_LOCAL_MACHINE
            attribute_count: 0,
            attributes: ptr::null(),
            target_alias: ptr::null(),
            user_name: user_wide.as_ptr(),
        };

        unsafe {
            let _ = CredDeleteW(target_wide.as_ptr(), 1, 0);
            let res = CredWriteW(&cred, 0);
            if res == 0 {
                let err = std::io::Error::last_os_error();
                return Err(format!("Windows CredWriteW failed: {}", err));
            }
        }
    }

    Ok(())
}

/// Format Unix timestamp as RFC3339 with microseconds (matching Manager's chrono output)
fn format_timestamp_rfc3339(timestamp: i64) -> String {
    // Simple manual formatting to avoid adding chrono dependency
    // For 4070908800: 2099-01-01T00:00:00.000000Z
    let secs_per_day: i64 = 86400;
    let days_since_epoch = timestamp / secs_per_day;
    let time_of_day = timestamp % secs_per_day;

    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Simple date calculation from days since Unix epoch
    let (year, month, day) = days_to_date(days_since_epoch);
    
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.000000Z", year, month, day, hours, minutes, seconds)
}

fn days_to_date(mut days: i64) -> (i64, i64, i64) {
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    days += 719468;
    let era = if days >= 0 { days } else { days - 146096 } / 146097;
    let doe = days - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Legacy SQLite injection (for Antigravity < 2.0.0)
fn inject_to_sqlite(token: &str, proxy_url: &str, email: &str, expiry: i64, ide_type: &str, custom_db_path: Option<&str>) -> Result<String, String> {
    let db_path = get_db_path(ide_type, custom_db_path)?;
    if !db_path.exists() {
        return Err(format!("Antigravity IDE database not found at {:?}", db_path));
    }

    let conn = Connection::open(&db_path).map_err(|e| format!("Failed to open DB: {}", e))?;
    let _ = conn.execute("PRAGMA journal_mode=DELETE;", []);

    // Create OAuth info (simulated for the client)
    let oauth_info = protobuf::create_oauth_info(token, token, expiry, false, None, Some(email));
    
    // Create auth state as "loggedIn" — without this, IDE shows "authentication error"
    // because a stale "loginError" authState persists from previous failed attempts
    let auth_state = protobuf::create_auth_state_logged_in();
    
    // Write BOTH entries into a single oauthToken field
    let outer_b64 = protobuf::create_multi_unified_state_entry(&[
        ("oauthTokenInfoSentinelKey", &oauth_info),
        ("authStateWithContextSentinelKey", &auth_state),
    ]);
    
    conn.execute(
        "INSERT OR REPLACE INTO ItemTable (key, value) VALUES (?, ?)",
        ["antigravityUnifiedStateSync.oauthToken", &outer_b64],
    ).map_err(|e| format!("Failed to write oauth token: {}", e))?;

    let user_status_payload = protobuf::create_minimal_user_status_payload(email);
    let user_status_entry_b64 = protobuf::create_unified_state_entry("userStatusSentinelKey", &user_status_payload);
    conn.execute(
        "INSERT OR REPLACE INTO ItemTable (key, value) VALUES (?, ?)",
        ["antigravityUnifiedStateSync.userStatus", &user_status_entry_b64],
    ).map_err(|e| format!("Failed to write user status: {}", e))?;

    if proxy_url.is_empty() {
        conn.execute(
            "DELETE FROM ItemTable WHERE key = ?",
            ["antigravity.proxyBaseUrl"],
        ).map_err(|e| format!("Failed to clear proxy url: {}", e))?;
    } else {
        conn.execute(
            "INSERT OR REPLACE INTO ItemTable (key, value) VALUES (?, ?)",
            ["antigravity.proxyBaseUrl", proxy_url],
        ).map_err(|e| format!("Failed to write proxy url: {}", e))?;
    }

    conn.execute(
        "INSERT OR REPLACE INTO ItemTable (key, value) VALUES (?, ?)",
        ["antigravityOnboarding", "true"],
    ).map_err(|e| format!("Failed to write onboarding flag: {}", e))?;

    Ok("Successfully injected token and proxy URL into Antigravity IDE.".to_string())
}

pub fn clear_proxy_settings(ide_type: &str) -> Result<(), String> {
    // 1. Clear SQLite setting
    if let Ok(db_path) = get_db_path(ide_type, None) {
        if db_path.exists() {
            if let Ok(conn) = Connection::open(&db_path) {
                let _ = conn.execute("PRAGMA journal_mode=DELETE;", []);
                let _ = conn.execute(
                    "DELETE FROM ItemTable WHERE key = ?",
                    ["antigravity.proxyBaseUrl"],
                );
                eprintln!("[Client] Cleared proxyBaseUrl from SQLite database for {}", ide_type);
            }
        }
    }
    
    // 2. Clear settings.json setting
    let mut settings_path = std::path::PathBuf::new();
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = dirs::home_dir() {
            let subfolder = if ide_type == "Antigravity 2.0" { "Antigravity" } else { "Antigravity IDE" };
            settings_path = home.join(format!("Library/Application Support/{}/User/settings.json", subfolder));
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            let subfolder = if ide_type == "Antigravity 2.0" { "Antigravity" } else { "Antigravity IDE" };
            settings_path = std::path::PathBuf::from(appdata).join(format!("{}\\User\\settings.json", subfolder));
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(home) = dirs::home_dir() {
            let subfolder = if ide_type == "Antigravity 2.0" { "Antigravity" } else { "Antigravity IDE" };
            settings_path = home.join(format!(".config/{}/User/settings.json", subfolder));
        }
    }

    if settings_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&settings_path) {
            if let Ok(mut settings) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(obj) = settings.as_object_mut() {
                    obj.remove("antigravity.proxyBaseUrl");
                    if let Ok(new_content) = serde_json::to_string_pretty(&settings) {
                        let _ = std::fs::write(&settings_path, new_content);
                        eprintln!("[Client] Cleared proxyBaseUrl from settings.json for {}", ide_type);
                    }
                }
            }
        }
    }

    Ok(())
}

pub fn clear_keyring_credentials() -> Result<(), String> {
    use std::process::Command;
    
    #[cfg(target_os = "linux")]
    {
        let _ = Command::new("secret-tool")
            .args(["clear", "service", "gemini", "username", "antigravity"])
            .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus")
            .output();
            
        if let Ok(_) = std::env::var("WSL_DISTRO_NAME") {
            let _ = Command::new("cmdkey.exe")
                .args(["/delete:gemini:antigravity"])
                .output();
        }
    }

    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("security")
            .args(["delete-generic-password", "-s", "gemini", "-a", "antigravity"])
            .output();
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        
        #[link(name = "advapi32")]
        extern "system" {
            fn CredDeleteW(target_name: *const u16, type_: u32, flags: u32) -> i32;
        }

        let target = "gemini:antigravity";
        let target_wide: Vec<u16> = std::ffi::OsStr::new(target)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        unsafe {
            let _ = CredDeleteW(target_wide.as_ptr(), 1, 0);
        }
    }

    Ok(())
}

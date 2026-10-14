//! Application appearance and reminder preferences, independent of business runtime.
use crate::{
    db,
    models::{AppSettings, AppSettingsInput},
    AppState,
};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use std::fs;
use tauri::{image::Image, State};

#[tauri::command]
pub fn get_app_settings(state: State<AppState>) -> Result<AppSettings, String> {
    let connection = db::open(&state.db_path)?;
    let value = |key: &str, fallback: &str| -> String {
        connection
            .query_row(
                "SELECT value FROM app_settings WHERE key=?1",
                [key],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| fallback.to_string())
    };
    Ok(AppSettings {
        reminder_days: value("reminder_days", "7").parse().unwrap_or(7),
        custom_icon: value("custom_icon", "false") == "true",
        deduplicated_assets: value("last_deduplicated", "0").parse().unwrap_or(0),
    })
}

#[tauri::command]
pub fn get_app_icon_data_url(state: State<AppState>) -> Result<String, String> {
    let custom = state.app_data_dir.join("custom-app-icon.png");
    let bytes = if custom.is_file() {
        let bytes = fs::read(custom).map_err(|error| error.to_string())?;
        if Image::from_bytes(&bytes).is_ok() {
            bytes
        } else {
            include_bytes!("../../icons/brand-icon.png").to_vec()
        }
    } else {
        include_bytes!("../../icons/brand-icon.png").to_vec()
    };
    // A single base64 string is materially cheaper to serialize across Tauri
    // IPC than hundreds of thousands of JSON array elements.
    Ok(format!(
        "data:image/png;base64,{}",
        BASE64_STANDARD.encode(bytes)
    ))
}

#[tauri::command]
pub fn save_app_settings(state: State<AppState>, input: AppSettingsInput) -> Result<(), String> {
    if !(1..=365).contains(&input.reminder_days) {
        return Err("提醒天数必须在 1 到 365 之间".into());
    }
    db::open(&state.db_path)?
        .execute(
            "INSERT INTO app_settings(key,value) VALUES('reminder_days',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [input.reminder_days.to_string()],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn save_app_icon(state: State<AppState>, bytes: Vec<u8>) -> Result<(), String> {
    if bytes.is_empty() || bytes.len() > 5 * 1024 * 1024 {
        return Err("图标必须是小于 5MB 的 PNG 文件".into());
    }
    let icon = Image::from_bytes(&bytes).map_err(|_| "无法解析 PNG 图标".to_string())?;
    if icon.width() < 32 || icon.height() < 32 {
        return Err("图标尺寸至少为 32×32".into());
    }
    let path = state.app_data_dir.join("custom-app-icon.png");
    fs::write(&path, bytes).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    db::open(&state.db_path)?
        .execute(
            "INSERT INTO app_settings(key,value) VALUES('custom_icon','true') ON CONFLICT(key) DO UPDATE SET value='true'",
            [],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn reset_app_icon(state: State<AppState>) -> Result<(), String> {
    let _ = fs::remove_file(state.app_data_dir.join("custom-app-icon.png"));
    for directory in &state.legacy_icon_dirs {
        let _ = fs::remove_file(directory.join("custom-app-icon.png"));
    }
    db::open(&state.db_path)?
        .execute(
            "INSERT INTO app_settings(key,value) VALUES('custom_icon','false') ON CONFLICT(key) DO UPDATE SET value='false'",
            [],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

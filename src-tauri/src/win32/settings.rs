//! Application Settings & Split-Tunneling Configuration
//!
//! Stores user preferences for:
//! - Split-Tunneling Mode: "full" (all sites), "strict" (YouTube & Discord only), "game_mode" (gaming port filter)
//! - Game Filter port ranges and enabled status
//! - Preferred DNS provider (DoH / System)

use std::fs;
use std::path::{Path, PathBuf};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitTunnelMode {
    Full,     // Full bypass: YouTube + Discord + All blocked websites (ipset-all)
    Strict,   // Strict: YouTube & Discord ONLY (other traffic goes 100% direct)
    GameMode, // Gaming mode: Includes gaming UDP/TCP ports with ACTIVE_GAME_UDP.bin desync
}

impl Default for SplitTunnelMode {
    fn default() -> Self {
        SplitTunnelMode::Full
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub split_tunnel_mode: SplitTunnelMode,
    pub game_filter_tcp: String,
    pub game_filter_udp: String,
    pub dns_provider: String, // "system", "cloudflare", "google", "quad9", "comss"
    pub last_strategy_id: Option<String>,
    pub auto_start_bypass: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            split_tunnel_mode: SplitTunnelMode::Full,
            game_filter_tcp: "1024-65535".into(),
            game_filter_udp: "1024-65535".into(),
            dns_provider: "system".into(),
            last_strategy_id: Some("flowseal-alt".into()),
            auto_start_bypass: true,
        }
    }
}

static CACHED_SETTINGS: RwLock<Option<AppSettings>> = RwLock::new(None);

fn get_settings_file_path() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let p = parent.join("configs");
            if p.is_dir() {
                return p.join("app_settings.json");
            }
        }
    }

    let local_path = PathBuf::from("configs/app_settings.json");
    if local_path.parent().map_or(false, |p| p.is_dir()) {
        return local_path;
    }

    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        let dir = Path::new(&local_app_data).join("zapret2").join("configs");
        let _ = fs::create_dir_all(&dir);
        return dir.join("app_settings.json");
    }

    local_path
}

pub fn load_settings() -> AppSettings {
    let read_guard = CACHED_SETTINGS.read();
    if let Some(ref s) = *read_guard {
        return s.clone();
    }
    drop(read_guard);

    let path = get_settings_file_path();
    let settings = if path.is_file() {
        match fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str::<AppSettings>(&content).unwrap_or_default(),
            Err(_) => AppSettings::default(),
        }
    } else {
        AppSettings::default()
    };

    *CACHED_SETTINGS.write() = Some(settings.clone());
    settings
}

pub fn save_settings(new_settings: AppSettings) -> Result<(), String> {
    let path = get_settings_file_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let json = serde_json::to_string_pretty(&new_settings)
        .map_err(|e| format!("Failed to serialize settings: {}", e))?;

    fs::write(&path, json).map_err(|e| format!("Failed to save settings to disk: {}", e))?;

    *CACHED_SETTINGS.write() = Some(new_settings);
    Ok(())
}

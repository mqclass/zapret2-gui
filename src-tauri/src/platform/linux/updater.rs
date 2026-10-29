//! Self-update pipeline (Linux).
//!
//! Not available in phase 1, deliberately: published releases ship
//! Windows assets only, so there is nothing truthful for a Linux build
//! to check against, download, or install. The entry points that would
//! touch the network or the filesystem refuse with that reason (the UI
//! shows it) instead of reporting a fake "you are up to date".
//!
//! The settings-backed preferences ARE real — the auto-check toggle and
//! its bookkeeping live in settings on every platform, and the frontend
//! reads/writes them through the same commands. When Linux releases
//! exist, the check/download/install flow lands here behind the same
//! signatures the Windows build exposes.

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::engine::managed::ManagedSidecar;
use crate::engine::supervisor::SupervisorState;
use crate::platform::settings::{load_settings, save_settings};

/// Why every network/filesystem update operation refuses on Linux.
const LINUX_UPDATE_UNAVAILABLE: &str =
    "Обновления на Linux пока недоступны: релизы zapret2-gui выходят только \
     для Windows. Автообновление появится вместе с Linux-сборками.";

/// Release information returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub release_notes: String,
    pub release_url: String,
    pub asset_name: String,
    pub asset_size: u64,
    pub published_at: String,
}

/// Download progress snapshot emitted over the `update-download-progress` event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateProgress {
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub percent: u8,
}

/// Status snapshot of one registered sidecar, for the frontend update flow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SidecarStatusView {
    pub name: String,
    pub active: bool,
}

/// Scheduler preferences: the auto-check toggle plus bookkeeping for the
/// "at most once per 6h" schedule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePrefs {
    pub auto_check_enabled: bool,
    pub last_check_epoch: u64,
}

/// Checks for a newer release. Refused on Linux in phase 1: releases
/// carry Windows assets only, so no honest answer exists yet (see the
/// module docs) — an error, not a fabricated "no update available".
pub async fn check_for_updates() -> Result<UpdateInfo, String> {
    Err(LINUX_UPDATE_UNAVAILABLE.to_string())
}

/// Requests cancellation of an in-flight `download_update`. Downloads
/// cannot start on Linux (see `download_update`), so there is never
/// anything to cancel — a plain no-op.
pub fn cancel_download_update() {}

/// Downloads the release asset. Refused on Linux in phase 1 (module docs).
pub async fn download_update(_app: AppHandle) -> Result<PathBuf, String> {
    Err(LINUX_UPDATE_UNAVAILABLE.to_string())
}

/// Installs a previously staged update. Refused on Linux in phase 1:
/// nothing can ever have been staged by `download_update`.
pub fn install_update(
    _engine_state: SupervisorState,
    _sidecars: &[Arc<dyn ManagedSidecar>],
) -> Result<(), String> {
    Err(LINUX_UPDATE_UNAVAILABLE.to_string())
}

/// Returns the auto-check toggle and the last successful check timestamp.
pub fn get_update_prefs() -> UpdatePrefs {
    let s = load_settings();
    UpdatePrefs {
        auto_check_enabled: s.auto_update_enabled,
        last_check_epoch: s.last_update_check.unwrap_or(0),
    }
}

/// Enables or disables the scheduled update check.
pub fn set_auto_update_enabled(enable: bool) -> Result<bool, String> {
    let mut s = load_settings();
    s.auto_update_enabled = enable;
    save_settings(s).map_err(|e| e.to_string())?;
    Ok(enable)
}

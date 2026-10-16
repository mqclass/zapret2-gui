//! Self-update pipeline: GitHub release check, streaming download with SHA256
//! verification, and atomic directory-swap installation with rollback.
//!
//! Design notes:
//! - Everything that touches the network or the filesystem runs here in the
//!   elevated supervisor process (admin rights, access to Program Files).
//! - Nothing is ever downloaded or installed silently: the frontend shows a
//!   confirmation dialog with the release notes before `download_update` runs,
//!   and stops the bypass engine (with the user's explicit consent) before
//!   `install_update` runs.
//! - Windows file semantics: renaming a *running* executable is allowed, but
//!   overwriting it is not. Installation is therefore a rename-cycle of whole
//!   directories (install -> backup, staging -> install) — there is never a
//!   half-replaced tree on disk.
//! - `install_update` is fail-closed: it refuses to run while the bypass
//!   engine is active so an update can never silently kill live connections.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};

use crate::engine::supervisor::{Supervisor, SupervisorState};
use crate::win32::settings::{load_settings, save_settings};

/// GitHub API endpoint for the latest zapret2-gui release.
const GITHUB_API_LATEST: &str = "https://api.github.com/repos/mqclass/zapret2-gui/releases/latest";
/// GitHub API rejects requests without a User-Agent (HTTP 403), even on a
/// perfectly working network — this is not a "block", it is our bug if missing.
const USER_AGENT: &str = "zapret2-gui-updater";
const GITHUB_ACCEPT: &str = "application/vnd.github+json";

/// Subset of the GitHub release-asset JSON we rely on.
#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    size: u64,
    /// Content digest in the form `sha256:<hex>` (may be absent on old releases).
    digest: Option<String>,
    browser_download_url: String,
}

/// Subset of the GitHub release JSON we rely on.
#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    body: Option<String>,
    html_url: String,
    prerelease: bool,
    published_at: Option<String>,
    assets: Vec<GhAsset>,
}

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
    pub prerelease: bool,
}

/// Download progress snapshot emitted over the `update-download-progress` event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateProgress {
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub percent: u8,
}

/// Scheduler preferences: the auto-check toggle plus bookkeeping for the
/// "at most once per 6h" schedule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePrefs {
    pub auto_check_enabled: bool,
    pub last_check_epoch: u64,
}

/// Staging directory of a verified, extracted update, set by `download_update`
/// and consumed (taken) by `install_update`. Keeping it here instead of as an
/// IPC argument means the frontend can never point the installer at an
/// arbitrary directory.
static STAGED_UPDATE: RwLock<Option<PathBuf>> = RwLock::new(None);

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Parses `1.0.10` / `v1.0.10` style tags into a comparable tuple.
/// Returns `None` for non-semver tags (caller treats those as "no update").
fn parse_semver(tag: &str) -> Option<(u64, u64, u64)> {
    let t = tag.trim().trim_start_matches(|c| c == 'v' || c == 'V');
    let mut parts = t.split('.');
    let major = parts.next()?.trim().parse::<u64>().ok()?;
    let minor = parts.next().unwrap_or("0").trim().parse::<u64>().ok()?;
    // The patch component may carry a pre-release/build suffix ("10-rc1").
    let patch_raw = parts.next().unwrap_or("0").trim();
    let digits_end = patch_raw
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(patch_raw.len());
    let patch = patch_raw[..digits_end].parse::<u64>().ok()?;
    Some((major, minor, patch))
}

fn build_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| format!("NETWORK: failed to build HTTP client: {e}"))
}

async fn fetch_latest_release(client: &reqwest::Client) -> Result<GhRelease, String> {
    let resp = client
        .get(GITHUB_API_LATEST)
        .header("Accept", GITHUB_ACCEPT)
        .send()
        .await
        .map_err(|e| format!("NETWORK: GitHub API unreachable: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!(
            "NETWORK: GitHub API returned HTTP {}",
            resp.status()
        ));
    }

    let text = resp
        .text()
        .await
        .map_err(|e| format!("NETWORK: failed reading GitHub response: {e}"))?;
    serde_json::from_str::<GhRelease>(&text)
        .map_err(|e| format!("Failed to parse GitHub release metadata: {e}"))
}

/// Selects the Windows installer asset for `tag`.
///
/// Primary match is the exact CI asset name `zapret2-<tag>-windows-x64.zip`;
/// falls back to any `-windows-x64.zip` asset so a tag-naming drift on the
/// release side degrades to "still works" instead of "update broken".
fn find_asset<'a>(release: &'a GhRelease, tag: &str) -> Option<&'a GhAsset> {
    let exact = format!("zapret2-{tag}-windows-x64.zip");
    release
        .assets
        .iter()
        .find(|a| a.name == exact)
        .or_else(|| {
            release
                .assets
                .iter()
                .find(|a| a.name.ends_with("-windows-x64.zip"))
        })
}

/// Scratch directory for downloads and staging (`%TEMP%\zapret2-update`).
fn update_work_dir() -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join("zapret2-update");
    fs::create_dir_all(&dir)
        .map_err(|e| format!("Cannot create update work dir {}: {e}", dir.display()))?;
    Ok(dir)
}

/// Checks GitHub for a newer release and compares it against the running version.
///
/// A successful check records `last_update_check` in settings so the frontend
/// scheduler measures the 6h interval from the last *success*, not the last
/// attempt — GitHub being unreachable simply shifts the next attempt instead
/// of spamming the user.
pub async fn check_for_updates() -> Result<UpdateInfo, String> {
    let current = env!("CARGO_PKG_VERSION").to_string();
    let client = build_client()?;
    let release = fetch_latest_release(&client).await?;

    let latest = release.tag_name.trim().to_string();
    let update_available = match (parse_semver(&current), parse_semver(&latest)) {
        (Some(cur), Some(lat)) => lat > cur,
        // Non-semver tag (e.g. "nightly"): never auto-offer, never error.
        _ => false,
    };

    let (asset_name, asset_size) = find_asset(&release, &latest)
        .map(|a| (a.name.clone(), a.size))
        .unwrap_or_default();

    let mut settings = load_settings();
    settings.last_update_check = Some(unix_now());
    let _ = save_settings(settings);

    Ok(UpdateInfo {
        current_version: current,
        latest_version: latest,
        update_available,
        release_notes: release.body.unwrap_or_default(),
        release_url: release.html_url,
        asset_name,
        asset_size,
        published_at: release.published_at.unwrap_or_default(),
        prerelease: release.prerelease,
    })
}

/// Extracts `zip_path` into `dest`, rejecting entries that would escape `dest`
/// (zip-slip protection).
fn extract_zip(zip_path: &Path, dest: &Path) -> Result<(), String> {
    let file = fs::File::open(zip_path)
        .map_err(|e| format!("Cannot open downloaded archive: {e}"))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("Downloaded file is not a valid zip archive: {e}"))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Corrupt zip entry #{i}: {e}"))?;
        let out_path = match entry.enclosed_name() {
            Some(p) => dest.join(p),
            None => {
                return Err(format!(
                    "Zip entry #{} has an unsafe path; aborting extraction",
                    i
                ))
            }
        };
        if entry.is_dir() {
            fs::create_dir_all(&out_path)
                .map_err(|e| format!("Cannot create dir {}: {e}", out_path.display()))?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("Cannot create dir {}: {e}", parent.display()))?;
            }
            let mut out = fs::File::create(&out_path)
                .map_err(|e| format!("Cannot write {}: {e}", out_path.display()))?;
            std::io::copy(&mut entry, &mut out)
                .map_err(|e| format!("Failed extracting {}: {e}", out_path.display()))?;
        }
    }
    Ok(())
}

/// Downloads the release asset with streaming SHA256 verification.
///
/// The hash is computed incrementally while chunks arrive and compared against
/// the `sha256:<hex>` digest from the asset metadata (fetched over TLS from
/// the GitHub API — no hardcoded hashes). On mismatch the partial file is
/// deleted and installation never starts. Emits `update-download-progress`
/// events for the UI progress bar. Returns the staging directory and records
/// it for `install_update`.
pub async fn download_update(app: AppHandle) -> Result<PathBuf, String> {
    let client = build_client()?;

    // Re-resolve the release here so the download URL and digest always come
    // from fresh API metadata, never from a stale frontend cache.
    let release = fetch_latest_release(&client).await?;
    let tag = release.tag_name.trim().to_string();
    let asset = find_asset(&release, &tag)
        .ok_or_else(|| format!("Release {tag} has no Windows x64 zip asset"))?;
    let expected_hex = asset
        .digest
        .as_deref()
        .and_then(|d| d.strip_prefix("sha256:"))
        .ok_or_else(|| {
            format!(
                "Asset {} has no sha256 digest in release metadata; refusing download",
                asset.name
            )
        })?
        .to_lowercase();

    let work_dir = update_work_dir()?;
    let tmp_path = work_dir.join(format!("{}.part", asset.name));
    let final_path = work_dir.join(&asset.name);

    let mut resp = client
        .get(&asset.browser_download_url)
        .send()
        .await
        .map_err(|e| format!("NETWORK: download failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!(
            "NETWORK: asset download returned HTTP {}",
            resp.status()
        ));
    }
    let total = resp.content_length().unwrap_or(0);

    let mut file = fs::File::create(&tmp_path)
        .map_err(|e| format!("Cannot write {}: {e}", tmp_path.display()))?;
    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;

    // Chunked streaming (see engine/speedtest.rs): the whole archive is never
    // buffered in RAM, and the hash is incremental.
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| format!("NETWORK: download interrupted: {e}"))?
    {
        hasher.update(&chunk);
        file.write_all(&chunk)
            .map_err(|e| format!("Cannot write {}: {e}", tmp_path.display()))?;
        downloaded += chunk.len() as u64;
        let percent = if total > 0 {
            ((downloaded * 100 / total).min(100)) as u8
        } else {
            0
        };
        let _ = app.emit(
            "update-download-progress",
            UpdateProgress {
                downloaded_bytes: downloaded,
                total_bytes: total,
                percent,
            },
        );
    }
    drop(file);

    let actual_hex = hex::encode(hasher.finalize());
    if actual_hex != expected_hex {
        let _ = fs::remove_file(&tmp_path);
        return Err(format!(
            "SHA256 mismatch for {}: expected {expected_hex}, got {actual_hex}. \
             The temporary file was deleted; the update was NOT installed.",
            asset.name
        ));
    }

    // Hash verified — promote the download and extract it to a staging dir.
    // (Remove any stale file first: a crashed previous attempt must never
    // leave a half-written archive behind.)
    if final_path.exists() {
        fs::remove_file(&final_path)
            .map_err(|e| format!("Cannot clear stale download: {e}"))?;
    }
    fs::rename(&tmp_path, &final_path)
        .map_err(|e| format!("Cannot finalize download: {e}"))?;
    let staging = work_dir.join(format!("staging-{tag}"));
    if staging.exists() {
        fs::remove_dir_all(&staging)
            .map_err(|e| format!("Cannot clear stale staging dir: {e}"))?;
    }
    fs::create_dir_all(&staging)
        .map_err(|e| format!("Cannot create staging dir: {e}"))?;
    extract_zip(&final_path, &staging)?;

    *STAGED_UPDATE.write() = Some(staging.clone());
    Ok(staging)
}

/// Copies user-owned data from the backup into the fresh installation tree.
///
/// The release zip ships *default* configs and lists; without this step an
/// update would silently wipe the user's settings, custom profiles and
/// personal domain additions.
fn restore_user_data(backup: &Path, install: &Path) -> Result<(), String> {
    // 1. Known user-owned config files.
    for rel in ["configs/app_settings.json", "configs/custom_profiles.json"] {
        let src = backup.join(rel);
        if src.is_file() {
            let dst = install.join(rel);
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("Cannot create {}: {e}", parent.display()))?;
            }
            fs::copy(&src, &dst)
                .map_err(|e| format!("Cannot restore {}: {e}", src.display()))?;
        }
    }

    // 2. Personal domain additions follow the `*-user.txt` convention.
    let backup_lists = backup.join("lists");
    if backup_lists.is_dir() {
        let install_lists = install.join("lists");
        let entries = fs::read_dir(&backup_lists)
            .map_err(|e| format!("Cannot read {}: {e}", backup_lists.display()))?;
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.ends_with("-user.txt") && entry.path().is_file() {
                let dst = install_lists.join(&name);
                if let Some(parent) = dst.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                fs::copy(entry.path(), &dst)
                    .map_err(|e| format!("Cannot restore {}: {e}", name_str))?;
            }
        }
    }
    Ok(())
}

/// Best-effort rollback after a failed swap: move the new tree aside and put
/// the backup back in place.
fn rollback_swap(install_dir: &Path, backup_dir: &Path, staging_dir: &Path) {
    // If the new tree made it into place, move it aside first.
    if install_dir.exists() {
        let _ = fs::rename(install_dir, staging_dir);
    }
    let _ = fs::rename(backup_dir, install_dir);
}

/// Atomically installs a previously staged update and restarts the app.
///
/// Fail-closed on a running engine: refuses with `ENGINE_RUNNING` unless the
/// bypass is fully stopped, so an update can never silently drop live
/// connections. On success this function spawns the new executable and
/// terminates the current process — it never returns `Ok`.
pub fn install_update(supervisor: &Supervisor) -> Result<(), String> {
    if supervisor.get_status().state != SupervisorState::Stopped {
        return Err(
            "ENGINE_RUNNING: stop the bypass engine before installing the update".to_string(),
        );
    }

    let staging = STAGED_UPDATE
        .write()
        .take()
        .ok_or_else(|| "No staged update found — download the update first".to_string())?;
    if !staging.is_dir() {
        return Err("Staged update directory is missing".to_string());
    }

    let current_exe = std::env::current_exe()
        .map_err(|e| format!("Cannot resolve current executable path: {e}"))?;
    let install_dir = current_exe
        .parent()
        .ok_or_else(|| "Cannot resolve installation directory".to_string())?
        .to_path_buf();

    // The release zip ships the exe as `zapret2.exe`; a dev build runs as
    // `zapret2-supervisor.exe`. Prefer the current exe name, fall back to the
    // release name.
    let exe_file_name = current_exe
        .file_name()
        .ok_or_else(|| "Cannot resolve executable file name".to_string())?;
    let staged_exe = [staging.join(exe_file_name), staging.join("zapret2.exe")]
        .into_iter()
        .find(|p| p.is_file())
        .ok_or_else(|| "Staged update does not contain the application executable".to_string())?;
    let staged_exe_name = staged_exe
        .file_name()
        .ok_or_else(|| "Cannot resolve staged executable file name".to_string())?
        .to_os_string();

    let backup_name = match install_dir.file_name() {
        Some(n) => format!("{}.bak", n.to_string_lossy()),
        None => "zapret2.bak".to_string(),
    };
    let backup_dir = install_dir.with_file_name(backup_name);

    // Single backup slot: drop the previous one so a stale backup can never
    // be mistaken for the current version during manual recovery.
    if backup_dir.exists() {
        fs::remove_dir_all(&backup_dir)
            .map_err(|e| format!("Cannot clear previous backup {}: {e}", backup_dir.display()))?;
    }

    // 1. Current installation -> backup (renaming a running exe is allowed).
    fs::rename(&install_dir, &backup_dir)
        .map_err(|e| format!("Failed to back up current installation: {e}"))?;

    // 2. Staging -> installation. Any failure from here on rolls back.
    if let Err(e) = fs::rename(&staging, &install_dir) {
        rollback_swap(&install_dir, &backup_dir, &staging);
        return Err(format!("Failed to install update (rolled back): {e}"));
    }

    // 3. Restore user data into the fresh tree.
    if let Err(e) = restore_user_data(&backup_dir, &install_dir) {
        rollback_swap(&install_dir, &backup_dir, &staging);
        return Err(format!(
            "Failed to restore user settings into the update (rolled back): {e}"
        ));
    }

    // 4. Launch the new version (inherits the elevated token — no new UAC
    //    prompt) and terminate this process.
    let new_exe = install_dir.join(&staged_exe_name);
    if let Err(e) = std::process::Command::new(&new_exe).spawn() {
        rollback_swap(&install_dir, &backup_dir, &staging);
        return Err(format!(
            "Update installed but the new version failed to launch (rolled back): {e}"
        ));
    }
    std::process::exit(0);
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

#[cfg(test)]
mod tests {
    use super::parse_semver;

    #[test]
    fn semver_parses_v_prefixed_tags() {
        assert_eq!(parse_semver("v1.0.10"), Some((1, 0, 10)));
        assert_eq!(parse_semver("1.0.10"), Some((1, 0, 10)));
        assert_eq!(parse_semver("v2.1"), Some((2, 1, 0)));
        assert_eq!(parse_semver("v1.0.10-rc1"), Some((1, 0, 10)));
    }

    #[test]
    fn semver_rejects_non_version_tags() {
        assert_eq!(parse_semver("nightly"), None);
        assert_eq!(parse_semver(""), None);
    }

    #[test]
    fn semver_orders_correctly() {
        assert!(parse_semver("v1.0.11").unwrap() > parse_semver("v1.0.10").unwrap());
        assert!(parse_semver("v1.0.9").unwrap() < parse_semver("v1.0.10").unwrap());
        assert!(parse_semver("v2.0.0").unwrap() > parse_semver("v1.99.99").unwrap());
    }
}

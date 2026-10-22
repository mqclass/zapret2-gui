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
//!   engine — or any registered sidecar process — is active, so an update can
//!   never silently kill live connections.
//!
//! Security model of the staging area (TOCTOU hardening):
//! - Each download gets a *random* work directory (`%TEMP%\zapret2-update-<rand>`)
//!   whose DACL is restricted to SYSTEM + Administrators (best-effort via
//!   icacls; the downloader runs elevated so this normally succeeds).
//! - The backend records the verified zip path, its expected SHA256 and the
//!   tag in `STAGED_UPDATE` — never a frontend-supplied path.
//! - `install_update` does NOT trust the tree left in %TEMP%: it re-hashes the
//!   zip against the recorded digest and re-extracts into a fresh directory
//!   before the swap. A same-user process that tampered with the staging area
//!   between download and install only triggers a clean refusal.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};

use crate::engine::managed::ManagedSidecar;
use crate::engine::supervisor::SupervisorState;
use crate::platform::settings::{load_settings, save_settings};

/// GitHub API endpoint for the latest zapret2-gui release.
const GITHUB_API_LATEST: &str = "https://api.github.com/repos/mqclass/zapret2-gui/releases/latest";
/// GitHub API rejects requests without a User-Agent (HTTP 403), even on a
/// perfectly working network — this is not a "block", it is our bug if missing.
const USER_AGENT: &str = "zapret2-gui-updater";
const GITHUB_ACCEPT: &str = "application/vnd.github+json";

/// Upper bound on the total uncompressed size of the release archive.
/// A compromised release is the only realistic source of a zip bomb; the
/// digest check alone cannot stop it from filling the disk.
const MAX_EXTRACT_BYTES: u64 = 2 * 1024 * 1024 * 1024; // 2 GiB

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

/// A verified download, recorded by `download_update` and consumed (taken) by
/// `install_update`. The zip path + expected hash + tag live here backend-side
/// so the frontend can never point the installer at an arbitrary directory,
/// and `install_update` can re-verify the payload it is about to install.
#[derive(Debug, Clone)]
struct StagedRecord {
    zip_path: PathBuf,
    work_dir: PathBuf,
    expected_sha256: String,
    tag: String,
}

static STAGED_UPDATE: RwLock<Option<StagedRecord>> = RwLock::new(None);

/// Tag of the most recent `check_for_updates`, recorded backend-side.
/// `download_update` refuses to fetch a different tag, so the release notes
/// the user confirmed in the dialog always match the downloaded bytes.
#[derive(Debug, Clone)]
struct CheckedRelease {
    tag: String,
}

static LAST_CHECKED: RwLock<Option<CheckedRelease>> = RwLock::new(None);

/// Cooperative cancellation flag for `download_update`, set by the
/// `cancel_update_download` IPC command.
static CANCEL_DOWNLOAD: AtomicBool = AtomicBool::new(false);

/// Counter mixed into the work-dir name so two downloads in the same
/// nanosecond still get distinct directories.
static WORKDIR_COUNTER: AtomicU64 = AtomicU64::new(0);

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

/// Best-effort restrictive DACL on the work dir: strip inherited ACEs (the
/// user's %TEMP% grants the user full control) and grant full control only to
/// Administrators and SYSTEM. This is the primary barrier against a same-user
/// (non-admin) process tampering with the staged payload; the random directory
/// name and the re-verification in `install_update` are the fallback layers.
///
/// Best-effort by design: if icacls is unavailable we continue anyway — the
/// update must keep working, and `install_update` re-verifies regardless.
#[cfg(windows)]
fn restrict_dir_to_admins(dir: &Path) {
    let ok = std::process::Command::new("icacls")
        .arg(dir)
        .arg("/inheritance:r")
        .arg("/grant:r")
        .arg("*S-1-5-32-544:(OI)(CI)F")
        .arg("/grant:r")
        .arg("SYSTEM:(OI)(CI)F")
        .arg("/Q")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !ok {
        eprintln!(
            "updater: warning: could not restrict DACL on {}",
            dir.display()
        );
    }
}

/// Scratch directory for one download (`%TEMP%\zapret2-update-<random>`).
///
/// The name is unpredictable per download so a same-user process cannot
/// pre-create or reliably watch the staging area. The directory is created
/// with a restrictive DACL (see `restrict_dir_to_admins`).
fn update_work_dir() -> Result<PathBuf, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let uniq = format!(
        "{:x}-{:x}-{:x}",
        nanos,
        std::process::id(),
        WORKDIR_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let dir = std::env::temp_dir().join(format!("zapret2-update-{uniq}"));
    fs::create_dir_all(&dir)
        .map_err(|e| format!("Cannot create update work dir {}: {e}", dir.display()))?;
    restrict_dir_to_admins(&dir);
    Ok(dir)
}

/// Removes stale update work directories: the legacy fixed name from older
/// versions plus orphaned `zapret2-update-*` dirs from crashed downloads.
///
/// Only directories older than 24h are removed — a concurrent download in
/// another process may still be using a fresh one.
fn cleanup_stale_work_dirs() {
    let tmp = std::env::temp_dir();
    let entries = match fs::read_dir(&tmp) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_ours =
            name == "zapret2-update" || name.starts_with("zapret2-update-");
        if !is_ours || !entry.path().is_dir() {
            continue;
        }
        let old_enough = fs::metadata(entry.path())
            .and_then(|m| m.modified())
            .and_then(|mtime| SystemTime::now().duration_since(mtime).map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::Other, "clock skew")
            }))
            .map(|age| age >= Duration::from_secs(24 * 3600))
            .unwrap_or(false);
        if old_enough {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// Checks GitHub for a newer release and compares it against the running version.
///
/// A successful check records `last_update_check` in settings so the frontend
/// scheduler measures the 6h interval from the last *success*, not the last
/// attempt — GitHub being unreachable simply shifts the next attempt instead
/// of spamming the user. It also records the tag backend-side so
/// `download_update` can refuse a tag the user never confirmed (the
/// `/releases/latest` re-resolution race).
pub async fn check_for_updates() -> Result<UpdateInfo, String> {
    let current = env!("CARGO_PKG_VERSION").to_string();
    let client = build_client()?;
    let release = fetch_latest_release(&client).await?;

    let latest = release.tag_name.trim().to_string();
    // `/releases/latest` never returns prereleases, but gate explicitly:
    // a prerelease must never be auto-offered even if the API ever changes.
    let update_available = !release.prerelease
        && match (parse_semver(&current), parse_semver(&latest)) {
            (Some(cur), Some(lat)) => lat > cur,
            // Non-semver tag (e.g. "nightly"): never auto-offer, never error.
            _ => false,
        };

    let (asset_name, asset_size) = find_asset(&release, &latest)
        .map(|a| (a.name.clone(), a.size))
        .unwrap_or_default();

    *LAST_CHECKED.write() = Some(CheckedRelease { tag: latest.clone() });

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
    })
}

/// Extracts `zip_path` into `dest`, rejecting entries that would escape `dest`
/// (zip-slip protection) and aborting if the uncompressed total exceeds
/// `MAX_EXTRACT_BYTES` (zip-bomb protection — a compromised release is the only
/// realistic source of such an archive, and the digest check cannot stop it
/// from filling the disk).
fn extract_zip(zip_path: &Path, dest: &Path) -> Result<(), String> {
    let file = fs::File::open(zip_path)
        .map_err(|e| format!("Cannot open downloaded archive: {e}"))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("Downloaded file is not a valid zip archive: {e}"))?;

    let mut total_uncompressed: u64 = 0;
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
            let written = std::io::copy(&mut entry, &mut out)
                .map_err(|e| format!("Failed extracting {}: {e}", out_path.display()))?;
            total_uncompressed = total_uncompressed.saturating_add(written);
            if total_uncompressed > MAX_EXTRACT_BYTES {
                drop(out);
                let _ = fs::remove_dir_all(dest);
                return Err(format!(
                    "Archive expands beyond {} bytes; refusing to extract (possible zip bomb)",
                    MAX_EXTRACT_BYTES
                ));
            }
        }
    }
    Ok(())
}

/// Streams SHA256 over `path` without loading it into memory.
fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path)
        .map_err(|e| format!("Cannot open {} for verification: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 1024 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Requests cancellation of an in-flight `download_update`.
pub fn cancel_download_update() {
    CANCEL_DOWNLOAD.store(true, Ordering::SeqCst);
}

/// Downloads the release asset with streaming SHA256 verification.
///
/// The hash is computed incrementally while chunks arrive and compared against
/// the `sha256:<hex>` digest from the asset metadata (fetched over TLS from
/// the GitHub API — no hardcoded hashes). On mismatch the partial file is
/// deleted and installation never starts. Emits `update-download-progress`
/// events for the UI progress bar.
///
/// The release is re-resolved here so the download URL and digest always come
/// from fresh API metadata — but the tag must equal the one from the most
/// recent `check_for_updates` (recorded backend-side). If a newer release was
/// published after the user confirmed the notes, the download is refused with
/// `RELEASE_CHANGED` instead of silently fetching different bytes.
///
/// Returns the staging directory (informational — `install_update` re-verifies
/// the zip and re-extracts into a fresh directory rather than trusting it).
pub async fn download_update(app: AppHandle) -> Result<PathBuf, String> {
    CANCEL_DOWNLOAD.store(false, Ordering::SeqCst);
    cleanup_stale_work_dirs();

    let client = build_client()?;

    let release = fetch_latest_release(&client).await?;
    let tag = release.tag_name.trim().to_string();

    match LAST_CHECKED.read().clone() {
        Some(checked) if checked.tag == tag => {}
        Some(checked) => {
            return Err(format!(
                "RELEASE_CHANGED: release {} was published after the update check \
                 (checked {}); check for updates again to see the new release notes",
                tag, checked.tag
            ))
        }
        None => {
            return Err(
                "NO_CHECK: run check_for_updates before downloading".to_string(),
            )
        }
    }

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
        let _ = fs::remove_dir_all(&work_dir);
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
    // buffered in RAM, and the hash is incremental. File writes are small and
    // network-bound; the heavy extraction below runs in spawn_blocking.
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| format!("NETWORK: download interrupted: {e}"))?
    {
        if CANCEL_DOWNLOAD.load(Ordering::SeqCst) {
            drop(file);
            let _ = fs::remove_dir_all(&work_dir);
            *STAGED_UPDATE.write() = None;
            return Err("DOWNLOAD_CANCELLED: download cancelled by user".to_string());
        }
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
        let _ = fs::remove_dir_all(&work_dir);
        return Err(format!(
            "SHA256 mismatch for {}: expected {expected_hex}, got {actual_hex}. \
             The temporary files were deleted; the update was NOT installed.",
            asset.name
        ));
    }

    // Hash verified — promote the download and extract it to a staging dir.
    fs::rename(&tmp_path, &final_path)
        .map_err(|e| format!("Cannot finalize download: {e}"))?;
    let staging = work_dir.join("staging");
    fs::create_dir_all(&staging)
        .map_err(|e| format!("Cannot create staging dir: {e}"))?;
    // Extraction is CPU/disk-heavy (tens of MB) — keep it off the async worker.
    let ez_path = final_path.clone();
    let ez_dest = staging.clone();
    tokio::task::spawn_blocking(move || extract_zip(&ez_path, &ez_dest))
        .await
        .map_err(|e| format!("Extraction task failed: {e}"))??;

    *STAGED_UPDATE.write() = Some(StagedRecord {
        zip_path: final_path,
        work_dir,
        expected_sha256: expected_hex,
        tag,
    });
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

/// Recursively copies a directory tree.
fn copy_dir_all(src: &Path, dst: &Path) -> Result<(), std::io::Error> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let dst_path = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&entry.path(), &dst_path)?;
        } else {
            fs::copy(entry.path(), dst_path)?;
        }
    }
    Ok(())
}

/// Moves a directory, falling back to recursive copy + delete when `src` and
/// `dst` are on different volumes. Windows `rename` cannot cross devices, and
/// `%TEMP%` is commonly relocated to another disk — without the fallback the
/// update would always fail on such machines.
fn move_dir(src: &Path, dst: &Path) -> Result<(), String> {
    match fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {
            copy_dir_all(src, dst).map_err(|ce| {
                format!(
                    "Cannot copy {} to {} across volumes: {ce}",
                    src.display(),
                    dst.display()
                )
            })?;
            fs::remove_dir_all(src).map_err(|re| {
                format!(
                    "Copied {} to {}, but cannot remove the source: {re}",
                    src.display(),
                    dst.display()
                )
            })
        }
        Err(e) => Err(format!("Cannot move {}: {e}", src.display())),
    }
}

/// Best-effort rollback after a failed swap: move the new tree aside and put
/// the backup back in place.
fn rollback_swap(install_dir: &Path, backup_dir: &Path, staging_dir: &Path) {
    // If the new tree made it into place, move it aside first.
    if install_dir.exists() {
        let _ = move_dir(install_dir, staging_dir);
    }
    let _ = move_dir(backup_dir, install_dir);
}

/// Fail-closed gate: the bypass engine and every registered sidecar must be
/// fully stopped before the installation directory is touched.
///
/// Called twice by `install_update` — once at entry and once immediately
/// before the rename cycle — so a start racing the install is caught as late
/// as possible.
fn check_engines_stopped(
    engine_state: SupervisorState,
    sidecars: &[Arc<dyn ManagedSidecar>],
) -> Result<(), String> {
    if engine_state != SupervisorState::Stopped {
        return Err(
            "ENGINE_RUNNING: stop the bypass engine before installing the update".to_string(),
        );
    }
    for sidecar in sidecars {
        if sidecar.is_active() {
            return Err(format!(
                "SIDECAR_RUNNING: {} is still active — stop it before installing the update",
                sidecar.name()
            ));
        }
    }
    Ok(())
}

/// Removes a staged record's whole work directory (zip + staging trees).
fn discard_staged(staged: &StagedRecord) {
    let _ = fs::remove_dir_all(&staged.work_dir);
}

/// Atomically installs a previously staged update and restarts the app.
///
/// Fail-closed on a running engine or an active sidecar: refuses with
/// `ENGINE_RUNNING` / `SIDECAR_RUNNING` unless everything is fully stopped,
/// so an update can never silently drop live connections or rename a
/// directory a sidecar still executes from.
///
/// TOCTOU hardening: the tree left in %TEMP% by `download_update` is NOT
/// trusted. The recorded zip is re-hashed against the recorded digest, and
/// only then re-extracted into a *fresh* directory that is swapped into
/// place. Any tampering between download and install ends in a clean refusal.
///
/// Also refuses to install a staged version that is not newer than the
/// running one (defense in depth — the UI already blocks downgrades).
///
/// Runs on a blocking thread (see the `install_update` command): hashing,
/// extraction and the cross-volume copy fallback are disk-heavy. On success
/// this function spawns the new executable and terminates the current
/// process — it never returns `Ok`.
pub fn install_update(
    engine_state: SupervisorState,
    sidecars: &[Arc<dyn ManagedSidecar>],
) -> Result<(), String> {
    check_engines_stopped(engine_state, sidecars)?;

    // Stop a running svc before the file swap: its exe cannot be replaced
    // while the process is alive, and an old svc would otherwise keep
    // serving the new GUI from the pipe (version skew).
    crate::svc_client::shutdown_svc_blocking();

    let staged = STAGED_UPDATE
        .write()
        .take()
        .ok_or_else(|| "No staged update found — download the update first".to_string())?;

    // Defense in depth: never install a version that is not newer than the
    // running one, no matter what the frontend claims.
    let current = env!("CARGO_PKG_VERSION");
    match (parse_semver(current), parse_semver(&staged.tag)) {
        (Some(cur), Some(lat)) if lat > cur => {}
        _ => {
            discard_staged(&staged);
            return Err(format!(
                "STAGED_VERSION_INVALID: staged version {} is not newer than the running {}",
                staged.tag, current
            ));
        }
    }

    // Re-hash the zip: the %TEMP% tree is untrusted between download and
    // install, so the recorded digest is checked again *now*.
    let actual_hex =
        sha256_file(&staged.zip_path).map_err(|e| {
            discard_staged(&staged);
            e
        })?;
    if actual_hex != staged.expected_sha256 {
        discard_staged(&staged);
        return Err(format!(
            "STAGED_TAMPERED: staged archive hash mismatch (expected {}, got {}). \
             The staged files were deleted; the update was NOT installed.",
            staged.expected_sha256, actual_hex
        ));
    }

    // Re-extract into a fresh directory — never reuse the %TEMP% tree that
    // sat there, potentially tampered with, since the download.
    let fresh_staging = staged.work_dir.join("install-staging");
    if fresh_staging.exists() {
        let _ = fs::remove_dir_all(&fresh_staging);
    }
    fs::create_dir_all(&fresh_staging)
        .map_err(|e| format!("Cannot create staging dir: {e}"))?;
    if let Err(e) = extract_zip(&staged.zip_path, &fresh_staging) {
        discard_staged(&staged);
        return Err(e);
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
    let staged_exe = [
        fresh_staging.join(exe_file_name),
        fresh_staging.join("zapret2.exe"),
    ]
    .into_iter()
    .find(|p| p.is_file())
    .ok_or_else(|| {
        discard_staged(&staged);
        "Staged update does not contain the application executable".to_string()
    })?;
    let staged_exe_name = staged_exe
        .file_name()
        .ok_or_else(|| "Cannot resolve staged executable file name".to_string())?
        .to_os_string();

    // Narrow the TOCTOU window on the engine state: re-check immediately
    // before the first rename of the swap.
    check_engines_stopped(engine_state, sidecars)?;

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
    //    move_dir falls back to copy+delete across volumes.
    if let Err(e) = move_dir(&install_dir, &backup_dir) {
        discard_staged(&staged);
        return Err(format!("Failed to back up current installation: {e}"));
    }

    // 2. Staging -> installation. Any failure from here on rolls back.
    if let Err(e) = move_dir(&fresh_staging, &install_dir) {
        rollback_swap(&install_dir, &backup_dir, &fresh_staging);
        discard_staged(&staged);
        return Err(format!("Failed to install update (rolled back): {e}"));
    }

    // 3. Restore user data into the fresh tree.
    if let Err(e) = restore_user_data(&backup_dir, &install_dir) {
        rollback_swap(&install_dir, &backup_dir, &fresh_staging);
        discard_staged(&staged);
        return Err(format!(
            "Failed to restore user settings into the update (rolled back): {e}"
        ));
    }

    // 4. Launch the new version (inherits the elevated token — no new UAC
    //    prompt) and terminate this process.
    let new_exe = install_dir.join(&staged_exe_name);
    if let Err(e) = std::process::Command::new(&new_exe).spawn() {
        rollback_swap(&install_dir, &backup_dir, &fresh_staging);
        discard_staged(&staged);
        return Err(format!(
            "Update installed but the new version failed to launch (rolled back): {e}"
        ));
    }
    // The staged work dir is left for the next download's stale cleanup;
    // %TEMP% is also routinely cleared by Windows itself.
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

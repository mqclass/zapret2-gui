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

use std::collections::HashSet;
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

/// Directories inside an installation (or backup) tree that can hold
/// user-owned list files, in priority order.
///
/// The lists the running program actually reads and writes live next to
/// the engine binaries: `resolve_binaries` anchors at `<exe_dir>/bin`,
/// so the working directory is `bin/lists` (that is where the supervisor
/// passes `--hostlist`/`--hostlist-auto` paths from and where the list
/// editor writes). The tree-root `lists/` is a mirror the release zip
/// ships; in a tree backed up by an older version it may be the only
/// place user files were ever restored to, so it stays as a fallback
/// source — but never as the restore target.
fn user_list_source_dirs(tree: &Path) -> Vec<PathBuf> {
    vec![tree.join("bin").join("lists"), tree.join("lists")]
}

/// The directory user list files are restored INTO: the working lists
/// directory of the fresh installation (see `user_list_source_dirs`).
fn user_list_dest_dir(install: &Path) -> PathBuf {
    install.join("bin").join("lists")
}

/// Whether `name` is a user-owned list file: personal domain additions
/// follow the `*-user.txt` convention, and the learned auto-hostlist
/// (`list-auto.txt`) is user data in the same sense.
fn is_user_list_file(name: &str) -> bool {
    name.ends_with("-user.txt") || name == "list-auto.txt"
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
    //    The learned auto-hostlist (list-auto.txt) is user data in the
    //    same sense and is carried over too: without it an update
    //    would forget everything the engine learned about the user's
    //    provider (auto-hostlist design §9 step 3).
    //
    //    Both the source and the destination are the *working* lists
    //    directory (`bin/lists`): that is the directory the new version
    //    will actually read. The backup's tree-root `lists/` is scanned
    //    afterwards as a fallback for backups written by older versions,
    //    but a file already taken from `bin/lists` wins over its
    //    same-named root copy.
    let install_lists = user_list_dest_dir(install);
    let mut restored: HashSet<String> = HashSet::new();
    for backup_lists in user_list_source_dirs(backup) {
        if !backup_lists.is_dir() {
            continue;
        }
        let entries = fs::read_dir(&backup_lists)
            .map_err(|e| format!("Cannot read {}: {e}", backup_lists.display()))?;
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy().into_owned();
            if !is_user_list_file(&name_str) || !entry.path().is_file() {
                continue;
            }
            if !restored.insert(name_str.clone()) {
                continue;
            }
            let dst = install_lists.join(&name);
            if let Some(parent) = dst.parent() {
                let _ = fs::create_dir_all(parent);
            }
            fs::copy(entry.path(), &dst)
                .map_err(|e| format!("Cannot restore {}: {e}", name_str))?;
        }
    }
    Ok(())
}

/// The single backup slot next to the installation directory:
/// `<install name>.bak`. The slot holds the previous installation tree
/// verbatim (the swap renames the whole tree into it) and is cleared
/// only at the start of the NEXT update — so between an update and its
/// successor, the previous version's files still live there.
fn backup_slot_dir(install_dir: &Path) -> PathBuf {
    let backup_name = match install_dir.file_name() {
        Some(n) => format!("{}.bak", n.to_string_lossy()),
        None => "zapret2.bak".to_string(),
    };
    install_dir.with_file_name(backup_name)
}

/// One-shot repair for the 1.2.1 -> 1.3.0 transition.
///
/// `install_update` is a command of the *running* (old) app, so that
/// particular upgrade is performed by 1.2.1's defective restore, which
/// restored user lists into the tree-root `lists/` nobody reads and
/// read them from there too — the live `*-user.txt` files were never
/// carried into the new `bin/lists`. The fixed restore protects
/// upgrades starting from 1.3.0, but not this one. The files are not
/// lost, though: the backup slot is the old tree renamed wholesale, so
/// the live copies still sit in `backup/bin/lists`. On startup, copy
/// any user list file that the new working directory lacks from the
/// backup slot into it.
///
/// Existing working files are NEVER overwritten — the working copy is
/// by definition at least as fresh as the backup one. That same rule
/// is what makes the repair idempotent without a "done" flag: once a
/// file has been rescued it exists in the working directory, so every
/// later launch finds nothing to do. A missing backup slot (fresh
/// installs, manual zip updates) is a quiet no-op, as is any I/O
/// error — this runs on the startup path and must never block it.
pub fn rescue_user_lists_from_backup(install_dir: &Path, backup_dir: &Path) -> usize {
    if !backup_dir.is_dir() {
        return 0;
    }
    let dest = user_list_dest_dir(install_dir);
    let mut rescued = 0;
    for backup_lists in user_list_source_dirs(backup_dir) {
        if !backup_lists.is_dir() {
            continue;
        }
        let Ok(entries) = fs::read_dir(&backup_lists) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy().into_owned();
            if !is_user_list_file(&name_str) || !entry.path().is_file() {
                continue;
            }
            // Sources are visited in priority order; after a successful
            // copy the destination exists, so a same-named file from a
            // lower-priority source is skipped by this very check (and
            // a failed copy simply falls through to the next source).
            let dst = dest.join(&name);
            if dst.exists() {
                continue;
            }
            if let Some(parent) = dst.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if fs::copy(entry.path(), &dst).is_ok() {
                rescued += 1;
            }
        }
    }
    rescued
}

/// Startup entry point for `rescue_user_lists_from_backup`: resolves
/// this installation and its backup slot from the running executable,
/// mirroring how `install_update` derives them. Best-effort by design
/// (see the repair's docs) — failures are logged, never propagated.
pub fn rescue_user_lists_at_startup() {
    let Some(install_dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
    else {
        return;
    };
    let backup_dir = backup_slot_dir(&install_dir);
    let rescued = rescue_user_lists_from_backup(&install_dir, &backup_dir);
    if rescued > 0 {
        eprintln!(
            "Rescued {rescued} user list file(s) from the pre-update backup {}",
            backup_dir.display()
        );
    }
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

    // Stop a running svc before the file swap: its exe cannot be replaced
    // while the process is alive, and an old svc would otherwise keep
    // serving the new GUI from the pipe (version skew). This happens only
    // after every validation above has passed, so a refused install never
    // kills a healthy svc (and the bypass it serves) for nothing.
    crate::svc_client::shutdown_svc_blocking();

    let backup_dir = backup_slot_dir(&install_dir);

    // Single backup slot: drop the previous one so a stale backup can never
    // be mistaken for the current version during manual recovery.
    if backup_dir.exists() {
        fs::remove_dir_all(&backup_dir)
            .map_err(|e| format!("Cannot clear previous backup {}: {e}", backup_dir.display()))?;
    }

    // 1-4. Hand the swap itself to the staged binary running as a
    //    detached helper (--apply-update, see run_update_helper). This
    //    process cannot move its own installation directory: launched
    //    by double-click it stands *in* that directory (it is the
    //    process's working directory), and Windows refuses to move a
    //    directory a live process stands in — os error 32, the exact
    //    failure users hit updating to 1.3.1 ("Failed to back up
    //    current installation: Cannot move ..."). The helper lives in
    //    %TEMP%, waits for this process to exit, then performs the
    //    backup, the swap, the user-data restore and the relaunch of
    //    the new version (inheriting the elevated token — no new UAC
    //    prompt).
    let plan = UpdatePlan {
        install_dir: install_dir.clone(),
        backup_dir: backup_dir.clone(),
        staging_dir: fresh_staging.clone(),
        staged_exe_name: staged_exe_name.to_string_lossy().into_owned(),
        current_exe_name: exe_file_name.to_string_lossy().into_owned(),
    };
    let plan_path = std::env::temp_dir().join("zapret2-update-plan.json");
    let plan_json = match serde_json::to_string(&plan) {
        Ok(j) => j,
        Err(e) => {
            discard_staged(&staged);
            return Err(format!("Cannot serialize update plan: {e}"));
        }
    };
    if let Err(e) = fs::write(&plan_path, plan_json) {
        discard_staged(&staged);
        return Err(format!("Cannot write update plan: {e}"));
    }
    if let Err(e) = std::process::Command::new(&staged_exe)
        .arg("--apply-update")
        .arg(&plan_path)
        .spawn()
    {
        discard_staged(&staged);
        return Err(format!("Cannot start the update helper: {e}"));
    }
    // The helper takes over from here; leaving immediately is what
    // releases the installation directory.
    std::process::exit(0);
}

/// Handoff plan for the update helper: everything the detached staged
/// binary needs to perform the swap after this process has exited.
#[derive(serde::Serialize, serde::Deserialize)]
struct UpdatePlan {
    install_dir: PathBuf,
    backup_dir: PathBuf,
    staging_dir: PathBuf,
    staged_exe_name: String,
    current_exe_name: String,
}

/// Entry point for `--apply-update <plan.json>`, run by the staged
/// binary out of %TEMP% (see main). Performs the swap the GUI could
/// not do itself: backup move (retried until the handed-off GUI has
/// exited and released the install dir), staging move, user-data
/// restore, relaunch — with rollback to the previous version on any
/// failure. Progress is logged to %TEMP%/zapret2-update.log.
pub fn run_update_helper(plan_path: &Path) -> i32 {
    // Stand in neither the install nor the staging directory: a
    // process blocks moving its own working directory.
    let _ = std::env::set_current_dir(std::env::temp_dir());
    let log_path = std::env::temp_dir().join("zapret2-update.log");
    let log = |msg: &str| {
        eprintln!("[updater-helper] {msg}");
        if let Ok(mut f) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            use std::io::Write;
            let _ = writeln!(f, "{msg}");
        }
    };
    let raw = match fs::read_to_string(plan_path) {
        Ok(r) => r,
        Err(e) => {
            log(&format!("cannot read plan: {e}"));
            return 1;
        }
    };
    let plan: UpdatePlan = match serde_json::from_str(&raw) {
        Ok(p) => p,
        Err(e) => {
            log(&format!("cannot parse plan: {e}"));
            return 1;
        }
    };
    let relaunch_old = |plan: &UpdatePlan| {
        let old_exe = plan.install_dir.join(&plan.current_exe_name);
        if let Err(e) = std::process::Command::new(&old_exe).spawn() {
            log(&format!("cannot relaunch previous version: {e}"));
        }
    };
    // Self-heal: a previous helper died after the backup move but
    // before the swap — the install dir is gone and the backup holds
    // the only copy. Put it back and relaunch; the user can retry.
    if !plan.install_dir.exists() && plan.backup_dir.exists() {
        log("install dir missing, backup present: restoring previous installation");
        if move_dir(&plan.backup_dir, &plan.install_dir).is_ok() {
            relaunch_old(&plan);
        }
        return 1;
    }
    // The backup move only succeeds once the handed-off GUI has exited
    // and released the install dir; retry for up to ~60 s.
    let mut backed_up = false;
    let mut last_err = String::new();
    for _ in 0..120 {
        match move_dir(&plan.install_dir, &plan.backup_dir) {
            Ok(()) => {
                backed_up = true;
                break;
            }
            Err(e) => {
                last_err = e;
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        }
    }
    if !backed_up {
        log(&format!("backup move never succeeded: {last_err}"));
        relaunch_old(&plan);
        return 1;
    }
    if let Err(e) = move_dir(&plan.staging_dir, &plan.install_dir) {
        log(&format!("swap failed, rolling back: {e}"));
        rollback_swap(&plan.install_dir, &plan.backup_dir, &plan.staging_dir);
        relaunch_old(&plan);
        return 1;
    }
    if let Err(e) = restore_user_data(&plan.backup_dir, &plan.install_dir) {
        log(&format!("user-data restore failed, rolling back: {e}"));
        rollback_swap(&plan.install_dir, &plan.backup_dir, &plan.staging_dir);
        relaunch_old(&plan);
        return 1;
    }
    let new_exe = plan.install_dir.join(&plan.staged_exe_name);
    match std::process::Command::new(&new_exe).spawn() {
        Ok(_) => {
            log("update applied, new version launched");
            0
        }
        Err(e) => {
            log(&format!("new version failed to launch, rolling back: {e}"));
            rollback_swap(&plan.install_dir, &plan.backup_dir, &plan.staging_dir);
            relaunch_old(&plan);
            1
        }
    }
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
    use super::{
        backup_slot_dir, is_user_list_file, parse_semver, rescue_user_lists_from_backup,
        restore_user_data, user_list_dest_dir, user_list_source_dirs,
    };
    use std::fs;
    use std::path::Path;

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

    #[test]
    fn user_list_dirs_target_the_working_bin_lists() {
        let tree = Path::new("tree");
        // bin/lists is the working directory (resolve_binaries anchors
        // there) and must be both the restore target and the first
        // source; the tree-root lists/ is only a fallback source for
        // backups written by older versions.
        assert_eq!(user_list_dest_dir(tree), tree.join("bin").join("lists"));
        assert_eq!(
            user_list_source_dirs(tree),
            vec![tree.join("bin").join("lists"), tree.join("lists")]
        );
    }

    #[test]
    fn user_list_file_names_match_the_convention() {
        assert!(is_user_list_file("list-general-user.txt"));
        assert!(is_user_list_file("list-exclude-user.txt"));
        assert!(is_user_list_file("ipset-exclude-user.txt"));
        assert!(is_user_list_file("list-auto.txt"));
        assert!(!is_user_list_file("list-general.txt"));
        assert!(!is_user_list_file("list-auto.txt.bak"));
        assert!(!is_user_list_file("user.txt"));
    }

    #[test]
    fn restore_user_data_prefers_bin_lists_and_keeps_auto() {
        let base = std::env::temp_dir().join(format!(
            "zapret2-updater-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        let backup = base.join("backup");
        let install = base.join("install");
        // The live user file sits in the working bin/lists; a stale
        // namesake sits in the root lists of the same backup.
        fs::create_dir_all(backup.join("bin/lists")).unwrap();
        fs::create_dir_all(backup.join("lists")).unwrap();
        fs::write(
            backup.join("bin/lists/list-general-user.txt"),
            "live.example\r\n",
        )
        .unwrap();
        fs::write(
            backup.join("lists/list-general-user.txt"),
            "stale.example\r\n",
        )
        .unwrap();
        // An old-layout backup may hold the auto-hostlist only in the
        // root lists — the fallback source must still pick it up.
        fs::write(backup.join("lists/list-auto.txt"), "learned.example\r\n").unwrap();
        // A shipped (non-user) list must NOT be carried over.
        fs::write(backup.join("bin/lists/list-general.txt"), "shipped\r\n").unwrap();
        fs::create_dir_all(&install).unwrap();

        restore_user_data(&backup, &install).unwrap();

        let dest = install.join("bin/lists");
        assert_eq!(
            fs::read_to_string(dest.join("list-general-user.txt")).unwrap(),
            "live.example\r\n",
            "the bin/lists copy must win over the root-lists namesake"
        );
        assert_eq!(
            fs::read_to_string(dest.join("list-auto.txt")).unwrap(),
            "learned.example\r\n",
            "the fallback source must restore list-auto.txt"
        );
        assert!(!dest.join("list-general.txt").exists());
        assert!(
            !install.join("lists").exists(),
            "the new tree's root lists must stay untouched"
        );
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn backup_slot_dir_appends_bak_to_install_name() {
        let install = Path::new("apps").join("zapret2");
        assert_eq!(
            backup_slot_dir(&install),
            Path::new("apps").join("zapret2.bak")
        );
    }

    #[test]
    fn rescue_copies_missing_user_lists_from_backup_bin_lists() {
        let base = std::env::temp_dir().join(format!(
            "zapret2-updater-test-{}-rescue-a",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        let backup = base.join("backup");
        let install = base.join("install");
        // The stranded live user files sit in the backup's working
        // lists dir; a stale namesake sits in its root lists.
        fs::create_dir_all(backup.join("bin/lists")).unwrap();
        fs::create_dir_all(backup.join("lists")).unwrap();
        fs::write(
            backup.join("bin/lists/list-general-user.txt"),
            "live.example\r\n",
        )
        .unwrap();
        fs::write(
            backup.join("lists/list-general-user.txt"),
            "stale.example\r\n",
        )
        .unwrap();
        fs::write(backup.join("bin/lists/list-exclude-user.txt"), "x.example\r\n").unwrap();
        // The fresh install ships an empty working lists dir.
        fs::create_dir_all(install.join("bin/lists")).unwrap();

        let rescued = rescue_user_lists_from_backup(&install, &backup);

        assert_eq!(rescued, 2);
        let dest = install.join("bin/lists");
        assert_eq!(
            fs::read_to_string(dest.join("list-general-user.txt")).unwrap(),
            "live.example\r\n",
            "the bin/lists copy must win over the root-lists namesake"
        );
        assert_eq!(
            fs::read_to_string(dest.join("list-exclude-user.txt")).unwrap(),
            "x.example\r\n"
        );
        // Idempotency: a second launch finds everything in place.
        assert_eq!(rescue_user_lists_from_backup(&install, &backup), 0);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn rescue_never_overwrites_existing_working_files() {
        let base = std::env::temp_dir().join(format!(
            "zapret2-updater-test-{}-rescue-b",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        let backup = base.join("backup");
        let install = base.join("install");
        fs::create_dir_all(backup.join("bin/lists")).unwrap();
        fs::write(
            backup.join("bin/lists/list-general-user.txt"),
            "old.example\r\n",
        )
        .unwrap();
        // The working copy is fresher than the backup one and must win.
        fs::create_dir_all(install.join("bin/lists")).unwrap();
        fs::write(
            install.join("bin/lists/list-general-user.txt"),
            "fresh.example\r\n",
        )
        .unwrap();

        let rescued = rescue_user_lists_from_backup(&install, &backup);

        assert_eq!(rescued, 0);
        assert_eq!(
            fs::read_to_string(install.join("bin/lists/list-general-user.txt")).unwrap(),
            "fresh.example\r\n"
        );
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn rescue_falls_back_to_backup_root_lists() {
        let base = std::env::temp_dir().join(format!(
            "zapret2-updater-test-{}-rescue-c",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        let backup = base.join("backup");
        let install = base.join("install");
        // An old-layout backup holds the learned auto-hostlist only in
        // the tree-root lists.
        fs::create_dir_all(backup.join("lists")).unwrap();
        fs::write(backup.join("lists/list-auto.txt"), "learned.example\r\n").unwrap();
        fs::create_dir_all(&install).unwrap();

        let rescued = rescue_user_lists_from_backup(&install, &backup);

        assert_eq!(rescued, 1);
        assert_eq!(
            fs::read_to_string(install.join("bin/lists/list-auto.txt")).unwrap(),
            "learned.example\r\n"
        );
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn rescue_without_backup_is_a_quiet_noop() {
        let base = std::env::temp_dir().join(format!(
            "zapret2-updater-test-{}-rescue-d",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        let install = base.join("install");
        fs::create_dir_all(install.join("bin/lists")).unwrap();

        // No backup slot at all (fresh install / manual zip update).
        assert_eq!(
            rescue_user_lists_from_backup(&install, &base.join("backup")),
            0
        );
        assert!(fs::read_dir(install.join("bin/lists"))
            .unwrap()
            .next()
            .is_none());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn rescue_ignores_shipped_lists() {
        let base = std::env::temp_dir().join(format!(
            "zapret2-updater-test-{}-rescue-e",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        let backup = base.join("backup");
        let install = base.join("install");
        fs::create_dir_all(backup.join("bin/lists")).unwrap();
        fs::write(backup.join("bin/lists/list-general.txt"), "old-shipped\r\n").unwrap();
        fs::write(backup.join("bin/lists/ipset-all.txt"), "old-shipped\r\n").unwrap();
        // The fresh install's factory lists must stay exactly as shipped.
        fs::create_dir_all(install.join("bin/lists")).unwrap();
        fs::write(install.join("bin/lists/list-general.txt"), "new-shipped\r\n").unwrap();

        let rescued = rescue_user_lists_from_backup(&install, &backup);

        assert_eq!(rescued, 0);
        let dest = install.join("bin/lists");
        assert_eq!(
            fs::read_to_string(dest.join("list-general.txt")).unwrap(),
            "new-shipped\r\n"
        );
        assert!(!dest.join("ipset-all.txt").exists());
        let _ = fs::remove_dir_all(&base);
    }
}

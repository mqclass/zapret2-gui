//! Telegram MTProxy-mode sidecar: supervised lifecycle for the official
//! Flowseal `TgWsProxy_windows.exe` binary.
//!
//! # What this is
//! `tg-ws-proxy` is a *local* MTProto bridge: the Telegram client connects to
//! it exactly like to a regular MTProto proxy (host `127.0.0.1`, port `1443`,
//! 32-hex-char `secret`), and the proxy itself reaches Telegram datacenters
//! over WebSocket / Cloudflare fallbacks (default DC IPs are baked into the
//! proxy). There is NO external MTProxy server involved and the user does NOT
//! need to supply any upstream credentials — the only three values the
//! Telegram client needs are shown in the GUI card.
//!
//! # Trust & integrity model
//! - The binary is downloaded ONLY from the official Flowseal GitHub releases
//!   (`https://api.github.com/repos/Flowseal/tg-ws-proxy/releases/latest`).
//! - The asset is located strictly by file name (`TgWsProxy_windows.exe`);
//!   anything else in the release is ignored.
//! - The `digest` field of the asset metadata is MANDATORY and must look like
//!   `sha256:<64 hex chars>`; without it the download is refused.
//! - Pinned digests (`PINNED_RELEASE_DIGESTS`): the API-served digest must
//!   additionally match the pin for that exact version. A silently re-uploaded
//!   release or a new upstream version is refused until the pin is consciously
//!   updated in code (defense in depth against a compromised Flowseal
//!   account/pipeline — the digest alone only proves "bytes == what the API
//!   currently serves").
//! - The file is streamed into a temporary file while being hashed (same
//!   mechanics as upstream `utils/update_check.py::download_asset`); the hash
//!   is compared BEFORE the file is moved into place; on mismatch the temp
//!   file is deleted.
//! - A `manifest.json` next to the binary records asset name, release version
//!   and SHA-256; the hash is re-verified on every start.
//! - De-elevation (`win32::deelevate`): the GUI runs elevated (WinDivert), but
//!   the sidecar only listens on `127.0.0.1:1443`, so it is spawned with the
//!   interactive user's token via `CreateProcessAsUserW` — never with the
//!   Administrator token. Spawning refuses to fall back to elevated on error
//!   (fail-closed).
//!
//! # Process model
//! The published `TgWsProxy_windows.exe` is a PyInstaller tray GUI application
//! (not a headless CLI daemon): it keeps its own `config.json`, writes logs to
//! `proxy.log`, shows its own tray icon and enforces a single-instance mutex
//! (upstream name is `Local\\TgWsProxy_SingleInstance` — two backslashes —
//! see `standalone_instance_running`). We run it in `--portable` mode so all of
//! its state lives in `bin/tgproxy/TgWsProxy_data/` (isolated from any
//! standalone install), the first-run window is suppressed via the
//! `.first_run_done_mtproto` marker, and its own update dialogs are disabled
//! (`check_updates: false`) because updates are managed by zapret2-gui. The
//! child is spawned with `CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP` and
//! immediately assigned to a dedicated Windows Job Object with
//! `KILL_ON_JOB_CLOSE`, mirroring winws2. Because the exe has no console,
//! stdout/stderr carry nothing — logs are tailed from `proxy.log` and
//! forwarded into the supervisor's shared log buffer with level `"tgproxy"`.
//!
//! # Version drift
//! Four config keys are written (`host`, `port`, `secret`, plus
//! `check_updates: false` which silences the tray app's own updater — updates
//! are managed by zapret2-gui); everything else is filled in by the exe itself
//! from its defaults. The downloaded version is
//! recorded in the manifest and compared against `TGWS_MAX_TESTED_VERSION`;
//! a newer-than-tested version raises a visible warning in the GUI instead of
//! silently misbehaving.

use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::os::windows::io::RawHandle;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use parking_lot::{Mutex, RwLock};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::engine::supervisor::{Supervisor, SupervisorState};
use crate::win32::deelevate::RawChild;
use crate::win32::job_object::JobObject;

/// GitHub Releases API endpoint for the official tg-ws-proxy project.
const TGWS_RELEASES_API: &str = "https://api.github.com/repos/Flowseal/tg-ws-proxy/releases/latest";
/// Exact asset name we accept from a release. Anything else is ignored.
const TGWS_ASSET_NAME: &str = "TgWsProxy_windows.exe";
/// Newest upstream version whose config-file contract was verified by hand.
/// Newer versions still work, but the GUI shows a drift warning.
const TGWS_MAX_TESTED_VERSION: &str = "1.11.2";

/// Pinned SHA-256 digests of reviewed Flowseal releases: `(version, sha256)`.
///
/// Trust-on-first-use, recorded 2026-10-16 from the release API metadata —
/// cross-check against an independent source before extending. The
/// API-served digest must match the pin for that exact version; a silently
/// re-uploaded release (same version, different bytes) or a brand-new
/// upstream version is REFUSED until the pin is consciously updated here.
/// This is defense in depth against a compromised Flowseal account or release
/// pipeline: the `digest` field alone only proves "bytes == what the API
/// currently serves".
const PINNED_RELEASE_DIGESTS: &[(&str, &str)] = &[
    // v1.11.2, from https://api.github.com/repos/Flowseal/tg-ws-proxy/releases/latest
    (
        "1.11.2",
        "54acaa989858dd0f90eca3d18a93125fac3d8f702fc25cff8900d6098f725853",
    ),
];

const TGWS_DEFAULT_HOST: &str = "127.0.0.1";
const TGWS_DEFAULT_PORT: u16 = 1443;
const TGWS_DATA_DIR_NAME: &str = "TgWsProxy_data";
const TGWS_CONFIG_FILE: &str = "config.json";
const TGWS_LOG_FILE: &str = "proxy.log";
const TGWS_FIRST_RUN_MARKER: &str = ".first_run_done_mtproto";
const TGWS_MANIFEST_FILE: &str = "manifest.json";

/// How long we wait for the tray process to open its listen port.
const STARTUP_HEALTH_TIMEOUT: Duration = Duration::from_secs(8);
/// Interval of the proxy.log tail reader.
const LOG_TAIL_INTERVAL: Duration = Duration::from_secs(1);

/// Integrity + provenance record written next to the downloaded binary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TgProxyManifest {
    pub asset: String,
    pub version: String,
    pub sha256: String,
    pub downloaded_at: u64,
}

/// Snapshot of the sidecar for UI rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TgProxyStatus {
    pub state: SupervisorState,
    pub pid: Option<u32>,
    pub host: String,
    pub port: u16,
    /// 32-hex-char MTProto secret; empty when not generated yet.
    pub secret: String,
    /// Ready-to-use `tg://proxy?...` deep link (empty when no secret yet).
    pub tg_link: String,
    pub binary_present: bool,
    pub binary_version: Option<String>,
    /// False when the downloaded version is newer than `TGWS_MAX_TESTED_VERSION`.
    pub version_supported: bool,
    pub uptime_secs: u64,
    pub last_error: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum TgProxyError {
    #[error("Job Object error: {0}")]
    Job(#[from] crate::win32::job_object::JobObjectError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Не удалось сохранить настройки: {0}")]
    Settings(String),
    #[error("Не удалось получить данные релиза с GitHub: {0}")]
    ReleaseFetch(String),
    #[error("В последнем релизе Flowseal/tg-ws-proxy нет ассета TgWsProxy_windows.exe")]
    AssetNotFound,
    #[error("У ассета релиза нет валидного SHA-256 digest — скачивание отклонено")]
    MissingDigest,
    #[error("SHA-256 скачанного файла не совпал с digest релиза — временный файл удалён")]
    DigestMismatch,
    #[error("Бинарник Telegram-прокси не скачан. Откройте вкладку «Telegram» и нажмите «Скачать».")]
    NotDownloaded,
    #[error("Целостность бинарника нарушена (SHA-256 не совпал с manifest). Перекачайте прокси.")]
    IntegrityFailed,
    #[error("Порт {0} уже занят — возможно, запущен отдельный TgWsProxy. Закройте его или смените порт.")]
    PortBusy(u16),
    #[error("Обнаружен запущенный отдельный TgWsProxy (single-instance). Закройте его, прежде чем запускать прокси из zapret2-gui.")]
    StandaloneRunning,
    #[error("Прокси не открыл порт {0} за {1} сек. Хвост лога: {2}")]
    StartupTimeout(u16, u64, String),
    #[error("Процесс прокси завершился сразу после запуска ({0}). Возможно, уже запущен отдельный TgWsProxy — у него single-instance mutex.")]
    EarlyExit(String),
    #[error("Некорректный secret: нужно ровно 32 hex-символа (0-9, a-f)")]
    BadSecret,
    #[error("Некорректный порт: {0}")]
    BadPort(u16),
    #[error("Остановите прокси перед изменением настроек")]
    MustStopFirst,
    #[error("Не удалось запустить процесс: {0}")]
    Spawn(String),
    #[error("Не удалось запустить прокси без прав администратора: {0}")]
    Deelevate(String),
    #[error("Версия {0} tg-ws-proxy не закреплена (pin) в коде — скачивание отклонено. Обновление до новой версии upstream — осознанное действие: добавьте её SHA-256 в PINNED_RELEASE_DIGESTS после ручной проверки")]
    VersionNotPinned(String),
    #[error("Digest версии {0} не совпал с закреплённым в коде — возможно, релиз был тихо перезалит. Скачивание отклонено")]
    PinnedDigestMismatch(String),
}

/// Manages the TgWsProxy sidecar process lifecycle.
pub struct TgProxyManager {
    /// Serializes start()/stop() so they can never interleave.
    lifecycle: Arc<tokio::sync::Mutex<()>>,
    state: Arc<RwLock<SupervisorState>>,
    active_pid: Arc<RwLock<Option<u32>>>,
    active_child: Arc<Mutex<Option<RawChild>>>,
    job_object: Arc<JobObject>,
    start_time: Arc<RwLock<Option<Instant>>>,
    last_error: Arc<RwLock<Option<String>>>,
    /// Byte offset up to which proxy.log has already been forwarded.
    log_offset: Arc<Mutex<u64>>,
    app_handle: Arc<RwLock<Option<AppHandle>>>,
    /// Shared supervisor: the single funnel into the common log buffer.
    supervisor: Arc<Supervisor>,
}

/// [`ManagedSidecar`] integration: the self-update pipeline renames the
/// whole installation directory, so it must fail closed while tgproxy
/// still runs from inside it.
impl crate::engine::managed::ManagedSidecar for TgProxyManager {
    fn name(&self) -> &'static str {
        "tgproxy"
    }

    fn is_active(&self) -> bool {
        // Any non-stopped state counts as active: the child may still hold
        // files inside the installation directory.
        *self.state.read() != SupervisorState::Stopped
    }
}

impl TgProxyManager {
    pub fn new(supervisor: Arc<Supervisor>) -> Result<Self, TgProxyError> {
        Ok(Self {
            lifecycle: Arc::new(tokio::sync::Mutex::new(())),
            state: Arc::new(RwLock::new(SupervisorState::Stopped)),
            active_pid: Arc::new(RwLock::new(None)),
            active_child: Arc::new(Mutex::new(None)),
            job_object: Arc::new(JobObject::create()?),
            start_time: Arc::new(RwLock::new(None)),
            last_error: Arc::new(RwLock::new(None)),
            log_offset: Arc::new(Mutex::new(0)),
            app_handle: Arc::new(RwLock::new(None)),
            supervisor,
        })
    }

    /// Sets the Tauri application handle for emitting frontend events.
    pub fn set_app_handle(&self, handle: AppHandle) {
        *self.app_handle.write() = Some(handle);
    }

    fn emit_state(&self, state: SupervisorState) {
        if let Some(handle) = self.app_handle.read().as_ref() {
            let _ = handle.emit("tgproxy-state", state);
        }
    }

    /// Best-effort download progress for the UI (T12): emits the
    /// `tgproxy-download-progress` event with 0–100 (percent only advances
    /// when the server reports a content length).
    fn emit_download_progress(&self, percent: u64) {
        if let Some(handle) = self.app_handle.read().as_ref() {
            let _ = handle.emit("tgproxy-download-progress", percent);
        }
    }

    // ------------------------------------------------------------------
    // Paths
    // ------------------------------------------------------------------

    /// Candidate directories for `bin/tgproxy`, anchored at the executable
    /// exactly like `win32::process::resolve_binaries` (never CWD, except an
    /// explicit `--dev` developer workflow).
    fn dir_candidates() -> Vec<PathBuf> {
        let mut out = Vec::new();
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                out.push(parent.join("bin").join("tgproxy"));
            }
        }
        if std::env::args().any(|a| a == "--dev") {
            if let Ok(cwd) = std::env::current_dir() {
                out.push(cwd.join("bin").join("tgproxy"));
            }
        }
        out
    }

    fn primary_dir() -> Result<PathBuf, TgProxyError> {
        Self::dir_candidates().into_iter().next().ok_or_else(|| {
            TgProxyError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "cannot anchor tgproxy directory at executable location",
            ))
        })
    }

    /// Directory that actually contains the binary (if downloaded).
    fn locate_dir() -> Option<PathBuf> {
        Self::dir_candidates()
            .into_iter()
            .find(|d| d.join(TGWS_ASSET_NAME).is_file())
    }

    fn ensure_dir() -> Result<PathBuf, TgProxyError> {
        let dir = Self::primary_dir()?;
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    fn data_dir(dir: &Path) -> PathBuf {
        dir.join(TGWS_DATA_DIR_NAME)
    }

    fn log_file_path(dir: &Path) -> PathBuf {
        Self::data_dir(dir).join(TGWS_LOG_FILE)
    }

    fn read_manifest(dir: &Path) -> Option<TgProxyManifest> {
        let raw = std::fs::read_to_string(dir.join(TGWS_MANIFEST_FILE)).ok()?;
        serde_json::from_str(&raw).ok()
    }

    // ------------------------------------------------------------------
    // Small pure helpers
    // ------------------------------------------------------------------

    /// The `digest` asset field must look like `sha256:<64 hex chars>`.
    fn parse_sha256_digest(raw: &str) -> Option<String> {
        let raw = raw.trim();
        let hex = raw
            .strip_prefix("sha256:")
            .or_else(|| raw.strip_prefix("SHA256:"))?;
        let hex = hex.trim().to_lowercase();
        if hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            Some(hex)
        } else {
            None
        }
    }

    fn is_valid_secret(s: &str) -> bool {
        s.len() == 32 && s.chars().all(|c| c.is_ascii_hexdigit())
    }

    /// Enforces the digest pin table (M-3): the API-served digest must match
    /// the pin recorded for that exact version.
    fn check_pinned_digest(version: &str, digest: &str) -> Result<(), TgProxyError> {
        match PINNED_RELEASE_DIGESTS
            .iter()
            .find(|(v, _)| *v == version)
        {
            Some((_, pinned)) if pinned.eq_ignore_ascii_case(digest) => Ok(()),
            Some(_) => Err(TgProxyError::PinnedDigestMismatch(version.to_string())),
            None => Err(TgProxyError::VersionNotPinned(version.to_string())),
        }
    }

    fn generate_secret() -> String {
        let mut buf = [0u8; 16];
        OsRng.fill_bytes(&mut buf);
        let mut out = String::with_capacity(32);
        for b in buf {
            out.push_str(&format!("{:02x}", b));
        }
        out
    }

    fn normalize_port(raw: u16) -> u16 {
        if raw == 0 {
            TGWS_DEFAULT_PORT
        } else {
            raw
        }
    }

    fn version_tuple(v: &str) -> Vec<u64> {
        v.trim()
            .trim_start_matches(['v', 'V'])
            .split('.')
            .map(|p| {
                p.chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
                    .parse::<u64>()
                    .unwrap_or(0)
            })
            .collect()
    }

    fn version_lte(a: &str, b: &str) -> bool {
        let (mut x, mut y) = (Self::version_tuple(a), Self::version_tuple(b));
        let n = x.len().max(y.len()).max(1);
        x.resize(n, 0);
        y.resize(n, 0);
        x <= y
    }

    fn sha256_file(path: &Path) -> Result<String, TgProxyError> {
        let mut file = std::fs::File::open(path)?;
        let mut hasher = Sha256::new();
        let mut buf = [0u8; 65536];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Ok(format!("{:x}", hasher.finalize()))
    }

    async fn is_port_listening(host: &str, port: u16) -> bool {
        timeout(Duration::from_millis(400), TcpStream::connect((host, port)))
            .await
            .map(|r| r.is_ok())
            .unwrap_or(false)
    }

    /// Best-effort detection of a standalone TgWsProxy via its single-instance
    /// mutex name (see upstream `windows.py::_acquire_win_mutex`: the Python
    /// literal `"Local\\\\TgWsProxy_SingleInstance"` is a runtime string with
    /// TWO backslashes — matched byte-for-byte here). Failure-open: any error
    /// means "unknown", not "running".
    fn standalone_instance_running() -> bool {
        use windows::core::w;
        use windows::Win32::Foundation::{CloseHandle, GetLastError, BOOL};
        use windows::Win32::System::Threading::CreateMutexW;
        const ERROR_ALREADY_EXISTS: u32 = 183;
        unsafe {
            match CreateMutexW(None, BOOL::from(false), w!("Local\\\\TgWsProxy_SingleInstance")) {
                Ok(handle) => {
                    let already = GetLastError().0 == ERROR_ALREADY_EXISTS;
                    let _ = CloseHandle(handle);
                    already
                }
                Err(_) => false,
            }
        }
    }

    // ------------------------------------------------------------------
    // Status
    // ------------------------------------------------------------------

    /// Retrieves current sidecar status for UI rendering.
    pub fn get_status(&self) -> TgProxyStatus {
        let settings = crate::win32::settings::load_settings();
        let port = Self::normalize_port(settings.tgproxy_port);
        let secret = settings.tgproxy_secret.clone();
        let secret_valid = Self::is_valid_secret(&secret);

        let dir = Self::locate_dir();
        let binary_present = dir.is_some();
        let manifest = dir.as_ref().and_then(Self::read_manifest);
        let binary_version = manifest.as_ref().map(|m| m.version.clone());
        let version_supported = manifest
            .as_ref()
            .map(|m| Self::version_lte(&m.version, TGWS_MAX_TESTED_VERSION))
            .unwrap_or(true);

        let uptime_secs = self
            .start_time
            .read()
            .map(|t| t.elapsed().as_secs())
            .unwrap_or(0);

        TgProxyStatus {
            state: *self.state.read(),
            pid: *self.active_pid.read(),
            host: TGWS_DEFAULT_HOST.to_string(),
            port,
            secret: if secret_valid { secret.clone() } else { String::new() },
            tg_link: if secret_valid {
                format!(
                    "tg://proxy?server={}&port={}&secret={}",
                    TGWS_DEFAULT_HOST, port, secret
                )
            } else {
                String::new()
            },
            binary_present,
            binary_version,
            version_supported,
            uptime_secs,
            last_error: self.last_error.read().clone(),
        }
    }

    // ------------------------------------------------------------------
    // Download with mandatory SHA-256 verification
    // ------------------------------------------------------------------

    /// Downloads the official `TgWsProxy_windows.exe` from the latest Flowseal
    /// release. The asset `digest` is mandatory; the file is hashed while
    /// streaming and compared before being moved into place.
    ///
    /// Returns the release version (e.g. `"1.11.2"`).
    pub async fn download_binary(&self) -> Result<String, TgProxyError> {
        // T6: refuse while the sidecar is alive — on Windows a rename over a
        // running exe fails with a sharing violation, and swapping the binary
        // under a live process is never what the user asked for.
        match *self.state.read() {
            SupervisorState::Stopped | SupervisorState::Error => {}
            _ => return Err(TgProxyError::MustStopFirst),
        }

        let dir = Self::ensure_dir()?;

        let client = reqwest::Client::builder()
            .user_agent("zapret2-gui-tgproxy")
            .timeout(Duration::from_secs(120))
            .build()?;

        let text = client
            .get(TGWS_RELEASES_API)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?
            .error_for_status()
            .map_err(|e| TgProxyError::ReleaseFetch(e.to_string()))?
            .text()
            .await?;

        let release: serde_json::Value = serde_json::from_str(&text)?;
        let tag = release
            .get("tag_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TgProxyError::ReleaseFetch("в ответе нет tag_name".into()))?;
        let version = tag.trim().trim_start_matches(['v', 'V']).to_string();

        let assets = release
            .get("assets")
            .and_then(|v| v.as_array())
            .ok_or_else(|| TgProxyError::ReleaseFetch("в ответе нет assets".into()))?;
        let asset = assets
            .iter()
            .find(|a| a.get("name").and_then(|n| n.as_str()) == Some(TGWS_ASSET_NAME))
            .ok_or(TgProxyError::AssetNotFound)?;
        let url = asset
            .get("browser_download_url")
            .and_then(|u| u.as_str())
            .ok_or_else(|| TgProxyError::ReleaseFetch("у ассета нет browser_download_url".into()))?
            .to_string();
        let digest_raw = asset.get("digest").and_then(|d| d.as_str()).unwrap_or("");
        let expected = Self::parse_sha256_digest(digest_raw).ok_or(TgProxyError::MissingDigest)?;
        // M-3: the API digest must additionally match the in-code pin for
        // this exact version (fail fast, before downloading any bytes).
        Self::check_pinned_digest(&version, &expected)?;

        self.supervisor.log(
            "info",
            &format!("Скачивание {} ({}) с GitHub…", TGWS_ASSET_NAME, tag),
        );

        // Stream into a temp file while hashing (upstream update_check.py mechanics).
        let mut resp = client
            .get(&url)
            .send()
            .await?
            .error_for_status()
            .map_err(|e| TgProxyError::ReleaseFetch(e.to_string()))?;
        let total = resp.content_length().unwrap_or(0);
        let tmp_path = dir.join(format!("{}.download", TGWS_ASSET_NAME));
        let mut out = std::fs::File::create(&tmp_path)?;
        let mut hasher = Sha256::new();
        let mut downloaded: u64 = 0;
        let mut last_percent: u64 = 0;
        self.emit_download_progress(0);
        loop {
            match resp.chunk().await? {
                Some(bytes) => {
                    hasher.update(&bytes);
                    out.write_all(&bytes)?;
                    // T12: progress for the UI (percent only when the server
                    // reports a content length).
                    downloaded += bytes.len() as u64;
                    if total > 0 {
                        let percent = downloaded.saturating_mul(100) / total;
                        if percent != last_percent {
                            last_percent = percent;
                            self.emit_download_progress(percent);
                        }
                    }
                }
                None => break,
            }
        }
        drop(out);
        self.emit_download_progress(100);

        let actual = format!("{:x}", hasher.finalize());
        if actual != expected {
            let _ = std::fs::remove_file(&tmp_path);
            self.supervisor.log(
                "error",
                "SHA-256 скачанного файла не совпал с digest релиза — файл удалён",
            );
            return Err(TgProxyError::DigestMismatch);
        }

        // L-5: allow re-download / upgrade to a newer pinned version. The old
        // binary and its manifest are removed only AFTER the new file is
        // verified, and Windows cannot rename over an existing file anyway.
        let exe_path = dir.join(TGWS_ASSET_NAME);
        if exe_path.exists() {
            std::fs::remove_file(&exe_path)?;
        }
        let manifest_path = dir.join(TGWS_MANIFEST_FILE);
        if manifest_path.exists() {
            std::fs::remove_file(&manifest_path)?;
        }
        std::fs::rename(&tmp_path, &exe_path)?;

        let manifest = TgProxyManifest {
            asset: TGWS_ASSET_NAME.to_string(),
            version: version.clone(),
            sha256: expected,
            downloaded_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        };
        std::fs::write(
            dir.join(TGWS_MANIFEST_FILE),
            serde_json::to_string_pretty(&manifest)?,
        )?;

        self.supervisor.log(
            "info",
            &format!("Telegram-прокси {} скачан, SHA-256 совпал", version),
        );
        Ok(version)
    }

    /// Re-verifies the on-disk binary against the manifest digest.
    fn verify_binary_integrity(dir: &Path) -> Result<(), TgProxyError> {
        let manifest = Self::read_manifest(dir).ok_or(TgProxyError::NotDownloaded)?;
        let actual = Self::sha256_file(&dir.join(TGWS_ASSET_NAME))?;
        if actual != manifest.sha256 {
            return Err(TgProxyError::IntegrityFailed);
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Config preparation (portable mode)
    // ------------------------------------------------------------------

    /// Writes `TgWsProxy_data/config.json` (merging with any existing file so
    /// user-tuned keys like cfproxy domains survive) and suppresses the
    /// official first-run window. Returns `(port, secret)`.
    ///
    /// Four keys are written (`host`, `port`, `secret`, plus
    /// `check_updates: false` — the tray app's own updater is silenced because
    /// updates are managed by zapret2-gui); everything else is filled in by
    /// the exe itself from its defaults.
    ///
    /// `dir` must be the directory the binary was located in (L-6): in
    /// `--dev` mode that can be `./bin/tgproxy`, while `ensure_dir()` always
    /// anchors at the executable — writing the config next to a *different*
    /// binary than the one being launched would silently misconfigure it.
    fn prepare_config(&self, dir: &Path) -> Result<(u16, String), TgProxyError> {
        // L-4: the whole read-modify-write goes through the serialized
        // `update_settings`, so a concurrent `regenerate_secret` cannot be
        // silently clobbered (or vice versa).
        let (port, secret) =
            crate::win32::settings::update_settings(|settings| {
                let port = Self::normalize_port(settings.tgproxy_port);
                let mut secret = settings.tgproxy_secret.clone();
                if !Self::is_valid_secret(&secret) {
                    // No invented defaults: a missing/invalid secret is generated with
                    // a CSPRNG and persisted, so the UI always shows the real value.
                    secret = Self::generate_secret();
                }
                settings.tgproxy_port = port;
                settings.tgproxy_secret = secret.clone();
                (port, secret)
            })
            .map_err(TgProxyError::Settings)?;

        let data_dir = Self::data_dir(dir);
        std::fs::create_dir_all(&data_dir)?;

        let cfg_path = data_dir.join(TGWS_CONFIG_FILE);
        let mut cfg: serde_json::Map<String, serde_json::Value> = std::fs::read_to_string(&cfg_path)
            .ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        cfg.insert(
            "host".to_string(),
            serde_json::Value::String(TGWS_DEFAULT_HOST.to_string()),
        );
        cfg.insert("port".to_string(), serde_json::Value::from(port));
        cfg.insert(
            "secret".to_string(),
            serde_json::Value::String(secret.clone()),
        );
        // Updates are managed by zapret2-gui; the tray app's own update
        // dialogs would be intrusive when we supervise the process.
        cfg.insert("check_updates".to_string(), serde_json::Value::Bool(false));

        let pretty = serde_json::to_string_pretty(&serde_json::Value::Object(cfg))?;
        crate::win32::settings::atomic_write(&cfg_path, pretty.as_bytes())?;

        // Suppress the official first-run instruction window.
        let marker = data_dir.join(TGWS_FIRST_RUN_MARKER);
        if !marker.exists() {
            std::fs::write(&marker, b"managed by zapret2-gui")?;
        }

        Ok((port, secret))
    }

    // ------------------------------------------------------------------
    // Lifecycle
    // ------------------------------------------------------------------

    /// Records a terminal failure: log + UI-visible error + Error state.
    fn fail(&self, err: TgProxyError) -> Result<(), TgProxyError> {
        let msg = err.to_string();
        self.supervisor
            .log("error", &format!("Telegram-прокси: {}", msg));
        *self.last_error.write() = Some(msg);
        if let Some(mut child) = self.active_child.lock().take() {
            let _ = child.start_kill();
        }
        // T7: unlike `stop()`, also terminate the Job Object — if the tray
        // exe managed to spawn grandchildren in the same job, `start_kill`
        // alone would leave them behind. The job handle stays valid for the
        // next `start()` (`TerminateJobObject` does not destroy the job).
        let _ = self.job_object.terminate(1);
        *self.active_pid.write() = None;
        *self.start_time.write() = None;
        *self.state.write() = SupervisorState::Error;
        self.emit_state(SupervisorState::Error);
        // T4: a deterministically failing proxy must not be retried on every
        // app launch — drop the persisted "left enabled" intent.
        self.clear_enabled_flag();
        Err(err)
    }

    /// Drops the persisted auto-start intent so a failing proxy is not
    /// retried on every app launch (T4). The user re-enables it manually
    /// after fixing the underlying problem.
    fn clear_enabled_flag(&self) {
        if !crate::win32::settings::load_settings().tgproxy_enabled {
            return;
        }
        match crate::win32::settings::update_settings(|s| s.tgproxy_enabled = false) {
            Ok(()) => self.supervisor.log(
                "info",
                "Telegram-прокси: автозапуск отключён — при следующем запуске приложения прокси не стартует, пока вы не включите его вручную",
            ),
            Err(e) => self.supervisor.log(
                "error",
                &format!("Telegram-прокси: не удалось сбросить флаг автозапуска: {}", e),
            ),
        }
    }

    /// Spawns the sidecar and binds it to the Job Object immediately.
    ///
    /// M-3: the GUI is elevated (WinDivert), but the sidecar only listens on
    /// 127.0.0.1 — it must never inherit the Administrator token. When
    /// elevated we spawn de-elevated via the interactive user's token
    /// (`win32::deelevate`); any de-elevation failure refuses the spawn
    /// (fail-closed, no elevated fallback). When the GUI itself runs
    /// non-elevated there is nothing to drop and the child spawns with the
    /// current token.
    fn spawn_sidecar(
        exe: &Path,
        workdir: &Path,
        job: &JobObject,
    ) -> Result<(RawChild, u32), TgProxyError> {
        let elevated = crate::win32::privileges::is_elevated().unwrap_or(true);
        let mut child = if elevated {
            crate::win32::deelevate::spawn_deelevated(exe, &["--portable"], workdir)
                .map_err(|e| TgProxyError::Deelevate(e.to_string()))?
        } else {
            crate::win32::deelevate::spawn_current_token(exe, &["--portable"], workdir)
                .map_err(|e| TgProxyError::Spawn(e.to_string()))?
        };

        // T3: the process is already alive at this point — every error path
        // below must kill it first. Dropping a child handle never terminates
        // the process, so a bare `?` here would leak a live tray exe outside
        // the Job Object and outside the manager's accounting.
        let raw_handle: RawHandle = match child.raw_handle() {
            Some(h) => h,
            None => {
                let _ = child.start_kill();
                return Err(TgProxyError::Spawn("no raw handle".to_string()));
            }
        };
        if let Err(e) = job.assign_raw_handle(raw_handle) {
            let _ = child.start_kill();
            return Err(e.into());
        }
        let pid = match child.id() {
            Some(p) => p,
            None => {
                let _ = child.start_kill();
                return Err(TgProxyError::Spawn("no process id".to_string()));
            }
        };

        Ok((child, pid))
    }

    /// Waits until the child is alive AND its listen port accepts connections.
    async fn wait_healthy(&self, port: u16) -> Result<(), TgProxyError> {
        let deadline = Instant::now() + STARTUP_HEALTH_TIMEOUT;
        loop {
            let exited_status = {
                let mut guard = self.active_child.lock();
                match guard.as_mut() {
                    Some(child) => match child.try_wait() {
                        Ok(Some(status)) => {
                            *guard = None;
                            Some(status.to_string())
                        }
                        _ => None,
                    },
                    None => Some("disappeared".to_string()),
                }
            };
            if let Some(status) = exited_status {
                return Err(TgProxyError::EarlyExit(status));
            }
            if Self::is_port_listening(TGWS_DEFAULT_HOST, port).await {
                return Ok(());
            }
            if Instant::now() >= deadline {
                let tail = self.read_log_tail(8).join(" | ");
                return Err(TgProxyError::StartupTimeout(
                    port,
                    STARTUP_HEALTH_TIMEOUT.as_secs(),
                    if tail.is_empty() {
                        "лог пуст".to_string()
                    } else {
                        tail
                    },
                ));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Starts the sidecar: integrity check → config → spawn → Job Object →
    /// health check → log tailer + exit monitor.
    pub async fn start(self: Arc<Self>) -> Result<(), TgProxyError> {
        // Serialized with stop(): never a half-initialized sidecar.
        let _lifecycle_guard = self.lifecycle.lock().await;

        {
            let mut state = self.state.write();
            match *state {
                SupervisorState::Starting | SupervisorState::Stopping => return Ok(()),
                SupervisorState::Running => return Ok(()),
                _ => *state = SupervisorState::Starting,
            }
        }
        self.emit_state(SupervisorState::Starting);
        *self.last_error.write() = None;
        self.supervisor.log("info", "Запуск Telegram-прокси…");

        let dir = match Self::locate_dir() {
            Some(d) => d,
            None => return self.fail(TgProxyError::NotDownloaded),
        };
        if let Err(e) = Self::verify_binary_integrity(&dir) {
            return self.fail(e);
        }
        let (port, _secret) = match self.prepare_config(&dir) {
            Ok(v) => v,
            Err(e) => return self.fail(e),
        };
        // A standalone TgWsProxy (or anything else) already on this port
        // would make our health check meaningless — fail fast instead.
        if Self::standalone_instance_running() {
            return self.fail(TgProxyError::StandaloneRunning);
        }
        if Self::is_port_listening(TGWS_DEFAULT_HOST, port).await {
            return self.fail(TgProxyError::PortBusy(port));
        }

        let exe = dir.join(TGWS_ASSET_NAME);
        let (child, pid) = match Self::spawn_sidecar(&exe, &dir, &self.job_object) {
            Ok(v) => v,
            Err(e) => return self.fail(e),
        };

        *self.active_child.lock() = Some(child);
        *self.active_pid.write() = Some(pid);
        *self.start_time.write() = Some(Instant::now());
        // Start tailing from the current end of proxy.log: only fresh lines
        // go into the shared buffer.
        *self.log_offset.lock() = std::fs::metadata(Self::log_file_path(&dir))
            .map(|m| m.len())
            .unwrap_or(0);

        self.supervisor.log(
            "info",
            &format!(
                "Telegram-прокси запущен (PID {}), ожидание порта {}…",
                pid, port
            ),
        );

        if let Err(e) = self.wait_healthy(port).await {
            return self.fail(e);
        }

        *self.state.write() = SupervisorState::Running;
        self.emit_state(SupervisorState::Running);
        self.supervisor.log(
            "info",
            &format!(
                "Telegram-прокси слушает {}:{} — введите параметры в Telegram-клиенте",
                TGWS_DEFAULT_HOST, port
            ),
        );

        // Persist the "enabled" intent so it survives app restarts.
        let _ = crate::win32::settings::update_settings(|s| s.tgproxy_enabled = true);

        self.spawn_log_tailer(dir.clone());
        self.spawn_exit_monitor();
        Ok(())
    }

    /// Stops the sidecar. The dedicated Job Object has KILL_ON_JOB_CLOSE, so
    /// no orphaned tray process can survive us; `terminate` is the safety net.
    pub async fn stop(&self) -> Result<(), TgProxyError> {
        let _lifecycle_guard = self.lifecycle.lock().await;

        {
            let mut state = self.state.write();
            if *state == SupervisorState::Stopped || *state == SupervisorState::Stopping {
                return Ok(());
            }
            *state = SupervisorState::Stopping;
        }
        self.emit_state(SupervisorState::Stopping);
        self.supervisor.log("info", "Остановка Telegram-прокси…");

        if let Some(mut child) = self.active_child.lock().take() {
            let _ = child.start_kill();
            let _ = timeout(Duration::from_secs(5), child.wait()).await;
        }
        let _ = self.job_object.terminate(1);

        *self.active_pid.write() = None;
        *self.start_time.write() = None;
        *self.state.write() = SupervisorState::Stopped;
        self.emit_state(SupervisorState::Stopped);

        let _ = crate::win32::settings::update_settings(|s| s.tgproxy_enabled = false);

        self.supervisor.log("info", "Telegram-прокси остановлен");
        Ok(())
    }

    // ------------------------------------------------------------------
    // Background tasks
    // ------------------------------------------------------------------

    /// Tails `proxy.log` (the GUI exe has no console output) into the shared
    /// supervisor log buffer with level `"tgproxy"`.
    fn spawn_log_tailer(self: &Arc<Self>, dir: PathBuf) {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(LOG_TAIL_INTERVAL).await;
                if *this.state.read() != SupervisorState::Running {
                    break;
                }
                for line in this.drain_new_log_lines(&dir) {
                    this.supervisor.log("tgproxy", &line);
                }
            }
        });
    }

    /// Reads lines appended to proxy.log since the last call. Handles
    /// rotation/truncation by resetting the offset when the file shrinks.
    fn drain_new_log_lines(&self, dir: &Path) -> Vec<String> {
        let path = Self::log_file_path(dir);
        let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let mut offset = self.log_offset.lock();
        if len < *offset {
            *offset = 0;
        }
        if len == *offset {
            return Vec::new();
        }
        let mut file = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(_) => return Vec::new(),
        };
        if file.seek(SeekFrom::Start(*offset)).is_err() {
            return Vec::new();
        }
        let mut text = String::new();
        if file.read_to_string(&mut text).is_err() {
            return Vec::new();
        }
        *offset = len;
        text.lines()
            .map(|l| l.trim_end().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    }

    /// Last `n` non-empty lines of proxy.log (for error diagnostics).
    fn read_log_tail(&self, n: usize) -> Vec<String> {
        let dir = match Self::locate_dir() {
            Some(d) => d,
            None => return Vec::new(),
        };
        let text = std::fs::read_to_string(Self::log_file_path(&dir)).unwrap_or_default();
        let lines: Vec<String> = text
            .lines()
            .map(|l| l.trim_end().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        lines.into_iter().rev().take(n).collect::<Vec<_>>().into_iter().rev().collect()
    }

    /// Watches the child; on unexpected exit reports Stopped to the UI.
    fn spawn_exit_monitor(self: &Arc<Self>) {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;
                let exited = {
                    let mut guard = this.active_child.lock();
                    match guard.as_mut() {
                        Some(child) => match child.try_wait() {
                            Ok(Some(status)) => {
                                let pid = *this.active_pid.read();
                                this.supervisor.log(
                                    "info",
                                    &format!(
                                        "Telegram-прокси (PID {:?}) завершён: {}",
                                        pid, status
                                    ),
                                );
                                *guard = None;
                                true
                            }
                            Ok(None) => false,
                            Err(e) => {
                                this.supervisor.log(
                                    "error",
                                    &format!("Telegram-прокси: ошибка опроса процесса: {}", e),
                                );
                                *guard = None;
                                true
                            }
                        },
                        None => break,
                    }
                };
                if exited {
                    // T4: an unexpected exit of a Running proxy usually means
                    // a deterministic crash — drop the auto-start intent so
                    // the app does not retry it on every launch. (A Stopping
                    // proxy was halted by the user; `stop()` clears the flag
                    // itself.)
                    let unexpected = {
                        let mut state = this.state.write();
                        let unexpected = *state == SupervisorState::Running;
                        if unexpected || *state == SupervisorState::Stopping {
                            *state = SupervisorState::Stopped;
                        }
                        *this.active_pid.write() = None;
                        *this.start_time.write() = None;
                        unexpected
                    };
                    if unexpected {
                        this.clear_enabled_flag();
                    }
                    this.emit_state(SupervisorState::Stopped);
                    break;
                }
            }
        });
    }

    // ------------------------------------------------------------------
    // User configuration
    // ------------------------------------------------------------------

    /// Config changes are only allowed when the sidecar is fully stopped (or
    /// in `Error` after a failed start, so the user can fix what broke it).
    /// During `Starting`/`Stopping`, `prepare_config` may already have
    /// snapshotted the old values — allowing a change then would desync the
    /// on-disk config from the actually spawned process (T5).
    fn ensure_config_change_allowed(&self) -> Result<(), TgProxyError> {
        match *self.state.read() {
            SupervisorState::Stopped | SupervisorState::Error => Ok(()),
            _ => Err(TgProxyError::MustStopFirst),
        }
    }

    /// Updates port/secret. Refused while running — the exe reads config only
    /// at startup, so a silent no-op would be worse than an explicit error.
    pub fn set_config(&self, port: u16, secret: String) -> Result<(), TgProxyError> {
        self.ensure_config_change_allowed()?;
        if port == 0 {
            return Err(TgProxyError::BadPort(port));
        }
        let secret = secret.trim().to_lowercase();
        if !Self::is_valid_secret(&secret) {
            return Err(TgProxyError::BadSecret);
        }
        crate::win32::settings::update_settings(|settings| {
            settings.tgproxy_port = port;
            settings.tgproxy_secret = secret.clone();
        })
        .map_err(TgProxyError::Settings)?;
        Ok(())
    }

    /// Generates a fresh CSPRNG secret and persists it.
    pub fn regenerate_secret(&self) -> Result<String, TgProxyError> {
        self.ensure_config_change_allowed()?;
        crate::win32::settings::update_settings(|settings| {
            let secret = Self::generate_secret();
            settings.tgproxy_secret = secret.clone();
            secret
        })
        .map_err(TgProxyError::Settings)
    }
}

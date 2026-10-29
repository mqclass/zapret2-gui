//! Application Settings & Split-Tunneling Configuration (Linux)
//!
//! Same structures, defaults and on-disk format as the Windows build — a
//! settings file written on one platform loads on the other. Differences are
//! confined to this module:
//! - The fallback settings directory is XDG (`$XDG_CONFIG_HOME/zapret2/`
//!   or `~/.config/zapret2/`) instead of `%LOCALAPPDATA%\zapret2\`.
//! - There is no DPAPI on Linux. The MTProto secret is stored in plaintext
//!   (exactly like the `TgWsProxy_data/config.json` copy the bridge itself
//!   reads) and the settings file is created with 0600 permissions as the
//!   at-rest mitigation. A `dpapi:`-wrapped value written by a Windows
//!   install cannot be unwrapped here — it is treated as empty, the same
//!   semantics the Windows build applies to a profile migrated from another
//!   machine — and the bridge regenerates the secret on next start.

use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};

use crate::engine::transport::TransportConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitTunnelMode {
    Full,     // Full bypass: YouTube + Discord + All blocked websites (ipset-all)
    Strict,   // Strict: YouTube & Discord ONLY (other traffic goes 100% direct)
    GameMode, // Gaming mode: Includes gaming UDP/TCP ports with ACTIVE_GAME_UDP.bin desync
}

/// Per-application split tunneling mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AppSplitMode {
    #[default]
    Disabled,
    /// Only apps in app_split_list go through the bypass; rest go direct.
    Allowlist,
    /// Apps in app_split_list go direct; rest go through the bypass.
    Blocklist,
}

impl Default for SplitTunnelMode {
    fn default() -> Self {
        SplitTunnelMode::Full
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DnsScope {
    Full,  // Apply to active network adapter (all system traffic)
    Split, // Selective Split-DNS via NRPT (YouTube, Discord, social media only)
}

impl Default for DnsScope {
    fn default() -> Self {
        DnsScope::Full
    }
}

/// A user-defined DNS resolver entry (custom DNS servers).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomDnsEntry {
    /// Stable id, e.g. "custom-1700000000". Provider id becomes "custom:<id>".
    pub id: String,
    /// Display name chosen by the user.
    pub name: String,
    /// Primary IPv4 address.
    pub primary: String,
    /// Secondary IPv4 address (may be empty).
    #[serde(default)]
    pub secondary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub split_tunnel_mode: SplitTunnelMode,
    /// Per-app split tunneling: Disabled, Allowlist (only listed apps use bypass),
    /// or Blocklist (listed apps bypass the bypass, go direct).
    #[serde(default)]
    pub app_split_mode: AppSplitMode,
    /// List of executable paths (lowercase) for per-app split tunneling.
    #[serde(default)]
    pub app_split_list: Vec<String>,
    /// User-defined domains for Split-DNS (added to the built-in NRPT list).
    #[serde(default)]
    pub dns_split_custom_domains: Vec<String>,
    pub game_filter_tcp: String,
    pub game_filter_udp: String,
    pub dns_provider: String, // "system", "cloudflare", "google", "quad9", "comss" or "custom:<id>"
    #[serde(default)]
    pub dns_scope: DnsScope,
    /// User-defined DNS servers (shown alongside built-in presets).
    #[serde(default)]
    pub custom_dns: Vec<CustomDnsEntry>,
    pub last_strategy_id: Option<String>,
    pub auto_start_bypass: bool,
    /// Whether Windows toast notifications are shown for engine events.
    #[serde(default = "default_true")]
    pub notifications_enabled: bool,
    /// Whether the app periodically checks GitHub for new releases.
    #[serde(default = "default_true")]
    pub auto_update_enabled: bool,
    /// Unix timestamp of the last *successful* update check (scheduler bookkeeping).
    #[serde(default)]
    pub last_update_check: Option<u64>,
    /// Telegram MTProxy-mode sidecar: listen port (0 = default 1443).
    #[serde(default = "default_tgproxy_port")]
    pub tgproxy_port: u16,
    /// Telegram MTProxy-mode sidecar: 32-hex-char MTProto secret (empty = not generated yet).
    #[serde(default)]
    pub tgproxy_secret: String,
    /// Whether the Telegram proxy was left enabled (auto-start on app launch).
    #[serde(default)]
    pub tgproxy_enabled: bool,
    /// Telegram bridge connection mode: "auto" (default ladder), "cfproxy"
    /// (Cloudflare first), or "direct" (direct WebSocket/TCP only).
    #[serde(default = "default_tgproxy_mode")]
    pub tgproxy_mode: String,
    /// When true, the bridge binds to 0.0.0.0 so phones on the LAN can use
    /// it (QR code). Off by default: loopback-only is the safe default.
    #[serde(default)]
    pub tgproxy_lan: bool,
    /// Active camouflage transport id (see engine::transport). `None` means
    /// the legacy direct launch — the only fully working mode today.
    /// `#[serde(default)]` is load-bearing: settings files written by v1.0 do
    /// not have this key, and without a default the whole file fails to parse
    /// and every user setting resets on upgrade.
    #[serde(default)]
    pub active_transport_id: Option<String>,
    /// Per-transport JSON configs, keyed by transport id.
    /// `#[serde(default)]` keeps old settings files loadable: missing field
    /// defaults to empty instead of resetting all settings.
    #[serde(default)]
    pub transport_configs: HashMap<String, TransportConfig>,
    /// Smart hosts: whether community hosts lists are merged into the
    /// system hosts file.
    #[serde(default)]
    pub smart_hosts_enabled: bool,
    /// Enabled smart-hosts source ids (see platform::smart_hosts::HOST_SOURCES).
    #[serde(default = "default_smart_hosts_sources")]
    pub smart_hosts_sources: Vec<String>,
    /// Whether `0.0.0.0` adblock entries from sources are applied too.
    #[serde(default)]
    pub smart_hosts_include_adblock: bool,
    /// Unix timestamp of the last successful smart-hosts refresh.
    #[serde(default)]
    pub smart_hosts_last_update: Option<u64>,
    /// Entries written by the last successful refresh (UI display).
    #[serde(default)]
    pub smart_hosts_entry_count: usize,
    /// Watchdog: background re-checks of the current strategy and automatic
    /// escalation to a better one on confirmed degradation.
    #[serde(default = "default_true")]
    pub watchdog_enabled: bool,
    /// Watchdog check interval in minutes.
    #[serde(default = "default_watchdog_interval")]
    pub watchdog_interval_minutes: u64,
    /// UI accent color override as a hex string ("#RRGGBB"). None means the
    /// built-in violet accent from index.css. Only the accent family is
    /// overridden; the dark glass base theme is never changed.
    #[serde(default)]
    pub accent_color: Option<String>,
}

fn default_true() -> bool {
    true
}

fn default_watchdog_interval() -> u64 {
    15
}

fn default_tgproxy_port() -> u16 {
    1443
}

fn default_tgproxy_mode() -> String {
    "auto".to_string()
}

fn default_smart_hosts_sources() -> Vec<String> {
    vec!["malw".to_string(), "geohide".to_string()]
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            split_tunnel_mode: SplitTunnelMode::Full,
            game_filter_tcp: "1024-65535".into(),
            game_filter_udp: "1024-65535".into(),
            dns_provider: "system".into(),
            dns_scope: DnsScope::Full,
            last_strategy_id: Some("flowseal-alt".into()),
            auto_start_bypass: true,
            notifications_enabled: true,
            auto_update_enabled: true,
            last_update_check: None,
            tgproxy_port: 1443,
            tgproxy_secret: String::new(),
            tgproxy_enabled: false,
            tgproxy_mode: default_tgproxy_mode(),
            tgproxy_lan: false,
            active_transport_id: None,
            transport_configs: HashMap::new(),
            custom_dns: Vec::new(),
            app_split_mode: AppSplitMode::Disabled,
            app_split_list: Vec::new(),
            dns_split_custom_domains: Vec::new(),
            smart_hosts_enabled: false,
            smart_hosts_sources: default_smart_hosts_sources(),
            smart_hosts_include_adblock: false,
            smart_hosts_last_update: None,
            smart_hosts_entry_count: 0,
            watchdog_enabled: true,
            watchdog_interval_minutes: 15,
            accent_color: None,
        }
    }
}

static CACHED_SETTINGS: RwLock<Option<AppSettings>> = RwLock::new(None);

/// File stamp (mtime + length) of the settings file at the moment the cache
/// was filled. The GUI and zapret2-svc are separate processes sharing one
/// settings file; without revalidation each process would mutate its own
/// stale snapshot and silently roll back the other's writes (e.g. a Telegram
/// secret regenerated by svc being replaced by the GUI's old copy on the
/// GUI's next save). `load_settings` re-reads the file whenever the stamp
/// no longer matches what is on disk.
static CACHED_FILE_STAMP: RwLock<Option<(std::time::SystemTime, u64)>> = RwLock::new(None);

fn settings_file_stamp(path: &Path) -> Option<(std::time::SystemTime, u64)> {
    let meta = fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    Some((modified, meta.len()))
}

/// Serializes whole read-modify-write cycles on the settings file (L-4).
/// The `RwLock` on the cache alone does not prevent lost updates such as
/// `regenerate_secret` racing `start()` → `prepare_config`: both would load
/// the same snapshot and the second save would silently clobber the first.
/// All mutations must go through [`update_settings`].
static SETTINGS_WRITE_LOCK: Mutex<()> = Mutex::new(());

/// Monotonic counter for unique temporary file names in [`atomic_write`].
static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Marker for DPAPI-protected values in a settings file written by the
/// Windows build. Stored form: `dpapi:<hex of the CryptProtectData blob>`.
/// Linux cannot unwrap these (no DPAPI); the prefix is still recognised on
/// read so such a value is treated as "stored but unusable", never as a
/// plaintext secret.
const DPAPI_PREFIX: &str = "dpapi:";

/// Loads the latest settings, applies `f`, and persists the result —
/// atomically with respect to other `update_settings` callers (L-4).
pub fn update_settings<R>(f: impl FnOnce(&mut AppSettings) -> R) -> Result<R, String> {
    let _guard = SETTINGS_WRITE_LOCK.lock();
    let mut settings = load_settings();
    let result = f(&mut settings);
    save_settings(settings)?;
    Ok(result)
}

/// XDG config home: `$XDG_CONFIG_HOME`, or `~/.config` when unset (the
/// XDG Base Directory fallback chain).
fn xdg_config_home() -> PathBuf {
    if let Ok(x) = std::env::var("XDG_CONFIG_HOME") {
        if !x.is_empty() {
            return PathBuf::from(x);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".config");
    }
    PathBuf::from(".config")
}

/// The directory holding `app_settings.json` (and `custom_profiles.json`).
///
/// Mirrors the Windows resolution order: an existing `configs/` next to the
/// executable wins (portable installs), then an existing `./configs`, and
/// only then the per-user config home — XDG `zapret2/configs` here, where
/// `%LOCALAPPDATA%\zapret2\configs` sits on Windows.
pub(crate) fn config_dir() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let p = parent.join("configs");
            if p.is_dir() {
                return p;
            }
        }
    }

    let local = PathBuf::from("configs");
    if local.is_dir() {
        return local;
    }

    let dir = xdg_config_home().join("zapret2").join("configs");
    let _ = fs::create_dir_all(&dir);
    dir
}

fn get_settings_file_path() -> PathBuf {
    config_dir().join("app_settings.json")
}

pub fn load_settings() -> AppSettings {
    let path = get_settings_file_path();
    let disk_stamp = settings_file_stamp(&path);
    {
        let read_guard = CACHED_SETTINGS.read();
        if let Some(ref s) = *read_guard {
            // Serve the cache only while the file on disk is byte-identical
            // to the one the cache was built from; another process (GUI/svc)
            // may have written since.
            if disk_stamp.is_some() && *CACHED_FILE_STAMP.read() == disk_stamp {
                return s.clone();
            }
            if disk_stamp.is_none() && CACHED_FILE_STAMP.read().is_none() {
                return s.clone();
            }
        }
    }

    let settings = read_settings_from_disk(&path);
    *CACHED_SETTINGS.write() = Some(settings.clone());
    *CACHED_FILE_STAMP.write() = settings_file_stamp(&path);
    settings
}

/// True when the settings file on disk stores a non-empty tgproxy_secret
/// (a DPAPI envelope from a Windows install or a plaintext value). Used by
/// the bridge at start() to tell a lost secret apart from a first run: an
/// absent value means the user never had one, a stored-but-unusable value
/// means old tg:// links just died. Reads the raw file, bypassing the cache
/// and the unwrap — the unwrap is exactly what discards this distinction.
pub fn stored_tgproxy_secret_present() -> bool {
    let path = get_settings_file_path();
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let value: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return false,
    };
    value
        .get("tgproxy_secret")
        .and_then(|v| v.as_str())
        .map(|s| !s.is_empty())
        .unwrap_or(false)
}

/// Reads and parses the settings file from disk, bypassing the cache.
/// Corrupt files are quarantined (never silently reset), exactly as the
/// Windows build has always done.
fn read_settings_from_disk(path: &Path) -> AppSettings {
    let mut settings = if path.is_file() {
        match fs::read_to_string(path) {
            Ok(content) => match serde_json::from_str::<AppSettings>(&content) {
                Ok(s) => s,
                Err(e) => {
                    // L-4: never silently reset to defaults — quarantine the
                    // corrupt file so it can be inspected/recovered, then
                    // fall back to defaults.
                    let stamp = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let backup =
                        path.with_extension(format!("json.corrupt-{stamp}"));
                    let _ = fs::rename(path, &backup);
                    eprintln!(
                        "settings: {} is corrupt ({e}); moved to {} and using defaults",
                        path.display(),
                        backup.display()
                    );
                    AppSettings::default()
                }
            },
            Err(_) => AppSettings::default(),
        }
    } else {
        AppSettings::default()
    };

    // L-3 (Linux form): a `dpapi:` envelope written by a Windows install
    // cannot be unwrapped here. Treat it exactly like the Windows build
    // treats an unprotect failure (profile migrated to another
    // user/machine): the secret becomes empty and the proxy regenerates
    // it on next start.
    if settings.tgproxy_secret.starts_with(DPAPI_PREFIX) {
        eprintln!(
            "settings: tgproxy_secret is DPAPI-wrapped (written by a Windows install) \
             and cannot be unwrapped on Linux; treating as empty"
        );
        settings.tgproxy_secret = String::new();
    }

    settings
}

/// Writes `contents` to `path` atomically via a temporary file + rename.
///
/// A crash or power loss mid-write can never leave a truncated file at `path`
/// (readers either see the old complete file or the new complete file).
pub fn atomic_write(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    // Unique temp name per call (pid + monotonic counter): concurrent writers
    // to the same path must never share a tmp file, or a reader can observe
    // a torn JSON document (L-4).
    let id = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp_path = path.with_extension(format!("tmp-{}-{id}", std::process::id()));
    fs::write(&tmp_path, contents)?;
    if let Err(e) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e);
    }
    Ok(())
}

pub fn save_settings(new_settings: AppSettings) -> Result<(), String> {
    let path = get_settings_file_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    // L-3 (Linux form): there is no DPAPI here, so the MTProto secret is
    // stored in plaintext — the same form the bridge's own
    // `TgWsProxy_data/config.json` has always used, since the bridge reads
    // its config directly and cannot consume wrapped blobs on any platform.
    // The at-rest mitigation on Linux is file permissions: the settings
    // file is forced to 0600 (owner-only) after every save.
    let json = serde_json::to_string_pretty(&new_settings)
        .map_err(|e| format!("Failed to serialize settings: {}", e))?;

    atomic_write(&path, json.as_bytes()).map_err(|e| format!("Failed to save settings to disk: {}", e))?;
    let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));

    *CACHED_SETTINGS.write() = Some(new_settings);
    *CACHED_FILE_STAMP.write() = settings_file_stamp(&path);
    Ok(())
}

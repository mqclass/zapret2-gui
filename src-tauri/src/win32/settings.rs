//! Application Settings & Split-Tunneling Configuration
//!
//! Stores user preferences for:
//! - Split-Tunneling Mode: "full" (all sites), "strict" (YouTube & Discord only), "game_mode" (gaming port filter)
//! - Game Filter port ranges and enabled status
//! - Preferred DNS provider (DoH / System)

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN,
};

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
    /// Enabled smart-hosts source ids (see win32::smart_hosts::HOST_SOURCES).
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

/// Marker for DPAPI-protected values in the settings file (L-3).
/// Stored form: `dpapi:<hex of the CryptProtectData blob>`.
const DPAPI_PREFIX: &str = "dpapi:";

/// Protects `plaintext` with DPAPI (current user + machine) for at-rest
/// storage. Returns the `"dpapi:<hex>"` envelope for the settings file.
fn dpapi_protect(plaintext: &str) -> Result<String, String> {
    let bytes = plaintext.as_bytes();
    let blob_in = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let mut blob_out = CRYPT_INTEGER_BLOB::default();
    // `szDataDescr` is optional; pass an explicit null PCWSTR.
    let descr = PCWSTR(std::ptr::null());
    unsafe {
        CryptProtectData(
            &blob_in,
            descr,
            None,
            None,
            // pvReserved must be NULL.
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut blob_out,
        )
    }
    .map_err(|e| format!("CryptProtectData: {e}"))?;
    if blob_out.pbData.is_null() || blob_out.cbData == 0 {
        return Err("CryptProtectData returned an empty blob".to_string());
    }
    let protected_bytes =
        unsafe { std::slice::from_raw_parts(blob_out.pbData, blob_out.cbData as usize) };
    let mut hex = String::with_capacity(protected_bytes.len() * 2 + DPAPI_PREFIX.len());
    hex.push_str(DPAPI_PREFIX);
    for b in protected_bytes {
        hex.push_str(&format!("{:02x}", b));
    }
    unsafe {
        let _ = LocalFree(HLOCAL(blob_out.pbData as *mut std::ffi::c_void));
    }
    Ok(hex)
}

/// Unwraps a `"dpapi:<hex>"` envelope produced by [`dpapi_protect`].
fn dpapi_unprotect(stored: &str) -> Result<String, String> {
    let hex = stored
        .strip_prefix(DPAPI_PREFIX)
        .ok_or_else(|| "not a dpapi envelope".to_string())?;
    if hex.len() % 2 != 0 || hex.is_empty() {
        return Err("malformed dpapi envelope".to_string());
    }
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for i in (0..hex.len()).step_by(2) {
        let byte =
            u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| "malformed dpapi envelope")?;
        bytes.push(byte);
    }
    let blob_in = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_mut_ptr(),
    };
    let mut blob_out = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptUnprotectData(
            &blob_in,
            None,
            None,
            None,
            // pvReserved must be NULL.
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut blob_out,
        )
    }
    .map_err(|e| format!("CryptUnprotectData: {e}"))?;
    if blob_out.pbData.is_null() || blob_out.cbData == 0 {
        return Err("CryptUnprotectData returned an empty blob".to_string());
    }
    let plain_bytes =
        unsafe { std::slice::from_raw_parts(blob_out.pbData, blob_out.cbData as usize) };
    let text =
        String::from_utf8(plain_bytes.to_vec()).map_err(|_| "dpapi payload is not UTF-8")?;
    unsafe {
        let _ = LocalFree(HLOCAL(blob_out.pbData as *mut std::ffi::c_void));
    }
    Ok(text)
}

/// Loads the latest settings, applies `f`, and persists the result —
/// atomically with respect to other `update_settings` callers (L-4).
pub fn update_settings<R>(f: impl FnOnce(&mut AppSettings) -> R) -> Result<R, String> {
    let _guard = SETTINGS_WRITE_LOCK.lock();
    let mut settings = load_settings();
    let result = f(&mut settings);
    save_settings(settings)?;
    Ok(result)
}

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
/// (a DPAPI envelope or a legacy plaintext value). Used by the bridge at
/// start() to tell a lost secret apart from a first run: an absent value
/// means the user never had one, a stored-but-unusable value means old
/// tg:// links just died. Reads the raw file, bypassing the cache and the
/// DPAPI unwrap — the unwrap is exactly what discards this distinction.
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
/// Corrupt files are quarantined (never silently reset) and DPAPI-wrapped
/// secrets are unwrapped, exactly as `load_settings` has always done.
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

    // L-3: unwrap the DPAPI envelope. Unprotect failure (e.g. profile
    // migrated to another user/machine) yields an empty secret — the proxy
    // regenerates it on next start and the UI shows the real value.
    if settings.tgproxy_secret.starts_with(DPAPI_PREFIX) {
        match dpapi_unprotect(&settings.tgproxy_secret) {
            Ok(plain) => settings.tgproxy_secret = plain,
            Err(e) => {
                eprintln!("settings: cannot unprotect tgproxy_secret with DPAPI ({e}); treating as empty");
                settings.tgproxy_secret = String::new();
            }
        }
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

    // L-3: the MTProto secret never hits the disk in plaintext — DPAPI it
    // with the current user+machine key. Fail-closed: if protection fails we
    // refuse to persist rather than silently downgrading to plaintext (the
    // in-memory value keeps working for this session).
    // Note: `TgWsProxy_data/config.json` still carries the plaintext secret —
    // the upstream exe reads its own config and cannot consume DPAPI blobs.
    let mut to_store = new_settings.clone();
    if !to_store.tgproxy_secret.is_empty() && !to_store.tgproxy_secret.starts_with(DPAPI_PREFIX)
    {
        to_store.tgproxy_secret = dpapi_protect(&to_store.tgproxy_secret)?;
    }

    let json = serde_json::to_string_pretty(&to_store)
        .map_err(|e| format!("Failed to serialize settings: {}", e))?;

    atomic_write(&path, json.as_bytes()).map_err(|e| format!("Failed to save settings to disk: {}", e))?;

    *CACHED_SETTINGS.write() = Some(new_settings);
    *CACHED_FILE_STAMP.write() = settings_file_stamp(&path);
    Ok(())
}

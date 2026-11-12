//! sing-box sidecar manager (WARP mode, stage 3).
//!
//! The manager owns the second engine child: a forked sing-box binary
//! that runs the Cloudflare WARP (MASQUE) tunnel next to winws2. It
//! mirrors the supervisor's own lifecycle for winws2 - same Job
//! Object, same stdout/stderr pumps, same lifecycle mutex and
//! generation counter - because the tunnel is a peer of the desync
//! engine, not a fire-and-forget subprocess.
//!
//! Lifecycle:
//! - `start()` fills the generator input (system resolvers, IPv6,
//!   ipset subnets of the active preset, DoH provider), builds and
//!   checks the config, spawns the engine into the shared Job Object,
//!   waits for Clash API readiness, warms up (exit measurement), and
//!   arms a monitor task. Two switches stay independent: the tunnel
//!   can run with the desync engine on or off (stage 4 wires the
//!   start side; this stage provides the manager and the stop side).
//! - `stop()` kills the child and waits up to 5 s. It deliberately
//!   does NOT terminate the shared Job Object: winws2 lives in the
//!   same job, and terminating it would kill the desync engine too.
//!   sing-box spawns no grandchildren, so kill+wait is a complete
//!   stop; KILL_ON_JOB_CLOSE still covers the case where svc itself
//!   dies.
//! - The monitor task (500 ms, like the supervisor's) watches the
//!   child; an unexpected exit moves the tunnel to `Error`. A dead
//!   *exit* is not a dead engine: warmup and (stage 6) the watchdog
//!   report that as `Degraded` / `Unregistered` and never restart
//!   the engine for it. Restart-on-broken is a stage 6 concern.
//!
//! Port TOCTOU (stage 2 note): the Clash and mixed ports are
//! allocated at config generation, but the engine binds them later.
//! If the engine dies on `bind: address already in use`, the start
//! regenerates the config (fresh ports and secrets) and retries once
//! instead of failing the start.
//!
//! Boundaries: no IPC, no UI - the manager is internal. The
//! supervisor holds it as a field, starts it (stage 4: order,
//! `--wf-raw-filter`, the api.cloudflareclient.com recipe) and stops
//! it on engine stop; stage 5 exposes it over IPC. The manager is
//! registered in `AppState::sidecars` (stage 4, `main.rs`), so
//! `install_update` refuses while the tunnel is active - in the GUI
//! process only; when the GUI is attached to the svc, the tunnel
//! lives in the svc and the local manager is Stopped, so the updater
//! cannot see it there yet (stage 5/6 follow-up: IPC TunnelGetStatus).

use std::collections::{HashSet, VecDeque};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc::Sender;

use crate::engine::clash_api::{ClashApiClient, ClashApiError};
use crate::engine::logger::FileLogger;
use crate::engine::managed::ManagedSidecar;
use crate::engine::singbox_config::{
    build_config, check_config, write_config_file, DohEndpoint, SingBoxConfigError,
    SingBoxConfigInput, TunScope, MASQUE_TAG,
};
use crate::engine::LogEntry;
use crate::platform::job_object::JobObject;
use crate::platform::process::{spawn_process, ProcessError};

#[cfg(windows)]
use std::net::{Ipv4Addr, Ipv6Addr};
#[cfg(windows)]
use windows::core::{w, PWSTR};
#[cfg(windows)]
use windows::Win32::Foundation::ERROR_SUCCESS;
#[cfg(windows)]
use windows::Win32::NetworkManagement::IpHelper::{
    GetAdaptersAddresses, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_MULTICAST, GAA_FLAG_SKIP_UNICAST,
    IF_TYPE_SOFTWARE_LOOPBACK, IP_ADAPTER_ADDRESSES_LH,
};
#[cfg(windows)]
use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
#[cfg(windows)]
use windows::Win32::Networking::WinSock::{
    AF_INET, AF_INET6, AF_UNSPEC, SOCKADDR_IN, SOCKADDR_IN6, SOCKET_ADDRESS,
};
#[cfg(windows)]
use windows::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ, REG_DWORD,
};

/// How long `start()` waits for the Clash API to answer after spawn
/// (NETZapret's readiness timeout is also 20 s).
const READINESS_TIMEOUT: Duration = Duration::from_secs(20);
/// Poll interval of the readiness wait.
const READINESS_POLL_INTERVAL: Duration = Duration::from_millis(250);
/// Engine-side deadline for the warmup exit measurement.
const WARMUP_DELAY_TIMEOUT: Duration = Duration::from_secs(10);
/// Probe target for the warmup exit measurement (a tiny 204).
const WARMUP_DELAY_URL: &str = "https://www.google.com/generate_204";
/// Start attempts: the first try plus one regeneration on a port-bind
/// conflict (TOCTOU). A second bind failure means something is
/// systematically wrong, so the start fails instead of looping.
const MAX_START_ATTEMPTS: u32 = 2;
/// How long `stop()` (and the timeout path of `start()`) waits for
/// the killed child to exit before giving up on the wait.
const STOP_WAIT_TIMEOUT: Duration = Duration::from_secs(5);
/// Monitor poll interval, same as the supervisor's winws2 monitor.
const MONITOR_POLL_INTERVAL: Duration = Duration::from_millis(500);
/// How many recent stderr lines are kept per start attempt, so a
/// failed start can report why the engine died instead of only its
/// exit status.
const STDERR_TAIL_LINES: usize = 5;

/// Tunnel states (design section 7). Separate from the desync engine
/// states: the tunnel has its own lifecycle, and a dead exit is not a
/// dead engine (`Degraded` / `Unregistered` vs `Error`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TunnelState {
    /// The engine is not running.
    Stopped,
    /// A start is in progress (config, spawn, readiness, warmup).
    Starting,
    /// The engine is up and the warmup exit measurement answered.
    Running,
    /// A stop is in progress.
    Stopping,
    /// The engine is gone (exited unexpectedly, or a start failed).
    Error,
    /// The engine is up but the exit did not answer during warmup
    /// (or the watchdog's deep check, stage 6). Not a reason to
    /// restart the engine.
    Degraded,
    /// The engine is up but WARP has not registered yet
    /// ("tunnel not initialized").
    Unregistered,
}

/// Snapshot of the tunnel status. Carries the Clash API *port* (the
/// expert view shows it, stage 5) but never the secret.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SingBoxStatus {
    pub state: TunnelState,
    pub pid: Option<u32>,
    pub engine_version: Option<String>,
    pub clash_api_port: Option<u16>,
    pub warmup_delay_ms: Option<u64>,
    pub last_error: Option<String>,
}

/// Everything `start()` needs that the manager cannot read itself.
#[derive(Debug, Clone)]
pub struct SingBoxStartOptions {
    /// Path to `sing-box.exe` (resolved by `resolve_binaries`, stage 1).
    pub binary: PathBuf,
    /// Working directory for the engine process: `bin/` where
    /// `wintun.dll` lives (the engine loads it from the working dir).
    pub work_dir: PathBuf,
    /// `%LOCALAPPDATA%\zapret2\singbox\` (see `singbox_home_dir`):
    /// config, cache, and engine log live here.
    pub home_dir: PathBuf,
    /// TUN scope: proxy domains only while the desync engine is on,
    /// everything when the tunnel runs alone (TUN inversion).
    pub tun_scope: TunScope,
    /// Whether the desync engine is running: only then are the ipset
    /// subnets of the active preset excluded from the tunnel.
    pub desync_on: bool,
    /// Directory with `ipset-all.txt` (the lists directory of the
    /// resolved binaries). `None` means no exclusion.
    pub lists_dir: Option<PathBuf>,
    /// `settings.dns_provider` - drives the DoH ladder of the
    /// generated config.
    pub dns_provider: String,
    /// Whether the generated config includes the TUN inbound.
    /// Production always sets this `true`; `false` is the stage 4
    /// live-test seam (the sandbox cannot create a TUN device).
    /// See `SingBoxConfigInput::tun_enabled`.
    pub tun_enabled: bool,
    /// Pin the Clash API port for the first start attempt (`None` in
    /// production). The stage 4 live retry test pins a port it
    /// already occupies; the pin is cleared after the first config
    /// build so the retry allocates fresh ports.
    /// See `SingBoxConfigInput::fixed_clash_port`.
    pub fixed_clash_port: Option<u16>,
}

/// Errors from the sing-box manager.
#[derive(Debug, Error)]
pub enum SingBoxError {
    #[error("sing-box binary not found: {}", .0.display())]
    BinaryMissing(PathBuf),

    #[error("failed to create the sing-box home directory {}: {source}", .path.display())]
    HomeDirFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("sing-box version check failed: {0}")]
    VersionFailed(String),

    #[error("sing-box config error: {0}")]
    Config(#[from] SingBoxConfigError),

    #[error("sing-box rejected the generated config: {0}")]
    CheckFailed(String),

    #[error("failed to spawn sing-box: {0}")]
    SpawnFailed(#[from] ProcessError),

    #[error("sing-box did not become ready in {0:?}: {1}")]
    NotReady(Duration, String),

    #[error("sing-box exited during start: {0}")]
    ExitedDuringStart(String),
}

impl From<SingBoxError> for ProcessError {
    fn from(e: SingBoxError) -> Self {
        ProcessError::Tunnel(e.to_string())
    }
}

/// Machine state the config generator needs but cannot read itself.
#[derive(Debug, Clone, Default)]
pub struct SystemState {
    /// Resolver addresses from up, non-loopback adapters (own tunnel
    /// excluded, loopback resolvers filtered). Empty on non-Windows.
    pub resolvers: Vec<IpAddr>,
    /// Whether the machine can give the TUN adapter an IPv6 address.
    pub ipv6: bool,
}

/// Read the machine state for the config generator.
///
/// Windows: resolvers come from `GetAdaptersAddresses`, IPv6 from the
/// `DisabledComponents` registry value (fail open on any read
/// failure, like NETZapret). Other platforms: no resolver discovery
/// (WARP is a Windows-target feature; elsewhere the manager only runs
/// in tests, without TUN), IPv6 from `/proc/net/if_inet6`.
#[cfg(windows)]
pub fn collect_system_state() -> SystemState {
    SystemState {
        resolvers: discover_system_resolvers(),
        ipv6: !ipv6_disabled_by_registry(),
    }
}

#[cfg(not(windows))]
pub fn collect_system_state() -> SystemState {
    SystemState {
        resolvers: Vec::new(),
        ipv6: linux_ipv6_available(),
    }
}

#[cfg(not(windows))]
fn linux_ipv6_available() -> bool {
    match std::fs::read_to_string("/proc/net/if_inet6") {
        Ok(content) => if_inet6_has_entries(&content),
        // No /proc entry (a kernel without IPv6, or a non-Linux
        // system): fail open, like the Windows registry path.
        Err(_) => true,
    }
}

/// Does `/proc/net/if_inet6` list any interface? An empty file means
/// no interface has an IPv6 address.
fn if_inet6_has_entries(content: &str) -> bool {
    content.lines().any(|line| !line.trim().is_empty())
}

/// Discover the system's DNS resolvers via `GetAdaptersAddresses`
/// (NETZapret's `SystemResolvers.Discover`): up adapters only, the
/// loopback adapter skipped, our own tunnel adapter skipped (its
/// resolvers would be our own fake ones), loopback resolver addresses
/// filtered out, deduplicated.
#[cfg(windows)]
fn discover_system_resolvers() -> Vec<IpAddr> {
    // WinError.h: the first call returns ERROR_BUFFER_OVERFLOW and
    // the required buffer size.
    const ERROR_BUFFER_OVERFLOW: u32 = 111;
    // We need DNS servers, OperStatus, IfType, and the adapter names
    // only - no address lists.
    let flags = GAA_FLAG_SKIP_UNICAST | GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST;
    let family = u32::from(AF_UNSPEC.0);

    let mut size: u32 = 0;
    let first = unsafe { GetAdaptersAddresses(family, flags, None, None, &mut size) };
    if first != ERROR_BUFFER_OVERFLOW || size == 0 {
        return Vec::new();
    }
    let mut buffer = vec![0u8; size as usize];
    let filled = unsafe {
        GetAdaptersAddresses(
            family,
            flags,
            None,
            Some(buffer.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH),
            &mut size,
        )
    };
    if filled != 0 {
        return Vec::new();
    }

    let mut resolvers = Vec::new();
    let mut seen = HashSet::new();
    let mut current = buffer.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
    while !current.is_null() {
        let adapter = unsafe { &*current };
        if adapter.OperStatus == IfOperStatusUp
            && adapter.IfType != IF_TYPE_SOFTWARE_LOOPBACK
            && !is_own_tunnel_adapter(adapter)
        {
            let mut server = adapter.FirstDnsServerAddress;
            while !server.is_null() {
                let entry = unsafe { &*server };
                if let Some(ip) = socket_address_to_ip(&entry.Address) {
                    if !ip.is_loopback() && seen.insert(ip) {
                        resolvers.push(ip);
                    }
                }
                server = entry.Next;
            }
        }
        current = adapter.Next;
    }
    resolvers
}

/// Is this adapter our own sing-box TUN interface (or any wintun
/// interface)? The markers are the same as NETZapret's, with our own
/// name in place of theirs.
#[cfg(windows)]
fn is_own_tunnel_adapter(adapter: &IP_ADAPTER_ADDRESSES_LH) -> bool {
    const TUNNEL_MARKERS: [&str; 3] = ["sing-tun", "wintun", "zapret2"];
    let description = pwstr_to_string(adapter.Description).to_lowercase();
    let friendly = pwstr_to_string(adapter.FriendlyName).to_lowercase();
    TUNNEL_MARKERS
        .iter()
        .any(|marker| description.contains(marker) || friendly.contains(marker))
}

#[cfg(windows)]
fn pwstr_to_string(value: PWSTR) -> String {
    if value.is_null() {
        return String::new();
    }
    unsafe {
        let mut len = 0usize;
        while *value.0.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(value.0, len))
    }
}

/// Convert a `SOCKET_ADDRESS` from the adapter list into an `IpAddr`.
/// IPv6 zone indices (`fe80::1%6`) cannot appear here: DNS server
/// addresses from `GetAdaptersAddresses` carry no zone.
#[cfg(windows)]
fn socket_address_to_ip(address: &SOCKET_ADDRESS) -> Option<IpAddr> {
    if address.lpSockaddr.is_null() {
        return None;
    }
    unsafe {
        let family = (*address.lpSockaddr).sa_family;
        if family == AF_INET {
            if address.iSockaddrLength < std::mem::size_of::<SOCKADDR_IN>() as i32 {
                return None;
            }
            let sin = &*(address.lpSockaddr as *const SOCKADDR_IN);
            let bytes = sin.sin_addr.S_un.S_un_b;
            Some(IpAddr::V4(Ipv4Addr::new(
                bytes.s_b1, bytes.s_b2, bytes.s_b3, bytes.s_b4,
            )))
        } else if family == AF_INET6 {
            if address.iSockaddrLength < std::mem::size_of::<SOCKADDR_IN6>() as i32 {
                return None;
            }
            let sin6 = &*(address.lpSockaddr as *const SOCKADDR_IN6);
            Some(IpAddr::V6(Ipv6Addr::from(sin6.sin6_addr.u.Byte)))
        } else {
            None
        }
    }
}

/// Is IPv6 disabled for non-tunnel adapters? Reads
/// `HKLM\SYSTEM\CurrentControlSet\Services\Tcpip6\Parameters\DisabledComponents`,
/// bit 0x10 (NonTunnel). Wintun counts as a non-tunnel adapter here,
/// so bit 0x10 alone is enough to break the TUN IPv6 address
/// (NETZapret issue 34: `0xffffffff` crashed sing-box with
/// "configure tun interface: set ipv6 address: Element not found").
/// Bit 0x01 alone does not count. Any read failure means "IPv6
/// present" (fail open).
#[cfg(windows)]
fn ipv6_disabled_by_registry() -> bool {
    unsafe {
        let mut key = HKEY::default();
        let opened = RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            w!("SYSTEM\\CurrentControlSet\\Services\\Tcpip6\\Parameters"),
            0,
            KEY_READ,
            &mut key,
        );
        if opened != ERROR_SUCCESS {
            return false;
        }
        let mut data = [0u8; 4];
        let mut data_len = data.len() as u32;
        let mut value_type = REG_DWORD;
        let queried = RegQueryValueExW(
            key,
            w!("DisabledComponents"),
            None,
            Some(&mut value_type),
            Some(data.as_mut_ptr()),
            Some(&mut data_len),
        );
        let _ = RegCloseKey(key);
        if queried != ERROR_SUCCESS || value_type != REG_DWORD || data_len != 4 {
            return false;
        }
        disabled_components_disables_ipv6(u32::from_le_bytes(data))
    }
}

/// Bit 0x10 of `DisabledComponents` disables IPv6 on all non-tunnel
/// adapters; any other bit combination leaves it on.
fn disabled_components_disables_ipv6(value: u32) -> bool {
    value & 0x10 != 0
}

/// Read the desync preset's ipset subnets for `route_exclude_address`
/// (the safety net that keeps desync-covered subnets out of the
/// tunnel). Reads `ipset-all.txt` from the lists directory; a missing
/// file yields an empty list - the exclusion is a safety net, not a
/// gate (NETZapret's `ReadListFile` behaves the same). Returns the
/// CIDRs and the count of skipped unparseable lines.
pub fn read_ipset_cidrs(lists_dir: Option<&Path>) -> (Vec<String>, usize) {
    let Some(dir) = lists_dir else {
        return (Vec::new(), 0);
    };
    match std::fs::read_to_string(dir.join("ipset-all.txt")) {
        Ok(content) => parse_ipset_content(&content),
        Err(_) => (Vec::new(), 0),
    }
}

/// Parse an ipset list file (NETZapret's `ReadListFile` +
/// `NormalizeCidr`): blank lines and `#` comments skipped, inline
/// `#` comments stripped, bare IPs converted to /32 or /128,
/// duplicates removed. Returns the CIDRs and the count of skipped
/// unparseable lines.
pub fn parse_ipset_content(content: &str) -> (Vec<String>, usize) {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut skipped = 0usize;
    for raw in content.lines() {
        let mut line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((before, _comment)) = line.split_once('#') {
            line = before.trim();
        }
        if line.is_empty() {
            continue;
        }
        match normalize_cidr(line) {
            Some(cidr) => {
                if seen.insert(cidr.clone()) {
                    out.push(cidr);
                }
            }
            None => skipped += 1,
        }
    }
    (out, skipped)
}

/// Validate a CIDR (or convert a bare IP to /32 or /128). CIDR text
/// is passed through unchanged - sing-box parses it itself.
fn normalize_cidr(value: &str) -> Option<String> {
    if let Some((ip, prefix)) = value.split_once('/') {
        let addr: IpAddr = ip.parse().ok()?;
        let prefix_len: u32 = prefix.parse().ok()?;
        let max = if addr.is_ipv4() { 32 } else { 128 };
        if prefix_len > max {
            return None;
        }
        return Some(value.to_string());
    }
    let addr: IpAddr = value.parse().ok()?;
    let bits = if addr.is_ipv6() { 128 } else { 32 };
    Some(format!("{addr}/{bits}"))
}

/// Default sing-box home directory:
/// `%LOCALAPPDATA%\zapret2\singbox\`, with the same fallbacks as the
/// log directory (exe dir, then temp). The engine's config, cache
/// file, and log live here; `start()` creates it before the first
/// start because the engine crashes without it.
pub fn singbox_home_dir() -> PathBuf {
    if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
        if !local_appdata.is_empty() {
            return PathBuf::from(local_appdata).join("zapret2").join("singbox");
        }
    }
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(parent) = exe_path.parent() {
            return parent.join("singbox");
        }
    }
    std::env::temp_dir().join("zapret2-singbox")
}

/// Run `sing-box version` and return the first output line
/// (e.g. `sing-box version 1.14.3-extended-2.7.3`). Synchronous: the
/// command prints and exits immediately. Failing to run it means the
/// binary cannot execute at all, so the caller fails the start.
/// The version line goes into the log at every start (stage 1 item,
/// deferred here because a version check is a process launch).
pub fn singbox_version(binary: &Path) -> Result<String, SingBoxError> {
    let mut command = std::process::Command::new(binary);
    command.arg("version");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Same flag as spawn_process: no console window for the
        // short-lived helper.
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command
        .output()
        .map_err(|error| SingBoxError::VersionFailed(error.to_string()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(SingBoxError::VersionFailed(format!(
            "exit status {}: {}",
            output.status,
            stderr.trim()
        )));
    }
    parse_version_output(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| SingBoxError::VersionFailed("empty output".to_string()))
}

/// First non-empty line of `sing-box version` output.
fn parse_version_output(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

/// Does this stderr line report a port-bind conflict? The engine
/// prints Go's `bind: address already in use` when the Clash or mixed
/// port was taken between allocation and bind (stage 2 TOCTOU).
fn is_bind_error_line(line: &str) -> bool {
    line.to_lowercase().contains("address already in use")
}

/// Does this Clash error message mean the engine is up but WARP has
/// not registered yet? sing-box answers delay measurements with
/// "tunnel not initialized" until the first MASQUE registration
/// completes (or fails).
fn is_tunnel_not_initialized(message: &str) -> bool {
    message.to_lowercase().contains("tunnel not initialized")
}

/// Fill the generator input from the start options, the machine state,
/// and the preset's ipset CIDRs. Pure: the caller reads the system
/// state and the ipset file, this function only assembles the input.
pub fn fill_config_input(
    options: &SingBoxStartOptions,
    system: &SystemState,
    ipset_cidrs: &[String],
) -> SingBoxConfigInput {
    SingBoxConfigInput {
        tun_scope: options.tun_scope,
        system_resolvers: system.resolvers.clone(),
        // The exclusion only makes sense while the desync engine is
        // actually running: with the tunnel alone, every subnet is
        // the tunnel's.
        desync_exclude_cidrs: if options.desync_on {
            ipset_cidrs.to_vec()
        } else {
            Vec::new()
        },
        doh: DohEndpoint::from_provider_id(&options.dns_provider),
        ipv6: system.ipv6,
        cache_file_path: options.home_dir.join("engine-cache.db"),
        log_file_path: options.home_dir.join("sing-box.log"),
        tun_enabled: options.tun_enabled,
        fixed_clash_port: options.fixed_clash_port,
    }
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Outcome of one readiness wait.
enum Readiness {
    /// The Clash API answered.
    Ready,
    /// The child died before becoming ready.
    Died(std::process::ExitStatus),
    /// The deadline passed with the child alive but the API silent.
    Timeout,
}

/// Error from one spawn attempt. `bind_error` marks a port-bind
/// conflict (TOCTOU on the allocated ports): the caller regenerates
/// the config and retries instead of failing the start.
#[derive(Debug)]
struct SpawnAttemptError {
    error: SingBoxError,
    bind_error: bool,
}

/// Manager for the sing-box sidecar: spawn, monitor, stop, warmup.
/// Clone is cheap (all state is behind Arcs) - the monitor task holds
/// a clone, and the supervisor holds the manager in an `Arc`.
#[derive(Clone)]
pub struct SingBoxManager {
    job_object: Arc<JobObject>,
    log_tx: Sender<LogEntry>,
    file_logger: Arc<FileLogger>,
    lifecycle: Arc<tokio::sync::Mutex<()>>,
    generation: Arc<RwLock<u64>>,
    state: Arc<RwLock<TunnelState>>,
    child: Arc<Mutex<Option<tokio::process::Child>>>,
    pid: Arc<RwLock<Option<u32>>>,
    engine_version: Arc<RwLock<Option<String>>>,
    clash_api_port: Arc<RwLock<Option<u16>>>,
    warmup_delay_ms: Arc<RwLock<Option<u64>>>,
    last_error: Arc<RwLock<Option<String>>>,
    /// The TUN scope the running engine started with (`None` while
    /// stopped). The supervisor reads it to reconcile the tunnel
    /// after a desync start: a tunnel running with the wrong scope
    /// (desync came up after it) is restarted with ProxyOnly.
    tun_scope: Arc<RwLock<Option<TunScope>>>,
}

impl SingBoxManager {
    /// Build a manager. Cheap and infallible; nothing is started
    /// here. `log_tx` feeds the shared in-memory log, `file_logger`
    /// the shared log file - both get `[SINGBOX]`-prefixed lines.
    pub fn new(
        job_object: Arc<JobObject>,
        log_tx: Sender<LogEntry>,
        file_logger: Arc<FileLogger>,
    ) -> Self {
        Self {
            job_object,
            log_tx,
            file_logger,
            lifecycle: Arc::new(tokio::sync::Mutex::new(())),
            generation: Arc::new(RwLock::new(0)),
            state: Arc::new(RwLock::new(TunnelState::Stopped)),
            child: Arc::new(Mutex::new(None)),
            pid: Arc::new(RwLock::new(None)),
            engine_version: Arc::new(RwLock::new(None)),
            clash_api_port: Arc::new(RwLock::new(None)),
            warmup_delay_ms: Arc::new(RwLock::new(None)),
            last_error: Arc::new(RwLock::new(None)),
            tun_scope: Arc::new(RwLock::new(None)),
        }
    }

    /// Current tunnel status (never contains the Clash secret).
    pub fn status(&self) -> SingBoxStatus {
        SingBoxStatus {
            state: *self.state.read(),
            pid: *self.pid.read(),
            engine_version: self.engine_version.read().clone(),
            clash_api_port: *self.clash_api_port.read(),
            warmup_delay_ms: *self.warmup_delay_ms.read(),
            last_error: self.last_error.read().clone(),
        }
    }

    /// Current tunnel state.
    pub fn state(&self) -> TunnelState {
        *self.state.read()
    }

    /// The TUN scope the running engine started with, or `None`
    /// while stopped (or after an unexpected death). The supervisor
    /// uses it for the tunnel reconcile after a desync start.
    pub fn tun_scope(&self) -> Option<TunScope> {
        *self.tun_scope.read()
    }

    /// Start the tunnel: home dir, version check, config generation
    /// and check, spawn into the shared Job Object, Clash readiness,
    /// warmup, monitor. Idempotent: a start while the engine is
    /// already up (or coming up) returns the current status without
    /// restarting anything.
    pub async fn start(&self, options: SingBoxStartOptions) -> Result<SingBoxStatus, SingBoxError> {
        let _guard = self.lifecycle.lock().await;

        match *self.state.read() {
            TunnelState::Running
            | TunnelState::Degraded
            | TunnelState::Unregistered
            | TunnelState::Starting => return Ok(self.status()),
            TunnelState::Stopped | TunnelState::Stopping | TunnelState::Error => {}
        }

        // The bump invalidates any monitor task from a previous
        // start. It must come AFTER the idempotency check above:
        // bumping first would orphan the monitor of the running
        // engine (review M1) - the monitor gates the death report
        // on the generation, and a redundant start() would leave
        // it stale, so a later engine death would be lost silently
        // while the state stayed Running/Degraded forever.
        *self.generation.write() += 1;

        *self.state.write() = TunnelState::Starting;
        self.set_last_error(None);
        *self.warmup_delay_ms.write() = None;
        *self.engine_version.write() = None;
        *self.clash_api_port.write() = None;
        *self.pid.write() = None;

        match self.start_inner(&options).await {
            Ok(()) => {
                *self.tun_scope.write() = Some(options.tun_scope);
                Ok(self.status())
            }
            Err(error) => {
                // The attempt layer already cleaned up the child; this
                // is a guard against a future path that forgets.
                self.kill_child().await;
                *self.pid.write() = None;
                *self.clash_api_port.write() = None;
                *self.tun_scope.write() = None;
                self.set_last_error(Some(error.to_string()));
                *self.state.write() = TunnelState::Error;
                self.log("error", &format!("failed to start sing-box: {error}"));
                Err(error)
            }
        }
    }

    /// The start itself, under the lifecycle lock. On success the
    /// state is Running, Degraded, or Unregistered and the monitor
    /// is armed; on failure no child is running.
    async fn start_inner(&self, options: &SingBoxStartOptions) -> Result<(), SingBoxError> {
        if !options.binary.is_file() {
            return Err(SingBoxError::BinaryMissing(options.binary.clone()));
        }

        // The engine crashes if its home directory does not exist
        // (cache file + log). Create it before the first start.
        std::fs::create_dir_all(&options.home_dir).map_err(|source| {
            SingBoxError::HomeDirFailed {
                path: options.home_dir.clone(),
                source,
            }
        })?;

        // Version first: it proves the binary executes at all, and
        // the version line goes into the log for every start.
        let version = singbox_version(&options.binary)?;
        self.log("info", &format!("sing-box version: {version}"));
        *self.engine_version.write() = Some(version);

        // Clear any stale child from a previous failed or cancelled
        // start; normally the slot is empty.
        self.kill_child().await;

        let system = collect_system_state();
        // One line so a silent resolver-discovery failure is visible:
        // with zero resolvers ProxyOnly loses hijack-dns coverage of
        // the system resolvers (review m5). Empty is expected on
        // non-Windows (no discovery there - tests only).
        self.log(
            "info",
            &format!(
                "system state: {} resolver(s), ipv6={}",
                system.resolvers.len(),
                system.ipv6
            ),
        );
        let (ipset_cidrs, skipped) = read_ipset_cidrs(options.lists_dir.as_deref());
        if skipped > 0 {
            self.log(
                "warn",
                &format!("ignored {skipped} unparseable line(s) in ipset-all.txt"),
            );
        }
        let mut input = fill_config_input(options, &system, &ipset_cidrs);
        let config_path = options.home_dir.join("config.json");

        for attempt in 1..=MAX_START_ATTEMPTS {
            // Fresh ports and secrets on every build - that is what
            // makes the TOCTOU retry work.
            let generated = build_config(&input)?;
            // The Clash port pin exists only to force the first
            // attempt onto a port the test occupies (stage 4 live
            // retry test): clear it after the first build, so a
            // retry allocates fresh ports instead of failing the
            // same way on the same occupied port.
            input.fixed_clash_port = None;
            write_config_file(&generated, &config_path)?;
            let check = check_config(&options.binary, &config_path)?;
            if !check.success {
                return Err(SingBoxError::CheckFailed(check.stderr));
            }
            match self
                .spawn_and_warm(
                    &options.binary,
                    &options.work_dir,
                    &config_path,
                    generated.clash_api_port,
                    &generated.clash_api_secret,
                )
                .await
            {
                Ok(()) => return Ok(()),
                Err(attempt_error) if attempt_error.bind_error && attempt < MAX_START_ATTEMPTS => {
                    self.log(
                        "warn",
                        &format!(
                            "sing-box port conflict on attempt {attempt}, regenerating config: {}",
                            attempt_error.error
                        ),
                    );
                }
                Err(attempt_error) => return Err(attempt_error.error),
            }
        }
        // Unreachable with MAX_START_ATTEMPTS >= 1: the loop returns
        // on the last attempt. Kept as a guard so a future change to
        // the constant cannot silently report success.
        Err(SingBoxError::NotReady(
            READINESS_TIMEOUT,
            "start attempts exhausted".to_string(),
        ))
    }

    /// Spawn the engine from an already-written config file, wait for
    /// Clash API readiness, warm up, and arm the monitor task.
    ///
    /// Called by `start()` (after config generation) and by the live
    /// test (with a hand-written minimal config, no TUN - tests must
    /// not raise a TUN adapter). The caller holds the lifecycle lock
    /// (production) or runs single-threaded (test); this method does
    /// not take it.
    ///
    /// On success the state is Running, Degraded, or Unregistered.
    /// On failure the (dead) child is removed and the state is left
    /// as Starting - the caller decides whether to retry (bind
    /// conflict) or fail the start.
    async fn spawn_and_warm(
        &self,
        binary: &Path,
        work_dir: &Path,
        config_path: &Path,
        clash_port: u16,
        clash_secret: &str,
    ) -> Result<(), SpawnAttemptError> {
        let client = ClashApiClient::new(clash_port, clash_secret);
        let args = vec![
            "run".to_string(),
            "-c".to_string(),
            config_path.to_string_lossy().into_owned(),
        ];
        let spawned =
            spawn_process(binary, work_dir, &args, self.job_object.clone()).map_err(|error| {
                SpawnAttemptError {
                    error: SingBoxError::SpawnFailed(error),
                    bind_error: false,
                }
            })?;
        *self.pid.write() = Some(spawned.pid);
        *self.clash_api_port.write() = Some(clash_port);
        self.log("info", &format!("sing-box started (pid {})", spawned.pid));

        let bind_error = Arc::new(AtomicBool::new(false));
        let stderr_tail: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(VecDeque::new()));
        self.spawn_pumps(
            spawned.stdout,
            spawned.stderr,
            bind_error.clone(),
            stderr_tail.clone(),
        );
        *self.child.lock() = Some(spawned.child);

        match self.wait_ready(&client).await {
            Readiness::Ready => {}
            Readiness::Died(status) => {
                let _ = self.child.lock().take();
                // Give the stderr pump a moment to drain the pipe:
                // the FATAL line is written before the exit, but the
                // pump task may not have processed it yet when
                // try_wait reports the death - and the bind-error
                // flag (TOCTOU retry) depends on that line.
                // Known limitation (review m2): 100 ms is a
                // heuristic, not a guarantee - on a loaded machine
                // the pump may still be behind, and the start then
                // fails instead of retrying. The failure direction
                // is safe (a clear error, no retry); a structural
                // fix would join on the pump task.
                tokio::time::sleep(Duration::from_millis(100)).await;
                let tail = {
                    let tail = stderr_tail.lock();
                    tail.iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                let reason = if tail.is_empty() {
                    format!("exit status: {status}")
                } else {
                    format!("exit status: {status}; stderr:\n{tail}")
                };
                return Err(SpawnAttemptError {
                    error: SingBoxError::ExitedDuringStart(reason),
                    bind_error: bind_error.load(Ordering::SeqCst),
                });
            }
            Readiness::Timeout => {
                // The child is alive but the API never answered -
                // kill it instead of leaving a stray engine behind.
                self.kill_child().await;
                return Err(SpawnAttemptError {
                    error: SingBoxError::NotReady(
                        READINESS_TIMEOUT,
                        "clash api did not answer".to_string(),
                    ),
                    bind_error: false,
                });
            }
        }

        if let Err(error) = self.warmup(&client).await {
            return Err(SpawnAttemptError {
                error,
                bind_error: false,
            });
        }
        self.spawn_monitor();
        Ok(())
    }

    /// Poll Clash API `alive()` until the engine answers, the child
    /// dies, or the deadline passes.
    async fn wait_ready(&self, client: &ClashApiClient) -> Readiness {
        let deadline = Instant::now() + READINESS_TIMEOUT;
        loop {
            // Child first: a dead engine can never become ready.
            if let Some(status) = self.try_wait_child() {
                return Readiness::Died(status);
            }
            if client.alive().await {
                return Readiness::Ready;
            }
            if Instant::now() >= deadline {
                return Readiness::Timeout;
            }
            tokio::time::sleep(READINESS_POLL_INTERVAL).await;
        }
    }

    /// Warmup: measure the WARP exit once before the first health
    /// judgment (design section 6). A working exit means Running; a
    /// "tunnel not initialized" answer means Unregistered; anything
    /// else means Degraded - the engine is up, the exit is not, and
    /// neither case restarts the engine.
    ///
    /// The design's selector step (explicitly set the selector via
    /// Clash API to fix cache stickiness) is intentionally skipped:
    /// the v1 config has no selector group - the MASQUE outbound is
    /// referenced directly - so there is nothing to select.
    async fn warmup(&self, client: &ClashApiClient) -> Result<(), SingBoxError> {
        // The engine was alive a moment ago; if it died in between,
        // say so instead of measuring a corpse.
        if let Some(status) = self.try_wait_child() {
            let _ = self.child.lock().take();
            return Err(SingBoxError::ExitedDuringStart(format!(
                "exit status: {status}"
            )));
        }
        match client
            .measure_exit(MASQUE_TAG, WARMUP_DELAY_TIMEOUT, WARMUP_DELAY_URL)
            .await
        {
            Ok(delay) => {
                *self.warmup_delay_ms.write() = Some(delay);
                *self.state.write() = TunnelState::Running;
                self.log("info", &format!("warp exit warm: {delay} ms"));
                Ok(())
            }
            Err(ClashApiError::ExitFailed(message)) if is_tunnel_not_initialized(&message) => {
                *self.state.write() = TunnelState::Unregistered;
                self.log(
                    "warn",
                    "engine is up but warp is not registered yet (tunnel not initialized)",
                );
                Ok(())
            }
            Err(error) => {
                *self.state.write() = TunnelState::Degraded;
                self.log(
                    "warn",
                    &format!("warp exit did not answer during warmup: {error}"),
                );
                Ok(())
            }
        }
    }

    /// Arm the monitor task: poll the child every 500 ms (same as the
    /// supervisor's winws2 monitor). An unexpected exit moves the
    /// tunnel to `Error`, guarded by the generation counter so a
    /// stale monitor cannot act on a newer start's child. The
    /// generation is read here, at arm time - the caller holds the
    /// lifecycle lock, so no other start/stop can bump it between
    /// the bump in `start()` and this read.
    fn spawn_monitor(&self) {
        let generation = *self.generation.read();
        let this = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(MONITOR_POLL_INTERVAL).await;
                let mut guard = this.child.lock();
                match guard.as_mut() {
                    Some(child) => match child.try_wait() {
                        Ok(Some(status)) => {
                            *guard = None;
                            drop(guard);
                            this.on_engine_gone(
                                &format!("sing-box exited unexpectedly: {status}"),
                                generation,
                            );
                            return;
                        }
                        Ok(None) => {}
                        Err(error) => {
                            this.log(
                                "error",
                                &format!("sing-box monitor: try_wait failed: {error}"),
                            );
                            *guard = None;
                            drop(guard);
                            this.on_engine_gone(
                                &format!("sing-box monitor lost the child: {error}"),
                                generation,
                            );
                            return;
                        }
                    },
                    // Taken by stop(): the monitor's job is done.
                    None => return,
                }
            }
        });
    }

    /// Record an unexpected engine death, unless a newer start owns
    /// the slot now (generation guard) or the tunnel is already
    /// stopping/stopped.
    fn on_engine_gone(&self, reason: &str, generation: u64) {
        if *self.generation.read() != generation {
            return;
        }
        *self.pid.write() = None;
        // Same cleanup as stop(): the port dies with the engine, and
        // a stale port in status() would point at nothing (review m3).
        // The scope goes with it - there is no running engine to
        // reconcile against.
        *self.clash_api_port.write() = None;
        *self.tun_scope.write() = None;
        let mut state = self.state.write();
        if matches!(
            *state,
            TunnelState::Running | TunnelState::Degraded | TunnelState::Unregistered
        ) {
            *state = TunnelState::Error;
            drop(state);
            self.set_last_error(Some(reason.to_string()));
            self.log("error", reason);
        }
    }

    /// Stop the tunnel: kill the child, wait up to 5 s, clear the
    /// state. A no-op while stopped. Deliberately does NOT terminate
    /// the shared Job Object (winws2 lives in the same job - the
    /// independent-switches matrix requires tunnel-off/desync-on to
    /// keep working); sing-box spawns no grandchildren, so kill+wait
    /// is a complete stop.
    pub async fn stop(&self) -> Result<(), SingBoxError> {
        let _guard = self.lifecycle.lock().await;
        *self.generation.write() += 1;

        if matches!(
            *self.state.read(),
            TunnelState::Stopped | TunnelState::Stopping
        ) {
            return Ok(());
        }
        *self.state.write() = TunnelState::Stopping;
        self.log("info", "stopping sing-box...");

        self.kill_child().await;

        *self.pid.write() = None;
        *self.clash_api_port.write() = None;
        *self.warmup_delay_ms.write() = None;
        *self.tun_scope.write() = None;
        *self.state.write() = TunnelState::Stopped;
        self.file_logger.flush();
        self.log("info", "sing-box stopped");
        Ok(())
    }

    /// Kill the current child (if any) and wait for it, up to
    /// `STOP_WAIT_TIMEOUT`. Takes the child out of the slot first so
    /// the monitor task sees `None` and exits.
    async fn kill_child(&self) {
        // Take the child out of the slot first (dropping the guard
        // before the awaits: parking_lot guards are not Send).
        let child = self.child.lock().take();
        if let Some(mut child) = child {
            let _ = child.kill().await;
            let _ = tokio::time::timeout(STOP_WAIT_TIMEOUT, child.wait()).await;
        }
    }

    /// Non-blocking exit check for the current child.
    fn try_wait_child(&self) -> Option<std::process::ExitStatus> {
        let mut guard = self.child.lock();
        let child = guard.as_mut()?;
        match child.try_wait() {
            Ok(Some(status)) => Some(status),
            _ => None,
        }
    }

    /// Spawn the stdout/stderr pumps (same shape as the supervisor's
    /// winws2 pumps): lines go to the shared log file with level
    /// SINGBOX and to the in-memory log with a `[SINGBOX]` prefix.
    /// The stderr pump additionally watches for bind-error lines
    /// (TOCTOU) and keeps a short tail for start-failure diagnostics.
    fn spawn_pumps(
        &self,
        stdout: Option<tokio::process::ChildStdout>,
        stderr: Option<tokio::process::ChildStderr>,
        bind_error: Arc<AtomicBool>,
        stderr_tail: Arc<Mutex<VecDeque<String>>>,
    ) {
        if let Some(stdout) = stdout {
            let file_logger = self.file_logger.clone();
            let log_tx = self.log_tx.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    file_logger.write_entry("SINGBOX", &line);
                    let _ = log_tx.try_send(LogEntry {
                        timestamp: unix_timestamp(),
                        level: "stdout".to_string(),
                        message: format!("[SINGBOX] {line}"),
                    });
                }
            });
        }
        if let Some(stderr) = stderr {
            let file_logger = self.file_logger.clone();
            let log_tx = self.log_tx.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    if is_bind_error_line(&line) {
                        bind_error.store(true, Ordering::SeqCst);
                    }
                    {
                        let mut tail = stderr_tail.lock();
                        tail.push_back(line.clone());
                        while tail.len() > STDERR_TAIL_LINES {
                            tail.pop_front();
                        }
                    }
                    file_logger.write_entry("SINGBOX", &line);
                    let _ = log_tx.try_send(LogEntry {
                        timestamp: unix_timestamp(),
                        level: "stderr".to_string(),
                        message: format!("[SINGBOX] {line}"),
                    });
                }
            });
        }
    }

    fn set_last_error(&self, error: Option<String>) {
        *self.last_error.write() = error;
    }

    /// Manager's own log line: file level SINGBOX, in-memory level
    /// as given, message prefixed `[SINGBOX]`.
    fn log(&self, level: &str, message: &str) {
        self.file_logger.write_entry("SINGBOX", message);
        let _ = self.log_tx.try_send(LogEntry {
            timestamp: unix_timestamp(),
            level: level.to_string(),
            message: format!("[SINGBOX] {message}"),
        });
    }
}

// Registered in `AppState::sidecars` (stage 4, `main.rs`), so
// `install_update` refuses while the tunnel is active - in the GUI
// process only (see the module docs for the svc caveat).
impl ManagedSidecar for SingBoxManager {
    fn name(&self) -> &'static str {
        "sing-box (WARP tunnel)"
    }

    fn is_active(&self) -> bool {
        !matches!(self.state(), TunnelState::Stopped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::singbox_config::check_config;

    fn test_manager() -> SingBoxManager {
        let job = JobObject::create().expect("test job object");
        let (log_tx, _rx) = tokio::sync::mpsc::channel(100);
        let file_logger = Arc::new(FileLogger::new());
        SingBoxManager::new(Arc::new(job), log_tx, file_logger)
    }

    fn test_options(binary: PathBuf, home_dir: PathBuf) -> SingBoxStartOptions {
        SingBoxStartOptions {
            binary,
            work_dir: home_dir.clone(),
            home_dir,
            tun_scope: TunScope::ProxyOnly,
            desync_on: false,
            lists_dir: None,
            dns_provider: "system".to_string(),
            // The live tests run in the sandbox, which cannot create
            // a TUN device: the generated config must not include it.
            tun_enabled: false,
            fixed_clash_port: None,
        }
    }

    /// Real sing-box binary for the live tests, or None (skip) when
    /// SINGBOX_TEST_BINARY is unset or missing - CI without the
    /// binary stays green.
    fn live_test_binary() -> Option<PathBuf> {
        std::env::var("SINGBOX_TEST_BINARY")
            .ok()
            .map(PathBuf::from)
            .filter(|path| path.is_file())
    }

    /// A fresh loopback port (bind-then-release, the same allocation
    /// the generator uses).
    fn free_loopback_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        port
    }

    /// Minimal live-test config (no TUN, no MASQUE outbound): just
    /// the log, a direct outbound, and the Clash API on the given
    /// port. Returns the config path.
    fn write_minimal_live_config(dir: &Path, clash_port: u16) -> PathBuf {
        let config_path = dir.join("config.json");
        let config = serde_json::json!({
            "log": { "level": "error", "output": dir.join("sing-box.log") },
            "outbounds": [{ "type": "direct", "tag": "direct" }],
            "experimental": {
                "clash_api": {
                    "external_controller": format!("127.0.0.1:{clash_port}"),
                    "secret": "test-secret"
                }
            }
        });
        std::fs::write(&config_path, serde_json::to_string_pretty(&config).unwrap()).unwrap();
        config_path
    }

    /// Kill a process by pid from the outside (the M1 regression
    /// test needs an external death, not a manager stop): SIGTERM
    /// on unix, taskkill on Windows.
    fn kill_pid_external(pid: u32) {
        #[cfg(unix)]
        {
            // SAFETY: kill() with a pid we own from spawn_and_warm.
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGTERM);
            }
        }
        #[cfg(windows)]
        {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/F"])
                .status();
        }
    }

    #[test]
    fn ipset_parsing_skips_comments_and_blanks() {
        let (cidrs, skipped) =
            parse_ipset_content("# comment\n\n  \n1.2.3.0/24\n# another comment\n");
        assert_eq!(cidrs, vec!["1.2.3.0/24".to_string()]);
        assert_eq!(skipped, 0);
    }

    #[test]
    fn ipset_parsing_strips_inline_comments_and_converts_bare_ips() {
        let (cidrs, skipped) =
            parse_ipset_content("203.0.113.7 # a single host\n2001:db8::1\n192.0.2.0/24\n");
        assert_eq!(
            cidrs,
            vec![
                "203.0.113.7/32".to_string(),
                "2001:db8::1/128".to_string(),
                "192.0.2.0/24".to_string(),
            ]
        );
        assert_eq!(skipped, 0);
    }

    #[test]
    fn ipset_parsing_dedupes_and_counts_bad_lines() {
        let (cidrs, skipped) =
            parse_ipset_content("10.0.0.0/8\n10.0.0.0/8\nnot-an-ip\n10.0.0.0/33\n1.2.3.4/24\n");
        assert_eq!(
            cidrs,
            vec!["10.0.0.0/8".to_string(), "1.2.3.4/24".to_string()]
        );
        assert_eq!(skipped, 2);
    }

    #[test]
    fn read_ipset_cidrs_missing_file_is_empty() {
        let dir =
            std::env::temp_dir().join(format!("zapret2-singbox-noipset-{}", std::process::id()));
        let (cidrs, skipped) = read_ipset_cidrs(Some(&dir));
        assert!(cidrs.is_empty());
        assert_eq!(skipped, 0);
        let (cidrs, skipped) = read_ipset_cidrs(None);
        assert!(cidrs.is_empty());
        assert_eq!(skipped, 0);
    }

    #[test]
    fn read_ipset_cidrs_reads_file() {
        let dir =
            std::env::temp_dir().join(format!("zapret2-singbox-ipset-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ipset-all.txt"), "10.0.0.0/8\n# c\n1.2.3.4\n").unwrap();
        let (cidrs, skipped) = read_ipset_cidrs(Some(&dir));
        assert_eq!(
            cidrs,
            vec!["10.0.0.0/8".to_string(), "1.2.3.4/32".to_string()]
        );
        assert_eq!(skipped, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn disabled_components_bit_0x10_only() {
        assert!(!disabled_components_disables_ipv6(0));
        assert!(!disabled_components_disables_ipv6(0x01));
        assert!(disabled_components_disables_ipv6(0x10));
        assert!(disabled_components_disables_ipv6(0xff));
        assert!(disabled_components_disables_ipv6(0xffff_ffff));
    }

    #[test]
    fn if_inet6_entries_detection() {
        assert!(!if_inet6_has_entries(""));
        assert!(!if_inet6_has_entries("\n  \n"));
        assert!(if_inet6_has_entries(
            "00000000000000000000000000000001 01 80 10 80 lo\n"
        ));
    }

    #[test]
    fn version_output_parsing() {
        assert_eq!(
            parse_version_output("sing-box version 1.14.3-extended-2.7.3\n").as_deref(),
            Some("sing-box version 1.14.3-extended-2.7.3")
        );
        assert_eq!(
            parse_version_output("\n  sing-box version 1.0.0\n").as_deref(),
            Some("sing-box version 1.0.0")
        );
        assert_eq!(parse_version_output(""), None);
        assert_eq!(parse_version_output("\n\n"), None);
    }

    #[test]
    fn bind_error_line_detection() {
        assert!(is_bind_error_line(
            "FATAL[0000] listen tcp 127.0.0.1:54321: bind: address already in use"
        ));
        assert!(is_bind_error_line("Address Already In Use"));
        assert!(!is_bind_error_line("INFO[0000] sing-box started"));
        assert!(!is_bind_error_line(""));
    }

    #[test]
    fn tunnel_not_initialized_detection() {
        assert!(is_tunnel_not_initialized("tunnel not initialized"));
        assert!(is_tunnel_not_initialized("Tunnel Not Initialized"));
        assert!(!is_tunnel_not_initialized("proxy not found"));
        assert!(!is_tunnel_not_initialized(""));
    }

    #[test]
    fn fill_config_input_wires_everything() {
        let options = test_options(
            PathBuf::from("sing-box.exe"),
            PathBuf::from("/tmp/singbox-home"),
        );
        let options = SingBoxStartOptions {
            tun_scope: TunScope::Everything,
            desync_on: true,
            dns_provider: "quad9".to_string(),
            ..options
        };
        let system = SystemState {
            resolvers: vec!["192.0.2.53".parse().unwrap()],
            ipv6: false,
        };
        let input = fill_config_input(&options, &system, &["10.0.0.0/8".to_string()]);
        assert_eq!(input.tun_scope, TunScope::Everything);
        assert_eq!(input.system_resolvers, system.resolvers);
        assert_eq!(input.desync_exclude_cidrs, vec!["10.0.0.0/8".to_string()]);
        assert_eq!(input.doh.server, "9.9.9.9");
        assert!(!input.ipv6);
        assert_eq!(
            input.cache_file_path,
            PathBuf::from("/tmp/singbox-home/engine-cache.db")
        );
        assert_eq!(
            input.log_file_path,
            PathBuf::from("/tmp/singbox-home/sing-box.log")
        );
        // The stage 4 seams pass through untouched (test_options
        // sets tun_enabled=false for the sandbox live tests).
        assert!(!input.tun_enabled);
        assert_eq!(input.fixed_clash_port, None);
    }

    #[test]
    fn fill_config_input_drops_exclusion_when_desync_off() {
        let options = test_options(
            PathBuf::from("sing-box.exe"),
            PathBuf::from("/tmp/singbox-home"),
        );
        let system = SystemState::default();
        let input = fill_config_input(&options, &system, &["10.0.0.0/8".to_string()]);
        assert!(input.desync_exclude_cidrs.is_empty());
        // "system" has no DoH hostname: Cloudflare fallback.
        assert_eq!(input.doh.server, "1.1.1.1");
    }

    #[test]
    fn home_dir_ends_with_singbox() {
        assert_eq!(
            singbox_home_dir().file_name().and_then(|n| n.to_str()),
            Some("singbox")
        );
    }

    #[test]
    fn tunnel_state_serde_is_lowercase() {
        assert_eq!(
            serde_json::to_string(&TunnelState::Unregistered).unwrap(),
            "\"unregistered\""
        );
        assert_eq!(
            serde_json::from_str::<TunnelState>("\"degraded\"").unwrap(),
            TunnelState::Degraded
        );
    }

    #[tokio::test]
    async fn stop_when_stopped_is_noop() {
        let manager = test_manager();
        manager.stop().await.unwrap();
        assert_eq!(manager.state(), TunnelState::Stopped);
        assert!(!manager.is_active());
        assert_eq!(manager.name(), "sing-box (WARP tunnel)");
    }

    #[tokio::test]
    async fn start_with_missing_binary_fails_cleanly() {
        let home =
            std::env::temp_dir().join(format!("zapret2-singbox-missing-{}", std::process::id()));
        let options = test_options(home.join("no-such-sing-box"), home);
        let manager = test_manager();
        let error = manager.start(options).await.unwrap_err();
        assert!(matches!(error, SingBoxError::BinaryMissing(_)));
        assert_eq!(manager.state(), TunnelState::Error);
        assert!(manager.status().last_error.is_some());
        // A failed start can be cleared by stop().
        manager.stop().await.unwrap();
        assert_eq!(manager.state(), TunnelState::Stopped);
    }

    /// Live test: spawn the real sing-box with a minimal config (no
    /// TUN - tests must not raise a tunnel adapter), check Clash API
    /// readiness and warmup, then stop. Gated on SINGBOX_TEST_BINARY
    /// like the stage 2 live tests, so CI without the binary stays
    /// green.
    ///
    /// The minimal config has no MASQUE outbound, so warmup cannot
    /// measure an exit and the tunnel must land in Degraded - that
    /// is the precise assertion for "engine up, exit missing".
    #[tokio::test]
    async fn live_spawn_readiness_warmup_and_stop() {
        let Some(binary) = live_test_binary() else {
            return;
        };

        // The version check (stage 1 item) against the real binary.
        let version = singbox_version(&binary).expect("sing-box version");
        assert!(
            version.starts_with("sing-box version"),
            "unexpected version line: {version}"
        );

        let dir =
            std::env::temp_dir().join(format!("zapret2-singbox-live-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let clash_port = free_loopback_port();
        let config_path = write_minimal_live_config(&dir, clash_port);

        let check = check_config(&binary, &config_path).expect("sing-box check");
        assert!(
            check.success,
            "minimal config rejected by sing-box check: {}",
            check.stderr
        );

        let manager = test_manager();
        manager
            .spawn_and_warm(&binary, &dir, &config_path, clash_port, "test-secret")
            .await
            .expect("spawn and warmup");
        let status = manager.status();
        assert_eq!(status.state, TunnelState::Degraded);
        assert!(status.pid.is_some());
        assert_eq!(status.clash_api_port, Some(clash_port));
        assert!(manager.is_active());

        manager.stop().await.unwrap();
        assert_eq!(manager.state(), TunnelState::Stopped);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Live regression test for the stage 3 review finding M1: a
    /// redundant `start()` must not orphan the monitor of the
    /// running engine. Spawn the real engine, call `start()` again
    /// while it is up, then kill the engine externally and expect
    /// the monitor to report the death (state Error). Before the
    /// fix the state stayed Degraded forever, because `start()`
    /// bumped the generation before the idempotency check and the
    /// monitor's generation guard then dropped the death report.
    ///
    /// Also pins m3: after an unexpected death the status must not
    /// carry a stale Clash API port.
    #[tokio::test]
    async fn live_redundant_start_keeps_monitor_armed() {
        let Some(binary) = live_test_binary() else {
            return;
        };

        let dir =
            std::env::temp_dir().join(format!("zapret2-singbox-live-m1-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let clash_port = free_loopback_port();
        let config_path = write_minimal_live_config(&dir, clash_port);

        let manager = test_manager();
        manager
            .spawn_and_warm(&binary, &dir, &config_path, clash_port, "test-secret")
            .await
            .expect("spawn and warmup");
        assert_eq!(manager.state(), TunnelState::Degraded);
        let pid = manager.status().pid.expect("engine pid");

        // A redundant start: idempotent, same status, same pid - and
        // (the M1 fix) it must not bump the generation the monitor
        // was armed with.
        let status = manager
            .start(test_options(binary.clone(), dir.clone()))
            .await
            .expect("redundant start is ok");
        assert_eq!(status.state, TunnelState::Degraded);
        assert_eq!(status.pid, Some(pid));

        // External death: the monitor must report it as Error.
        kill_pid_external(pid);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if manager.state() == TunnelState::Error {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "monitor did not report the engine death, state stuck at {:?}",
                manager.state()
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let status = manager.status();
        assert!(status.pid.is_none());
        assert_eq!(status.clash_api_port, None);
        assert!(status.last_error.is_some());

        manager.stop().await.unwrap();
        assert_eq!(manager.state(), TunnelState::Stopped);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Live test for the TOCTOU bind-error detection (stage 3 review
    /// m1): with the Clash port already occupied, `spawn_and_warm`
    /// must fail with `bind_error=true`, so the start loop retries
    /// with a fresh config instead of failing the start.
    #[tokio::test]
    async fn live_spawn_with_occupied_clash_port_reports_bind_error() {
        let Some(binary) = live_test_binary() else {
            return;
        };

        let dir =
            std::env::temp_dir().join(format!("zapret2-singbox-live-bind-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        // Occupy a loopback port and keep the listener alive while
        // the engine tries to bind its Clash API to the same port.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let clash_port = listener.local_addr().unwrap().port();
        let config_path = write_minimal_live_config(&dir, clash_port);

        // check does not bind, so the config itself is fine.
        let check = check_config(&binary, &config_path).expect("sing-box check");
        assert!(check.success, "minimal config rejected: {}", check.stderr);

        let manager = test_manager();
        let attempt = manager
            .spawn_and_warm(&binary, &dir, &config_path, clash_port, "test-secret")
            .await
            .expect_err("engine must die on the occupied Clash port");
        assert!(
            attempt.bind_error,
            "expected bind_error=true, got error: {:?}",
            attempt.error
        );
        assert!(matches!(attempt.error, SingBoxError::ExitedDuringStart(_)));
        // spawn_and_warm does not touch the state on failure: the
        // caller (start loop) decides whether to retry or fail. In
        // production start() sets Starting beforehand; here the
        // manager was never started, so the state is still Stopped.
        assert_eq!(manager.state(), TunnelState::Stopped);

        manager.stop().await.unwrap();
        assert_eq!(manager.state(), TunnelState::Stopped);

        drop(listener);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Live test for the stage 3 TOCTOU retry loop (stage 4, task 7):
    /// the full `start()` path against the real binary, with the
    /// Clash port pinned to one the test already occupies. The first
    /// attempt must die on the bind conflict, the retry must
    /// regenerate the config with fresh ports and come up. The proof
    /// is the final Clash port: it must differ from the occupied one
    /// (the retry really reallocated), and the engine must be up.
    /// The config has no TUN (the sandbox cannot create one), and
    /// the MASQUE exit cannot connect from the sandbox, so the final
    /// state is Degraded or Unregistered, never Running.
    #[tokio::test]
    async fn live_start_retries_once_on_bind_conflict() {
        let Some(binary) = live_test_binary() else {
            return;
        };

        let dir =
            std::env::temp_dir().join(format!("zapret2-singbox-live-retry-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        // Occupy a loopback port and hold the listener for the whole
        // test: the first attempt must hit "address already in use".
        let squatter = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let occupied_port = squatter.local_addr().unwrap().port();

        let options = SingBoxStartOptions {
            fixed_clash_port: Some(occupied_port),
            ..test_options(binary, dir.clone())
        };

        let manager = test_manager();
        let status = manager
            .start(options)
            .await
            .expect("start retries past the bind conflict");
        assert_ne!(
            status.clash_api_port,
            Some(occupied_port),
            "the retry must reallocate - the final port cannot be the occupied one"
        );
        assert!(status.pid.is_some());
        assert!(
            matches!(
                status.state,
                TunnelState::Degraded | TunnelState::Unregistered
            ),
            "unexpected final state: {:?}",
            status.state
        );
        assert_eq!(manager.tun_scope(), Some(TunScope::ProxyOnly));

        manager.stop().await.unwrap();
        assert_eq!(manager.state(), TunnelState::Stopped);
        assert_eq!(manager.tun_scope(), None);

        drop(squatter);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The start loop retries once on a port-bind conflict (stage 3
    /// review m1): a fake engine that always dies with "address
    /// already in use" must be launched exactly MAX_START_ATTEMPTS
    /// times before the start fails. Unix-only: the fake engine is
    /// a shell script. (The loop itself is platform-independent;
    /// the live bind-detection test above covers the real engine.)
    #[cfg(unix)]
    #[tokio::test]
    async fn start_retries_once_on_bind_conflict() {
        let dir =
            std::env::temp_dir().join(format!("zapret2-singbox-retry-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let counter = dir.join("run-counter.txt");
        let fake = dir.join("fake-sing-box");
        // The script answers `version` and `check` like a healthy
        // engine, but every `run` dies with the Go bind error on
        // stderr and records the launch in the counter file.
        let script = format!(
            "#!/bin/sh\n\
             case \"$1\" in\n\
             \x20 version)\n\
             \x20\x20 echo \"sing-box version 9.9.9-fake\"\n\
             \x20\x20 ;;\n\
             \x20 check)\n\
             \x20\x20 exit 0\n\
             \x20\x20 ;;\n\
             \x20 run)\n\
             \x20\x20 echo run >> '{}'\n\
             \x20\x20 echo 'FATAL[0000] listen tcp 127.0.0.1:1: bind: address already in use' >&2\n\
             \x20\x20 exit 1\n\
             \x20\x20 ;;\n\
             esac\n",
            counter.display()
        );
        std::fs::write(&fake, script).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let manager = test_manager();
        let error = manager
            .start(test_options(fake, dir.clone()))
            .await
            .expect_err("start must fail after the bind retries are spent");
        assert!(matches!(error, SingBoxError::ExitedDuringStart(_)));
        assert_eq!(manager.state(), TunnelState::Error);

        // Exactly two launches: the first try plus one regeneration
        // on the bind conflict. The literal 2 (not the constant)
        // is deliberate: the test pins the retry behaviour, so a
        // MAX_START_ATTEMPTS change that drops the retry fails here
        // instead of silently following the constant.
        let runs = std::fs::read_to_string(&counter)
            .unwrap_or_default()
            .lines()
            .count();
        assert_eq!(
            runs, 2,
            "expected one retry after the bind conflict (2 launches), got {runs}"
        );

        manager.stop().await.unwrap();
        assert_eq!(manager.state(), TunnelState::Stopped);

        let _ = std::fs::remove_dir_all(&dir);
    }
}

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
//!   Before any of that, the upstream gate (stage 7) checks for a
//!   default route out of the machine: with no network the start
//!   fails as `UpstreamDown` instead of spawning an engine that
//!   cannot register, and the auto-restart loop keeps re-checking
//!   the gate (without spending the restart budget) so the tunnel
//!   recovers on its own when the network returns.
//! - `stop()` kills the child and waits up to 5 s. It deliberately
//!   does NOT terminate the shared Job Object: winws2 lives in the
//!   same job, and terminating it would kill the desync engine too.
//!   sing-box spawns no grandchildren, so kill+wait is a complete
//!   stop; KILL_ON_JOB_CLOSE still covers the case where svc itself
//!   dies. `stop()` bumps the generation *before* taking the
//!   lifecycle lock (stage 7, O2): a start in progress polls the
//!   generation in its wait loops and aborts within ~250 ms, so a
//!   stop no longer waits out the start's ~70 s worst case.
//! - The monitor task (500 ms, like the supervisor's) watches the
//!   child; an unexpected exit moves the tunnel to `Error` and
//!   schedules an auto-restart (stage 6: at most 5, doubling backoff
//!   2 s -> 2 min, generation-gated). A dead *exit* is not a dead
//!   engine: warmup and the watchdog (stage 6) report that as
//!   `Degraded` / `Unregistered` and never restart the engine for
//!   it - the watchdog instead walks the DNS ladder (Rule -> dns-tcp
//!   -> dns-spare) through `PATCH /configs` and re-measures after
//!   each step. A silent Clash API with a live child is Broken and
//!   restarts like an unexpected exit.
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
//! it on engine stop; stage 5 exposes it over IPC (TunnelStart /
//! TunnelStop / TunnelGetStatus, `TunnelState` events, the `tunnel`
//! field of `SupervisorStatus`). The manager is registered in
//! `AppState::sidecars` (stage 4, `main.rs`), so `install_update`
//! refuses while the tunnel is active; when the GUI is attached to
//! the svc, the updater consults the svc's tunnel over IPC instead
//! (stage 5: the local manager is Stopped there, the tunnel lives
//! in the svc).

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
use crate::engine::events::SinkSlot;
use crate::engine::logger::FileLogger;
use crate::engine::managed::ManagedSidecar;
use crate::engine::singbox_config::{
    build_config, check_config, write_config_file, DohEndpoint, SingBoxConfigError,
    SingBoxConfigInput, TunScope, DNS_SPARE_MODE, DNS_TCP_MODE, MASQUE_TAG,
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
    FreeMibTable, GetAdaptersAddresses, GetIpForwardTable2, GAA_FLAG_SKIP_ANYCAST,
    GAA_FLAG_SKIP_MULTICAST, GAA_FLAG_SKIP_UNICAST, IF_TYPE_SOFTWARE_LOOPBACK,
    IP_ADAPTER_ADDRESSES_LH, MIB_IPFORWARD_TABLE2,
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
/// Maximum consecutive automatic restarts of a broken engine
/// (NETZapret's MaxRestarts). After the budget is spent the tunnel
/// stays in Error until the user acts (the retry button or a fresh
/// start). A manual start resets the budget.
pub const MAX_AUTO_RESTARTS: u32 = 5;
/// First auto-restart backoff; doubles per attempt, capped at
/// RESTART_BACKOFF_MAX (NETZapret: 2 s doubling to 2 min).
const RESTART_BACKOFF_BASE_SECS: u64 = 2;
const RESTART_BACKOFF_MAX: Duration = Duration::from_secs(120);
/// An engine that ran this long earns a fresh restart budget: the
/// counter tracks *consecutive* flapping, not lifetime restarts.
const RESTART_STABLE_WINDOW: Duration = Duration::from_secs(600);
/// Watchdog tick: Clash API liveness, like NETZapret's 5 s poll.
const WATCHDOG_TICK: Duration = Duration::from_secs(5);
/// Deep-check cadence in watchdog ticks: every 6th tick (~30 s),
/// like NETZapret's deep check every 6 polls.
const WATCHDOG_DEEP_EVERY_TICKS: u32 = 6;
/// Consecutive silent liveness polls before the engine counts as
/// Broken (NETZapret's 2-miss threshold): one 2 s stall is a
/// hiccup, two in a row is a dead HTTP stack.
const ALIVE_MISSES_BEFORE_BROKEN: u32 = 2;

/// Backoff before auto-restart attempt `attempt` (1-based): 2 s,
/// doubling, capped at 2 min. Pure, so the table is unit-testable.
fn backoff_delay(attempt: u32) -> Duration {
    let shift = attempt.saturating_sub(1).min(6);
    Duration::from_secs(RESTART_BACKOFF_BASE_SECS << shift).min(RESTART_BACKOFF_MAX)
}

/// Delay before auto-restart attempt `attempt` (1-based), or None
/// when the budget is spent. Pure, so the exhaustion rule is
/// unit-testable.
fn restart_delay(attempt: u32) -> Option<Duration> {
    if attempt == 0 || attempt > MAX_AUTO_RESTARTS {
        None
    } else {
        Some(backoff_delay(attempt))
    }
}

/// DNS ladder step (design section 9, NETZapret's DnsTcpStep /
/// DnsSpareStep). The engine stores the Clash mode in its cache
/// file, so a step survives restarts - the manager resets it to
/// Rule once per launch (NETZapret's ResetDnsLeversAsync) and
/// tracks the current step here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DnsStep {
    /// Default: DoH `remote`, the `final` server.
    Rule,
    /// The same resolver over plain TCP.
    Tcp,
    /// A different provider's DoH.
    Spare,
}

impl DnsStep {
    /// The Clash mode string the engine stores for this step.
    /// "Rule" is capitalised: that is how the engine reports the
    /// default mode (its clashmode manager defaults to "Rule").
    pub fn mode(&self) -> &'static str {
        match self {
            DnsStep::Rule => "Rule",
            DnsStep::Tcp => DNS_TCP_MODE,
            DnsStep::Spare => DNS_SPARE_MODE,
        }
    }

    /// One step up the ladder (Rule -> Tcp -> Spare), or None at
    /// the top.
    pub fn next_up(&self) -> Option<DnsStep> {
        match self {
            DnsStep::Rule => Some(DnsStep::Tcp),
            DnsStep::Tcp => Some(DnsStep::Spare),
            DnsStep::Spare => None,
        }
    }

    /// One step down the ladder (Spare -> Tcp -> Rule), or None at
    /// the bottom.
    pub fn next_down(&self) -> Option<DnsStep> {
        match self {
            DnsStep::Rule => None,
            DnsStep::Tcp => Some(DnsStep::Rule),
            DnsStep::Spare => Some(DnsStep::Tcp),
        }
    }

    /// Parse the mode the engine reports (GET /configs). Unknown
    /// modes map to Rule: the ladder rules only match the two
    /// step modes, so anything else behaves like the default.
    pub fn from_mode(mode: &str) -> DnsStep {
        if mode.eq_ignore_ascii_case(DNS_TCP_MODE) {
            DnsStep::Tcp
        } else if mode.eq_ignore_ascii_case(DNS_SPARE_MODE) {
            DnsStep::Spare
        } else {
            DnsStep::Rule
        }
    }
}

/// Tunnel states (design section 7, with the eighth state from
/// section 8 - the design lists seven in section 7 but names
/// UpstreamDown as a state in section 8; stage 7 closes that
/// inconsistency by making it a real state). Separate from the
/// desync engine states: the tunnel has its own lifecycle, and a
/// dead exit is not a dead engine (`Degraded` / `Unregistered` vs
/// `Error`).
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
    /// No network: no up, non-loopback adapter with a default route
    /// (design section 8, stage 7). A *start-gate* state only - the
    /// engine is never running in it: the start refused to spawn.
    /// A network loss mid-run does NOT move here (the engine stays
    /// up and the watchdog reports it as Degraded, recovering on
    /// its own when the network returns); this state means "the
    /// tunnel was (re)started while offline". The retry button and
    /// the auto-restart loop both re-check the gate.
    UpstreamDown,
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
    /// Consecutive auto-restarts used in the current session
    /// (stage 6): 0 when the engine never needed one. `serde
    /// default` so older snapshots (stage 5, without the fields)
    /// still parse.
    #[serde(default)]
    pub restarts_used: u32,
    /// The auto-restart budget (`MAX_AUTO_RESTARTS`): the UI shows
    /// "used / max" without hardcoding the policy.
    #[serde(default)]
    pub restarts_max: u32,
}

impl Default for SingBoxStatus {
    /// The stopped snapshot: what a status field defaults to when
    /// no tunnel ever ran (and what `#[serde(default)]` on
    /// `SupervisorStatus.tunnel` produces for older snapshots).
    fn default() -> Self {
        Self {
            state: TunnelState::Stopped,
            pid: None,
            engine_version: None,
            clash_api_port: None,
            warmup_delay_ms: None,
            last_error: None,
            restarts_used: 0,
            restarts_max: MAX_AUTO_RESTARTS,
        }
    }
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
    /// Override the upstream gate for tests (`None` in production):
    /// `Some(false)` forces the UpstreamDown path without pulling
    /// the network cable, `Some(true)` forces the gate open. Same
    /// seam pattern as `tun_enabled` / `fixed_clash_port` above.
    pub upstream_override: Option<bool>,
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

    /// The upstream gate (stage 7): no up, non-loopback adapter
    /// with a default route, so the tunnel refuses to spawn. The
    /// text goes to the UI as `last_error` (Russian: the user-facing
    /// tunnel errors in this codebase are Russian - see
    /// `restart_loop`'s budget message).
    #[error("нет сети: не найден поднятый сетевой адаптер с маршрутом в интернет")]
    UpstreamDown,

    /// The start was cancelled by a stop (stage 7, O2): `stop()`
    /// bumped the generation while the start waited for readiness
    /// or warmup. Not a failure - the stop owns the state from
    /// here, so the caller must not report it as Error.
    #[error("start cancelled by stop")]
    Cancelled,
}

impl From<SingBoxError> for ProcessError {
    fn from(e: SingBoxError) -> Self {
        // Cancelled stays typed: the supervisor wrapper must not
        // record a cancelled start as a failure (stage 7 review
        // m1), and string-matching the error text would be fragile.
        if matches!(e, SingBoxError::Cancelled) {
            ProcessError::TunnelCancelled
        } else {
            ProcessError::Tunnel(e.to_string())
        }
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
///
/// Non-Windows-only in production (the `/proc` reader is
/// `#[cfg(not(windows))]`); kept in test builds so the truth table
/// stays covered on Windows - the mirror of
/// `disabled_components_disables_ipv6` (stage 5 review m2).
#[cfg(any(not(windows), test))]
fn if_inet6_has_entries(content: &str) -> bool {
    content.lines().any(|line| !line.trim().is_empty())
}

/// Walk the adapter list once, calling `f` for each adapter.
/// Returns false when the enumeration itself failed - the caller
/// decides what that means (resolvers degrade to an empty list,
/// the upstream gate fails open). The buffer dance is the same as
/// before: the first call returns ERROR_BUFFER_OVERFLOW with the
/// required size.
#[cfg(windows)]
fn for_each_adapter(mut f: impl FnMut(&IP_ADAPTER_ADDRESSES_LH)) -> bool {
    // WinError.h: the first call returns ERROR_BUFFER_OVERFLOW and
    // the required buffer size.
    const ERROR_BUFFER_OVERFLOW: u32 = 111;
    // We need DNS servers, OperStatus, IfType, IfIndex, and the
    // adapter names only - no address lists.
    let flags = GAA_FLAG_SKIP_UNICAST | GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST;
    let family = u32::from(AF_UNSPEC.0);

    let mut size: u32 = 0;
    let first = unsafe { GetAdaptersAddresses(family, flags, None, None, &mut size) };
    if first != ERROR_BUFFER_OVERFLOW || size == 0 {
        return false;
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
        return false;
    }

    let mut current = buffer.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
    while !current.is_null() {
        let adapter = unsafe { &*current };
        f(adapter);
        current = adapter.Next;
    }
    true
}

/// Discover the system's DNS resolvers via `GetAdaptersAddresses`
/// (NETZapret's `SystemResolvers.Discover`): up adapters only, the
/// loopback adapter skipped, our own tunnel adapter skipped (its
/// resolvers would be our own fake ones), loopback resolver addresses
/// filtered out, deduplicated.
#[cfg(windows)]
fn discover_system_resolvers() -> Vec<IpAddr> {
    let mut resolvers = Vec::new();
    let mut seen = HashSet::new();
    for_each_adapter(|adapter| {
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
    });
    resolvers
}

/// Is there an upstream path out of this machine? (stage 7 gate,
/// design section 8: "no up, non-loopback adapter with a route -
/// UpstreamDown, the tunnel does not start").
///
/// Windows: an IPv4 default route (prefix length 0 in the forward
/// table) on an adapter that is up, non-loopback, and not our own
/// tunnel. IPv4 only, deliberately: the MASQUE endpoint is the
/// hardcoded IPv4 address 162.159.198.2, so an IPv6-only machine
/// cannot reach the tunnel anyway. Any enumeration failure fails
/// OPEN (assume upstream): a broken query must not block a tunnel
/// that could work - the warmup still reports a dead exit honestly
/// as Degraded.
#[cfg(windows)]
pub fn upstream_available() -> bool {
    let Some(route_ifaces) = default_route_interface_indices() else {
        return true;
    };
    let mut found = false;
    let enumerated = for_each_adapter(|adapter| {
        if !found
            && adapter_is_upstream(
                // IfIndex lives in the anonymous union of
                // IP_ADAPTER_ADDRESSES_LH (windows 0.58) - the
                // direct field does not exist.
                unsafe { adapter.Anonymous1.Anonymous.IfIndex },
                adapter.OperStatus == IfOperStatusUp,
                adapter.IfType == IF_TYPE_SOFTWARE_LOOPBACK,
                is_own_tunnel_adapter(adapter),
                &route_ifaces,
            )
        {
            found = true;
        }
    });
    if !enumerated {
        return true;
    }
    found
}

/// Non-Windows upstream gate (tests only - WARP is a
/// Windows-target feature): the IPv4 default route in
/// `/proc/net/route`, the same rule as on Windows. A
/// missing/unreadable file fails open, like the Windows
/// enumeration failure.
#[cfg(not(windows))]
pub fn upstream_available() -> bool {
    match std::fs::read_to_string("/proc/net/route") {
        Ok(content) => route_table_has_default_route(&content),
        Err(_) => true,
    }
}

/// Pure core of the Windows gate: an adapter is an upstream path
/// when it is up, not loopback, not our own tunnel, and its
/// interface index carries a default route. Windows-only in
/// production; kept in test builds so the truth table stays
/// covered on Linux (the mirror of
/// `disabled_components_disables_ipv6`).
#[cfg(any(windows, test))]
fn adapter_is_upstream(
    if_index: u32,
    is_up: bool,
    is_loopback: bool,
    is_own_tunnel: bool,
    default_route_ifaces: &HashSet<u32>,
) -> bool {
    is_up && !is_loopback && !is_own_tunnel && default_route_ifaces.contains(&if_index)
}

/// Interface indices carrying an IPv4 default route (prefix length
/// 0 in the forward table). `None` when the table cannot be read -
/// the caller fails open. The rows are borrowed from the table, so
/// the set is built before `FreeMibTable` releases it.
#[cfg(windows)]
fn default_route_interface_indices() -> Option<HashSet<u32>> {
    let mut table: *mut MIB_IPFORWARD_TABLE2 = std::ptr::null_mut();
    let result = unsafe { GetIpForwardTable2(AF_INET, &mut table) };
    if result != ERROR_SUCCESS || table.is_null() {
        return None;
    }
    let mut indices = HashSet::new();
    unsafe {
        let count = (*table).NumEntries as usize;
        let rows = (*table).Table.as_ptr();
        for i in 0..count {
            let row = &*rows.add(i);
            if row.DestinationPrefix.PrefixLength == 0 {
                indices.insert(row.InterfaceIndex);
            }
        }
        FreeMibTable(table as *const std::ffi::c_void);
    }
    Some(indices)
}

/// Does this `/proc/net/route` content list an IPv4 default route
/// (destination `00000000`) on a non-loopback interface that is up?
/// Columns: Iface Destination Gateway Flags RefCnt Use Metric Mask
/// MTU Window IRTT; Flags is hex, RTF_UP is bit 0. A gateway flag is
/// NOT required: a directly-connected default (some PPP links) is
/// still an upstream path. Non-Windows-only in production (the
/// `/proc` reader is `#[cfg(not(windows))]`); kept in test builds
/// so the fixtures stay covered on Windows.
#[cfg(any(not(windows), test))]
fn route_table_has_default_route(content: &str) -> bool {
    content.lines().skip(1).any(|line| {
        let mut fields = line.split_whitespace();
        let iface = fields.next().unwrap_or("");
        let destination = fields.next().unwrap_or("");
        let _gateway = fields.next();
        let flags = fields.next().unwrap_or("");
        if iface == "lo" || destination != "00000000" {
            return false;
        }
        u32::from_str_radix(flags, 16)
            .map(|flags| flags & 0x1 != 0)
            .unwrap_or(false)
    })
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
/// adapters; any other bit combination leaves it on. Windows-only in
/// production (the registry reader is `#[cfg(windows)]`); kept in
/// test builds so the truth table stays covered on Linux.
#[cfg(any(windows, test))]
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

/// How much of the engine log the UI shows for an unregistered
/// tunnel (design section 8: the last 64 KiB, like NETZapret's
/// `Warp.RegistrationFailing` diagnostic). GUI-only: read by the
/// `get_tunnel_engine_log` command in `commands.rs`, never by the
/// svc binary - hence the allow, without it the svc build warns
/// (same pattern as `SINGBOX_SIDECAR_NAME`, stage 5 review m2).
#[allow(dead_code)]
pub const ENGINE_LOG_TAIL_MAX_BYTES: usize = 64 * 1024;

/// Read the tail of the engine log (`sing-box.log` in the home
/// directory), up to `ENGINE_LOG_TAIL_MAX_BYTES`. A missing or
/// unreadable file yields an empty string - the UI shows its "log
/// unavailable" line instead of an error. The first (possibly
/// partial) line is dropped so the tail starts on a line boundary,
/// unless the tail holds no newline at all (then the whole tail is
/// shown - a partial line beats no line for a diagnostic).
/// GUI-only (see above): the allow covers the svc build, where
/// nothing calls this.
#[allow(dead_code)]
pub fn read_engine_log_tail(home_dir: &Path) -> String {
    read_engine_log_tail_limited(home_dir, ENGINE_LOG_TAIL_MAX_BYTES)
}

/// Testable core: the same tail read with an explicit byte budget.
/// GUI-only (see above): only the public wrapper calls this
/// outside tests.
#[allow(dead_code)]
fn read_engine_log_tail_limited(home_dir: &Path, max_bytes: usize) -> String {
    let Ok(bytes) = std::fs::read(home_dir.join("sing-box.log")) else {
        return String::new();
    };
    let start = bytes.len().saturating_sub(max_bytes);
    let text = String::from_utf8_lossy(&bytes[start..]);
    if start == 0 {
        return text.into_owned();
    }
    match text.find('\n') {
        Some(pos) => text[pos + 1..].to_string(),
        None => text.into_owned(),
    }
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
    /// A stop bumped the generation while waiting (stage 7, O2):
    /// the start aborts instead of running out the deadline.
    Cancelled,
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
    /// UI event sink (stage 5): a clone of the supervisor's slot,
    /// so tunnel state transitions reach the GUI (Tauri sink) and
    /// the svc pipe clients (PipeSink) exactly like the desync
    /// engine's own events. The sink is swappable at runtime; the
    /// manager reads the current one on every emit.
    sink: SinkSlot,
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
    /// Start options of the last start (stage 6): the restart task
    /// re-runs them. Set in `start_with`; never cleared - a restart
    /// after a stop is impossible, the stop bumps the generation
    /// and the restart task exits on the generation check.
    last_options: Arc<RwLock<Option<SingBoxStartOptions>>>,
    /// Consecutive auto-restarts used in the current session
    /// (stage 6). Reset by a manual start, by `stop()`, and when
    /// an engine proves stable for `RESTART_STABLE_WINDOW`. Read
    /// by `status()` so the UI can show the attempts honestly.
    restart_attempts: Arc<RwLock<u32>>,
    /// When the current engine instance was spawned (`None` while
    /// stopped): the stability window for the restart budget.
    engine_started_at: Arc<RwLock<Option<Instant>>>,
    /// Current DNS ladder step (stage 6). Reset to Rule once per
    /// launch - the engine persists the mode in its cache file.
    dns_step: Arc<RwLock<DnsStep>>,
}

impl SingBoxManager {
    /// Build a manager. Cheap and infallible; nothing is started
    /// here. `log_tx` feeds the shared in-memory log, `file_logger`
    /// the shared log file - both get `[SINGBOX]`-prefixed lines.
    /// `sink` is the supervisor's event slot (stage 5): every state
    /// transition is emitted through it.
    pub fn new(
        job_object: Arc<JobObject>,
        log_tx: Sender<LogEntry>,
        file_logger: Arc<FileLogger>,
        sink: SinkSlot,
    ) -> Self {
        Self {
            job_object,
            log_tx,
            file_logger,
            sink,
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
            last_options: Arc::new(RwLock::new(None)),
            restart_attempts: Arc::new(RwLock::new(0)),
            engine_started_at: Arc::new(RwLock::new(None)),
            dns_step: Arc::new(RwLock::new(DnsStep::Rule)),
        }
    }

    /// Write a new tunnel state and emit it to the UI sink (stage
    /// 5). Every state transition goes through here (except
    /// `on_engine_gone`, which checks-then-sets under one guard and
    /// emits inline) so the GUI and the svc pipe clients always see
    /// the same lifecycle the manager sees.
    fn set_state(&self, state: TunnelState) {
        *self.state.write() = state;
        self.sink.emit_tunnel_state(state);
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
            restarts_used: *self.restart_attempts.read(),
            restarts_max: MAX_AUTO_RESTARTS,
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
    /// warmup, monitor, watchdog. Idempotent: a start while the
    /// engine is already up (or coming up) returns the current
    /// status without restarting anything. A manual start: it
    /// resets the auto-restart budget (stage 6) - this is what the
    /// retry button relies on after the budget was spent.
    pub async fn start(&self, options: SingBoxStartOptions) -> Result<SingBoxStatus, SingBoxError> {
        self.start_with(options, true).await
    }

    /// Auto-restart entry (stage 6): like `start()`, but does not
    /// reset the restart budget - the restart task owns the counter,
    /// and a restart that reset it would never exhaust it.
    async fn start_restart(
        &self,
        options: SingBoxStartOptions,
    ) -> Result<SingBoxStatus, SingBoxError> {
        self.start_with(options, false).await
    }

    async fn start_with(
        &self,
        options: SingBoxStartOptions,
        manual: bool,
    ) -> Result<SingBoxStatus, SingBoxError> {
        let _guard = self.lifecycle.lock().await;

        match *self.state.read() {
            TunnelState::Running
            | TunnelState::Degraded
            | TunnelState::Unregistered
            | TunnelState::Starting => return Ok(self.status()),
            // UpstreamDown is a failed start, not a running tunnel:
            // a retry re-checks the gate (the retry button relies
            // on this, and so does the auto-restart loop).
            TunnelState::Stopped
            | TunnelState::Stopping
            | TunnelState::Error
            | TunnelState::UpstreamDown => {}
        }

        // The bump invalidates any monitor task from a previous
        // start. It must come AFTER the idempotency check above:
        // bumping first would orphan the monitor of the running
        // engine (review M1) - the monitor gates the death report
        // on the generation, and a redundant start() would leave
        // it stale, so a later engine death would be lost silently
        // while the state stayed Running/Degraded forever.
        *self.generation.write() += 1;
        // The generation this start belongs to, captured once and
        // threaded through the whole start (stage 7, O2): `stop()`
        // bumps the generation *before* taking the lifecycle lock,
        // so a start that re-read the counter later (in
        // `spawn_and_warm`, say) could pick up the stop's bump and
        // miss the cancellation entirely.
        let generation = *self.generation.read();
        if manual {
            // A user-initiated start is a fresh session: the
            // auto-restart budget is restored. This sits next to
            // the generation bump (after the idempotency check) so
            // a redundant start does not reset the budget either.
            *self.restart_attempts.write() = 0;
        }
        // The restart task re-runs these options; a start that
        // fails here never arms a monitor, so nothing restarts it
        // (by design: a deterministically failing start would burn
        // the budget in one go).
        *self.last_options.write() = Some(options.clone());

        self.set_state(TunnelState::Starting);
        self.set_last_error(None);
        *self.warmup_delay_ms.write() = None;
        *self.engine_version.write() = None;
        *self.clash_api_port.write() = None;
        *self.pid.write() = None;

        match self.start_inner(&options, generation).await {
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
                match error {
                    // The upstream gate (stage 7): no network, no
                    // spawn - a state of its own, not Error. The
                    // retry button and the auto-restart loop both
                    // re-check the gate from here.
                    SingBoxError::UpstreamDown => {
                        self.set_last_error(Some(error.to_string()));
                        self.set_state(TunnelState::UpstreamDown);
                        self.log("warn", &format!("tunnel start refused: {error}"));
                    }
                    // Cancelled by a stop (stage 7, O2): only stop()
                    // bumps the generation mid-start, so a stop is
                    // in flight and owns the state from here - leave
                    // it (Starting) and the last error alone.
                    // Setting Error would flash a spurious failure
                    // the stop immediately clears.
                    SingBoxError::Cancelled => {
                        self.log("info", "sing-box start cancelled by stop");
                    }
                    _ => {
                        self.set_last_error(Some(error.to_string()));
                        self.set_state(TunnelState::Error);
                        self.log("error", &format!("failed to start sing-box: {error}"));
                    }
                }
                Err(error)
            }
        }
    }

    /// Record a failed tunnel start from outside `start()`: the
    /// error text plus the Error state, emitted to the UI sink.
    /// `start()` does this itself on its own failures; the
    /// supervisor calls it for failures that happen before the
    /// manager is ever started (no binary, desync restart failed,
    /// capture never became ready), so the remote GUI learns about
    /// them through the tunnel state - the svc `TunnelStart`
    /// handler is fire-and-forget and the `Result` never reaches it
    /// (stage 5 review m1). The text is updated even when already
    /// in Error (a retry that fails early must not leave a stale
    /// message); the state transition - and its event and log line -
    /// fires only once, so a failure already recorded by `start()`
    /// does not double-emit.
    ///
    /// Two states are left alone (stage 7): `UpstreamDown` - the
    /// manager already recorded the gate failure with its own text,
    /// overwriting it with Error would lose the "no network"
    /// distinction; and `Starting` - the only path that reaches
    /// here with the state still Starting is a start cancelled by
    /// a stop (every other `start()` failure sets Error itself),
    /// and the stop in flight owns the state from there.
    ///
    /// `Stopped` is deliberately NOT guarded: the early failures
    /// this exists for (no binary, desync restart failed, capture
    /// never ready) arrive with the state still Stopped - the
    /// manager was never started. A cancelled start is filtered
    /// by the supervisor wrapper instead, on the error type
    /// (`ProcessError::TunnelCancelled`, stage 7 review m1).
    pub fn fail_start(&self, error: String) {
        if matches!(
            self.state(),
            TunnelState::UpstreamDown | TunnelState::Starting
        ) {
            return;
        }
        self.set_last_error(Some(error.clone()));
        if self.state() != TunnelState::Error {
            self.log("error", &format!("failed to start tunnel: {error}"));
            self.set_state(TunnelState::Error);
        }
    }

    /// The start itself, under the lifecycle lock. On success the
    /// state is Running, Degraded, or Unregistered and the monitor
    /// is armed; on failure no child is running. `generation` is
    /// the start's own generation (captured in `start_with`): the
    /// attempt loop and the readiness/warmup waits abort with
    /// `Cancelled` when it changes (a stop in progress, stage 7 O2).
    async fn start_inner(
        &self,
        options: &SingBoxStartOptions,
        generation: u64,
    ) -> Result<(), SingBoxError> {
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

        // The upstream gate (stage 7, design section 8): with no
        // network, refuse to spawn - an engine that cannot register
        // would only report Unregistered/Degraded and burn the
        // restart budget. The gate sits here, in the single start
        // path, so manual starts, the supervisor reconcile, and
        // the auto-restart loop all respect it. `upstream_override`
        // is the test seam (`None` in production).
        let upstream_ok = options.upstream_override.unwrap_or_else(upstream_available);
        if !upstream_ok {
            return Err(SingBoxError::UpstreamDown);
        }

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
            // A stop that landed between attempts (stage 7, O2):
            // do not spawn an engine the stop is about to kill.
            if !self.generation_ok(generation) {
                return Err(SingBoxError::Cancelled);
            }
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
                    generation,
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
    /// not take it. `generation` is the start's own generation
    /// (stage 7, O2): the readiness and warmup waits abort with
    /// `Cancelled` when it changes. Direct test callers pass the
    /// manager's current generation (0 on a fresh manager - nothing
    /// bumps it in those tests).
    ///
    /// On success the state is Running, Degraded, or Unregistered.
    /// On failure the (dead) child is removed and the state is left
    /// as Starting - the caller decides whether to retry (bind
    /// conflict) or fail the start. A start cancelled in readiness
    /// kills the live child first; a start cancelled in warmup
    /// returns without killing it - the caller's error arm
    /// (`start()`) kills it before the stop takes the lock.
    async fn spawn_and_warm(
        &self,
        binary: &Path,
        work_dir: &Path,
        config_path: &Path,
        clash_port: u16,
        clash_secret: &str,
        generation: u64,
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
        // The stability window for the restart budget (stage 6)
        // starts at the spawn, not at the warmup: a long warmup
        // must not count against the engine.
        *self.engine_started_at.write() = Some(Instant::now());
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

        match self.wait_ready(&client, generation).await {
            Readiness::Ready => {}
            Readiness::Cancelled => {
                // The stop that cancelled this start is waiting on
                // the lifecycle lock: kill the live child now, so
                // the stop finds nothing to wait for.
                self.kill_child().await;
                return Err(SpawnAttemptError {
                    error: SingBoxError::Cancelled,
                    bind_error: false,
                });
            }
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

        // Reset the DNS ladder once per launch, before the warmup
        // measures the exit (stage 6): the engine persists the
        // Clash mode in its cache file, so a step from a previous
        // run would otherwise survive into this one.
        self.reset_dns_mode(&client).await;

        if let Err(error) = self.warmup(&client, generation).await {
            return Err(SpawnAttemptError {
                error,
                bind_error: false,
            });
        }
        self.spawn_monitor();
        self.spawn_watchdog(client);
        Ok(())
    }

    /// Reset the DNS ladder once per launch (NETZapret's
    /// ResetDnsLeversAsync): set the Clash mode back to Rule and
    /// sync the step tracker from the engine's actual mode.
    /// Best-effort - a stale step is not fatal, and the tracker is
    /// synced from the engine afterwards, so a failed reset cannot
    /// desync it. The start never fails on this.
    async fn reset_dns_mode(&self, client: &ClashApiClient) {
        if let Err(error) = client.set_mode(DnsStep::Rule.mode()).await {
            self.log(
                "warn",
                &format!("could not reset the dns mode to Rule: {error}"),
            );
        }
        match client.get_mode().await {
            Ok(mode) => {
                *self.dns_step.write() = DnsStep::from_mode(&mode);
            }
            Err(error) => {
                *self.dns_step.write() = DnsStep::Rule;
                self.log(
                    "warn",
                    &format!("could not read the dns mode after reset: {error}"),
                );
            }
        }
    }

    /// Poll Clash API `alive()` until the engine answers, the child
    /// dies, the deadline passes, or a stop bumps the generation
    /// (stage 7, O2 - the poll interval bounds the cancellation
    /// latency to ~250 ms).
    async fn wait_ready(&self, client: &ClashApiClient, generation: u64) -> Readiness {
        let deadline = Instant::now() + READINESS_TIMEOUT;
        loop {
            // Cancellation first: a stop is waiting on the
            // lifecycle lock, and every extra poll delays it.
            if !self.generation_ok(generation) {
                return Readiness::Cancelled;
            }
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

    /// Wait until the generation changes (a stop in progress, stage
    /// 7 O2). Races the warmup measurement in `warmup`: the
    /// measurement is a single HTTP request with a ~15 s ceiling,
    /// so without this the stop would wait it out.
    async fn cancelled(&self, generation: u64) {
        loop {
            if !self.generation_ok(generation) {
                return;
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
    async fn warmup(&self, client: &ClashApiClient, generation: u64) -> Result<(), SingBoxError> {
        // The engine was alive a moment ago; if it died in between,
        // say so instead of measuring a corpse.
        if let Some(status) = self.try_wait_child() {
            let _ = self.child.lock().take();
            return Err(SingBoxError::ExitedDuringStart(format!(
                "exit status: {status}"
            )));
        }
        // The measurement is one HTTP request (up to ~15 s with the
        // grace); race it against a stop (stage 7, O2) so a stop
        // during warmup does not wait it out. Dropping the request
        // future mid-flight is safe - reqwest cancels the request.
        let measured = tokio::select! {
            result = client.measure_exit(MASQUE_TAG, WARMUP_DELAY_TIMEOUT, WARMUP_DELAY_URL) => Some(result),
            _ = self.cancelled(generation) => None,
        };
        let Some(measured) = measured else {
            return Err(SingBoxError::Cancelled);
        };
        match measured {
            Ok(delay) => {
                *self.warmup_delay_ms.write() = Some(delay);
                self.set_state(TunnelState::Running);
                self.log("info", &format!("warp exit warm: {delay} ms"));
                Ok(())
            }
            Err(ClashApiError::ExitFailed(message)) if is_tunnel_not_initialized(&message) => {
                self.set_state(TunnelState::Unregistered);
                self.log(
                    "warn",
                    "engine is up but warp is not registered yet (tunnel not initialized)",
                );
                Ok(())
            }
            Err(error) => {
                self.set_state(TunnelState::Degraded);
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

    /// Arm the watchdog task (stage 6): every 5 s (NETZapret's
    /// poll) it checks Clash API liveness, and every ~30 s it
    /// measures the exit itself. A silent API with a live child is
    /// Broken (restart); a dead exit with a live engine is Degraded
    /// (never a restart). Generation-gated like the monitor: a
    /// newer start cancels it. The client is the one built for this
    /// engine instance (port and secret from its generated config).
    fn spawn_watchdog(&self, client: ClashApiClient) {
        let generation = *self.generation.read();
        let this = self.clone();
        tokio::spawn(async move {
            let mut tick: u32 = 0;
            let mut alive_misses: u32 = 0;
            loop {
                tokio::time::sleep(WATCHDOG_TICK).await;
                if !this.generation_ok(generation) {
                    return;
                }
                if !matches!(
                    this.state(),
                    TunnelState::Running | TunnelState::Degraded | TunnelState::Unregistered
                ) {
                    return;
                }
                // A dead child is the monitor's job: it reports the
                // death (and schedules the restart). The watchdog
                // must not race it with a Broken report.
                if this.try_wait_child().is_some() {
                    return;
                }
                // Stability earns a fresh budget: reset the counter
                // once the engine has run past the stable window,
                // so the UI stops showing a stale "N / 5" (m3).
                this.maybe_reset_restart_budget();
                tick += 1;
                if tick % WATCHDOG_DEEP_EVERY_TICKS == 0 {
                    // The deep check proves liveness itself (a
                    // transport failure there reports Broken), so
                    // the miss counter resets unconditionally.
                    alive_misses = 0;
                    this.deep_check(&client, generation).await;
                } else if client.alive().await {
                    alive_misses = 0;
                } else {
                    alive_misses += 1;
                    if alive_misses >= ALIVE_MISSES_BEFORE_BROKEN {
                        this.report_broken(
                            "clash api stopped answering while the engine process is alive",
                            generation,
                        )
                        .await;
                        return;
                    }
                }
            }
        });
    }

    /// Reset the auto-restart budget once the engine has run past
    /// RESTART_STABLE_WINDOW (stage 6, review m3): the counter
    /// tracks consecutive flapping, not lifetime restarts, and the
    /// UI must not keep showing "1 / 5" for a tunnel that runs
    /// stably for days. Idempotent: a zero counter is left alone
    /// (no emit), and `schedule_restart` recalculates the same
    /// reset by the same threshold when the next death is
    /// recorded, so the two paths cannot disagree.
    fn maybe_reset_restart_budget(&self) {
        if *self.restart_attempts.read() == 0 {
            return;
        }
        if let Some(started_at) = *self.engine_started_at.read() {
            if started_at.elapsed() >= RESTART_STABLE_WINDOW {
                *self.restart_attempts.write() = 0;
                self.sink.emit_tunnel_state(self.state());
            }
        }
    }

    /// One deep check (stage 6): measure the WARP exit itself, not
    /// just the listening port. The exit is measured the same way
    /// the warmup measures it - the engine's own delay probe of the
    /// MASQUE outbound - not through the mixed health inbound: a
    /// request through the mixed inbound would measure the same
    /// exit through an extra hop. A working exit recovers Degraded
    /// / Unregistered to Running (and tries one DNS step back
    /// toward Rule); "tunnel not initialized" is Unregistered; any
    /// other failure walks the DNS ladder up, re-measuring after
    /// each step. All steps failing means Degraded, never a
    /// restart: a dead exit is not a broken engine. A transport
    /// failure (the API itself unreachable) is Broken and restarts.
    async fn deep_check(&self, client: &ClashApiClient, generation: u64) {
        if !self.generation_ok(generation) {
            return;
        }
        match client
            .measure_exit(MASQUE_TAG, WARMUP_DELAY_TIMEOUT, WARMUP_DELAY_URL)
            .await
        {
            Ok(delay) => {
                if !self.generation_ok(generation) {
                    return;
                }
                *self.warmup_delay_ms.write() = Some(delay);
                self.try_step_dns_down(client, generation).await;
                if self.generation_ok(generation)
                    && matches!(
                        self.state(),
                        TunnelState::Degraded | TunnelState::Unregistered
                    )
                {
                    self.set_state(TunnelState::Running);
                    self.log("info", &format!("warp exit recovered: {delay} ms"));
                }
            }
            Err(ClashApiError::ExitFailed(message)) if is_tunnel_not_initialized(&message) => {
                if self.generation_ok(generation) && self.state() != TunnelState::Unregistered {
                    self.set_state(TunnelState::Unregistered);
                    self.log(
                        "warn",
                        "warp exit reports tunnel not initialized (registration pending)",
                    );
                }
            }
            Err(ClashApiError::Transport(_)) => {
                self.report_broken("clash api unreachable during the exit check", generation)
                    .await;
            }
            Err(error) => {
                self.log(
                    "info",
                    &format!("warp exit did not answer, trying the dns ladder: {error}"),
                );
                self.step_dns_ladder_up(client, generation).await;
            }
        }
    }

    /// One DNS step back toward Rule after a successful exit
    /// measurement (the return path of the ladder): a transient DoH
    /// outage must not pin the engine to the spare forever. The
    /// step is re-measured; if the lower step fails, the previous
    /// step is restored immediately. Best-effort: any failure
    /// keeps the current step.
    async fn try_step_dns_down(&self, client: &ClashApiClient, generation: u64) {
        let current = *self.dns_step.read();
        let Some(lower) = current.next_down() else {
            return;
        };
        if !self.set_dns_step(client, lower, generation).await {
            return;
        }
        match client
            .measure_exit(MASQUE_TAG, WARMUP_DELAY_TIMEOUT, WARMUP_DELAY_URL)
            .await
        {
            Ok(delay) => {
                if !self.generation_ok(generation) {
                    return;
                }
                *self.warmup_delay_ms.write() = Some(delay);
                self.log(
                    "info",
                    &format!("dns ladder stepped down to {}", lower.mode()),
                );
            }
            Err(_) => {
                // The lower step does not work (yet): restore the
                // step that just answered. No re-measure needed -
                // it worked a moment ago. If the restore itself
                // fails (the API died mid-step), the next watchdog
                // tick's liveness check reports it as Broken.
                if !self.set_dns_step(client, current, generation).await {
                    self.log(
                        "warn",
                        "could not restore the dns step after a failed step-down",
                    );
                }
            }
        }
    }

    /// Walk the DNS ladder up (Rule -> Tcp -> Spare), re-measuring
    /// the exit after each step. A step that recovers the exit
    /// stays; all steps failing means the exit itself is dead -
    /// reset to Rule and report Degraded (never a restart).
    async fn step_dns_ladder_up(&self, client: &ClashApiClient, generation: u64) {
        loop {
            let current = *self.dns_step.read();
            let Some(next) = current.next_up() else {
                break;
            };
            if !self.set_dns_step(client, next, generation).await {
                // The step change itself failed (or the task went
                // stale): stop climbing, the state below reports it.
                break;
            }
            match client
                .measure_exit(MASQUE_TAG, WARMUP_DELAY_TIMEOUT, WARMUP_DELAY_URL)
                .await
            {
                Ok(delay) => {
                    if !self.generation_ok(generation) {
                        return;
                    }
                    *self.warmup_delay_ms.write() = Some(delay);
                    if self.state() != TunnelState::Running {
                        self.set_state(TunnelState::Running);
                    }
                    self.log(
                        "info",
                        &format!(
                            "warp exit recovered after switching dns to {}: {delay} ms",
                            next.mode()
                        ),
                    );
                    return;
                }
                Err(ClashApiError::ExitFailed(message)) if is_tunnel_not_initialized(&message) => {
                    if self.generation_ok(generation) && self.state() != TunnelState::Unregistered {
                        self.set_state(TunnelState::Unregistered);
                        self.log(
                            "warn",
                            "warp exit reports tunnel not initialized (registration pending)",
                        );
                    }
                    return;
                }
                Err(ClashApiError::Transport(_)) => {
                    self.report_broken("clash api unreachable during the dns ladder", generation)
                        .await;
                    return;
                }
                Err(_) => {
                    // This step did not help either: climb on.
                }
            }
        }
        // Every step failed: the exit is dead, not the resolver.
        // Reset the ladder so the next cycle retries from the top.
        let _ = self.set_dns_step(client, DnsStep::Rule, generation).await;
        if self.generation_ok(generation) && self.state() != TunnelState::Degraded {
            self.set_state(TunnelState::Degraded);
            self.log(
                "warn",
                "warp exit did not answer on any dns step - the exit is down, the engine is not restarted",
            );
        }
    }

    /// Set the DNS ladder step: PATCH the Clash mode, then verify
    /// with GET - the engine silently ignores unknown modes (still
    /// 204), so an unverified set would look like success while
    /// nothing changed. Returns false on any failure or a stale
    /// generation; the step tracker only moves on a verified set.
    async fn set_dns_step(&self, client: &ClashApiClient, step: DnsStep, generation: u64) -> bool {
        if !self.generation_ok(generation) {
            return false;
        }
        if let Err(error) = client.set_mode(step.mode()).await {
            self.log(
                "warn",
                &format!("could not switch the dns mode to {}: {error}", step.mode()),
            );
            return false;
        }
        match client.get_mode().await {
            Ok(mode) if DnsStep::from_mode(&mode) == step => {
                *self.dns_step.write() = step;
                true
            }
            Ok(mode) => {
                self.log(
                    "warn",
                    &format!(
                        "dns mode change to {} did not stick (engine reports {mode})",
                        step.mode()
                    ),
                );
                false
            }
            Err(error) => {
                self.log(
                    "warn",
                    &format!("could not verify the dns mode after switching: {error}"),
                );
                false
            }
        }
    }

    /// Is this task still the current generation's task? Every
    /// watchdog and restart step re-checks: a newer start (or a
    /// stop) bumps the generation, and a stale task must not touch
    /// state, the engine, or the restart budget.
    fn generation_ok(&self, generation: u64) -> bool {
        *self.generation.read() == generation
    }

    /// Record an unexpected engine death, unless a newer start owns
    /// the slot now (generation guard) or the tunnel is already
    /// stopping/stopped. A recorded death schedules the auto-restart
    /// (stage 6).
    fn on_engine_gone(&self, reason: &str, generation: u64) {
        if self.record_unexpected_failure(reason, generation) {
            self.schedule_restart(generation);
        }
    }

    /// The Broken path (stage 6): the engine process is alive but
    /// its Clash API stopped answering - a dead HTTP stack, not a
    /// dead exit. Kill the process first (a restart cannot spawn a
    /// second engine on the same TUN), then record the failure and
    /// schedule the restart like an unexpected death.
    async fn report_broken(&self, reason: &str, generation: u64) {
        if !self.generation_ok(generation) {
            return;
        }
        self.kill_child().await;
        if self.record_unexpected_failure(reason, generation) {
            self.schedule_restart(generation);
        }
    }

    /// The shared core of `on_engine_gone` and `report_broken`
    /// (stage 6): generation-guard, cleanup of the dead engine's
    /// snapshot, and the transition to Error - only from a state
    /// where the engine was actually running. Returns whether the
    /// transition happened: only then does the caller schedule a
    /// restart, so two reporters of the same death (the monitor and
    /// the watchdog racing) schedule exactly one restart task.
    fn record_unexpected_failure(&self, reason: &str, generation: u64) -> bool {
        if !self.generation_ok(generation) {
            return false;
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
            // The check-then-set above holds the guard, so this
            // transition cannot go through set_state - emit inline.
            self.sink.emit_tunnel_state(TunnelState::Error);
            self.log("error", reason);
            true
        } else {
            false
        }
    }

    /// Schedule the auto-restart of a broken engine (stage 6): a
    /// detached task that waits the backoff and re-runs the last
    /// start options. Detached, not awaited: the caller is the
    /// monitor or the watchdog, and the restart must survive them.
    /// The budget is spent here, one attempt per scheduling, so a
    /// flapping engine cannot restart forever; an engine that ran
    /// for `RESTART_STABLE_WINDOW` earns a fresh budget. A restart
    /// is scheduled only after a death of a running engine - a
    /// failed start never gets one (`start()` already has its own
    /// TOCTOU retry, and a deterministically failing start would
    /// burn the budget in one go).
    fn schedule_restart(&self, generation: u64) {
        if !self.generation_ok(generation) {
            return;
        }
        let options = match self.last_options.read().clone() {
            Some(options) => options,
            // No options: the engine was spawned by the test helper
            // (`spawn_and_warm` directly), not by `start()` - there
            // is nothing to re-run.
            None => return,
        };
        // Stability earns a fresh budget: the counter tracks
        // consecutive flapping, not lifetime restarts.
        if let Some(started_at) = *self.engine_started_at.read() {
            if started_at.elapsed() >= RESTART_STABLE_WINDOW {
                *self.restart_attempts.write() = 0;
            }
        }
        // The counter is written by the loop when an attempt
        // actually starts (after its backoff) - not here: a
        // scheduled attempt that never runs (the user stopped the
        // tunnel during the backoff) must not spend the budget,
        // and the UI must never show "6 / 5".
        let attempt = *self.restart_attempts.read() + 1;
        let this = self.clone();
        tokio::spawn(async move {
            this.restart_loop(generation, attempt, options).await;
        });
    }

    /// One auto-restart chain (stage 6): wait the backoff, check
    /// the generation and the state (the user may have stopped or
    /// restarted the tunnel meanwhile), re-run the saved start
    /// options. A successful restart ends the chain; a failed one
    /// waits the next backoff and tries again until the budget is
    /// spent. The warmup runs again after every restart - the new
    /// engine instance is judged on its own exit.
    ///
    /// UpstreamDown (stage 7) is not a failure of the engine, so it
    /// does not spend the budget: the chain waits out the capped
    /// backoff and re-checks the gate, for as long as the tunnel
    /// stays in UpstreamDown. The tunnel then recovers on its own
    /// when the network returns, instead of sitting dead until the
    /// user notices. The wait is generation-gated like everything
    /// else here - a stop ends the chain on the next check.
    async fn restart_loop(&self, generation: u64, mut attempt: u32, options: SingBoxStartOptions) {
        loop {
            let Some(delay) = restart_delay(attempt) else {
                if self.generation_ok(generation) && self.state() == TunnelState::Error {
                    // The budget is spent: say so in the status the
                    // UI shows, so the user learns the tunnel needs
                    // a manual retry instead of waiting silently.
                    self.set_last_error(Some(format!(
                        "автоперезапуски исчерпаны: {} попыток не подняли движок. Нажмите «Попробовать снова».",
                        MAX_AUTO_RESTARTS
                    )));
                    self.sink.emit_tunnel_state(TunnelState::Error);
                    self.log(
                        "error",
                        &format!(
                            "auto-restart budget spent ({} attempts), tunnel stays in Error",
                            MAX_AUTO_RESTARTS
                        ),
                    );
                }
                return;
            };
            tokio::time::sleep(delay).await;
            if !self.generation_ok(generation)
                || !matches!(self.state(), TunnelState::Error | TunnelState::UpstreamDown)
            {
                return;
            }
            // The attempt counts as used from the moment it starts:
            // after 5 failed attempts the counter reads 5, not 6.
            *self.restart_attempts.write() = attempt;
            match self.start_restart(options.clone()).await {
                Ok(status) => {
                    self.log(
                        "info",
                        &format!(
                            "tunnel auto-restarted on attempt {attempt} (state: {:?})",
                            status.state
                        ),
                    );
                    return;
                }
                Err(SingBoxError::UpstreamDown) => {
                    // No network, no engine failure: give the
                    // attempt back (the counter tracks engine
                    // restarts, and none happened) and wait out the
                    // capped backoff before re-checking the gate.
                    // `attempt` is unchanged, so the loop-top sleep
                    // stays short - the long wait is this one.
                    *self.restart_attempts.write() = attempt.saturating_sub(1);
                    self.log(
                        "info",
                        "no network during the auto-restart: tunnel stays UpstreamDown, re-checking the gate",
                    );
                    tokio::time::sleep(RESTART_BACKOFF_MAX).await;
                }
                Err(SingBoxError::Cancelled) => {
                    // A stop cancelled this restart attempt: the
                    // generation check at the top of the loop would
                    // end the chain anyway - return directly instead
                    // of logging a misleading "attempt failed".
                    return;
                }
                Err(error) => {
                    self.log(
                        "warn",
                        &format!("tunnel auto-restart attempt {attempt} failed: {error}"),
                    );
                    attempt += 1;
                }
            }
        }
    }

    /// Request cancellation of an in-flight start (stage 7, O2):
    /// bumps the generation without taking the lifecycle lock, so a
    /// start in progress aborts within ~250 ms. `stop()` does this
    /// itself; the supervisor calls it before taking ITS lifecycle
    /// lock, so a `stop_tunnel` queued behind a `start_tunnel` does
    /// not wait out the whole start.
    pub fn request_cancel(&self) {
        *self.generation.write() += 1;
    }

    /// Stop the tunnel: kill the child, wait up to 5 s, clear the
    /// state. A no-op while stopped. Deliberately does NOT terminate
    /// the shared Job Object (winws2 lives in the same job - the
    /// independent-switches matrix requires tunnel-off/desync-on to
    /// keep working); sing-box spawns no grandchildren, so kill+wait
    /// is a complete stop.
    ///
    /// The generation bump comes BEFORE the lifecycle lock (stage 7,
    /// O2): a start in progress holds the lock through readiness and
    /// warmup (up to ~70 s worst case), and bumping first lets its
    /// wait loops abort within ~250 ms instead. The bump also
    /// invalidates the monitor/watchdog/restart tasks of a running
    /// engine - they exit on their next tick, and the stop below
    /// kills the child anyway. A redundant stop (already Stopped)
    /// still bumps - harmless, there is no live task left to
    /// invalidate by then.
    pub async fn stop(&self) -> Result<(), SingBoxError> {
        self.request_cancel();
        let _guard = self.lifecycle.lock().await;

        if matches!(
            *self.state.read(),
            TunnelState::Stopped | TunnelState::Stopping
        ) {
            return Ok(());
        }
        self.set_state(TunnelState::Stopping);
        self.log("info", "stopping sing-box...");

        self.kill_child().await;

        *self.pid.write() = None;
        *self.clash_api_port.write() = None;
        *self.warmup_delay_ms.write() = None;
        *self.tun_scope.write() = None;
        // A stop is a fresh session (stage 6): the restart budget
        // and the stability clock reset with it. The DNS step
        // resets too - the next launch resets the engine's mode
        // anyway, so the tracker must not remember the old step.
        *self.restart_attempts.write() = 0;
        *self.engine_started_at.write() = None;
        *self.dns_step.write() = DnsStep::Rule;
        self.set_state(TunnelState::Stopped);
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
// `install_update` refuses while the tunnel is active. This is the
// local manager: when the GUI is attached to the svc, the tunnel
// lives in the svc and the updater asks it over IPC instead (stage
// 5: `TunnelGetStatus`, see `commands.rs`).
/// The sidecar name `get_sidecar_statuses` matches on to override
/// the entry with the IPC status when attached to the svc (stage 5).
/// GUI-only: read by `commands.rs`, never by the svc binary (its
/// `name()` is never called there), hence the allow - without it
/// the svc build warns (stage 5 review m2).
#[allow(dead_code)]
pub const SINGBOX_SIDECAR_NAME: &str = "sing-box (WARP tunnel)";

impl ManagedSidecar for SingBoxManager {
    fn name(&self) -> &'static str {
        SINGBOX_SIDECAR_NAME
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
        SingBoxManager::new(Arc::new(job), log_tx, file_logger, SinkSlot::new())
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
            upstream_override: None,
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
        // Stage 7: the eighth state serializes the same way
        // ("upstreamdown", no separator - the lowercase convention).
        assert_eq!(
            serde_json::to_string(&TunnelState::UpstreamDown).unwrap(),
            "\"upstreamdown\""
        );
        assert_eq!(
            serde_json::from_str::<TunnelState>("\"upstreamdown\"").unwrap(),
            TunnelState::UpstreamDown
        );
    }

    #[test]
    fn route_table_default_route_detection() {
        let header =
            "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n";
        // A default route on a real interface, up with a gateway.
        assert!(route_table_has_default_route(&format!(
            "{header}eth0\t00000000\t0101A8C0\t0003\t0\t0\t0\t00000000\t0\t0\t0\n"
        )));
        // Up without a gateway (directly-connected default) still
        // counts: traffic has a way out.
        assert!(route_table_has_default_route(&format!(
            "{header}ppp0\t00000000\t00000000\t0001\t0\t0\t0\t00000000\t0\t0\t0\n"
        )));
        // A default route on loopback does not count.
        assert!(!route_table_has_default_route(&format!(
            "{header}lo\t00000000\t00000000\t0003\t0\t0\t0\t00000000\t0\t0\t0\n"
        )));
        // A default route on a down interface does not count.
        assert!(!route_table_has_default_route(&format!(
            "{header}eth0\t00000000\t0101A8C0\t0000\t0\t0\t0\t00000000\t0\t0\t0\n"
        )));
        // Connected routes only, no default: a LAN without a way out.
        assert!(!route_table_has_default_route(&format!(
            "{header}eth0\t0001A8C0\t00000000\t0001\t0\t0\t0\t00FFFFFF\t0\t0\t0\n"
        )));
        // Empty table (header only) and garbage.
        assert!(!route_table_has_default_route(header));
        assert!(!route_table_has_default_route(""));
        assert!(!route_table_has_default_route(&format!(
            "{header}eth0\t00000000\t0101A8C0\tZZZZ\t0\t0\t0\t00000000\t0\t0\t0\n"
        )));
    }

    #[test]
    fn adapter_upstream_decision_table() {
        let routes: HashSet<u32> = [7, 9].into_iter().collect();
        // Up, not loopback, not our tunnel, carries a default route.
        assert!(adapter_is_upstream(7, true, false, false, &routes));
        // Down.
        assert!(!adapter_is_upstream(7, false, false, false, &routes));
        // Loopback.
        assert!(!adapter_is_upstream(7, true, true, false, &routes));
        // Our own tunnel adapter.
        assert!(!adapter_is_upstream(7, true, false, true, &routes));
        // Up and clean, but no default route on it.
        assert!(!adapter_is_upstream(11, true, false, false, &routes));
        // No default routes at all.
        assert!(!adapter_is_upstream(7, true, false, false, &HashSet::new()));
    }

    #[test]
    fn engine_log_tail_reads_last_lines() {
        let dir =
            std::env::temp_dir().join(format!("zapret2-singbox-logtail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("sing-box.log");
        // Whole file fits the budget: returned as-is.
        std::fs::write(&log, "line one\nline two\n").unwrap();
        assert_eq!(
            read_engine_log_tail_limited(&dir, 1024),
            "line one\nline two\n"
        );
        // Budget cuts mid-file: the tail starts on a line boundary,
        // the partial first line is dropped.
        std::fs::write(&log, "aaaaaaaaaa\nbbbbbbbbbb\ncccccccccc\n").unwrap();
        // 15 bytes from the end: "bbb\ncccccccccc\n" - the partial
        // "bbb" line is dropped, only the full last line survives.
        assert_eq!(read_engine_log_tail_limited(&dir, 15), "cccccccccc\n");
        // No newline in the tail at all: the partial line is shown
        // anyway - a diagnostic with one cut line beats an empty one.
        std::fs::write(&log, "0123456789abcdef").unwrap();
        assert_eq!(read_engine_log_tail_limited(&dir, 6), "abcdef");
        // Missing file: empty, not an error.
        std::fs::remove_file(&log).unwrap();
        assert_eq!(read_engine_log_tail_limited(&dir, 1024), "");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restart_backoff_doubles_and_caps() {
        assert_eq!(backoff_delay(1), Duration::from_secs(2));
        assert_eq!(backoff_delay(2), Duration::from_secs(4));
        assert_eq!(backoff_delay(3), Duration::from_secs(8));
        assert_eq!(backoff_delay(4), Duration::from_secs(16));
        assert_eq!(backoff_delay(5), Duration::from_secs(32));
        // The cap binds at 2 min even for attempts beyond the budget.
        assert_eq!(backoff_delay(10), Duration::from_secs(120));
    }

    #[test]
    fn restart_delay_allows_five_attempts_then_none() {
        assert_eq!(restart_delay(0), None);
        for attempt in 1..=5 {
            assert_eq!(restart_delay(attempt), Some(backoff_delay(attempt)));
        }
        assert_eq!(restart_delay(6), None);
        assert_eq!(restart_delay(100), None);
    }

    #[test]
    fn restart_budget_resets_after_stable_window() {
        // Review m3: an engine that restarted once and then runs
        // stably must not show "1 / 5" forever.
        let manager = test_manager();
        *manager.restart_attempts.write() = 3;
        *manager.engine_started_at.write() =
            Some(Instant::now() - RESTART_STABLE_WINDOW - Duration::from_secs(1));
        manager.maybe_reset_restart_budget();
        assert_eq!(manager.status().restarts_used, 0);
    }

    #[test]
    fn restart_budget_kept_within_stable_window() {
        let manager = test_manager();
        *manager.restart_attempts.write() = 3;
        *manager.engine_started_at.write() = Some(Instant::now());
        manager.maybe_reset_restart_budget();
        assert_eq!(manager.status().restarts_used, 3);
    }

    #[test]
    fn dns_step_transitions() {
        assert_eq!(DnsStep::Rule.next_up(), Some(DnsStep::Tcp));
        assert_eq!(DnsStep::Tcp.next_up(), Some(DnsStep::Spare));
        assert_eq!(DnsStep::Spare.next_up(), None);
        assert_eq!(DnsStep::Rule.next_down(), None);
        assert_eq!(DnsStep::Tcp.next_down(), Some(DnsStep::Rule));
        assert_eq!(DnsStep::Spare.next_down(), Some(DnsStep::Tcp));
    }

    #[test]
    fn dns_step_modes_are_the_generator_strings() {
        // "Rule" is capitalised: that is how the engine reports the
        // default mode. The step modes are the generator's constants
        // (the same strings the config's clash_mode rules use).
        assert_eq!(DnsStep::Rule.mode(), "Rule");
        assert_eq!(DnsStep::Tcp.mode(), DNS_TCP_MODE);
        assert_eq!(DnsStep::Spare.mode(), DNS_SPARE_MODE);
    }

    #[test]
    fn dns_step_parses_engine_modes() {
        assert_eq!(DnsStep::from_mode("Rule"), DnsStep::Rule);
        assert_eq!(DnsStep::from_mode("rule"), DnsStep::Rule);
        assert_eq!(DnsStep::from_mode("dns-tcp"), DnsStep::Tcp);
        assert_eq!(DnsStep::from_mode("DNS-TCP"), DnsStep::Tcp);
        assert_eq!(DnsStep::from_mode("dns-spare"), DnsStep::Spare);
        // Unknown modes behave like the default: the ladder rules
        // only match the two step modes.
        assert_eq!(DnsStep::from_mode("bogus-mode"), DnsStep::Rule);
        assert_eq!(DnsStep::from_mode(""), DnsStep::Rule);
    }

    #[test]
    fn singbox_status_serde_defaults_restart_fields() {
        // A stage 5 snapshot (no restart fields) must still parse:
        // the counters default to 0. That is safe for the UI: it
        // renders the restart line only when restarts_used > 0, and
        // an old snapshot always has restarts_used == 0. A live
        // manager always sends the full snapshot.
        let old: SingBoxStatus = serde_json::from_str(
            r#"{"state":"degraded","pid":4242,"engine_version":null,"clash_api_port":null,"warmup_delay_ms":null,"last_error":null}"#,
        )
        .unwrap();
        assert_eq!(old.restarts_used, 0);
        assert_eq!(old.restarts_max, 0);
        // And a current snapshot round-trips the counters.
        let current = SingBoxStatus {
            restarts_used: 2,
            restarts_max: MAX_AUTO_RESTARTS,
            ..Default::default()
        };
        let back: SingBoxStatus =
            serde_json::from_str(&serde_json::to_string(&current).unwrap()).unwrap();
        assert_eq!(back.restarts_used, 2);
        assert_eq!(back.restarts_max, MAX_AUTO_RESTARTS);
        assert_eq!(SingBoxStatus::default().restarts_max, MAX_AUTO_RESTARTS);
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

    /// Stage 7: with the upstream gate closed, the start refuses to
    /// spawn and lands in UpstreamDown (not Error), with a message
    /// the UI can show. The binary is a dummy file that is never
    /// executed - the gate runs before the version check. The
    /// `upstream_override` seam forces the gate closed without
    /// pulling the network cable.
    #[tokio::test]
    async fn start_with_no_upstream_reports_upstream_down() {
        let dir =
            std::env::temp_dir().join(format!("zapret2-singbox-nogate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dummy = dir.join("dummy-sing-box");
        std::fs::write(&dummy, "").unwrap();
        let options = SingBoxStartOptions {
            upstream_override: Some(false),
            ..test_options(dummy, dir.clone())
        };
        let manager = test_manager();
        let error = manager.start(options).await.unwrap_err();
        assert!(matches!(error, SingBoxError::UpstreamDown));
        assert_eq!(manager.state(), TunnelState::UpstreamDown);
        let status = manager.status();
        assert!(status.pid.is_none(), "the gate must not spawn anything");
        assert!(status.clash_api_port.is_none());
        let message = status.last_error.expect("a gate message for the UI");
        assert!(
            message.contains("нет сети"),
            "unexpected gate message: {message}"
        );
        // UpstreamDown counts as active (the tunnel is wanted, the
        // restart loop may still re-check the gate), and a stop
        // clears it like any other failed start.
        assert!(manager.is_active());
        manager.stop().await.unwrap();
        assert_eq!(manager.state(), TunnelState::Stopped);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Stage 7: `fail_start` (the supervisor's wrapper for early
    /// failures) must not overwrite UpstreamDown with Error - the
    /// manager already recorded the gate failure, and the "no
    /// network" distinction is the whole point of the state.
    #[tokio::test]
    async fn fail_start_keeps_upstream_down() {
        let dir =
            std::env::temp_dir().join(format!("zapret2-singbox-failstart-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dummy = dir.join("dummy-sing-box");
        std::fs::write(&dummy, "").unwrap();
        let options = SingBoxStartOptions {
            upstream_override: Some(false),
            ..test_options(dummy, dir.clone())
        };
        let manager = test_manager();
        manager.start(options).await.unwrap_err();
        assert_eq!(manager.state(), TunnelState::UpstreamDown);
        let before = manager.status().last_error.clone();
        manager.fail_start("some supervisor-side failure".to_string());
        assert_eq!(manager.state(), TunnelState::UpstreamDown);
        assert_eq!(manager.status().last_error, before);
        manager.stop().await.unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Stage 7 review m1: the early failures `fail_start` exists
    /// for (no binary, desync restart failed, capture never ready)
    /// arrive with the state still Stopped - the manager was never
    /// started - so a Stopped guard would swallow them. This pins
    /// that `fail_start` on Stopped records the Error; the
    /// cancelled-start case is filtered by the supervisor wrapper
    /// on the error type instead.
    #[test]
    fn fail_start_on_stopped_records_error() {
        let manager = test_manager();
        assert_eq!(manager.state(), TunnelState::Stopped);
        manager.fail_start("sing-box не найден в установке".to_string());
        assert_eq!(manager.state(), TunnelState::Error);
        assert_eq!(
            manager.status().last_error.as_deref(),
            Some("sing-box не найден в установке")
        );
    }

    /// Stage 7 review m1: the cancellation must survive the
    /// conversion to `ProcessError` as its own variant, so the
    /// supervisor wrapper can skip recording it - and every other
    /// error still lands in `Tunnel`.
    #[test]
    fn cancelled_start_error_stays_typed() {
        assert!(matches!(
            ProcessError::from(SingBoxError::Cancelled),
            ProcessError::TunnelCancelled
        ));
        assert!(matches!(
            ProcessError::from(SingBoxError::VersionFailed("x".to_string())),
            ProcessError::Tunnel(_)
        ));
        assert!(matches!(
            ProcessError::from(SingBoxError::UpstreamDown),
            ProcessError::Tunnel(_)
        ));
    }

    /// Stage 7, O2: a start stuck in readiness (a fake engine that
    /// never answers its Clash API) must abort within seconds when
    /// stop() is called - not run out the 20 s readiness timeout
    /// (and not the ~70 s worst case of a two-attempt start).
    /// Unix-only: the fake engine is a shell script. The error is
    /// the discriminating assertion: without the cancellation the
    /// start would fail with NotReady after 20 s, not Cancelled.
    #[cfg(unix)]
    #[tokio::test]
    async fn stop_cancels_a_start_stuck_in_readiness() {
        let dir =
            std::env::temp_dir().join(format!("zapret2-singbox-cancel-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("fake-sing-box");
        // Answers `version` and `check` like a healthy engine, but
        // `run` sleeps: the Clash API never comes up, so the start
        // sits in the readiness wait until it is cancelled.
        let script = "#!/bin/sh\n\
             case \"$1\" in\n\
             \x20 version)\n\
             \x20\x20 echo \"sing-box version 9.9.9-fake\"\n\
             \x20\x20 ;;\n\
             \x20 check)\n\
             \x20\x20 exit 0\n\
             \x20\x20 ;;\n\
             \x20 run)\n\
             \x20\x20 sleep 60\n\
             \x20\x20 ;;\n\
             esac\n";
        std::fs::write(&fake, script).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let manager = test_manager();
        let starter = manager.clone();
        let options = test_options(fake, dir.clone());
        let start_task = tokio::spawn(async move { starter.start(options).await });
        // Let the start reach the readiness wait (version + check +
        // spawn take well under a second in the sandbox).
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(manager.state(), TunnelState::Starting);

        let started = Instant::now();
        manager.stop().await.unwrap();
        let start_result = start_task.await.expect("start task joined");
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_secs(10),
            "stop took {elapsed:?} - the start was not cancelled (readiness timeout is 20 s)"
        );
        let error = start_result.expect_err("the cancelled start must fail");
        assert!(
            matches!(error, SingBoxError::Cancelled),
            "expected Cancelled, got {error:?}"
        );
        assert_eq!(manager.state(), TunnelState::Stopped);
        assert!(manager.status().pid.is_none());

        let _ = std::fs::remove_dir_all(&dir);
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
            .spawn_and_warm(&binary, &dir, &config_path, clash_port, "test-secret", 0)
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
            .spawn_and_warm(&binary, &dir, &config_path, clash_port, "test-secret", 0)
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
            .spawn_and_warm(&binary, &dir, &config_path, clash_port, "test-secret", 0)
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

    /// Read the Clash API secret back from a generated config file
    /// (the manager never exposes it - the tests read the artifact
    /// they generated, like the generator's own tests do).
    fn read_clash_secret(config_path: &Path) -> String {
        let text = std::fs::read_to_string(config_path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        value["experimental"]["clash_api"]["secret"]
            .as_str()
            .expect("generated config carries the clash secret")
            .to_string()
    }

    /// Live test for the stage 6 DNS ladder: the generated config's
    /// modes are switchable through the Clash API, an unknown mode
    /// is silently ignored (the engine still answers 204 - that is
    /// why every set is verified with a get), and a restart resets
    /// the mode to Rule (the engine persists the mode in its cache
    /// file, so without the reset a step from the previous run
    /// would survive). Gated on SINGBOX_TEST_BINARY.
    #[tokio::test]
    async fn live_dns_ladder_modes() {
        let Some(binary) = live_test_binary() else {
            return;
        };

        let dir = std::env::temp_dir().join(format!(
            "zapret2-singbox-live-ladder-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let config_path = dir.join("config.json");

        let manager = test_manager();
        let status = manager
            .start(test_options(binary.clone(), dir.clone()))
            .await
            .expect("start with the generated config");
        assert!(
            matches!(
                status.state,
                TunnelState::Degraded | TunnelState::Unregistered
            ),
            "unexpected state: {:?}",
            status.state
        );

        let client = ClashApiClient::new(
            status.clash_api_port.expect("clash port"),
            &read_clash_secret(&config_path),
        );
        // The launch reset the ladder: the mode is Rule even
        // though this is a fresh cache file (the reset runs on
        // every launch, not just when a stale step exists).
        assert_eq!(client.get_mode().await.expect("get mode"), "Rule");
        client.set_mode(DNS_TCP_MODE).await.expect("set dns-tcp");
        assert_eq!(client.get_mode().await.expect("get mode"), DNS_TCP_MODE);
        client
            .set_mode(DNS_SPARE_MODE)
            .await
            .expect("set dns-spare");
        assert_eq!(client.get_mode().await.expect("get mode"), DNS_SPARE_MODE);
        // The engine silently ignores an unknown mode: the set
        // "succeeds" (204) but the mode does not change.
        client.set_mode("bogus-mode").await.expect("set bogus mode");
        assert_eq!(
            client.get_mode().await.expect("get mode"),
            DNS_SPARE_MODE,
            "an unknown mode must not change the active mode"
        );

        manager.stop().await.unwrap();
        assert_eq!(manager.state(), TunnelState::Stopped);

        // Restart with the same home dir (the cache file survives):
        // the launch reset must bring the mode back to Rule.
        let status = manager
            .start(test_options(binary, dir.clone()))
            .await
            .expect("restart with the same home dir");
        let client = ClashApiClient::new(
            status.clash_api_port.expect("clash port"),
            &read_clash_secret(&config_path),
        );
        assert_eq!(
            client.get_mode().await.expect("get mode after restart"),
            "Rule",
            "a restart must reset the dns ladder to Rule"
        );

        manager.stop().await.unwrap();
        assert_eq!(manager.state(), TunnelState::Stopped);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Live test for the stage 6 auto-restart: an engine that dies
    /// unexpectedly must come back on its own (first backoff 2 s),
    /// with the restart counter advanced and a new pid. The sandbox
    /// cannot connect the MASQUE exit, so the restarted engine
    /// lands in Degraded or Unregistered - the state after a
    /// restart is judged by its own warmup, not assumed. Gated on
    /// SINGBOX_TEST_BINARY.
    #[tokio::test]
    async fn live_unexpected_death_restarts_the_tunnel() {
        let Some(binary) = live_test_binary() else {
            return;
        };

        let dir = std::env::temp_dir().join(format!(
            "zapret2-singbox-live-restart-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let manager = test_manager();
        let status = manager
            .start(test_options(binary, dir.clone()))
            .await
            .expect("start");
        assert!(
            matches!(
                status.state,
                TunnelState::Degraded | TunnelState::Unregistered
            ),
            "unexpected state: {:?}",
            status.state
        );
        let pid = status.pid.expect("engine pid");
        assert_eq!(status.restarts_used, 0);

        kill_pid_external(pid);
        // The monitor reports the death (<= 500 ms), the restart
        // task waits the 2 s backoff, then the restart runs the
        // full start again (warmup up to ~15 s in the sandbox).
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            let status = manager.status();
            if status.restarts_used == 1
                && matches!(
                    status.state,
                    TunnelState::Degraded | TunnelState::Unregistered
                )
                && status.pid.is_some()
                && status.pid != Some(pid)
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "tunnel did not auto-restart, stuck at {:?} (restarts_used={})",
                status.state,
                status.restarts_used
            );
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        manager.stop().await.unwrap();
        assert_eq!(manager.state(), TunnelState::Stopped);
        // A stop is a fresh session: the budget resets with it.
        assert_eq!(manager.status().restarts_used, 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Live test for the stage 6 Degraded rule: a dead exit must
    /// never restart the engine. The engine stays up (its API
    /// answers), only the exit is dead, so over a window covering
    /// the watchdog's first deep check (~30 s in) the pid must not
    /// change and no restart must be spent. The deep check's ladder
    /// may still be climbing when the wait ends (each step
    /// re-measures with a 10 s deadline) - the property holds
    /// regardless, because no path in the deep check schedules a
    /// restart. Gated on SINGBOX_TEST_BINARY.
    #[tokio::test]
    async fn live_degraded_exit_never_restarts() {
        let Some(binary) = live_test_binary() else {
            return;
        };

        let dir = std::env::temp_dir().join(format!(
            "zapret2-singbox-live-degraded-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let manager = test_manager();
        let status = manager
            .start(test_options(binary, dir.clone()))
            .await
            .expect("start");
        assert!(
            matches!(
                status.state,
                TunnelState::Degraded | TunnelState::Unregistered
            ),
            "unexpected state: {:?}",
            status.state
        );
        let pid = status.pid.expect("engine pid");

        // Past the first deep check (armed after the warmup, every
        // 6th 5 s tick), with margin for its first measurement.
        tokio::time::sleep(Duration::from_secs(45)).await;

        let status = manager.status();
        assert!(
            matches!(
                status.state,
                TunnelState::Degraded | TunnelState::Unregistered
            ),
            "unexpected state after the deep check: {:?}",
            status.state
        );
        assert_eq!(status.pid, Some(pid), "the engine must not have restarted");
        assert_eq!(
            status.restarts_used, 0,
            "a dead exit must never spend the restart budget"
        );

        manager.stop().await.unwrap();
        assert_eq!(manager.state(), TunnelState::Stopped);

        let _ = std::fs::remove_dir_all(&dir);
    }
}

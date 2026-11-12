//! sing-box config generator for the WARP mode (stage 2).
//!
//! The WARP mode runs a sing-box sidecar under the svc supervisor (design:
//! `~/workspace/reviews/zapret2-gui/warp-mode-design-2026-11-12.md`, section 4).
//! This module builds the sidecar's JSON config from the current state of the
//! app - TUN scope, system resolvers, desync exclusions, the app's DNS
//! provider - and validates the result with `sing-box check` before any
//! process is spawned (stage 3 owns the spawn).
//!
//! Why the config is generated instead of shipped as a static file: the TUN
//! scope, resolver addresses, desync exclusions and secrets depend on the
//! machine and on the app state at launch time; a static file would drift.
//! The generator is pure except for two OS interactions: it allocates two
//! free loopback ports (bind to port 0) and draws secrets from the OS RNG.
//!
//! Shape of the config (proven by NETZapret 1.6.1 on the same fork,
//! `~/workspace/reviews/zapret2-gui/netzapret-analysis-2026-11-12.md`):
//! - `log`: level `error`, output to a file. The audit
//!   (`singbox-fork-audit-2026-11-12.md`, section 7) corrects the design
//!   here: the design said `info`, but a dead endpoint makes the engine
//!   retry ~1700 times a second, and at `info` that is 25 MB of log in
//!   two minutes. `error` keeps the flood to ERROR lines only.
//! - `certificate`: `{"store": "mozilla"}`. NETZapret hit a 0xc0000005
//!   crash in Go's Windows system-cert verification (crypto/x509
//!   `systemVerify`); the bundled Mozilla roots avoid that code path
//!   entirely. Cost: hand-installed Windows roots are no longer trusted
//!   by the engine (corporate MITM, AV HTTPS interception).
//! - `dns`: `remote` (DoH, the app's DNS provider), `fake` (fakeip),
//!   `remote-tcp` (same resolver over TCP) and `remote-spare` (a second
//!   DoH) as the Rule -> Tcp -> Spare ladder, switched by Clash mode.
//!   The Firefox canary (`use-application-dns.net`) answers NXDOMAIN so
//!   Firefox does not run its own DoH past the engine. `final: remote`.
//! - `inbounds`: TUN (`zapret2tun0`, gvisor, `auto_route`, no
//!   `strict_route` - strict WFP filters break UDP/53 checks) and a
//!   mixed inbound on a random loopback port with a random password,
//!   used only for deep health checks (stage 6).
//! - `outbounds`: `direct`, `block`, and the MASQUE exit
//!   `Cloudflare WARP` over HTTP/2. The tag must never change: the
//!   engine's registration cache is keyed by it. (No `dns` outbound:
//!   the type was removed in sing-box 1.13.0 and the fork rejects the
//!   config for it - the design's `dns-out` line is stale.)
//! - `route`: sniff, hijack-dns, health-in to WARP, private IPs direct,
//!   proxy domains to WARP (ProxyOnly), `final` direct (ProxyOnly) or
//!   WARP (Everything), `default_domain_resolver` (required since 1.12).
//! - `experimental`: Clash API on a random loopback port with a random
//!   32-char secret, and the cache file with `store_masque_config`.
//!
//! Two hard requirements from the fork audit (section 7):
//! - The MASQUE outbound MUST carry `reconnect_delay: "5s"`. The
//!   default is 0, and with a dead endpoint the engine reconnects in a
//!   hot loop (~1700 attempts/s, 67% of one CPU core). Covered by a
//!   test: every generated config contains the field.
//! - The MASQUE outbound must NOT carry `server`, `server_port`,
//!   `license`, `fake_packets` or `tls.utls`: the fork rejects the
//!   whole config with FATAL `json: unknown field`. Covered by tests.
//!
//! TUN inversion (design section 3, NETZapret section 3.3): with desync
//! on, only proxy traffic enters the TUN - the fakeip ranges and the
//! Windows resolver addresses (so hijack-dns sees the queries). The v1
//! domain rules have no static addresses: their traffic reaches the TUN
//! through the fakeip range. Everything else keeps its real addresses and
//! stays visible to WinDivert, so fake-packet desync recipes keep
//! working. With desync off, the tunnel takes all traffic.
//!
//! IPv6 is gated on the `ipv6` input flag: NETZapret filters v6 entries
//! out when the adapter has no IPv6, because Windows cannot install a
//! route to a v6 network the TUN adapter does not have. Stage 3 detects
//! IPv6 presence and passes the flag; the generator never guesses.
//!
//! The config is rendered as pretty JSON, UTF-8 without BOM - Go
//! chokes on a BOM. Paths in the config (cache file, log output) are
//! always absolute; the manager (stage 3) resolves them under
//! `%LOCALAPPDATA%\zapret2\singbox\` and creates the directory before
//! the first start (the engine crashes if the cache directory is
//! missing).

use std::collections::BTreeSet;
use std::net::{IpAddr, TcpListener};
use std::path::{Path, PathBuf};

use rand::rngs::OsRng;
use rand::RngCore;
use serde_json::{json, Value};
use thiserror::Error;

/// Tag of the MASQUE outbound. Must never change: the engine's WARP
/// registration cache (`engine-cache.db`) is keyed by this tag.
pub const MASQUE_TAG: &str = "Cloudflare WARP";

/// Reconnect delay for the MASQUE outbound. The fork's default is 0,
/// which turns a dead endpoint into a hot reconnect loop (~1700
/// attempts/s, see the audit, section 7). The fork's own example uses
/// 5s; the generator always writes it.
pub const MASQUE_RECONNECT_DELAY: &str = "5s";

/// Fakeip ranges (NETZapret defaults, sing-box documentation values).
pub const FAKEIP_V4_RANGE: &str = "198.18.0.0/15";
pub const FAKEIP_V6_RANGE: &str = "fc00::/18";

/// TUN interface name (our own, not NETZapret's `netzapret0`).
pub const TUN_INTERFACE_NAME: &str = "zapret2tun0";
/// TUN interface addresses (NETZapret values, proven in the field).
pub const TUN_ADDRESS_V4: &str = "172.19.0.1/30";
pub const TUN_ADDRESS_V6: &str = "fdfe:dcba:9876::1/126";
/// TUN MTU (NETZapret default).
pub const TUN_MTU: u16 = 1400;

/// Firefox DoH canary: answering NXDOMAIN tells Firefox not to enable
/// its own DoH, so the engine's DNS wins.
pub const FIREFOX_CANARY_DOMAIN: &str = "use-application-dns.net";

/// v1 proxy domains (design section 3, open decision 5): only YouTube
/// video traffic goes through the tunnel. `domain_suffix` in sing-box
/// matches the apex domain and all its subdomains, so `googlevideo.com`
/// covers every `rrN---sn-*.googlevideo.com` node.
pub const PROXY_DOMAIN_SUFFIXES: &[&str] = &["googlevideo.com", "youtube.com"];

// Inbound / outbound / DNS server tags. Fixed strings, referenced from
// route and DNS rules; keep them in one place so a rename cannot drift.
const TAG_TUN: &str = "tun-in";
const TAG_HEALTH: &str = "health-in";
const TAG_DIRECT: &str = "direct";
const TAG_BLOCK: &str = "block";
const TAG_DNS_REMOTE: &str = "remote";
const TAG_DNS_FAKE: &str = "fake";
const TAG_DNS_TCP: &str = "remote-tcp";
const TAG_DNS_SPARE: &str = "remote-spare";

/// Clash modes that switch the DNS ladder steps (NETZapret names).
const DNS_TCP_MODE: &str = "dns-tcp";
const DNS_SPARE_MODE: &str = "dns-spare";

/// Username of the mixed (health-check) inbound. Fixed; only the
/// password is secret (NETZapret does the same).
const HEALTH_USERNAME: &str = "zapret2";

/// Lengths of the random secrets (hex chars). Clash API: 32, like
/// NETZapret's EngineKeys; mixed password: 24, same source.
const CLASH_SECRET_HEX_LEN: usize = 32;
const MIXED_PASSWORD_HEX_LEN: usize = 24;

/// TUN capture scope, derived from the two switches (design section 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TunScope {
    /// Desync on: only proxy traffic enters the TUN (fakeip ranges and
    /// Windows resolver addresses). The v1 domain rules have no static
    /// addresses; their traffic reaches the TUN through the fakeip
    /// range. Everything else keeps real addresses and stays visible
    /// to WinDivert.
    #[default]
    ProxyOnly,
    /// Desync off: the tunnel takes all traffic.
    Everything,
}

/// A DNS-over-HTTPS endpoint for the `remote` DNS server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DohEndpoint {
    /// IP address sing-box dials (the `server` field; an IP, not a
    /// hostname, so no bootstrap resolution is needed to reach it).
    pub server: &'static str,
    /// TLS server name (SNI + certificate verification).
    pub server_name: &'static str,
    /// DoH path. `/dns-query` for all built-ins; only written to the
    /// config when it differs (NETZapret behaviour).
    pub path: &'static str,
}

/// DoH endpoints of the app's built-in DNS providers (same providers as
/// the DNS card: `platform/windows/dns.rs`).
pub const DOH_CLOUDFLARE: DohEndpoint = DohEndpoint {
    server: "1.1.1.1",
    server_name: "cloudflare-dns.com",
    path: "/dns-query",
};
pub const DOH_GOOGLE: DohEndpoint = DohEndpoint {
    server: "8.8.8.8",
    server_name: "dns.google",
    path: "/dns-query",
};
pub const DOH_QUAD9: DohEndpoint = DohEndpoint {
    server: "9.9.9.9",
    server_name: "dns.quad9.net",
    path: "/dns-query",
};
pub const DOH_COMSS: DohEndpoint = DohEndpoint {
    server: "92.223.109.31",
    server_name: "dns.comss.one",
    path: "/dns-query",
};

impl DohEndpoint {
    /// Map an app DNS provider id (`settings.dns_provider`) to its DoH
    /// endpoint. `system` and `custom:*` have no DoH hostname, so they
    /// fall back to Cloudflare - the engine still answers through DoH,
    /// it just does not use the provider's own servers for that.
    pub fn from_provider_id(id: &str) -> DohEndpoint {
        match id {
            "google" => DOH_GOOGLE,
            "quad9" => DOH_QUAD9,
            "comss" => DOH_COMSS,
            _ => DOH_CLOUDFLARE,
        }
    }

    /// A spare DoH provider, always different from this one.
    pub fn spare(&self) -> DohEndpoint {
        if *self == DOH_CLOUDFLARE {
            DOH_GOOGLE
        } else {
            DOH_CLOUDFLARE
        }
    }
}

/// Inputs to the config generator. Everything the config depends on
/// comes in here explicitly - the generator reads no settings, no
/// registry and no network state itself, so it stays pure and testable.
/// Stage 3 (SingBoxManager) fills this in from the live machine state.
#[derive(Debug, Clone)]
pub struct SingBoxConfigInput {
    /// TUN capture scope, from the two switches (design section 3).
    pub tun_scope: TunScope,
    /// Windows resolver addresses (all up, non-loopback adapters except
    /// the tunnel itself). Captured into the TUN in ProxyOnly so
    /// hijack-dns sees the queries; loopback resolvers must already be
    /// filtered out by the caller (they would loop through our own
    /// process). Ignored in Everything mode - the tunnel takes all.
    pub system_resolvers: Vec<IpAddr>,
    /// Desync preset subnets (ipset CIDRs) kept out of the TUN as a
    /// safety net (`route_exclude_address`). Empty when desync is off.
    /// Each entry must be a valid CIDR (`192.0.2.0/24`); invalid
    /// entries fail the build rather than shipping a broken config.
    pub desync_exclude_cidrs: Vec<String>,
    /// DoH endpoint for the `remote` DNS server, from the app's DNS
    /// provider setting (`DohEndpoint::from_provider_id`).
    pub doh: DohEndpoint,
    /// Whether the machine has IPv6. Gates the TUN v6 address, the
    /// v6 fakeip range and v6 routes (NETZapret: Windows cannot install
    /// a route to a v6 network the TUN adapter does not have).
    pub ipv6: bool,
    /// Absolute path of the engine cache file (`engine-cache.db`).
    /// The manager creates the parent directory before the first start;
    /// the engine crashes if it is missing.
    pub cache_file_path: PathBuf,
    /// Absolute path of the engine log file (`sing-box.log`).
    pub log_file_path: PathBuf,
    /// Whether to include the TUN inbound. Production always sets
    /// this `true`; `false` exists only for the live retry test
    /// (stage 4): the sandbox cannot create a TUN device
    /// (`TUNSETIFF: operation not permitted`), so a config without
    /// TUN is the only one the real engine can start there. The
    /// config stays valid - the route rules never reference the TUN
    /// tag - and `sing-box check` accepts it.
    pub tun_enabled: bool,
    /// Pin the Clash API port instead of allocating a random one.
    /// `None` in production (random per build). The live retry test
    /// (stage 4) pins a port it already occupies, so the first start
    /// attempt fails with a bind error and the retry must reallocate
    /// fresh ports. Port 0 is rejected by validation.
    pub fixed_clash_port: Option<u16>,
}

/// A generated config: the JSON value plus the random ports and secrets
/// the manager and the watchdog need. Secrets live only here and in the
/// rendered config - nothing else keeps a copy (NETZapret's EngineKeys
/// rule: an unauthenticated SOCKS5 on localhost is a known
/// vulnerability class, so the mixed inbound is password-protected and
/// the Clash API secret is random per build).
#[derive(Debug, Clone)]
pub struct GeneratedSingBoxConfig {
    /// The full config as a JSON value (for inspection and tests).
    pub value: Value,
    /// Random Clash API port on 127.0.0.1 (never the fixed 9090: a
    /// foreign sing-box/Clash on the machine would make a live engine
    /// look broken).
    pub clash_api_port: u16,
    /// Random Clash API secret (32 hex chars).
    pub clash_api_secret: String,
    /// Random mixed (health-check) inbound port on 127.0.0.1.
    /// Not read yet: the warmup measures the exit through the Clash
    /// API, not the mixed inbound. Kept on the struct so the
    /// manager (and later health checks) can use it without
    /// re-deriving it from the rendered config.
    #[allow(dead_code)]
    pub mixed_port: u16,
    /// Random mixed inbound password (24 hex chars).
    #[allow(dead_code)]
    pub mixed_password: String,
    /// Echo of the input cache path, for the manager.
    #[allow(dead_code)]
    pub cache_file_path: PathBuf,
}

impl GeneratedSingBoxConfig {
    /// Render as pretty JSON, UTF-8 without BOM (Go chokes on a BOM).
    /// A `serde_json::Value` always serializes, so this cannot fail.
    pub fn render(&self) -> String {
        serde_json::to_string_pretty(&self.value).expect("serde_json::Value is always serializable")
    }
}

/// Errors from config generation and from `sing-box check`.
#[derive(Debug, Error)]
pub enum SingBoxConfigError {
    #[error("invalid config input: {0}")]
    InvalidInput(String),
    #[error("could not allocate a free loopback port: {0}")]
    PortAllocation(#[source] std::io::Error),
    #[error("failed to write the config file {path}: {source}")]
    WriteFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to run sing-box check ({binary}): {source}")]
    CheckSpawn {
        binary: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Outcome of `sing-box check`: the exit status plus captured output.
/// A rejected config is a successful *run* with `success == false`,
/// not an error - the caller decides what to do with the detail.
#[derive(Debug, Clone)]
pub struct SingBoxCheckOutcome {
    pub success: bool,
    /// Captured for debugging; only `success` and `stderr` drive
    /// decisions (a rejected config explains itself on stderr).
    #[allow(dead_code)]
    pub stdout: String,
    pub stderr: String,
}

/// Build the sing-box config for the given input.
///
/// Validates the inputs that come from outside the generator (desync
/// exclusion CIDRs, the DoH server address) and fails closed on bad
/// ones; allocates two distinct random loopback ports and fresh random
/// secrets on every call.
pub fn build_config(
    input: &SingBoxConfigInput,
) -> Result<GeneratedSingBoxConfig, SingBoxConfigError> {
    validate_input(input)?;
    let (clash_api_port, mixed_port) = match input.fixed_clash_port {
        Some(port) => (port, allocate_loopback_port_except(port)?),
        None => allocate_loopback_ports()?,
    };
    let clash_api_secret = random_hex(CLASH_SECRET_HEX_LEN / 2);
    let mixed_password = random_hex(MIXED_PASSWORD_HEX_LEN / 2);

    let value = json!({
        "log": build_log(&input.log_file_path),
        "certificate": { "store": "mozilla" },
        "dns": build_dns(input),
        "inbounds": build_inbounds(input, mixed_port, &mixed_password),
        "outbounds": build_outbounds(),
        "route": build_route(input.tun_scope),
        "experimental": build_experimental(input, clash_api_port, &clash_api_secret),
    });

    Ok(GeneratedSingBoxConfig {
        value,
        clash_api_port,
        clash_api_secret,
        mixed_port,
        mixed_password,
        cache_file_path: input.cache_file_path.clone(),
    })
}

fn validate_input(input: &SingBoxConfigInput) -> Result<(), SingBoxConfigError> {
    // The DoH server must be an IP literal: sing-box dials it directly,
    // and a hostname here would need a bootstrap resolver we do not
    // configure in v1.
    if input.doh.server.parse::<IpAddr>().is_err() {
        return Err(SingBoxConfigError::InvalidInput(format!(
            "DoH server is not an IP address: {}",
            input.doh.server
        )));
    }
    if input.fixed_clash_port == Some(0) {
        return Err(SingBoxConfigError::InvalidInput(
            "fixed Clash API port 0 is not a real port".to_string(),
        ));
    }
    for cidr in &input.desync_exclude_cidrs {
        validate_cidr(cidr)?;
    }
    Ok(())
}

fn validate_cidr(cidr: &str) -> Result<(), SingBoxConfigError> {
    let invalid = || SingBoxConfigError::InvalidInput(format!("invalid CIDR: {}", cidr));
    let (ip, prefix) = cidr.split_once('/').ok_or_else(invalid)?;
    let addr: IpAddr = ip.parse().map_err(|_| invalid())?;
    let prefix: u32 = prefix.parse().map_err(|_| invalid())?;
    let max = if addr.is_ipv4() { 32 } else { 128 };
    if prefix > max {
        return Err(invalid());
    }
    Ok(())
}

/// `log`: level `error` (audit section 7: `info` floods the disk when
/// the endpoint is dead), output to the engine log file. The engine
/// writes the log itself; stdout is still drained by pumps (stage 3)
/// so a full pipe cannot deadlock the child.
fn build_log(log_file_path: &Path) -> Value {
    json!({
        "level": "error",
        "output": path_to_string(log_file_path),
    })
}

/// `dns`: remote DoH + fakeip + the Rule -> Tcp -> Spare ladder.
/// The remote server is addressed by IP (no bootstrap needed to reach
/// it); the spare is a different provider's DoH, reached directly.
fn build_dns(input: &SingBoxConfigInput) -> Value {
    let spare = input.doh.spare();

    let servers = vec![
        doh_server(TAG_DNS_REMOTE, &input.doh),
        fakeip_server(input.ipv6),
        json!({
            "tag": TAG_DNS_TCP,
            "type": "tcp",
            "server": input.doh.server,
        }),
        doh_server(TAG_DNS_SPARE, &spare),
    ];

    let mut rules: Vec<Value> = vec![
        // Firefox canary first: NXDOMAIN tells Firefox not to enable
        // its own DoH, so the engine's DNS wins.
        json!({
            "domain": [FIREFOX_CANARY_DOMAIN],
            "action": "predefined",
            "rcode": "NXDOMAIN",
        }),
    ];

    match input.tun_scope {
        TunScope::ProxyOnly => {
            // Selective fakeip: only proxy domains get fake addresses;
            // everyone else resolves for real and stays out of the TUN.
            rules.push(json!({
                "domain_suffix": PROXY_DOMAIN_SUFFIXES,
                "server": TAG_DNS_FAKE,
            }));
        }
        TunScope::Everything => {
            // Full capture: every A/AAAA answer is a fakeip.
            rules.push(json!({
                "query_type": ["A", "AAAA"],
                "server": TAG_DNS_FAKE,
            }));
        }
    }

    // The ladder, last before `final`: whichever Clash mode the
    // supervisor sets, the matching rule reroutes the queries that
    // would otherwise go to `remote`. The modes are engine-wide, so
    // the TCP and spare steps cannot fight each other.
    rules.push(json!({
        "clash_mode": DNS_TCP_MODE,
        "server": TAG_DNS_TCP,
    }));
    rules.push(json!({
        "clash_mode": DNS_SPARE_MODE,
        "server": TAG_DNS_SPARE,
    }));

    json!({
        "servers": servers,
        "rules": rules,
        "final": TAG_DNS_REMOTE,
    })
}

/// A DoH server entry. `path` is only written when it differs from the
/// sing-box default `/dns-query` (NETZapret behaviour).
fn doh_server(tag: &str, endpoint: &DohEndpoint) -> Value {
    let mut server = json!({
        "tag": tag,
        "type": "https",
        "server": endpoint.server,
        "tls": { "server_name": endpoint.server_name },
    });
    if endpoint.path != "/dns-query" {
        server["path"] = json!(endpoint.path);
    }
    server
}

/// The fakeip server. The v6 range only when the machine has IPv6.
fn fakeip_server(ipv6: bool) -> Value {
    let mut fake = json!({
        "tag": TAG_DNS_FAKE,
        "type": "fakeip",
        "inet4_range": FAKEIP_V4_RANGE,
    });
    if ipv6 {
        fake["inet6_range"] = json!(FAKEIP_V6_RANGE);
    }
    fake
}

/// `inbounds`: the TUN inbound (unless `tun_enabled` is false - the
/// stage 4 live-test seam) plus the mixed health-check inbound.
fn build_inbounds(input: &SingBoxConfigInput, mixed_port: u16, mixed_password: &str) -> Value {
    let mut inbounds: Vec<Value> = Vec::new();
    if input.tun_enabled {
        inbounds.push(build_tun_inbound(input));
    }
    inbounds.push(build_mixed_inbound(mixed_port, mixed_password));
    json!(inbounds)
}

/// The TUN inbound. `strict_route` stays off: on Windows it installs
/// WFP filters that block UDP/53 for everyone else and kill all IPv6
/// when the TUN has no v6 address (NETZapret, BuildTunInbound).
fn build_tun_inbound(input: &SingBoxConfigInput) -> Value {
    let mut address = vec![TUN_ADDRESS_V4.to_string()];
    if input.ipv6 {
        address.push(TUN_ADDRESS_V6.to_string());
    }

    let mut tun = json!({
        "type": "tun",
        "tag": TAG_TUN,
        "interface_name": TUN_INTERFACE_NAME,
        "address": address,
        "mtu": TUN_MTU,
        "auto_route": true,
        "strict_route": false,
        "stack": "gvisor",
        // The TUN adapter gets an IPv4 DNS address only: without it
        // sing-tun also assigns an IPv6 DNS address, and Java takes
        // DNS from all adapters, IPv6 first, with our TUN first in
        // line (NETZapret, BuildTunInbound).
        "dns_address": [tun_dns_address()],
    });

    if input.tun_scope == TunScope::ProxyOnly {
        tun["route_address"] = json!(proxy_only_route_addresses(input));
    }

    if !input.desync_exclude_cidrs.is_empty() {
        // Desync subnets as a safety net: a proxy rule must never
        // swallow an address the desync preset owns. Written in both
        // scopes; the manager passes an empty list when desync is off.
        let mut excluded: Vec<String> = input
            .desync_exclude_cidrs
            .iter()
            .filter(|c| input.ipv6 || !c.contains(':'))
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        excluded.sort();
        // The v6 filter above can empty the list (ipv6=false with only
        // v6 CIDRs): do not write an empty array, the engine accepts
        // it but the key is noise.
        if !excluded.is_empty() {
            tun["route_exclude_address"] = json!(excluded);
        }
    }

    tun
}

/// ProxyOnly capture set: the fakeip ranges, the Windows resolver
/// addresses (so hijack-dns sees the queries) - and nothing else.
/// Deduplicated and sorted for a stable config. v6 entries are dropped
/// when the machine has no IPv6 (see the module docs).
fn proxy_only_route_addresses(input: &SingBoxConfigInput) -> Vec<String> {
    let mut captured: BTreeSet<String> = BTreeSet::new();
    captured.insert(FAKEIP_V4_RANGE.to_string());
    if input.ipv6 {
        captured.insert(FAKEIP_V6_RANGE.to_string());
    }
    for resolver in &input.system_resolvers {
        if !input.ipv6 && resolver.is_ipv6() {
            continue;
        }
        captured.insert(prefix_host(*resolver));
    }
    captured.into_iter().collect()
}

/// A bare IP as a host CIDR: `/32` for IPv4, `/128` for IPv6
/// (NETZapret's `Prefix` helper).
fn prefix_host(addr: IpAddr) -> String {
    match addr {
        IpAddr::V4(v4) => format!("{}/32", v4),
        IpAddr::V6(v6) => format!("{}/128", v6),
    }
}

/// The TUN adapter's DNS address: the TUN v4 address plus one
/// (NETZapret's `TunDnsAddress`): 172.19.0.1/30 -> 172.19.0.2.
fn tun_dns_address() -> String {
    let base: std::net::Ipv4Addr = TUN_ADDRESS_V4
        .split_once('/')
        .map(|(ip, _)| ip)
        .unwrap_or(TUN_ADDRESS_V4)
        .parse()
        .expect("TUN_ADDRESS_V4 is a valid IPv4 CIDR");
    std::net::Ipv4Addr::from(u32::from(base).wrapping_add(1)).to_string()
}

/// The mixed inbound: SOCKS5+HTTP on a random loopback port, password
/// protected. Only the deep health check (stage 6) talks to it - a
/// real request through this inbound proves traffic actually flows.
/// Unauthenticated SOCKS5 on localhost is a known vulnerability class,
/// hence the random password (NETZapret's EngineKeys rule).
fn build_mixed_inbound(port: u16, password: &str) -> Value {
    json!({
        "type": "mixed",
        "tag": TAG_HEALTH,
        "listen": "127.0.0.1",
        "listen_port": port,
        "users": [{ "username": HEALTH_USERNAME, "password": password }],
    })
}

/// `outbounds`: direct, block, and the MASQUE exit.
///
/// The MASQUE outbound is exactly `type` + `tag` + `use_http2` +
/// `reconnect_delay`. The fork rejects the whole config (FATAL
/// `json: unknown field`) if any of `server`, `server_port`,
/// `license`, `fake_packets` or `tls.utls` appears (audit section 4.5,
/// verified live with `sing-box check`).
///
/// No `dns` outbound: the design lists `dns-out` as standard, but the
/// type was removed in sing-box 1.13.0 and the fork rejects the config
/// for it (`dns outbound is deprecated in sing-box 1.11.0 and removed
/// in sing-box 1.13.0, use rule actions instead` - caught by the live
/// `sing-box check` in the test suite). DNS is handled by the
/// hijack-dns rule and the `dns` section, not by an outbound.
fn build_outbounds() -> Value {
    json!([
        { "type": "direct", "tag": TAG_DIRECT },
        { "type": "block", "tag": TAG_BLOCK },
        {
            "type": "masque",
            "tag": MASQUE_TAG,
            "use_http2": true,
            // Audit section 7: without this, a dead endpoint reconnects
            // in a hot loop (~1700 attempts/s). The field is part of
            // the fork's masque options (option/masque.go).
            "reconnect_delay": MASQUE_RECONNECT_DELAY,
        },
    ])
}

/// `route`: sniff, hijack-dns, health-in to WARP, private IPs direct,
/// proxy domains to WARP (ProxyOnly), final direct (ProxyOnly) or WARP
/// (Everything), `default_domain_resolver` (required since 1.12).
fn build_route(scope: TunScope) -> Value {
    let mut rules: Vec<Value> = vec![
        // Protocol sniffing first, so domain rules also match
        // connections that never touched DNS.
        json!({ "action": "sniff" }),
        // DNS queries captured by the TUN are answered by the engine's
        // DNS module (DoH), not forwarded to the system resolvers.
        json!({ "protocol": "dns", "action": "hijack-dns" }),
        // The health check must go through the tunnel: without this
        // rule its request follows the general rules and comes back
        // over direct in ProxyOnly - measuring the home network, not
        // the tunnel (NETZapret, 28.09).
        json!({ "inbound": [TAG_HEALTH], "outbound": MASQUE_TAG }),
        // Local networks never enter the tunnel.
        json!({ "ip_is_private": true, "outbound": TAG_DIRECT }),
    ];

    let final_outbound = match scope {
        TunScope::ProxyOnly => {
            rules.push(json!({
                "domain_suffix": PROXY_DOMAIN_SUFFIXES,
                "outbound": MASQUE_TAG,
            }));
            TAG_DIRECT
        }
        TunScope::Everything => MASQUE_TAG,
    };

    json!({
        "rules": rules,
        "final": final_outbound,
        "auto_detect_interface": true,
        "default_domain_resolver": {
            "server": TAG_DNS_REMOTE,
            // IPv4 first, IPv6 as fallback: on machines without IPv6
            // the engine otherwise wastes an attempt on unreachable
            // AAAA addresses (NETZapret, 25.09).
            "strategy": "prefer_ipv4",
        },
    })
}

/// `experimental`: Clash API on a random loopback port with a random
/// secret, and the cache file with `store_masque_config` (without it
/// the engine re-registers a new Cloudflare device on every start).
/// `store_fakeip` is deliberately not written: fakeip is re-issued
/// fresh on every start in v1, less state, fewer stale mappings.
fn build_experimental(
    input: &SingBoxConfigInput,
    clash_api_port: u16,
    clash_api_secret: &str,
) -> Value {
    json!({
        "clash_api": {
            "external_controller": format!("127.0.0.1:{}", clash_api_port),
            "secret": clash_api_secret,
        },
        "cache_file": {
            "enabled": true,
            "path": path_to_string(&input.cache_file_path),
            "store_masque_config": true,
        },
    })
}

/// Render a path as a JSON string. Paths in the config are absolute;
/// serde_json escapes the Windows backslashes.
fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Allocate two distinct free loopback ports by binding to port 0.
/// Both listeners are held open while the ports are read, so the OS
/// cannot hand the same port twice; they are dropped immediately
/// after. A TOCTOU remains (another process could grab a port before
/// sing-box binds it), but the window is milliseconds and a bind
/// failure at stage 3 surfaces as a failed start with a clear reason.
fn allocate_loopback_ports() -> Result<(u16, u16), SingBoxConfigError> {
    for _ in 0..8 {
        let first = TcpListener::bind("127.0.0.1:0").map_err(SingBoxConfigError::PortAllocation)?;
        let second =
            TcpListener::bind("127.0.0.1:0").map_err(SingBoxConfigError::PortAllocation)?;
        let first_port = first
            .local_addr()
            .map_err(SingBoxConfigError::PortAllocation)?
            .port();
        let second_port = second
            .local_addr()
            .map_err(SingBoxConfigError::PortAllocation)?
            .port();
        drop(first);
        drop(second);
        if first_port != 0 && second_port != 0 && first_port != second_port {
            return Ok((first_port, second_port));
        }
    }
    Err(SingBoxConfigError::InvalidInput(
        "could not allocate two distinct loopback ports".to_string(),
    ))
}

/// Allocate one free loopback port distinct from `except` (the pinned
/// Clash API port, stage 4 test seam). Same bind-to-:0 trick as
/// [`allocate_loopback_ports`]; the mixed port must never collide
/// with the pinned Clash port.
fn allocate_loopback_port_except(except: u16) -> Result<u16, SingBoxConfigError> {
    for _ in 0..8 {
        let listener =
            TcpListener::bind("127.0.0.1:0").map_err(SingBoxConfigError::PortAllocation)?;
        let port = listener
            .local_addr()
            .map_err(SingBoxConfigError::PortAllocation)?
            .port();
        drop(listener);
        if port != 0 && port != except {
            return Ok(port);
        }
    }
    Err(SingBoxConfigError::InvalidInput(
        "could not allocate a loopback port distinct from the pinned Clash port".to_string(),
    ))
}

/// `n` random bytes from the OS RNG, hex-encoded (2n chars).
/// Same pattern as the Telegram bridge secret (tgproxy.rs).
fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    OsRng.fill_bytes(&mut buf);
    hex::encode(buf)
}

/// Write the rendered config to `path`, creating the parent directory.
/// UTF-8 without BOM. The manager (stage 3) calls this before
/// `check_config` and before spawning the engine.
pub fn write_config_file(
    config: &GeneratedSingBoxConfig,
    path: &Path,
) -> Result<(), SingBoxConfigError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|source| SingBoxConfigError::WriteFailed {
                path: path.to_path_buf(),
                source,
            })?;
        }
    }
    std::fs::write(path, config.render()).map_err(|source| SingBoxConfigError::WriteFailed {
        path: path.to_path_buf(),
        source,
    })
}

/// Validate a config file with `sing-box check -c <path>`.
/// Synchronous: `check` only parses the config, it opens no listeners
/// and dials nothing, so it returns in well under a second on a sane
/// binary. Returns an error only when the binary cannot be spawned;
/// a config the engine rejects comes back as
/// `SingBoxCheckOutcome { success: false, .. }` with the engine's own
/// diagnostic in `stderr`.
pub fn check_config(
    binary: &Path,
    config_path: &Path,
) -> Result<SingBoxCheckOutcome, SingBoxConfigError> {
    let mut command = std::process::Command::new(binary);
    command.args(["check", "-c"]).arg(config_path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Same as every other child process in this codebase: no
        // console window, the svc has no console to show it in.
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command
        .output()
        .map_err(|source| SingBoxConfigError::CheckSpawn {
            binary: binary.to_path_buf(),
            source,
        })?;
    Ok(SingBoxCheckOutcome {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    fn test_input(scope: TunScope) -> SingBoxConfigInput {
        SingBoxConfigInput {
            tun_scope: scope,
            system_resolvers: vec!["192.168.1.1".parse().unwrap(), "8.8.8.8".parse().unwrap()],
            desync_exclude_cidrs: vec!["203.0.113.0/24".to_string()],
            doh: DOH_CLOUDFLARE,
            ipv6: false,
            cache_file_path: PathBuf::from("/tmp/zapret2-test/singbox/engine-cache.db"),
            log_file_path: PathBuf::from("/tmp/zapret2-test/singbox/sing-box.log"),
            tun_enabled: true,
            fixed_clash_port: None,
        }
    }

    fn masque_outbound(value: &Value) -> &Value {
        value["outbounds"]
            .as_array()
            .expect("outbounds is an array")
            .iter()
            .find(|o| o["type"] == "masque")
            .expect("a masque outbound exists")
    }

    fn tun_inbound(value: &Value) -> &Value {
        value["inbounds"]
            .as_array()
            .expect("inbounds is an array")
            .iter()
            .find(|i| i["type"] == "tun")
            .expect("a tun inbound exists")
    }

    #[test]
    fn masque_outbound_has_exact_shape() {
        let config = build_config(&test_input(TunScope::ProxyOnly)).unwrap();
        let masque = masque_outbound(&config.value);
        let obj = masque.as_object().expect("masque outbound is an object");
        // Exactly these four keys: type, tag, use_http2, reconnect_delay.
        // Anything else is either a forbidden field (the fork rejects
        // the whole config) or an unreviewed addition.
        assert_eq!(
            obj.len(),
            4,
            "masque outbound keys: {:?}",
            obj.keys().collect::<Vec<_>>()
        );
        assert_eq!(masque["type"], "masque");
        assert_eq!(masque["tag"], MASQUE_TAG);
        assert_eq!(masque["use_http2"], true);
    }

    #[test]
    fn masque_outbound_has_no_forbidden_fields() {
        // Audit section 4.5, verified live with `sing-box check`: these
        // fields make the fork fail the whole config with FATAL
        // `json: unknown field`.
        for scope in [TunScope::ProxyOnly, TunScope::Everything] {
            let config = build_config(&test_input(scope)).unwrap();
            let masque = masque_outbound(&config.value);
            for forbidden in ["server", "server_port", "license", "fake_packets", "tls"] {
                assert!(
                    masque.get(forbidden).is_none(),
                    "forbidden field {:?} present in masque outbound ({:?} scope)",
                    forbidden,
                    scope
                );
            }
        }
    }

    #[test]
    fn masque_outbound_always_has_reconnect_delay() {
        // Audit section 7: without reconnect_delay a dead endpoint
        // reconnects in a hot loop (~1700 attempts/s, 67% of one core).
        for scope in [TunScope::ProxyOnly, TunScope::Everything] {
            let config = build_config(&test_input(scope)).unwrap();
            let masque = masque_outbound(&config.value);
            assert_eq!(
                masque["reconnect_delay"], MASQUE_RECONNECT_DELAY,
                "reconnect_delay missing or wrong ({:?} scope)",
                scope
            );
        }
    }

    #[test]
    fn proxy_only_tun_captures_fakeip_and_resolvers() {
        let config = build_config(&test_input(TunScope::ProxyOnly)).unwrap();
        let tun = tun_inbound(&config.value);
        let route_address = tun["route_address"]
            .as_array()
            .expect("ProxyOnly TUN has route_address")
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        // Fakeip range, both resolvers as /32, nothing else.
        assert_eq!(
            route_address,
            vec!["192.168.1.1/32", "198.18.0.0/15", "8.8.8.8/32"],
            "route_address must be exactly the fakeip range + resolvers"
        );
        // No v6 range without IPv6.
        assert!(!route_address.iter().any(|c| c.contains(':')));
        // Desync subnets stay out of the TUN.
        assert_eq!(tun["route_exclude_address"], json!(["203.0.113.0/24"]));
        // TUN shape.
        assert_eq!(tun["interface_name"], TUN_INTERFACE_NAME);
        assert_eq!(tun["auto_route"], true);
        assert_eq!(tun["strict_route"], false);
        assert_eq!(tun["stack"], "gvisor");
        assert_eq!(tun["mtu"], TUN_MTU);
        assert_eq!(tun["address"], json!([TUN_ADDRESS_V4]));
        assert_eq!(tun["dns_address"], json!(["172.19.0.2"]));
    }

    #[test]
    fn proxy_only_with_ipv6_includes_v6_ranges() {
        let mut input = test_input(TunScope::ProxyOnly);
        input.ipv6 = true;
        input
            .system_resolvers
            .push("2001:4860:4860::8888".parse().unwrap());
        let config = build_config(&input).unwrap();
        let tun = tun_inbound(&config.value);
        let route_address = tun["route_address"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert!(route_address.contains(&FAKEIP_V6_RANGE.to_string()));
        assert!(route_address.contains(&"2001:4860:4860::8888/128".to_string()));
        assert_eq!(tun["address"], json!([TUN_ADDRESS_V4, TUN_ADDRESS_V6]));
        // The fakeip server gets the v6 range too.
        let fake = &config.value["dns"]["servers"][1];
        assert_eq!(fake["inet6_range"], FAKEIP_V6_RANGE);
    }

    #[test]
    fn everything_tun_has_no_route_address_and_final_is_warp() {
        let config = build_config(&test_input(TunScope::Everything)).unwrap();
        let tun = tun_inbound(&config.value);
        assert!(
            tun.get("route_address").is_none(),
            "Everything TUN must not restrict route_address"
        );
        assert_eq!(config.value["route"]["final"], MASQUE_TAG);
        // No proxy-domain rule needed: the final sends everything.
        let rules = config.value["route"]["rules"].as_array().unwrap();
        assert!(
            !rules.iter().any(|r| r.get("domain_suffix").is_some()),
            "Everything mode needs no proxy-domain route rule"
        );
        // ...but all DNS answers are fakeip.
        let dns_rules = config.value["dns"]["rules"].as_array().unwrap();
        assert!(dns_rules
            .iter()
            .any(|r| r["server"] == TAG_DNS_FAKE && r["query_type"] == json!(["A", "AAAA"])));
    }

    #[test]
    fn proxy_only_routes_v1_domains_to_warp() {
        let config = build_config(&test_input(TunScope::ProxyOnly)).unwrap();
        let rules = config.value["route"]["rules"].as_array().unwrap();
        let proxy_rule = rules
            .iter()
            .find(|r| r.get("domain_suffix").is_some())
            .expect("a proxy-domain route rule exists");
        assert_eq!(proxy_rule["domain_suffix"], json!(PROXY_DOMAIN_SUFFIXES));
        assert_eq!(proxy_rule["outbound"], MASQUE_TAG);
        assert_eq!(config.value["route"]["final"], TAG_DIRECT);
        // And only those domains get fakeip answers.
        let dns_rules = config.value["dns"]["rules"].as_array().unwrap();
        let fake_rule = dns_rules
            .iter()
            .find(|r| r["server"] == TAG_DNS_FAKE)
            .expect("a fakeip DNS rule exists");
        assert_eq!(fake_rule["domain_suffix"], json!(PROXY_DOMAIN_SUFFIXES));
    }

    #[test]
    fn route_has_sniff_hijack_health_and_private_rules() {
        for scope in [TunScope::ProxyOnly, TunScope::Everything] {
            let config = build_config(&test_input(scope)).unwrap();
            let rules = config.value["route"]["rules"].as_array().unwrap();
            assert_eq!(rules[0]["action"], "sniff", "sniff first ({:?})", scope);
            assert!(
                rules
                    .iter()
                    .any(|r| r["protocol"] == "dns" && r["action"] == "hijack-dns"),
                "hijack-dns rule ({:?})",
                scope
            );
            // The health check must go through the tunnel, or it would
            // measure the home network instead (NETZapret, 28.09).
            assert!(
                rules
                    .iter()
                    .any(|r| r["inbound"] == json!([TAG_HEALTH]) && r["outbound"] == MASQUE_TAG),
                "health-in -> WARP rule ({:?})",
                scope
            );
            assert!(
                rules
                    .iter()
                    .any(|r| r["ip_is_private"] == true && r["outbound"] == TAG_DIRECT),
                "private -> direct rule ({:?})",
                scope
            );
            assert_eq!(config.value["route"]["auto_detect_interface"], true);
            assert_eq!(
                config.value["route"]["default_domain_resolver"]["server"],
                TAG_DNS_REMOTE
            );
            assert_eq!(
                config.value["route"]["default_domain_resolver"]["strategy"],
                "prefer_ipv4"
            );
        }
    }

    #[test]
    fn dns_servers_and_ladder() {
        let config = build_config(&test_input(TunScope::ProxyOnly)).unwrap();
        let servers = config.value["dns"]["servers"].as_array().unwrap();
        let by_tag = |tag: &str| {
            servers
                .iter()
                .find(|s| s["tag"] == tag)
                .unwrap_or_else(|| panic!("DNS server {:?} missing", tag))
        };
        let remote = by_tag(TAG_DNS_REMOTE);
        assert_eq!(remote["type"], "https");
        assert_eq!(remote["server"], DOH_CLOUDFLARE.server);
        assert_eq!(remote["tls"]["server_name"], DOH_CLOUDFLARE.server_name);
        // Default path is not written (NETZapret behaviour).
        assert!(remote.get("path").is_none());

        let fake = by_tag(TAG_DNS_FAKE);
        assert_eq!(fake["type"], "fakeip");
        assert_eq!(fake["inet4_range"], FAKEIP_V4_RANGE);
        assert!(fake.get("inet6_range").is_none(), "no v6 without ipv6");

        let tcp = by_tag(TAG_DNS_TCP);
        assert_eq!(tcp["type"], "tcp");
        assert_eq!(tcp["server"], DOH_CLOUDFLARE.server);

        let spare = by_tag(TAG_DNS_SPARE);
        assert_eq!(spare["type"], "https");
        assert_eq!(spare["server"], DOH_GOOGLE.server);
        assert_ne!(
            spare["server"], remote["server"],
            "spare must differ from remote"
        );

        // Rules: canary first, then the ladder last, final remote.
        let rules = config.value["dns"]["rules"].as_array().unwrap();
        assert_eq!(rules[0]["domain"], json!([FIREFOX_CANARY_DOMAIN]));
        assert_eq!(rules[0]["action"], "predefined");
        assert_eq!(rules[0]["rcode"], "NXDOMAIN");
        let tail = &rules[rules.len() - 2..];
        assert_eq!(tail[0]["clash_mode"], DNS_TCP_MODE);
        assert_eq!(tail[0]["server"], TAG_DNS_TCP);
        assert_eq!(tail[1]["clash_mode"], DNS_SPARE_MODE);
        assert_eq!(tail[1]["server"], TAG_DNS_SPARE);
        assert_eq!(config.value["dns"]["final"], TAG_DNS_REMOTE);
    }

    #[test]
    fn clash_api_and_mixed_ports_and_secrets_are_fresh() {
        let a = build_config(&test_input(TunScope::ProxyOnly)).unwrap();
        let b = build_config(&test_input(TunScope::ProxyOnly)).unwrap();

        for config in [&a, &b] {
            // Ports are real, distinct within one build, never the
            // fixed 9090.
            assert_ne!(config.clash_api_port, 0);
            assert_ne!(config.mixed_port, 0);
            assert_ne!(config.clash_api_port, config.mixed_port);
            assert_ne!(config.clash_api_port, 9090);
            // Secrets: exact length, hex only.
            assert_eq!(config.clash_api_secret.len(), CLASH_SECRET_HEX_LEN);
            assert!(config
                .clash_api_secret
                .chars()
                .all(|c| c.is_ascii_hexdigit()));
            assert_eq!(config.mixed_password.len(), MIXED_PASSWORD_HEX_LEN);
            assert!(config.mixed_password.chars().all(|c| c.is_ascii_hexdigit()));
            // The config carries the same values (single source of truth).
            assert_eq!(
                config.value["experimental"]["clash_api"]["external_controller"],
                format!("127.0.0.1:{}", config.clash_api_port)
            );
            assert_eq!(
                config.value["experimental"]["clash_api"]["secret"],
                config.clash_api_secret
            );
            let mixed = config.value["inbounds"]
                .as_array()
                .unwrap()
                .iter()
                .find(|i| i["type"] == "mixed")
                .unwrap();
            assert_eq!(mixed["listen_port"], config.mixed_port);
            assert_eq!(mixed["users"][0]["password"], config.mixed_password);
        }
        // Secrets differ between builds (2^128 space; a collision would
        // mean the RNG is broken, not bad luck).
        assert_ne!(a.clash_api_secret, b.clash_api_secret);
        assert_ne!(a.mixed_password, b.mixed_password);
    }

    #[test]
    fn log_is_error_and_cache_file_stores_masque_config() {
        let config = build_config(&test_input(TunScope::ProxyOnly)).unwrap();
        assert_eq!(config.value["log"]["level"], "error");
        assert_eq!(
            config.value["log"]["output"],
            "/tmp/zapret2-test/singbox/sing-box.log"
        );
        let cache = &config.value["experimental"]["cache_file"];
        assert_eq!(cache["enabled"], true);
        assert_eq!(cache["path"], "/tmp/zapret2-test/singbox/engine-cache.db");
        assert_eq!(cache["store_masque_config"], true);
        assert!(cache.get("store_fakeip").is_none(), "v1: no store_fakeip");
        // Mozilla roots: avoids the Go/Windows system-cert crash path
        // (NETZapret, 0xc0000005 in crypto/x509 systemVerify).
        assert_eq!(config.value["certificate"]["store"], "mozilla");
    }

    #[test]
    fn doh_provider_mapping() {
        assert_eq!(DohEndpoint::from_provider_id("cloudflare"), DOH_CLOUDFLARE);
        assert_eq!(DohEndpoint::from_provider_id("google"), DOH_GOOGLE);
        assert_eq!(DohEndpoint::from_provider_id("quad9"), DOH_QUAD9);
        assert_eq!(DohEndpoint::from_provider_id("comss"), DOH_COMSS);
        // system / custom / unknown fall back to Cloudflare DoH.
        assert_eq!(DohEndpoint::from_provider_id("system"), DOH_CLOUDFLARE);
        assert_eq!(DohEndpoint::from_provider_id("custom:123"), DOH_CLOUDFLARE);
        assert_eq!(DohEndpoint::from_provider_id(""), DOH_CLOUDFLARE);
        // The spare is always a different provider.
        for endpoint in [DOH_CLOUDFLARE, DOH_GOOGLE, DOH_QUAD9, DOH_COMSS] {
            assert_ne!(endpoint.spare(), endpoint);
        }
        assert_eq!(DOH_CLOUDFLARE.spare(), DOH_GOOGLE);
    }

    #[test]
    fn render_roundtrip_is_stable_json() {
        let config = build_config(&test_input(TunScope::ProxyOnly)).unwrap();
        let rendered = config.render();
        // No BOM: Go chokes on it.
        assert!(!rendered.starts_with('\u{feff}'));
        let parsed: Value = serde_json::from_str(&rendered).expect("rendered config parses");
        assert_eq!(parsed, config.value, "render -> parse roundtrip");
        // All top-level sections present.
        for section in [
            "log",
            "certificate",
            "dns",
            "inbounds",
            "outbounds",
            "route",
            "experimental",
        ] {
            assert!(
                parsed.get(section).is_some(),
                "section {:?} missing",
                section
            );
        }
    }

    #[test]
    fn invalid_inputs_fail_closed() {
        let mut input = test_input(TunScope::ProxyOnly);
        input.desync_exclude_cidrs = vec!["not-a-cidr".to_string()];
        assert!(matches!(
            build_config(&input),
            Err(SingBoxConfigError::InvalidInput(_))
        ));

        let mut input = test_input(TunScope::ProxyOnly);
        input.desync_exclude_cidrs = vec!["192.0.2.0/33".to_string()];
        assert!(matches!(
            build_config(&input),
            Err(SingBoxConfigError::InvalidInput(_))
        ));

        let mut input = test_input(TunScope::ProxyOnly);
        input.doh = DohEndpoint {
            server: "dns.example.com",
            server_name: "dns.example.com",
            path: "/dns-query",
        };
        assert!(matches!(
            build_config(&input),
            Err(SingBoxConfigError::InvalidInput(_))
        ));

        // A valid CIDR (including IPv6) builds fine.
        let mut input = test_input(TunScope::ProxyOnly);
        input.ipv6 = true;
        input.desync_exclude_cidrs = vec!["2001:db8::/32".to_string()];
        assert!(build_config(&input).is_ok());
    }

    #[test]
    fn tun_dns_address_is_the_next_ip() {
        assert_eq!(tun_dns_address(), "172.19.0.2");
    }

    #[test]
    fn check_config_reports_missing_binary() {
        let missing = PathBuf::from("/nonexistent/sing-box");
        let config_path = PathBuf::from("/nonexistent/config.json");
        assert!(matches!(
            check_config(&missing, &config_path),
            Err(SingBoxConfigError::CheckSpawn { .. })
        ));
    }

    #[test]
    fn write_config_file_creates_parent_dirs() {
        let dir = std::env::temp_dir().join(format!(
            "zapret2-singbox-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = dir.join("nested").join("config.json");
        let config = build_config(&test_input(TunScope::ProxyOnly)).unwrap();
        write_config_file(&config, &path).unwrap();
        let read_back = std::fs::read_to_string(&path).unwrap();
        assert_eq!(read_back, config.render());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn check_config_maps_exit_status() {
        // /bin/true and /bin/false ignore their arguments, so they work
        // as stand-ins for a binary that accepts / rejects a config.
        let ok = check_config(Path::new("/bin/true"), Path::new("config.json")).unwrap();
        assert!(ok.success);
        let bad = check_config(Path::new("/bin/false"), Path::new("config.json")).unwrap();
        assert!(!bad.success);
    }

    /// Live validation against the real fork binary. Runs only when
    /// `SINGBOX_TEST_BINARY` points at a sing-box executable (the
    /// sandbox Linux amd64 build of 2.7.3); skipped silently otherwise,
    /// so CI without the binary stays green. Run it as:
    /// `SINGBOX_TEST_BINARY=/tmp/sb-test/sing-box-1.14.3-extended-2.7.3-linux-amd64/sing-box cargo test singbox_config`.
    #[test]
    fn check_generated_configs_with_real_binary() {
        let binary = match std::env::var("SINGBOX_TEST_BINARY") {
            Ok(path) if !path.is_empty() => PathBuf::from(path),
            _ => {
                eprintln!("SINGBOX_TEST_BINARY not set, skipping live sing-box check");
                return;
            }
        };
        let dir = std::env::temp_dir().join(format!(
            "zapret2-singbox-check-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for scope in [TunScope::ProxyOnly, TunScope::Everything] {
            let mut input = test_input(scope);
            input.ipv6 = true;
            let config = build_config(&input).unwrap();
            let path = dir.join(format!("config-{:?}.json", scope));
            write_config_file(&config, &path).unwrap();
            let outcome = check_config(&binary, &path).unwrap();
            assert!(
                outcome.success,
                "sing-box check rejected the {:?} config: stdout={} stderr={}",
                scope, outcome.stdout, outcome.stderr
            );
        }
        // The no-TUN config (stage 4 live-test seam) must also pass
        // `check`: the live retry test starts the real engine with it.
        let mut input = test_input(TunScope::ProxyOnly);
        input.ipv6 = true;
        input.tun_enabled = false;
        let config = build_config(&input).unwrap();
        let path = dir.join("config-no-tun.json");
        write_config_file(&config, &path).unwrap();
        let outcome = check_config(&binary, &path).unwrap();
        assert!(
            outcome.success,
            "sing-box check rejected the no-TUN config: stdout={} stderr={}",
            outcome.stdout, outcome.stderr
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prefix_host_uses_slash_32_and_128() {
        let v4: IpAddr = "192.0.2.1".parse().unwrap();
        let v6: IpAddr = "2001:db8::1".parse().unwrap();
        assert_eq!(prefix_host(v4), "192.0.2.1/32");
        assert_eq!(prefix_host(v6), "2001:db8::1/128");
    }

    #[test]
    fn tun_disabled_config_has_no_tun_inbound() {
        let mut input = test_input(TunScope::ProxyOnly);
        input.tun_enabled = false;
        let config = build_config(&input).unwrap();
        let inbounds = config.value["inbounds"]
            .as_array()
            .expect("inbounds is an array");
        assert_eq!(inbounds.len(), 1, "only the mixed inbound remains");
        assert_eq!(inbounds[0]["type"], "mixed");
        // The route rules never reference the TUN tag, so nothing else
        // changes: the health rule and the domain rule still stand.
        let rules = config.value["route"]["rules"]
            .as_array()
            .expect("route rules is an array");
        assert!(rules.iter().any(|r| r["inbound"] == json!([TAG_HEALTH])));
    }

    #[test]
    fn fixed_clash_port_is_used_and_mixed_differs() {
        let mut input = test_input(TunScope::ProxyOnly);
        input.fixed_clash_port = Some(45678);
        let config = build_config(&input).unwrap();
        assert_eq!(config.clash_api_port, 45678);
        assert_ne!(config.mixed_port, 45678);
        assert_ne!(config.mixed_port, 0);
        assert_eq!(
            config.value["experimental"]["clash_api"]["external_controller"],
            json!("127.0.0.1:45678")
        );
    }

    #[test]
    fn fixed_clash_port_zero_is_rejected() {
        let mut input = test_input(TunScope::ProxyOnly);
        input.fixed_clash_port = Some(0);
        let err = build_config(&input).unwrap_err();
        assert!(matches!(err, SingBoxConfigError::InvalidInput(_)));
    }
}

//! Tunnel capture exclusions for winws2 (WARP mode, stage 4).
//!
//! # Why this exists
//!
//! The desync preset captures all outbound TCP on 443-65535, and the
//! tunnel's own connection to its MASQUE server would go through
//! winws2 packet by packet. NETZapret measured it on 30.09: with
//! desync the tunnel ran ten times slower (14.6-15.4 Mbit/s against
//! 149.3 without), while single-threaded winws2 sat at 88-98% of one
//! core. The fix is to take the tunnel out of the WinDivert capture
//! itself: a desync profile (`pass`, `--ipset-exclude`) cannot help,
//! because by the time the packet reaches a profile it has already
//! been pulled off the network and handed to winws2.
//!
//! # How the exclusion works (verified, not assumed)
//!
//! * winws2's `--wf-raw-filter` is AND-combined with the whole
//!   capture expression (`nfq2/nfqws.c`, `wf_make_filter`, our pinned
//!   base 1.0.7: `"\nand\n(\n%s\n)"`). One flag per launch.
//! * Negation is only expressible as `(... ? false : true)`:
//!   `!(...)` and `not (...)` do not parse at all, and
//!   `ip.DstAddr!=...` is false on an IPv6 packet, where the field
//!   does not exist - it would drop all of IPv6 from the capture
//!   (NETZapret, verified 30.09 against WinDivertHelperCompileFilter
//!   and WinDivertHelperEvalFilter).
//! * Each address costs two tests (destination + source), and the
//!   WinDivert filter has a length budget: NETZapret measured 40
//!   addresses as the safe ceiling (their widest preset takes 48-52
//!   with everything included). Our v1 set is far below it.
//! * `api.cloudflareclient.com` is deliberately NOT excluded: WARP
//!   registration goes THROUGH desync (NETZapret, 19.09: with a
//!   desync section the handshake passes, without it TLS handshake
//!   timeout). The registration recipe is a separate winws2 section,
//!   see [`warp_recipe_args`].
//!
//! # winws v1 (Flowseal) cannot be excluded
//!
//! Verified against the pinned Flowseal 1.10.4 `winws.exe` (strings)
//! and the zapret v1 source (`nfq/nfqws.c`, `wf_make_filter`):
//! v1 has no `--wf-raw-filter`; its `--wf-raw-part` is OR-combined
//! with the port capture, so it can only ADD capture, never remove
//! it. `--wf-raw` replaces the whole filter, which would mean
//! reimplementing the port filter in raw WinDivert syntax - out of
//! scope for v1. So with a Flowseal strategy the tunnel stays in the
//! WinDivert capture and runs slower; the Flowseal preset desyncs
//! all TCP 443 globally, so registration still gets desynced. This
//! is an honest, documented limitation, not a bug: the supervisor
//! logs it when the tunnel starts under a Flowseal strategy.
//!
//! # What is excluded (v1)
//!
//! * The MASQUE endpoint `162.159.198.2` - hardcoded in the fork
//!   (`common/cloudflare/constant.go`, `DefaultEndpointH2V4`; audit
//!   section 4.5), so no DNS resolution is needed to find it. First
//!   in the list, so it survives the 40-address truncation.
//! * The fakeip range `198.18.0.0/15` (the v6 fakeip range needs no
//!   test: winws2 excludes `fc00::/7` itself).
//! * Our TUN addresses (`172.19.0.1`, `fdfe:dcba:9876::1`, from the
//!   generator constants): packets to and from the tunnel adapter.
//! * The virtual LANs (Radmin `26.0.0.0/8`, Hamachi `25.0.0.0/8` and
//!   `2620:9b::/96`): decrypted game traffic on those adapters.
//! * UDP-off networks: none in v1. NETZapret excludes Riot game UDP
//!   here, driven by its rules engine; zapret2-gui has no equivalent
//!   concept, so the parameter exists but is passed empty.
//!
//! # Probe ports
//!
//! NETZapret also keeps its recipe-trial probe ports (39100-39199)
//! out of the capture. zapret2-gui has no trial engine and no probe
//! ports yet (the classifier probes are plain TCP connects from the
//! app), so there is nothing to keep out. When recipe trials arrive,
//! they plug into [`merge_filter_into_args`] the same way.

use std::net::IpAddr;

/// The Cloudflare MASQUE endpoint the fork dials for WARP data
/// (hardcoded in the fork, audit section 4.5). Excluding it from the
/// WinDivert capture is what keeps the tunnel fast; excluding the
/// registration host as well would break registration, so only this
/// address - never `api.cloudflareclient.com` - goes here.
pub const MASQUE_ENDPOINT_IP: &str = "162.159.198.2";

/// How many server addresses fit in the filter (NETZapret's
/// measurement, see the module docs). The MASQUE endpoint is first
/// in our input, so it always survives truncation.
pub const SERVER_LIMIT: usize = 40;

/// Virtual LANs that never go through desync (NETZapret's
/// `LocalNetworks.VirtualLans`): Radmin and Hamachi adapters carry
/// decrypted game traffic.
pub const VIRTUAL_LANS: [&str; 3] = ["25.0.0.0/8", "26.0.0.0/8", "2620:9b::/96"];

/// The WARP registration host. Desynced by the recipe section, never
/// excluded from the capture (see the module docs).
pub const WARP_API_HOST: &str = "api.cloudflareclient.com";

/// The desync recipe for the WARP API, from NETZapret's Universal V11
/// preset ("Cloudflare WARP API" section). Verdict on the Go
/// ClientHello (the engine is written in Go) is a stage 7 live-test
/// item: a recipe tuned for a browser hello may not fit sing-box's.
pub const WARP_API_RECIPE: &str =
    "multidisorder:pos=1,host+2,sld+2,sld+5,sniext+1,sniext+2,endhost-2:seqovl=1";

/// Name of the recipe section in the winws2 argument list
/// (`--new=<name>`, the codebase's section convention).
pub const WARP_API_SECTION_NAME: &str = "warp_api";

/// The `--wf-raw-filter=` argument prefix (winws2 only).
const WF_RAW_FILTER_KEY: &str = "--wf-raw-filter=";

/// Input to the tunnel-exclusion filter builder. Mirrors NETZapret's
/// `TunnelCapture.Filter` parameters; the v1 fixed input is
/// [`warp_capture_input`].
#[derive(Debug, Clone, Default)]
pub struct TunnelCaptureInput {
    /// Tunnel server addresses (the MASQUE endpoint in v1). Truncated
    /// to [`SERVER_LIMIT`], first entries win.
    pub servers: Vec<IpAddr>,
    /// Fakeip range (e.g. `198.18.0.0/15`); `None` means "do not
    /// exclude it". The v6 fakeip range is never passed: winws2
    /// excludes `fc00::/7` itself.
    pub fake_range: Option<String>,
    /// Networks whose UDP never goes to desync (outbound only).
    /// Empty in v1 (see the module docs).
    pub udp_off: Vec<String>,
    /// Our TUN addresses (host IPs, no prefix): packets to and from
    /// the tunnel adapter.
    pub tun: Vec<IpAddr>,
    /// Virtual LAN CIDRs (see [`VIRTUAL_LANS`]).
    pub lans: Vec<String>,
}

/// The v1 fixed exclusion set: the MASQUE endpoint, the fakeip range,
/// our TUN addresses (both families, from the generator constants),
/// and the virtual LANs. No UDP-off networks in v1.
pub fn warp_capture_input() -> TunnelCaptureInput {
    use crate::engine::singbox_config::{TUN_ADDRESS_V4, TUN_ADDRESS_V6};
    let mut tun = Vec::new();
    for cidr in [TUN_ADDRESS_V4, TUN_ADDRESS_V6] {
        if let Some((host, _)) = cidr.split_once('/') {
            if let Ok(ip) = host.parse::<IpAddr>() {
                tun.push(ip);
            }
        }
    }
    TunnelCaptureInput {
        servers: vec![MASQUE_ENDPOINT_IP
            .parse()
            .expect("MASQUE endpoint is a valid IP")],
        fake_range: Some(crate::engine::singbox_config::FAKEIP_V4_RANGE.to_string()),
        udp_off: Vec::new(),
        tun,
        lans: VIRTUAL_LANS.iter().map(|s| s.to_string()).collect(),
    }
}

/// First and last address of a CIDR (or a bare IP, treated as a /32
/// or /128). Unparseable input yields `None` and is skipped by the
/// caller - a bad entry must never poison the whole filter.
pub fn cidr_bounds(cidr: &str) -> Option<(IpAddr, IpAddr)> {
    let cidr = cidr.trim();
    if cidr.is_empty() {
        return None;
    }
    let (ip_text, prefix) = match cidr.split_once('/') {
        Some((ip, prefix)) => (ip, Some(prefix)),
        None => (cidr, None),
    };
    let ip: IpAddr = ip_text.parse().ok()?;
    let width = if ip.is_ipv4() { 32 } else { 128 };
    let bits: u32 = match prefix {
        Some(p) => p.parse().ok()?,
        None => width,
    };
    if bits > width {
        return None;
    }
    match ip {
        IpAddr::V4(v4) => {
            let value = u32::from(v4);
            let host_bits = width - bits;
            let host_mask = if host_bits == 32 {
                u32::MAX
            } else {
                (1u32 << host_bits) - 1
            };
            let first = value & !host_mask;
            let last = first | host_mask;
            Some((IpAddr::V4(first.into()), IpAddr::V4(last.into())))
        }
        IpAddr::V6(v6) => {
            let value = u128::from(v6);
            let host_bits = width - bits;
            let host_mask = if host_bits == 128 {
                u128::MAX
            } else {
                (1u128 << host_bits) - 1
            };
            let first = value & !host_mask;
            let last = first | host_mask;
            Some((IpAddr::V6(first.into()), IpAddr::V6(last.into())))
        }
    }
}

/// CIDRs merged into sorted, non-overlapping ranges (IPv4 first, then
/// IPv6): adjacent and nested networks collapse into one range, so
/// `25.0.0.0/8` and `26.0.0.0/8` become a single range and cost two
/// tests instead of four. Unparseable entries are skipped.
pub fn merge_ranges(cidrs: &[String]) -> Vec<(IpAddr, IpAddr)> {
    let mut v4: Vec<(u32, u32)> = Vec::new();
    let mut v6: Vec<(u128, u128)> = Vec::new();
    for cidr in cidrs {
        match cidr_bounds(cidr) {
            Some((IpAddr::V4(first), IpAddr::V4(last))) => {
                v4.push((u32::from(first), u32::from(last)));
            }
            Some((IpAddr::V6(first), IpAddr::V6(last))) => {
                v6.push((u128::from(first), u128::from(last)));
            }
            _ => continue,
        }
    }
    let mut out = Vec::new();
    for (first, last) in merge_u32_ranges(&mut v4) {
        out.push((IpAddr::V4(first.into()), IpAddr::V4(last.into())));
    }
    for (first, last) in merge_u128_ranges(&mut v6) {
        out.push((IpAddr::V6(first.into()), IpAddr::V6(last.into())));
    }
    out
}

fn merge_u32_ranges(ranges: &mut Vec<(u32, u32)>) -> Vec<(u32, u32)> {
    ranges.sort();
    let mut merged: Vec<(u32, u32)> = Vec::new();
    for (first, last) in ranges.drain(..) {
        if let Some(prev) = merged.last_mut() {
            // Adjacent (last+1 == first) or overlapping: extend.
            // Saturating add: a range ending at u32::MAX merges
            // with anything after it.
            if first <= prev.1.saturating_add(1) {
                if last > prev.1 {
                    prev.1 = last;
                }
                continue;
            }
        }
        merged.push((first, last));
    }
    merged
}

fn merge_u128_ranges(ranges: &mut Vec<(u128, u128)>) -> Vec<(u128, u128)> {
    ranges.sort();
    let mut merged: Vec<(u128, u128)> = Vec::new();
    for (first, last) in ranges.drain(..) {
        if let Some(prev) = merged.last_mut() {
            if first <= prev.1.saturating_add(1) {
                if last > prev.1 {
                    prev.1 = last;
                }
                continue;
            }
        }
        merged.push((first, last));
    }
    merged
}

/// The WinDivert field name for an address family (`ip` for IPv4,
/// `ipv6` for IPv6).
fn field_for(addr: &IpAddr) -> &'static str {
    if addr.is_ipv4() {
        "ip"
    } else {
        "ipv6"
    }
}

/// Render an address for the filter. IPv6 prints without a zone
/// (`%12` has no place in a filter); `IpAddr` never carries one, so
/// this is just the display form.
fn filter_ip(addr: &IpAddr) -> String {
    addr.to_string()
}

/// Build the exclusion condition for the given input, in the only
/// negation form WinDivert parses: `(<tests> ? false : true)`.
/// Empty string when there is nothing to exclude (no tests).
pub fn build_filter(input: &TunnelCaptureInput) -> String {
    let mut tests: Vec<String> = Vec::new();

    // The fakeip range, as a >= / <= range test in both directions.
    if let Some(range) = &input.fake_range {
        if let Some((first, last)) = cidr_bounds(range) {
            tests.push(format!(
                "(ip.DstAddr>={} and ip.DstAddr<={})",
                filter_ip(&first),
                filter_ip(&last)
            ));
            tests.push(format!(
                "(ip.SrcAddr>={} and ip.SrcAddr<={})",
                filter_ip(&first),
                filter_ip(&last)
            ));
        }
    }

    // Virtual LANs, merged into ranges, both directions, any protocol.
    for (low, high) in merge_ranges(&input.lans) {
        let field = field_for(&low);
        tests.push(format!(
            "({field}.DstAddr>={} and {field}.DstAddr<={})",
            filter_ip(&low),
            filter_ip(&high)
        ));
        tests.push(format!(
            "({field}.SrcAddr>={} and {field}.SrcAddr<={})",
            filter_ip(&low),
            filter_ip(&high)
        ));
    }

    // UDP-off networks: outbound only (winws2 captures UDP on output;
    // inbound UDP is only matched by payload inspection in preset
    // filter parts). One test for all ranges.
    let udp_ranges = merge_ranges(&input.udp_off);
    if !udp_ranges.is_empty() {
        let inside: Vec<String> = udp_ranges
            .iter()
            .map(|(first, last)| {
                let field = field_for(first);
                format!(
                    "({field}.DstAddr>={} and {field}.DstAddr<={})",
                    filter_ip(first),
                    filter_ip(last)
                )
            })
            .collect();
        tests.push(format!("(udp and ({}))", inside.join(" or ")));
    }

    // TUN addresses and tunnel servers: exact match, both directions.
    // The server list is truncated to the limit (first entries win -
    // the MASQUE endpoint is first in our input).
    for server in input
        .tun
        .iter()
        .chain(input.servers.iter().take(SERVER_LIMIT))
    {
        let field = field_for(server);
        tests.push(format!("{field}.DstAddr={}", filter_ip(server)));
        tests.push(format!("{field}.SrcAddr={}", filter_ip(server)));
    }

    if tests.is_empty() {
        String::new()
    } else {
        format!("({} ? false : true)", tests.join(" or "))
    }
}

/// Outcome of merging the exclusion filter into an argument list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterMerge {
    /// The filter was added (as a new `--wf-raw-filter=` argument, or
    /// merged into an existing one).
    Applied,
    /// The argument list already carries this exact filter (the
    /// apply is idempotent - a second call changes nothing).
    AlreadyPresent,
    /// The preset already has its own `--wf-raw-filter` from a file
    /// (`--wf-raw-filter=@file`): nothing can be merged into it, so
    /// the tunnel stays in the capture. The caller logs this.
    BlockedByFile,
    /// The filter is empty (no tests): nothing to merge.
    Empty,
}

/// Merge the exclusion filter into a winws2 argument list. The key is
/// global (position-independent), so a fresh filter goes first; an
/// existing inline filter merges as `(<theirs>) and <filter>` - the
/// key may appear only once per launch.
pub fn merge_filter_into_args(args: &mut Vec<String>, filter: &str) -> FilterMerge {
    if filter.is_empty() {
        return FilterMerge::Empty;
    }
    if let Some(existing) = args.iter_mut().find(|a| a.starts_with(WF_RAW_FILTER_KEY)) {
        let theirs = &existing[WF_RAW_FILTER_KEY.len()..];
        if theirs.contains(filter) {
            return FilterMerge::AlreadyPresent;
        }
        // A file-based filter cannot be extended in place.
        if theirs.starts_with('@') {
            return FilterMerge::BlockedByFile;
        }
        *existing = format!("{WF_RAW_FILTER_KEY}({theirs}) and {filter}");
        return FilterMerge::Applied;
    }
    args.insert(0, format!("{WF_RAW_FILTER_KEY}{filter}"));
    FilterMerge::Applied
}

/// The WARP API desync section (winws2 only): a `--new` section that
/// desyncs `api.cloudflareclient.com:443` with the multidisorder
/// recipe, so WARP registration and the MASQUE handshake pass through
/// desync. No Lua in winws v1, so Flowseal strategies do not get this
/// section (their preset desyncs all TCP 443 globally anyway).
pub fn warp_recipe_args() -> Vec<String> {
    vec![
        format!("--new={WARP_API_SECTION_NAME}"),
        "--filter-tcp=443".to_string(),
        format!("--hostlist-domains={WARP_API_HOST}"),
        format!("--lua-desync={WARP_API_RECIPE}"),
    ]
}

/// Insert the recipe section into a winws2 argument list, before the
/// first existing `--new` section: profiles match in order, and the
/// recipe must win over catch-all TCP sections for
/// `api.cloudflareclient.com`. With no `--new` at all (a
/// single-profile launch) it appends at the end. Idempotent: a
/// section named `warp_api` is never added twice.
pub fn insert_recipe_section(args: &mut Vec<String>) -> bool {
    if args
        .iter()
        .any(|a| a == &format!("--new={WARP_API_SECTION_NAME}"))
    {
        return false;
    }
    let recipe = warp_recipe_args();
    match args
        .iter()
        .position(|a| a == "--new" || a.starts_with("--new="))
    {
        Some(at) => {
            for (offset, arg) in recipe.into_iter().enumerate() {
                args.insert(at + offset, arg);
            }
        }
        None => args.extend(recipe),
    }
    true
}

/// Outcome of applying the WARP launch arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarpArgsOutcome {
    /// Filter and recipe applied (winws2).
    Applied,
    /// The engine is winws v1 (Flowseal): no filter and no recipe
    /// (see the module docs). The caller logs the limitation.
    NotApplicableWinws1,
    /// The preset's own file-based `--wf-raw-filter` blocked the
    /// merge (the recipe section still applied).
    FilterBlockedByFile,
}

/// Apply the WARP launch arguments to a desync engine argument list:
/// the tunnel exclusion filter plus the registration recipe section.
/// `engine_is_winws2` comes from the supervisor, which knows the
/// engine with certainty (strategy metadata); guessing from the args
/// would be fragile. winws v1 gets nothing (see the module docs).
pub fn apply_warp_args(args: &mut Vec<String>, engine_is_winws2: bool) -> WarpArgsOutcome {
    if !engine_is_winws2 {
        return WarpArgsOutcome::NotApplicableWinws1;
    }
    let input = warp_capture_input();
    let filter = build_filter(&input);
    let merge = merge_filter_into_args(args, &filter);
    insert_recipe_section(args);
    match merge {
        FilterMerge::BlockedByFile => WarpArgsOutcome::FilterBlockedByFile,
        _ => WarpArgsOutcome::Applied,
    }
}

/// Best-effort engine detection from an argument list, for the
/// transport framework's `apply_args` (which does not know the
/// engine): `--dpi-desync` exists only in winws v1 (winws2's desync
/// is Lua-only), `--lua-desync` and the `--wf-*-in/out` flags exist
/// only in winws2. Ambiguous (neither marker) fails closed to
/// "not winws2": applying `--wf-raw-filter` to a winws v1 launch
/// would kill the engine with an unrecognized option, while skipping
/// it only costs tunnel speed.
pub fn args_look_like_winws2(args: &[String]) -> bool {
    let has_v1_marker = args.iter().any(|a| a.starts_with("--dpi-desync"));
    if has_v1_marker {
        return false;
    }
    args.iter().any(|a| {
        a.starts_with("--lua-desync")
            || a.starts_with("--wf-tcp-in=")
            || a.starts_with("--wf-tcp-out=")
            || a.starts_with("--wf-udp-in=")
            || a.starts_with("--wf-udp-out=")
            || a.starts_with(WF_RAW_FILTER_KEY)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v4(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn cidr_bounds_covers_v4_and_v6() {
        assert_eq!(
            cidr_bounds("198.18.0.0/15"),
            Some((v4("198.18.0.0"), v4("198.19.255.255")))
        );
        assert_eq!(
            cidr_bounds("25.0.0.0/8"),
            Some((v4("25.0.0.0"), v4("25.255.255.255")))
        );
        assert_eq!(
            cidr_bounds("2620:9b::/96"),
            Some((
                "2620:9b::".parse().unwrap(),
                "2620:9b::ffff:ffff".parse().unwrap()
            ))
        );
        // Bare IP is a host range.
        assert_eq!(
            cidr_bounds("162.159.198.2"),
            Some((v4("162.159.198.2"), v4("162.159.198.2")))
        );
        // Bad input is None, never a panic.
        assert_eq!(cidr_bounds("not-an-ip"), None);
        assert_eq!(cidr_bounds("10.0.0.0/33"), None);
        assert_eq!(cidr_bounds("10.0.0.0/-1"), None);
        assert_eq!(cidr_bounds(""), None);
        assert_eq!(cidr_bounds("10.0.0.0/24/extra"), None);
    }

    #[test]
    fn merge_ranges_collapses_adjacent_lans() {
        // 25/8 and 26/8 are adjacent: one range, two tests.
        let merged = merge_ranges(&[
            "25.0.0.0/8".to_string(),
            "26.0.0.0/8".to_string(),
            "2620:9b::/96".to_string(),
        ]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0], (v4("25.0.0.0"), v4("26.255.255.255")));
        // Nested ranges collapse too; garbage is skipped.
        let merged = merge_ranges(&[
            "10.0.0.0/8".to_string(),
            "10.1.0.0/16".to_string(),
            "garbage".to_string(),
        ]);
        assert_eq!(merged, vec![(v4("10.0.0.0"), v4("10.255.255.255"))]);
    }

    #[test]
    fn filter_uses_ternary_negation_and_both_directions() {
        let input = TunnelCaptureInput {
            servers: vec![v4("162.159.198.2")],
            ..Default::default()
        };
        let filter = build_filter(&input);
        assert!(filter.starts_with('('), "filter: {filter}");
        assert!(filter.ends_with(" ? false : true)"), "filter: {filter}");
        assert!(
            filter.contains("ip.DstAddr=162.159.198.2"),
            "filter: {filter}"
        );
        assert!(
            filter.contains("ip.SrcAddr=162.159.198.2"),
            "filter: {filter}"
        );
        // No other negation form anywhere.
        assert!(!filter.contains("!("), "filter: {filter}");
        assert!(!filter.contains("!="), "filter: {filter}");
    }

    #[test]
    fn filter_excludes_fake_range_as_bounds() {
        let input = TunnelCaptureInput {
            fake_range: Some("198.18.0.0/15".to_string()),
            ..Default::default()
        };
        let filter = build_filter(&input);
        assert!(
            filter.contains("(ip.DstAddr>=198.18.0.0 and ip.DstAddr<=198.19.255.255)"),
            "filter: {filter}"
        );
        assert!(
            filter.contains("(ip.SrcAddr>=198.18.0.0 and ip.SrcAddr<=198.19.255.255)"),
            "filter: {filter}"
        );
    }

    #[test]
    fn filter_udp_off_is_outbound_only() {
        let input = TunnelCaptureInput {
            udp_off: vec!["203.0.113.0/24".to_string()],
            ..Default::default()
        };
        let filter = build_filter(&input);
        // UDP test matches destination only, gated on `udp`.
        assert!(
            filter.contains("(udp and ((ip.DstAddr>=203.0.113.0 and ip.DstAddr<=203.0.113.255)))"),
            "filter: {filter}"
        );
        assert!(!filter.contains("udp and ((ip.SrcAddr"), "filter: {filter}");
    }

    #[test]
    fn filter_truncates_servers_to_limit() {
        let mut servers = Vec::new();
        for i in 0..50 {
            servers.push(v4(&format!("192.0.2.{i}")));
        }
        let input = TunnelCaptureInput {
            servers,
            ..Default::default()
        };
        let filter = build_filter(&input);
        // First 40 in, the rest out.
        assert!(filter.contains("ip.DstAddr=192.0.2.39"), "filter: {filter}");
        assert!(!filter.contains("192.0.2.40"), "filter: {filter}");
        assert!(!filter.contains("192.0.2.49"), "filter: {filter}");
    }

    #[test]
    fn filter_ipv6_address_has_no_zone() {
        let input = TunnelCaptureInput {
            servers: vec!["2001:db8::1".parse().unwrap()],
            ..Default::default()
        };
        let filter = build_filter(&input);
        assert!(
            filter.contains("ipv6.DstAddr=2001:db8::1"),
            "filter: {filter}"
        );
        assert!(!filter.contains('%'), "filter: {filter}");
    }

    #[test]
    fn filter_empty_input_is_empty() {
        let filter = build_filter(&TunnelCaptureInput::default());
        assert!(filter.is_empty());
    }

    #[test]
    fn warp_input_contains_masque_first_and_no_registration_host() {
        let input = warp_capture_input();
        assert_eq!(input.servers.first(), Some(&v4(MASQUE_ENDPOINT_IP)));
        assert_eq!(input.fake_range.as_deref(), Some("198.18.0.0/15"));
        assert!(input.udp_off.is_empty());
        assert!(input.tun.contains(&v4("172.19.0.1")));
        let filter = build_filter(&input);
        // The registration host must never appear in the exclusion.
        assert!(
            !filter.contains("api.cloudflareclient.com"),
            "filter: {filter}"
        );
        assert!(filter.contains("162.159.198.2"), "filter: {filter}");
        assert!(filter.contains("198.18.0.0"), "filter: {filter}");
        // Merged LANs: one range for 25/8+26/8.
        assert!(
            filter.contains("ip.DstAddr>=25.0.0.0 and ip.DstAddr<=26.255.255.255"),
            "filter: {filter}"
        );
    }

    #[test]
    fn merge_inserts_filter_first_and_is_idempotent() {
        let mut args = vec!["--wf-tcp=80,443".to_string()];
        let filter = "(ip.DstAddr=1.2.3.4 ? false : true)";
        assert_eq!(
            merge_filter_into_args(&mut args, filter),
            FilterMerge::Applied
        );
        assert_eq!(args[0], format!("--wf-raw-filter={filter}"));
        // Second apply changes nothing.
        assert_eq!(
            merge_filter_into_args(&mut args, filter),
            FilterMerge::AlreadyPresent
        );
        assert_eq!(args.len(), 2);
    }

    #[test]
    fn merge_combines_with_existing_inline_filter() {
        let mut args = vec!["--wf-raw-filter=(ip.DstAddr=9.9.9.9 ? false : true)".to_string()];
        let filter = "(ip.DstAddr=1.2.3.4 ? false : true)";
        assert_eq!(
            merge_filter_into_args(&mut args, filter),
            FilterMerge::Applied
        );
        assert_eq!(
            args[0],
            "--wf-raw-filter=((ip.DstAddr=9.9.9.9 ? false : true)) and (ip.DstAddr=1.2.3.4 ? false : true)"
        );
    }

    #[test]
    fn merge_refuses_file_based_filter() {
        let mut args = vec!["--wf-raw-filter=@custom.txt".to_string()];
        let filter = "(ip.DstAddr=1.2.3.4 ? false : true)";
        assert_eq!(
            merge_filter_into_args(&mut args, filter),
            FilterMerge::BlockedByFile
        );
        assert_eq!(args[0], "--wf-raw-filter=@custom.txt");
    }

    #[test]
    fn merge_empty_filter_is_noop() {
        let mut args = vec!["--wf-tcp=80,443".to_string()];
        assert_eq!(merge_filter_into_args(&mut args, ""), FilterMerge::Empty);
        assert_eq!(args.len(), 1);
    }

    #[test]
    fn recipe_section_goes_before_first_new() {
        let mut args = vec![
            "--wf-tcp-out=80,443".to_string(),
            "--lua-init=@zapret-lib.lua".to_string(),
            "--new=general".to_string(),
            "--filter-tcp=443".to_string(),
        ];
        assert!(insert_recipe_section(&mut args));
        let at = args
            .iter()
            .position(|a| a == "--new=warp_api")
            .expect("recipe section present");
        assert_eq!(at, 2);
        assert_eq!(args[at + 1], "--filter-tcp=443");
        assert_eq!(args[at + 2], "--hostlist-domains=api.cloudflareclient.com");
        assert_eq!(args[at + 3], format!("--lua-desync={WARP_API_RECIPE}"));
        // Idempotent.
        assert!(!insert_recipe_section(&mut args));
    }

    #[test]
    fn recipe_section_appends_when_no_new() {
        let mut args = vec!["--wf-tcp=80,443".to_string()];
        assert!(insert_recipe_section(&mut args));
        assert!(args.iter().any(|a| a == "--new=warp_api"));
    }

    #[test]
    fn apply_warp_args_skips_winws1() {
        let mut args = vec!["--dpi-desync=fake".to_string()];
        assert_eq!(
            apply_warp_args(&mut args, false),
            WarpArgsOutcome::NotApplicableWinws1
        );
        assert_eq!(args, vec!["--dpi-desync=fake".to_string()]);
    }

    #[test]
    fn apply_warp_args_applies_filter_and_recipe_to_winws2() {
        let mut args = vec![
            "--wf-tcp-out=80,443".to_string(),
            "--new=general".to_string(),
            "--filter-tcp=443".to_string(),
        ];
        assert_eq!(apply_warp_args(&mut args, true), WarpArgsOutcome::Applied);
        assert!(args[0].starts_with("--wf-raw-filter=("), "args: {args:?}");
        assert!(args.iter().any(|a| a == "--new=warp_api"), "args: {args:?}");
        // Idempotent as a whole.
        let before = args.clone();
        assert_eq!(apply_warp_args(&mut args, true), WarpArgsOutcome::Applied);
        assert_eq!(args, before);
    }

    #[test]
    fn apply_warp_args_reports_blocked_filter() {
        let mut args = vec![
            "--wf-raw-filter=@custom.txt".to_string(),
            "--new=general".to_string(),
        ];
        assert_eq!(
            apply_warp_args(&mut args, true),
            WarpArgsOutcome::FilterBlockedByFile
        );
        // The recipe still applied.
        assert!(args.iter().any(|a| a == "--new=warp_api"), "args: {args:?}");
    }

    #[test]
    fn engine_detection_from_args() {
        // winws v1: dpi-desync marker wins over everything.
        assert!(!args_look_like_winws2(&[
            "--dpi-desync=fake".to_string(),
            "--lua-desync=fake".to_string(),
        ]));
        // winws2 markers.
        assert!(args_look_like_winws2(&["--lua-desync=fake".to_string()]));
        assert!(args_look_like_winws2(&["--wf-tcp-out=80,443".to_string()]));
        assert!(args_look_like_winws2(&[
            "--wf-raw-filter=(x ? false : true)".to_string()
        ]));
        // winws v1 marker: --wf-tcp exists only in v1 (v2 has
        // --wf-tcp-out), so this is a v1 launch - not winws2.
        assert!(!args_look_like_winws2(&[
            "--wf-tcp=80,443".to_string(),
            "--filter-tcp=443".to_string(),
        ]));
        // Empty args: nothing to detect, fails closed.
        assert!(!args_look_like_winws2(&[]));
    }
}

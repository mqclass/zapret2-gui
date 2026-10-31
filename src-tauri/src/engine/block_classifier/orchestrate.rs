//! Probe orchestration for the block classifier: measures the signals
//! of design §1.2 in causal order (control → resolves → through path →
//! addresses → TLS battery → verdict), under the hard cycle deadline
//! and the cancel flag. The verdict itself is [`super::decide`]; this
//! module only produces honest signals.
//!
//! Security posture (§3.3): every address from every resolver passes
//! `is_public_ip` before any connect; DoH clients pin only verified
//! literal IPs and follow no redirects; TLS legs connect to pinned IPs
//! and never resolve anything themselves.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tokio::net::TcpStream;

use super::dns;
use super::tls::{self, AttemptFate};
use super::{
    AddrSignals, ClassifierStage, ClassifierTimeouts, Diagnosis, DiagnosisClass, ForeignOutcome,
    Signals, SysPathOutcome, TcpOutcome, TlsRealOutcome,
};
use crate::engine::probes::{is_public_ip, DiagnosticsService, ProbeStatus};

/// The neutral domain of §1.2: its name rides the foreign leg, its own
/// IP (resolved via the reference) anchors `ctrl_neutral`.
pub const NEUTRAL_HOST: &str = "www.wikipedia.org";

const CF_DOH_URL: &str = "https://cloudflare-dns.com/dns-query";
const CF_DOH_HOST: &str = "cloudflare-dns.com";
const GOOGLE_DOH_URL: &str = "https://dns.google/dns-query";
const GOOGLE_DOH_HOST: &str = "dns.google";

/// What the through-path (`sys_https`) measurement should run. Built-in
/// targets reuse their dedicated probe methods (the service's own URL,
/// exactly "as the browser sees it"); custom targets go through the
/// M-6 untrusted client.
#[derive(Debug, Clone)]
pub enum SysProbe {
    Youtube,
    Discord,
    DiscordVoice,
    Telegram,
    Twitter,
    Custom { url: String },
}

/// A classification target, already validated by the caller (built-ins
/// are constants; custom targets passed the M-6 checks in commands.rs).
#[derive(Debug, Clone)]
pub struct ClassifierTarget {
    /// DNS name, or a literal IP as text.
    pub host: String,
    /// Human display name ("YouTube", or the host itself for custom).
    pub name: String,
    pub port: u16,
    pub literal_ip: Option<IpAddr>,
    pub sys_probe: Option<SysProbe>,
}

/// The cycle was cancelled by the user: not an error, not a diagnosis
/// (§3.4). The caller restores the engine and shows nothing.
#[derive(Debug)]
pub struct Cancelled;

type Progress<'a> = dyn Fn(ClassifierStage) + Send + Sync + 'a;

fn cancelled(cancel: &AtomicBool) -> bool {
    cancel.load(Ordering::SeqCst)
}

/// Sleep that wakes early on cancel. Returns false when cancelled.
async fn sleep_or_cancel(d: Duration, cancel: &AtomicBool) -> bool {
    let step = Duration::from_millis(100);
    let mut left = d;
    while left > Duration::ZERO {
        if cancelled(cancel) {
            return false;
        }
        let chunk = left.min(step);
        tokio::time::sleep(chunk).await;
        left -= chunk;
    }
    !cancelled(cancel)
}

async fn tcp_probe(ip: IpAddr, port: u16, budget: Duration) -> TcpOutcome {
    match tokio::time::timeout(budget, TcpStream::connect(SocketAddr::new(ip, port))).await {
        Ok(Ok(_)) => TcpOutcome::Ok,
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused => TcpOutcome::Refused,
        Ok(Err(_)) => TcpOutcome::Dead,
        Err(_) => TcpOutcome::Dead,
    }
}

/// One control round (§1.2): the existing Cloudflare probe (literal
/// 1.1.1.1) plus bare TCP to the other two operators' anycast addresses.
/// Live = any of the three paths answered.
async fn control_round(diag: &DiagnosticsService, t: &ClassifierTimeouts) -> bool {
    let cf = diag.probe_cloudflare();
    let g = tcp_probe(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), 443, t.tcp_round1);
    let q = tcp_probe(IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9)), 443, t.tcp_round1);
    let (cf, g, q) = tokio::join!(cf, g, q);
    cf.status != ProbeStatus::Blocked || g == TcpOutcome::Ok || q == TcpOutcome::Ok
}

/// DoH client pinned to the operator's literal IPs (§2.1): certificate
/// validation on (reqwest default with webpki roots), no redirects, no
/// proxy (a proxy would silently un-pin the channel).
fn doh_client(host: &str, addrs: &[IpAddr], t: &ClassifierTimeouts) -> Option<reqwest::Client> {
    // reqwest 0.12 pins SocketAddr values; the port is the DoH port of
    // the URL (443), the IPs are the operator's verified literals.
    let socks: Vec<SocketAddr> = addrs.iter().map(|ip| SocketAddr::new(*ip, 443)).collect();
    reqwest::Client::builder()
        .resolve_to_addrs(host, &socks)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(t.doh_per_operator)
        .connect_timeout(t.doh_per_operator)
        .no_proxy()
        .build()
        .ok()
}

struct RefResolution {
    /// Evidence-grade addresses (empty unless `evidence` is set or the
    /// addresses degraded to plain union members — see `plain`).
    addrs: Vec<IpAddr>,
    evidence: bool,
    corroborated: bool,
    /// Non-evidence addresses that may still join the union as plain
    /// members (contradicting operators, plain-UDP fallback).
    plain: Vec<IpAddr>,
    /// A live DoH client for follow-up resolves (neutral domain),
    /// when any operator answered.
    live_client: Option<(reqwest::Client, &'static str)>,
}

fn public_only(addrs: &[IpAddr]) -> Vec<IpAddr> {
    addrs.iter().copied().filter(|ip| is_public_ip(ip)).collect()
}

fn sets_agree(a: &[IpAddr], b: &[IpAddr]) -> bool {
    // Agreement = a non-empty intersection. Two empty sets do NOT
    // agree: an empty answer is the *absence* of a reference, and an
    // absent reference must never read as evidence or corroboration
    // (§2.1). Disjoint non-empty sets from two live channels are a
    // contradiction.
    !a.is_empty() && a.iter().any(|ip| b.contains(ip))
}

/// Builds the capped measurement union (§1.2): evidence reference
/// addresses first, then system addresses, then plain members; IPv4
/// before IPv6 inside each group; an address present in several
/// groups merges its membership flags. Capped at 4 entries.
/// Returns `(ip, in_ref, in_sys)` triples.
fn build_union(
    ref_addrs: &[IpAddr],
    sys_addrs: &[IpAddr],
    plain_addrs: &[IpAddr],
) -> Vec<(IpAddr, bool, bool)> {
    let mut ordered: Vec<(IpAddr, bool, bool)> = Vec::new();
    let push_group = |addrs: &[IpAddr],
                      in_ref: bool,
                      in_sys: bool,
                      ordered: &mut Vec<(IpAddr, bool, bool)>| {
        let mut v4: Vec<IpAddr> = addrs.iter().copied().filter(|i| i.is_ipv4()).collect();
        let mut v6: Vec<IpAddr> = addrs.iter().copied().filter(|i| i.is_ipv6()).collect();
        v4.append(&mut v6);
        for ip in v4 {
            match ordered.iter_mut().find(|(e, _, _)| *e == ip) {
                Some(slot) => {
                    slot.1 |= in_ref;
                    slot.2 |= in_sys;
                }
                None => ordered.push((ip, in_ref, in_sys)),
            }
        }
    };
    push_group(ref_addrs, true, false, &mut ordered);
    push_group(sys_addrs, false, true, &mut ordered);
    push_group(plain_addrs, false, false, &mut ordered);
    ordered.truncate(4);
    ordered
}

/// Reference resolution per §2: DoH of two operators with mutual
/// control; a single live operator is re-asked once after a pause;
/// both dead → plain-UDP fallback (never evidence).
async fn resolve_reference(
    host: &str,
    t: &ClassifierTimeouts,
    cancel: &AtomicBool,
) -> RefResolution {
    let cf_client = doh_client(
        CF_DOH_HOST,
        &[
            IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
            IpAddr::V4(Ipv4Addr::new(1, 0, 0, 1)),
        ],
        t,
    );
    let gg_client = doh_client(
        GOOGLE_DOH_HOST,
        &[
            IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
            IpAddr::V4(Ipv4Addr::new(8, 8, 4, 4)),
        ],
        t,
    );

    // §3.4 budget: 3 s per operator for the WHOLE resolve (A+AAAA are
    // sequential inside the codec), not per query — wrap accordingly.
    let cf = match &cf_client {
        Some(c) => tokio::time::timeout(
            t.doh_per_operator,
            dns::resolve_via_doh(c, CF_DOH_URL, host),
        )
        .await
        .ok()
        .and_then(|r| r.ok()),
        None => None,
    };
    if cancelled(cancel) {
        return RefResolution {
            addrs: vec![],
            evidence: false,
            corroborated: false,
            plain: vec![],
            live_client: None,
        };
    }
    let gg = match &gg_client {
        Some(c) => tokio::time::timeout(
            t.doh_per_operator,
            dns::resolve_via_doh(c, GOOGLE_DOH_URL, host),
        )
        .await
        .ok()
        .and_then(|r| r.ok()),
        None => None,
    };

    match (cf, gg) {
        (Some(a), Some(b)) => {
            let a = public_only(&a);
            let b = public_only(&b);
            if sets_agree(&a, &b) {
                let mut union = a.clone();
                for ip in &b {
                    if !union.contains(ip) {
                        union.push(*ip);
                    }
                }
                RefResolution {
                    addrs: union,
                    evidence: true,
                    corroborated: true,
                    plain: vec![],
                    live_client: cf_client.map(|c| (c, CF_DOH_URL)),
                }
            } else {
                // Contradiction: the reference is not trusted. The
                // addresses degrade to plain union members (measured
                // directly like any other), never evidence (§2.1).
                let mut plain = a;
                for ip in &b {
                    if !plain.contains(ip) {
                        plain.push(*ip);
                    }
                }
                RefResolution {
                    addrs: vec![],
                    evidence: false,
                    corroborated: false,
                    plain,
                    live_client: None,
                }
            }
        }
        (Some(a), None) => {
            single_operator(cf_client, CF_DOH_URL, host, a, cancel).await
        }
        (None, Some(b)) => {
            single_operator(gg_client, GOOGLE_DOH_URL, host, b, cancel).await
        }
        (None, None) => {
            // Plain-UDP fallback: union members only, never evidence.
            let mut plain = Vec::new();
            for server in [IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)), IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))] {
                // Per-server cap (§3.4): one query + one retry inside.
                let res = tokio::time::timeout(
                    t.udp_per_try * 2,
                    dns::resolve_via_udp(server, host, t.udp_per_try),
                )
                .await
                .ok()
                .and_then(|r| r.ok());
                if let Some(addrs) = res {
                    for ip in public_only(&addrs) {
                        if !plain.contains(&ip) {
                            plain.push(ip);
                        }
                    }
                }
                if cancelled(cancel) {
                    break;
                }
            }
            RefResolution {
                addrs: vec![],
                evidence: false,
                corroborated: false,
                plain,
                live_client: None,
            }
        }
    }
}

/// Exactly one DoH operator answered (§2.1): its answer is re-asked
/// once after a 500 ms pause, and the reference counts as
/// evidence-grade only if the two answers agree. `corroborated` stays
/// false — one operator cannot corroborate itself. The live client is
/// kept for follow-up resolves (neutral domain) only when the channel
/// proved itself consistent.
async fn single_operator(
    client: Option<reqwest::Client>,
    url: &'static str,
    host: &str,
    first_raw: Vec<IpAddr>,
    cancel: &AtomicBool,
) -> RefResolution {
    let first = public_only(&first_raw);
    let client = match client {
        Some(c) => c,
        None => {
            return RefResolution {
                addrs: vec![],
                evidence: false,
                corroborated: false,
                plain: first,
                live_client: None,
            }
        }
    };
    if !sleep_or_cancel(Duration::from_millis(500), cancel).await {
        return RefResolution {
            addrs: vec![],
            evidence: false,
            corroborated: false,
            plain: first,
            live_client: None,
        };
    }
    let second = tokio::time::timeout(
        Duration::from_secs(3),
        dns::resolve_via_doh(&client, url, host),
    )
    .await
    .ok()
    .and_then(|r| r.ok());
    match second {
        Some(second_raw) => {
            let second = public_only(&second_raw);
            if sets_agree(&first, &second) {
                let mut union = first;
                for ip in &second {
                    if !union.contains(ip) {
                        union.push(*ip);
                    }
                }
                RefResolution {
                    addrs: union,
                    evidence: true,
                    corroborated: false,
                    plain: vec![],
                    live_client: Some((client, url)),
                }
            } else {
                // The single channel contradicts itself across 500 ms:
                // not evidence; its addresses degrade to plain members.
                let mut plain = first;
                for ip in &second {
                    if !plain.contains(ip) {
                        plain.push(*ip);
                    }
                }
                RefResolution {
                    addrs: vec![],
                    evidence: false,
                    corroborated: false,
                    plain,
                    live_client: None,
                }
            }
        }
        None => RefResolution {
            addrs: vec![],
            evidence: false,
            corroborated: false,
            plain: first,
            live_client: None,
        },
    }
}

/// System resolver, all addresses, two passes (§1.2). Returns the
/// public-filtered union of both passes.
async fn resolve_system(host: &str, port: u16, t: &ClassifierTimeouts) -> Vec<IpAddr> {
    let mut out: Vec<IpAddr> = Vec::new();
    for _ in 0..2 {
        if let Ok(Ok(iter)) = tokio::time::timeout(
            t.dns_sys,
            tokio::net::lookup_host((host, port)),
        )
        .await
        {
            for sa in iter {
                let ip = sa.ip();
                if is_public_ip(&ip) && !out.contains(&ip) {
                    out.push(ip);
                }
            }
        }
    }
    out
}

fn sys_outcome(m: &crate::engine::probes::ProbeMetric) -> SysPathOutcome {
    match m.status {
        ProbeStatus::Operational => SysPathOutcome::Operational,
        ProbeStatus::Degraded => SysPathOutcome::Degraded(m.http_status),
        ProbeStatus::Blocked => SysPathOutcome::Blocked,
        ProbeStatus::Checking => SysPathOutcome::NotMeasured,
    }
}

async fn sys_probe_once(diag: &DiagnosticsService, target: &ClassifierTarget) -> SysPathOutcome {
    let m = match &target.sys_probe {
        Some(SysProbe::Youtube) => diag.probe_youtube().await,
        Some(SysProbe::Discord) => diag.probe_discord().await,
        Some(SysProbe::DiscordVoice) => diag.probe_discord_voice().await,
        Some(SysProbe::Telegram) => diag.probe_telegram().await,
        Some(SysProbe::Twitter) => diag.probe_twitter().await,
        Some(SysProbe::Custom { url }) => {
            diag.measure_endpoint_untrusted(&target.host, &target.host, target.port, url)
                .await
        }
        None => return SysPathOutcome::NotMeasured,
    };
    sys_outcome(&m)
}

/// One address's TLS battery (§1.2/§1.3 row 3): two rounds with
/// alternating leg order (real→foreign, then foreign→real) and ≥800 ms
/// pauses between attempts. Fills `addr.tls_real` / `addr.tls_foreign`
/// and returns the last real attempt's certificate observation plus a
/// flag telling whether the cycle deadline cut the battery short —
/// the caller folds that into `Signals::deadline_hit`, so a truncated
/// battery is honestly marked incomplete instead of looking like a
/// full one that merely disagreed with itself (§3.4). The deadline is
/// checked before every leg: an individual attempt is still bounded
/// by its own `tls_attempt` budget, so the overshoot stays under one
/// attempt.
async fn tls_battery(
    addr: &mut AddrSignals,
    host: &str,
    port: u16,
    t: &ClassifierTimeouts,
    deadline: Instant,
    cancel: &AtomicBool,
) -> (Option<tls::CertObservation>, bool) {
    let roots = tls::webpki_roots();
    let mut last_cert = None;

    for round in 0..2 {
        if cancelled(cancel) {
            break;
        }
        let legs: [bool; 2] = if round == 0 {
            [true, false] // real, foreign
        } else {
            [false, true] // foreign, real
        };
        for (i, is_real) in legs.iter().enumerate() {
            if Instant::now() >= deadline {
                return (last_cert, true);
            }
            if i > 0 && !sleep_or_cancel(t.leg_pause, cancel).await {
                return (last_cert, false);
            }
            if *is_real {
                let r = tls::tls_real_attempt(addr.ip, port, host, roots.clone(), t.tls_attempt).await;
                last_cert = Some(r.cert.clone());
                if let AttemptFate::Real(o) = r.fate {
                    addr.tls_real.push(o);
                }
            } else {
                let r = tls::tls_foreign_attempt(addr.ip, port, NEUTRAL_HOST, t.tls_attempt).await;
                if let AttemptFate::Foreign(o) = r.fate {
                    addr.tls_foreign.push(o);
                }
            }
        }
        if round == 0 && !sleep_or_cancel(t.leg_pause, cancel).await {
            break;
        }
    }
    (last_cert, false)
}

/// Runs the full classification cycle for one target. Progress is
/// reported by stage; cancellation is checked between legs and inside
/// pauses.
pub async fn run_classification(
    diag: &DiagnosticsService,
    target: &ClassifierTarget,
    t: &ClassifierTimeouts,
    progress: &Progress<'_>,
    cancel: &AtomicBool,
) -> Result<Diagnosis, Cancelled> {
    let started = Instant::now();
    let mut sig = Signals {
        ctrl_dead_both_rounds: false,
        ctrl_alive: false,
        sys_https: Vec::new(),
        addrs: Vec::new(),
        ref_evidence_grade: false,
        ref_corroborated: false,
        ctrl_neutral_alive: false,
        deadline_hit: false,
    };

    // --- Stage: control -------------------------------------------------
    progress(ClassifierStage::Control);
    let r1 = control_round(diag, t).await;
    if cancelled(cancel) {
        return Err(Cancelled);
    }
    let r2 = control_round(diag, t).await;
    sig.ctrl_alive = r1 || r2;
    sig.ctrl_dead_both_rounds = !r1 && !r2;
    if cancelled(cancel) {
        return Err(Cancelled);
    }
    if sig.ctrl_dead_both_rounds {
        progress(ClassifierStage::Verdict);
        return Ok(Diagnosis::from_signals(
            DiagnosisClass::NoNetwork,
            &target.host,
            &target.name,
            &sig,
        ));
    }

    // --- Stage: resolves ------------------------------------------------
    progress(ClassifierStage::Resolve);
    let (sys_addrs, refr) = if let Some(ip) = target.literal_ip {
        (
            vec![ip],
            RefResolution {
                addrs: vec![],
                evidence: false,
                corroborated: false,
                plain: vec![],
                live_client: None,
            },
        )
    } else {
        let sys = resolve_system(&target.host, target.port, t).await;
        if cancelled(cancel) {
            return Err(Cancelled);
        }
        let refr = resolve_reference(&target.host, t, cancel).await;
        (sys, refr)
    };
    if cancelled(cancel) {
        return Err(Cancelled);
    }
    sig.ref_evidence_grade = refr.evidence;
    sig.ref_corroborated = refr.corroborated;

    // Union, capped at 4 (§1.2): evidence reference first, then
    // system, then plain members.
    let ordered = build_union(&refr.addrs, &sys_addrs, &refr.plain);
    sig.addrs = ordered
        .into_iter()
        .map(|(ip, in_ref, in_sys)| {
            let mut a = AddrSignals::new(ip);
            a.in_ref = in_ref;
            a.in_sys = in_sys;
            a
        })
        .collect();

    // --- Stage: through path --------------------------------------------
    progress(ClassifierStage::SysPath);
    for _ in 0..2 {
        if cancelled(cancel) {
            return Err(Cancelled);
        }
        sig.sys_https.push(sys_probe_once(diag, target).await);
    }

    // --- Stage: addresses (TCP rounds) ----------------------------------
    progress(ClassifierStage::Addresses);
    let round1_start = Instant::now();
    {
        // Parallel inside the round (§3.2): the §3.4 budget assumes it.
        let mut set = tokio::task::JoinSet::new();
        for (idx, a) in sig.addrs.iter().enumerate() {
            let (ip, port, budget) = (a.ip, target.port, t.tcp_round1);
            set.spawn(async move { (idx, tcp_probe(ip, port, budget).await) });
        }
        while let Some(joined) = set.join_next().await {
            if let Ok((idx, outcome)) = joined {
                sig.addrs[idx].tcp_round1 = outcome;
            }
        }
    }
    if cancelled(cancel) {
        return Err(Cancelled);
    }
    let all_dead = !sig.addrs.is_empty()
        && sig.addrs.iter().all(|a| {
            matches!(a.tcp_round1, TcpOutcome::Refused | TcpOutcome::Dead)
        });
    if all_dead {
        // Round 2 for the Ip class: time-separated, longer timeout.
        let elapsed = round1_start.elapsed();
        if elapsed < t.round_gap_ip {
            if !sleep_or_cancel(t.round_gap_ip - elapsed, cancel).await {
                return Err(Cancelled);
            }
        }
        let mut set = tokio::task::JoinSet::new();
        for (idx, a) in sig.addrs.iter().enumerate() {
            let (ip, port, budget) = (a.ip, target.port, t.tcp_round2);
            set.spawn(async move { (idx, tcp_probe(ip, port, budget).await) });
        }
        while let Some(joined) = set.join_next().await {
            if let Ok((idx, outcome)) = joined {
                sig.addrs[idx].tcp_round2 = outcome;
            }
        }
        if cancelled(cancel) {
            return Err(Cancelled);
        }
    }

    // --- Stage: TLS battery ---------------------------------------------
    progress(ClassifierStage::Tls);
    let mut cert_info: Option<tls::CertObservation> = None;
    if started.elapsed() > t.cycle_deadline {
        sig.deadline_hit = true;
    } else {
        // ctrl_neutral: the neutral domain on its OWN IP with its real
        // name, resolved via the reference channel (§1.2).
        sig.ctrl_neutral_alive = neutral_control(&refr, t, cancel).await;
        if cancelled(cancel) {
            return Err(Cancelled);
        }
        let alive_idxs: Vec<usize> = sig
            .addrs
            .iter()
            .enumerate()
            .filter(|(_, a)| a.tcp_round1 == TcpOutcome::Ok || a.tcp_round2 == TcpOutcome::Ok)
            .map(|(i, _)| i)
            .collect();
        let deadline = started + t.cycle_deadline;
        for (n, idx) in alive_idxs.iter().take(2).enumerate() {
            if n > 0 && started.elapsed() > t.cycle_deadline {
                sig.deadline_hit = true;
                break;
            }
            let mut addr = sig.addrs[*idx].clone();
            let (cert, truncated) =
                tls_battery(&mut addr, &target.host, target.port, t, deadline, cancel).await;
            if truncated {
                sig.deadline_hit = true;
            }
            sig.addrs[*idx] = addr;
            if cert_info.is_none() {
                cert_info = cert;
            }
            if cancelled(cancel) {
                return Err(Cancelled);
            }
            // A decisive picture from the first address ends the
            // battery (§3.4: the second address is for contradictions).
            if super::decide(&sig) != DiagnosisClass::Unknown {
                break;
            }
        }
    }

    // --- Verdict ----------------------------------------------------------
    progress(ClassifierStage::Verdict);
    let class = super::decide(&sig);
    let mut diagnosis = Diagnosis::from_signals(class, &target.host, &target.name, &sig);
    if let Some(c) = cert_info {
        diagnosis.details.cert_valid = c.valid;
        diagnosis.details.cert_subject = c.subject;
    }
    Ok(diagnosis)
}

/// `ctrl_neutral` (§1.2): resolve the neutral domain via the reference
/// channel and run a tls_real-analogous attempt against its own IP.
/// Alive = genuine contact (HTTP over a valid session, or death only
/// after a valid certificate).
async fn neutral_control(
    refr: &RefResolution,
    t: &ClassifierTimeouts,
    cancel: &AtomicBool,
) -> bool {
    let mut addrs: Vec<IpAddr> = Vec::new();
    if let Some((client, url)) = &refr.live_client {
        let res = tokio::time::timeout(
            Duration::from_secs(3),
            dns::resolve_via_doh(client, url, NEUTRAL_HOST),
        )
        .await
        .ok()
        .and_then(|r| r.ok());
        if let Some(a) = res {
            addrs = public_only(&a);
        }
    }
    if addrs.is_empty() {
        for server in [IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)), IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))] {
            let res = tokio::time::timeout(
                t.udp_per_try * 2,
                dns::resolve_via_udp(server, NEUTRAL_HOST, t.udp_per_try),
            )
            .await
            .ok()
            .and_then(|r| r.ok());
            if let Some(a) = res {
                addrs = public_only(&a);
                if !addrs.is_empty() {
                    break;
                }
            }
            if cancelled(cancel) {
                return false;
            }
        }
    }
    let ip = match addrs.iter().find(|i| i.is_ipv4()).or_else(|| addrs.first()) {
        Some(ip) => *ip,
        None => return false,
    };
    let r = tls::tls_real_attempt(ip, 443, NEUTRAL_HOST, tls::webpki_roots(), t.tls_attempt).await;
    matches!(
        r.fate,
        AttemptFate::Real(TlsRealOutcome::Http(_))
            | AttemptFate::Real(TlsRealOutcome::PostCertKilled)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn sets_agree_needs_a_shared_address() {
        assert!(sets_agree(&[ip("1.2.3.4")], &[ip("1.2.3.4"), ip("5.6.7.8")]));
        assert!(!sets_agree(&[ip("1.2.3.4")], &[ip("5.6.7.8")]));
    }

    #[test]
    fn sets_agree_never_agrees_on_empty() {
        // An empty reference is no reference: it must not produce
        // evidence or corroboration (§2.1).
        assert!(!sets_agree(&[], &[]));
        assert!(!sets_agree(&[ip("1.2.3.4")], &[]));
        assert!(!sets_agree(&[], &[ip("1.2.3.4")]));
    }

    #[test]
    fn union_orders_ref_then_sys_then_plain_v4_first() {
        let u = build_union(
            &[ip("2001:db8::1"), ip("1.1.1.1")],
            &[ip("2.2.2.2")],
            &[ip("3.3.3.3")],
        );
        let ips: Vec<IpAddr> = u.iter().map(|(a, _, _)| *a).collect();
        assert_eq!(
            ips,
            vec![ip("1.1.1.1"), ip("2001:db8::1"), ip("2.2.2.2"), ip("3.3.3.3")]
        );
        assert_eq!(u[0], (ip("1.1.1.1"), true, false));
        assert_eq!(u[2], (ip("2.2.2.2"), false, true));
        assert_eq!(u[3], (ip("3.3.3.3"), false, false));
    }

    #[test]
    fn union_merges_membership_flags() {
        let u = build_union(&[ip("1.2.3.4")], &[ip("1.2.3.4"), ip("5.6.7.8")], &[]);
        assert_eq!(u.len(), 2);
        assert_eq!(u[0], (ip("1.2.3.4"), true, true));
        assert_eq!(u[1], (ip("5.6.7.8"), false, true));
    }

    #[test]
    fn union_is_capped_at_four() {
        let many: Vec<IpAddr> = (1..=6).map(|n| ip(&format!("1.2.3.{}", n))).collect();
        let u = build_union(&many, &[ip("9.9.9.9")], &[ip("8.8.8.8")]);
        assert_eq!(u.len(), 4);
        // The cap keeps the highest-priority (reference) addresses.
        assert!(u.iter().all(|(_, in_ref, _)| *in_ref));
    }
}

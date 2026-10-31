//! Block-type classifier: answers "why does this site not open" with a
//! discriminating probe battery and a pure verdict function.
//!
//! Design: `~/workspace/reviews/zapret2-gui/BLOCK-CLASSIFIER-DESIGN-DRAFT.md`
//! (edition 2). The load-bearing idea from the design: the verdict is a
//! pure function `decide(&Signals) -> DiagnosisClass` over low-level
//! signals measured by separate probes — never a black-box reuse of
//! `measure_endpoint`, whose merged TCP+TLS phases and first-address DNS
//! destroy exactly the distinctions this module exists to make.
//!
//! An erroneous diagnosis is worse than no diagnosis: `Unknown` is a
//! first-class outcome, every discriminating signal is measured twice,
//! and a class needs two independent concordant signals (§1.1).

pub mod dns;
pub mod orchestrate;
pub mod tls;

use std::net::IpAddr;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Persisted dead-man record (§4.3). Written to settings *before* the
/// engine is stopped for a classification cycle; removed only after the
/// engine has been restored. Its presence at app start means the previous
/// cycle was interrupted (GUI crash, process kill, power loss) and the
/// engine may have been left stopped.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifierSession {
    pub started_epoch: u64,
    pub was_engine_running: bool,
    pub strategy_id: Option<String>,
    pub target: String,
}

/// The diagnosis vocabulary (§1.3). The classifier speaks only these
/// classes; availability stays with the probes' `ProbeStatus` scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosisClass {
    /// Gate outcome, not a target class: no control path is alive.
    NoNetwork,
    Open,
    Dns,
    Sni,
    Ip,
    ServerDown,
    Unknown,
}

/// Outcome of one raw TCP connect to one address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcpOutcome {
    Ok,
    Refused,
    /// Reset or timeout: for decide() both mean "no path" (§3.2).
    Dead,
    NotMeasured,
}

/// Outcome of one `tls_real` attempt (taxonomy §1.2). The variants are
/// measurement events of the TLS stack, nothing else: predicates in
/// `decide` never reference anything outside this list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsRealOutcome {
    /// HTTP response received over a valid TLS session (any status,
    /// including 3xx and 5xx).
    Http(u16),
    /// Handshake reached the certificate stage, the certificate was
    /// VALID for the real domain, but the connection died before an
    /// HTTP response arrived.
    PostCertKilled,
    /// Certificate stage reached, certificate NOT valid for the real
    /// domain while TCP was alive: the stub/MITM signature.
    CertInvalid,
    /// Connection died (RST, alert, EOF) before the certificate stage.
    KilledPreCert,
    /// Neither death nor progress within the attempt timeout. A silent
    /// drop and a reset are different worlds, so this is its own
    /// outcome, not a flavour of KilledPreCert.
    Timeout,
}

/// Outcome of one `tls_foreign` attempt, reduced to handshake survival.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForeignOutcome {
    /// Handshake completed OR reached the certificate stage: the path
    /// did not kill a handshake carrying a foreign name.
    Survived,
    KilledPreCert,
    Timeout,
}

/// One measured pass of the through path ("as the browser sees it").
/// `Degraded` carries the HTTP status when one was received:
/// `measure_endpoint` reports any non-OK status as Degraded, and the
/// decide() Open leg must be able to tell "Degraded because 404" from
/// "Degraded because 503 in both passes" — the latter is ServerDown
/// evidence, not openness (see the note at [`decide`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SysPathOutcome {
    Operational,
    Degraded(Option<u16>),
    Blocked,
    NotMeasured,
}

/// All signals measured for one address of the union
/// `dns_sys ∪ dns_ref ∪ dns_udp` (after the §3.3 public-IP filter).
#[derive(Debug, Clone)]
pub struct AddrSignals {
    pub ip: IpAddr,
    /// Address came from the system resolver.
    pub in_sys: bool,
    /// Address came from the evidence-grade reference (§2.1). Addresses
    /// from plain UDP are union members but never set this flag (§2.2).
    pub in_ref: bool,
    pub tcp_round1: TcpOutcome,
    pub tcp_round2: TcpOutcome,
    /// One entry per attempt (normally two). "Not measured" for a TLS
    /// leg is a short or empty vec (unlike the per-round TCP slots):
    /// predicates require ≥2 entries before believing a pattern. A
    /// killed-family entry means the certificate stage was not reached
    /// on that attempt; TCP liveness itself is established only by the
    /// dedicated `tcp` rounds, never inferred from TLS outcomes.
    pub tls_real: Vec<TlsRealOutcome>,
    pub tls_foreign: Vec<ForeignOutcome>,
}

impl Default for AddrSignals {
    fn default() -> Self {
        // IpAddr has no Default; UNSPECIFIED is the neutral filler —
        // real entries are always built via AddrSignals::new.
        Self::new(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
    }
}

impl Default for TcpOutcome {
    fn default() -> Self {
        TcpOutcome::NotMeasured
    }
}

/// Everything the verdict is allowed to look at. Built by the probe
/// orchestration, consumed by [`decide`].
#[derive(Debug, Clone)]
pub struct Signals {
    /// Control (§1.2) was dead in both rounds: no control address of
    /// any of the three operators gave a live path in either round.
    pub ctrl_dead_both_rounds: bool,
    /// At least one control path was alive in at least one round.
    pub ctrl_alive: bool,
    /// Through-path measurements (normally two).
    pub sys_https: Vec<SysPathOutcome>,
    /// Union of addresses, at most 4, evidence addresses first (§1.2).
    pub addrs: Vec<AddrSignals>,
    /// The reference resolution is evidence-grade (§2): DoH channel(s)
    /// alive and not self-contradictory.
    pub ref_evidence_grade: bool,
    /// Two independent DoH operators agreed (detail marker only).
    pub ref_corroborated: bool,
    /// The neutral domain answered on its own IP with its real SNI in
    /// this cycle; without it the foreign leg is invalid (§1.2).
    pub ctrl_neutral_alive: bool,
    /// The 90 s cycle deadline cut measurement short.
    pub deadline_hit: bool,
}

impl AddrSignals {
    pub fn new(ip: IpAddr) -> Self {
        Self {
            ip,
            in_sys: false,
            in_ref: false,
            tcp_round1: TcpOutcome::NotMeasured,
            tcp_round2: TcpOutcome::NotMeasured,
            tls_real: Vec::new(),
            tls_foreign: Vec::new(),
        }
    }

    fn tcp_ok(&self) -> bool {
        self.tcp_round1 == TcpOutcome::Ok || self.tcp_round2 == TcpOutcome::Ok
    }

    fn tcp_dead_measured(&self) -> bool {
        // Every measured round is dead, and at least one round measured.
        let rounds = [self.tcp_round1, self.tcp_round2];
        let measured: Vec<TcpOutcome> = rounds
            .into_iter()
            .filter(|r| *r != TcpOutcome::NotMeasured)
            .collect();
        !measured.is_empty()
            && measured
                .iter()
                .all(|r| matches!(r, TcpOutcome::Refused | TcpOutcome::Dead))
    }

    /// The genuine server of the target answers at this address (§1.2):
    /// TCP alive, a TLS attempt reached the genuine server (HTTP over a
    /// valid session, or death only after a valid certificate), and no
    /// attempt presented an invalid certificate.
    fn genuine_alive(&self) -> bool {
        if !self.tcp_ok() || self.tls_real.is_empty() {
            return false;
        }
        if self.tls_real.contains(&TlsRealOutcome::CertInvalid) {
            return false;
        }
        self.tls_real.iter().any(|o| {
            matches!(o, TlsRealOutcome::Http(_)) || *o == TlsRealOutcome::PostCertKilled
        })
    }

    /// The site genuinely opens via this address: TCP alive, EVERY TLS
    /// attempt produced an HTTP response (unanimous — a killed or
    /// timed-out repeat contradicts openness, §1.1 rule 1), and the
    /// pair is not the ServerDown pattern (5xx in both repeats is the
    /// server being broken, not the site opening — see [`decide`]).
    fn opens(&self) -> bool {
        self.tcp_ok()
            && !self.tls_real.is_empty()
            && self
                .tls_real
                .iter()
                .all(|o| matches!(o, TlsRealOutcome::Http(_)))
            && !self.server_down()
    }

    /// Dns leg (a) for one system address: the system path at this
    /// address is dead or does not lead to the genuine server (§1.3).
    fn sys_bad(&self) -> bool {
        if self.tcp_dead_measured() {
            return true;
        }
        // TCP alive (or unmeasured): only the TLS evidence can damn the
        // address, and it must be unanimous.
        !self.tls_real.is_empty()
            && self.tls_real.iter().all(|o| {
                matches!(
                    o,
                    TlsRealOutcome::KilledPreCert
                        | TlsRealOutcome::Timeout
                        | TlsRealOutcome::CertInvalid
                )
            })
    }

    /// Sni predicate (§1.3 row 3) for this address: TCP alive, the real
    /// name is killed in both attempts, the foreign name survives in
    /// both attempts on the same address.
    fn sni_pattern(&self) -> bool {
        self.tcp_round1 == TcpOutcome::Ok
            && self.tls_real.len() >= 2
            && self
                .tls_real
                .iter()
                .all(|o| matches!(o, TlsRealOutcome::KilledPreCert | TlsRealOutcome::Timeout))
            && self.tls_foreign.len() >= 2
            && self
                .tls_foreign
                .iter()
                .all(|o| *o == ForeignOutcome::Survived)
    }

    /// ServerDown predicate (§1.3 row 5): authenticated bytes — an HTTP
    /// 5xx over a valid TLS session, in both repeats.
    fn server_down(&self) -> bool {
        self.tcp_ok()
            && self.tls_real.len() >= 2
            && self
                .tls_real
                .iter()
                .all(|o| matches!(o, TlsRealOutcome::Http(code) if (500..=599).contains(code)))
    }
}

/// The verdict. Pure: no I/O, no clock. The check order IS the design's
/// table order (§1.3) — Open before Dns so a live system path is never
/// overridden by resolver disagreement; Dns before Sni/Ip because its
/// legs contain liveness facts about the reference addresses; Ip last of
/// the block classes because it demands the death of the whole union.
///
/// One recorded reading of the design (flagged for review): §1.2 defines
/// `alive()` including `PostCertKilled` (genuineness), while §1.3 row 5
/// states a post-certificate death without an HTTP response "goes to
/// Unknown". For the **Open** class this implementation requires an
/// actual HTTP response over the system path (`opens`): a
/// connection the provider kills right after the certificate flight
/// does not "open" for the user, and Open's user text claims the site
/// opens without the bypass. `PostCertKilled` still counts as genuine
/// contact for the Dns leg (b), where what matters is which stage is
/// broken, not whether the site fully opens.
///
/// Second recorded reading, same spirit: taken literally, row 1's
/// `Http(_)` would swallow row 5 whole — a server answering 5xx in
/// both repeats produces `Http(5xx)` outcomes and a Degraded through
/// path, so ServerDown could never fire and live scene S5 (§8), which
/// expects ServerDown for exactly that picture, would be unpassable.
/// The consistent reading implemented here: the both-repeats-5xx
/// pattern is carved out of Open's evidence (per-address in `opens`,
/// and for the through path via the statuses carried in
/// `SysPathOutcome::Degraded`), leaving row order otherwise untouched.
pub fn decide(s: &Signals) -> DiagnosisClass {
    // Row 0 — gate: no control path alive in either round.
    if s.ctrl_dead_both_rounds {
        return DiagnosisClass::NoNetwork;
    }

    // Row 1 — Open: the through path works, or a system address serves
    // real HTTP responses from the genuine server. Set disagreement
    // between resolvers is NOT evidence against Open (CDN/GeoDNS).
    let sys_path_open = {
        let measured: Vec<SysPathOutcome> = s
            .sys_https
            .iter()
            .copied()
            .filter(|o| *o != SysPathOutcome::NotMeasured)
            .collect();
        // 5xx in every measured pass (at least two) is the ServerDown
        // pattern arriving via the through path, not openness.
        let sys_5xx_consistent = measured.len() >= 2
            && measured.iter().all(|o| {
                matches!(o, SysPathOutcome::Degraded(Some(code)) if (500..=599).contains(code))
            });
        !measured.is_empty()
            && !sys_5xx_consistent
            && measured.iter().all(|o| {
                matches!(
                    o,
                    SysPathOutcome::Operational | SysPathOutcome::Degraded(_)
                )
            })
    };
    if sys_path_open || s.addrs.iter().any(|a| a.in_sys && a.opens()) {
        return DiagnosisClass::Open;
    }

    // Row 2 — Dns: system resolution is broken AND the evidence-grade
    // reference path reaches the genuine server. All three legs
    // mandatory; unreachable by construction in partial mode.
    if s.ref_evidence_grade {
        let sys_all_bad = s
            .addrs
            .iter()
            .filter(|a| a.in_sys)
            .all(|a| a.sys_bad());
        let ref_genuine = s.addrs.iter().any(|a| a.in_ref && a.genuine_alive());
        if sys_all_bad && ref_genuine {
            return DiagnosisClass::Dns;
        }
    }

    // Row 3 — Sni: same address, real name killed twice, foreign name
    // survived twice, neutral control alive this cycle.
    if s.ctrl_neutral_alive && s.addrs.iter().any(|a| a.sni_pattern()) {
        return DiagnosisClass::Sni;
    }

    // Row 4 — Ip: the whole union is TCP-dead in both (time-separated)
    // rounds while the control is alive.
    if s.ctrl_alive
        && !s.addrs.is_empty()
        && s.addrs.iter().all(|a| {
            matches!(a.tcp_round1, TcpOutcome::Refused | TcpOutcome::Dead)
                && matches!(a.tcp_round2, TcpOutcome::Refused | TcpOutcome::Dead)
        })
    {
        return DiagnosisClass::Ip;
    }

    // Row 5 — ServerDown: authenticated 5xx in both repeats.
    if s.addrs.iter().any(|a| a.server_down()) {
        return DiagnosisClass::ServerDown;
    }

    DiagnosisClass::Unknown
}

/// Raw-signal details for the collapsible "Details" section of the
/// result card and for the log line (§3.3 item 7: outcomes, never
/// response bodies).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosisDetails {
    pub ctrl_alive: bool,
    pub sys_addrs: Vec<String>,
    pub ref_addrs: Vec<String>,
    pub ref_evidence_grade: bool,
    pub ref_corroborated: bool,
    pub partial_mode: bool,
    pub ctrl_neutral_alive: bool,
    pub deadline_hit: bool,
    /// Per-address outcome summary, e.g.
    /// "203.0.113.7: tcp=Ok/NotMeasured tls=Http(200)/Http(200) foreign=Survived/Survived".
    pub addr_outcomes: Vec<String>,
    pub sys_https: Vec<String>,
    /// Certificate verdict of the first TLS battery's real leg (§3.3
    /// item 7: the fact of validity and the SAN line are the only
    /// certificate data that may leave the connector).
    pub cert_valid: Option<bool>,
    pub cert_subject: Option<String>,
}

/// Full classification result handed to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnosis {
    pub class: DiagnosisClass,
    /// Host that was classified (e.g. "www.youtube.com").
    pub target: String,
    /// Human display name of the target (e.g. "YouTube").
    pub target_name: String,
    /// Measurement context: the classifier always measures the raw
    /// provider path, i.e. "direct" (bypass stopped for the cycle, §4.1).
    pub context: String,
    /// Classification ran without an evidence-grade reference (§2.2):
    /// the UI adds the reduced-accuracy footnote (§6).
    pub partial_mode: bool,
    pub details: DiagnosisDetails,
}

impl Diagnosis {
    pub fn from_signals(
        class: DiagnosisClass,
        target: &str,
        target_name: &str,
        s: &Signals,
    ) -> Self {
        let fmt_addrs = |want_ref: bool, want_sys: bool| -> Vec<String> {
            s.addrs
                .iter()
                .filter(|a| (want_ref && a.in_ref) || (want_sys && a.in_sys))
                .map(|a| a.ip.to_string())
                .collect()
        };
        let addr_outcomes = s
            .addrs
            .iter()
            .map(|a| {
                let tls = a
                    .tls_real
                    .iter()
                    .map(|o| format!("{:?}", o))
                    .collect::<Vec<_>>()
                    .join("/");
                let foreign = a
                    .tls_foreign
                    .iter()
                    .map(|o| format!("{:?}", o))
                    .collect::<Vec<_>>()
                    .join("/");
                format!(
                    "{}: tcp={:?}/{:?} tls={} foreign={}",
                    a.ip, a.tcp_round1, a.tcp_round2, tls, foreign
                )
            })
            .collect();
        Self {
            class,
            target: target.to_string(),
            target_name: target_name.to_string(),
            context: "direct".to_string(),
            partial_mode: !s.ref_evidence_grade,
            details: DiagnosisDetails {
                ctrl_alive: s.ctrl_alive,
                sys_addrs: fmt_addrs(false, true),
                ref_addrs: fmt_addrs(true, false),
                ref_evidence_grade: s.ref_evidence_grade,
                ref_corroborated: s.ref_corroborated,
                partial_mode: !s.ref_evidence_grade,
                ctrl_neutral_alive: s.ctrl_neutral_alive,
                deadline_hit: s.deadline_hit,
                addr_outcomes,
                sys_https: s.sys_https.iter().map(|o| format!("{:?}", o)).collect(),
                cert_valid: None,
                cert_subject: None,
            },
        }
    }
}

/// Stage names reported to the UI while a cycle runs (human texts live
/// in the frontend i18n; these are stable machine keys).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassifierStage {
    Control,
    Resolve,
    SysPath,
    Addresses,
    Tls,
    Verdict,
}

/// Tunables fixed by the design §3.4. Changing them requires live-test
/// evidence (§8), not taste.
pub struct ClassifierTimeouts {
    pub dns_sys: Duration,
    pub doh_per_operator: Duration,
    pub udp_per_try: Duration,
    pub tcp_round1: Duration,
    pub tcp_round2: Duration,
    pub tls_attempt: Duration,
    pub cycle_deadline: Duration,
    pub round_gap_ip: Duration,
    pub leg_pause: Duration,
}

impl Default for ClassifierTimeouts {
    fn default() -> Self {
        Self {
            dns_sys: Duration::from_secs(2),
            doh_per_operator: Duration::from_secs(3),
            udp_per_try: Duration::from_secs(2),
            tcp_round1: Duration::from_secs(2),
            tcp_round2: Duration::from_secs(4),
            tls_attempt: Duration::from_secs(5),
            cycle_deadline: Duration::from_secs(90),
            round_gap_ip: Duration::from_secs(3),
            leg_pause: Duration::from_millis(800),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(n: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, n))
    }

    fn base() -> Signals {
        Signals {
            ctrl_dead_both_rounds: false,
            ctrl_alive: true,
            sys_https: vec![SysPathOutcome::Blocked, SysPathOutcome::Blocked],
            addrs: vec![],
            ref_evidence_grade: true,
            ref_corroborated: true,
            ctrl_neutral_alive: true,
            deadline_hit: false,
        }
    }

    fn addr(n: u8, sys: bool, refr: bool) -> AddrSignals {
        let mut a = AddrSignals::new(ip(n));
        a.in_sys = sys;
        a.in_ref = refr;
        a
    }

    // --- Gate ------------------------------------------------------------

    #[test]
    fn dead_control_both_rounds_is_no_network() {
        let mut s = base();
        s.ctrl_dead_both_rounds = true;
        s.ctrl_alive = false;
        s.sys_https = vec![SysPathOutcome::Operational];
        assert_eq!(decide(&s), DiagnosisClass::NoNetwork);
    }

    // --- Open ------------------------------------------------------------

    #[test]
    fn healthy_geodns_disjoint_sets_are_open() {
        // Review trap (edition 1 defect): system and reference sets are
        // DISJOINT (provider GGC vs anycast), the reference addresses are
        // even dead — but the system path serves the genuine site.
        // Disjoint sets are not spoofing evidence: Open.
        let mut s = base();
        let mut sys = addr(1, true, false);
        sys.tcp_round1 = TcpOutcome::Ok;
        sys.tls_real = vec![TlsRealOutcome::Http(200), TlsRealOutcome::Http(204)];
        let mut refr = addr(2, false, true);
        refr.tcp_round1 = TcpOutcome::Dead;
        refr.tcp_round2 = TcpOutcome::Dead;
        s.addrs = vec![sys, refr];
        s.sys_https = vec![SysPathOutcome::Blocked, SysPathOutcome::Blocked];
        assert_eq!(decide(&s), DiagnosisClass::Open);
    }

    #[test]
    fn sys_https_operational_is_open() {
        let mut s = base();
        s.sys_https = vec![
            SysPathOutcome::Operational,
            SysPathOutcome::Degraded(None),
        ];
        assert_eq!(decide(&s), DiagnosisClass::Open);
    }

    #[test]
    fn sys_https_contradiction_is_not_open() {
        // One pass says alive, the repeat says blocked: disagreement of
        // repeats must not yield a confident class (§1.1 rule 1).
        let mut s = base();
        s.sys_https = vec![SysPathOutcome::Operational, SysPathOutcome::Blocked];
        assert_eq!(decide(&s), DiagnosisClass::Unknown);
    }

    #[test]
    fn post_cert_killed_alone_is_unknown_not_open_not_server_down() {
        // Review counterexample: death after a valid certificate with
        // no HTTP response is indistinguishable from post-cert DPI
        // cutting — it proves neither Open nor ServerDown.
        let mut s = base();
        let mut a = addr(1, true, false);
        a.tcp_round1 = TcpOutcome::Ok;
        a.tls_real = vec![
            TlsRealOutcome::PostCertKilled,
            TlsRealOutcome::PostCertKilled,
        ];
        s.addrs = vec![a];
        s.ref_evidence_grade = false; // partial mode: Dns unreachable too
        assert_eq!(decide(&s), DiagnosisClass::Unknown);
    }

    // --- Dns -------------------------------------------------------------

    #[test]
    fn dead_sys_live_evidence_ref_is_dns() {
        // Task counterexample: system path dead + reference alive +
        // evidence-grade = Dns.
        let mut s = base();
        let mut sys = addr(1, true, false);
        sys.tcp_round1 = TcpOutcome::Ok;
        sys.tls_real = vec![
            TlsRealOutcome::CertInvalid,
            TlsRealOutcome::CertInvalid,
        ];
        let mut refr = addr(2, false, true);
        refr.tcp_round1 = TcpOutcome::Ok;
        refr.tls_real = vec![TlsRealOutcome::Http(200), TlsRealOutcome::Http(200)];
        s.addrs = vec![sys, refr];
        assert_eq!(decide(&s), DiagnosisClass::Dns);
    }

    #[test]
    fn dns_needs_live_ref_leg() {
        // Same broken system side, but the reference path is dead too:
        // "DNS is bad AND the site is blocked" must not become Dns.
        let mut s = base();
        let mut sys = addr(1, true, false);
        sys.tcp_round1 = TcpOutcome::Ok;
        sys.tls_real = vec![
            TlsRealOutcome::CertInvalid,
            TlsRealOutcome::CertInvalid,
        ];
        let mut refr = addr(2, false, true);
        refr.tcp_round1 = TcpOutcome::Dead;
        refr.tcp_round2 = TcpOutcome::Dead;
        s.addrs = vec![sys, refr];
        // sys tcp is Ok so Ip cannot fire either; Sni needs KilledPreCert.
        assert_eq!(decide(&s), DiagnosisClass::Unknown);
    }

    #[test]
    fn dns_unreachable_in_partial_mode() {
        // Plain-UDP "reference" is not evidence (§2.2): the very same
        // signal picture as the Dns case must not produce Dns.
        let mut s = base();
        s.ref_evidence_grade = false;
        s.ref_corroborated = false;
        let mut sys = addr(1, true, false);
        sys.tcp_round1 = TcpOutcome::Dead;
        sys.tcp_round2 = TcpOutcome::Dead;
        let mut refr = addr(2, false, false); // UDP member, not evidence
        refr.tcp_round1 = TcpOutcome::Ok;
        refr.tls_real = vec![TlsRealOutcome::Http(200), TlsRealOutcome::Http(200)];
        s.addrs = vec![sys, refr];
        // Not Dns. Not Ip (ref addr tcp ok). Not Open (sys addr not
        // http-alive; sys_https blocked). Falls to Unknown.
        assert_eq!(decide(&s), DiagnosisClass::Unknown);
    }

    #[test]
    fn dns_sys_resolution_empty_with_live_ref_is_dns() {
        // System resolver returns nothing at all; reference reaches the
        // genuine server. Leg (a) holds vacuously over the empty set.
        let mut s = base();
        let mut refr = AddrSignals::new(ip(9));
        refr.in_ref = true;
        refr.tcp_round1 = TcpOutcome::Ok;
        refr.tls_real = vec![TlsRealOutcome::Http(200), TlsRealOutcome::Http(301)];
        s.addrs = vec![refr];
        assert_eq!(decide(&s), DiagnosisClass::Dns);
    }

    // --- Sni -------------------------------------------------------------

    #[test]
    fn sni_contrast_both_rounds() {
        let mut s = base();
        let mut a = addr(1, true, true);
        a.tcp_round1 = TcpOutcome::Ok;
        a.tls_real = vec![
            TlsRealOutcome::KilledPreCert,
            TlsRealOutcome::Timeout,
        ];
        a.tls_foreign = vec![ForeignOutcome::Survived, ForeignOutcome::Survived];
        s.addrs = vec![a];
        assert_eq!(decide(&s), DiagnosisClass::Sni);
    }

    #[test]
    fn both_tls_killed_is_unknown_blind_zone() {
        // Task counterexample: real AND foreign killed (strict-SNI
        // hosting) — the contrast is unreachable, honesty is Unknown.
        let mut s = base();
        let mut a = addr(1, true, true);
        a.tcp_round1 = TcpOutcome::Ok;
        a.tls_real = vec![
            TlsRealOutcome::KilledPreCert,
            TlsRealOutcome::KilledPreCert,
        ];
        a.tls_foreign = vec![
            ForeignOutcome::KilledPreCert,
            ForeignOutcome::KilledPreCert,
        ];
        s.addrs = vec![a];
        assert_eq!(decide(&s), DiagnosisClass::Unknown);
    }

    #[test]
    fn sni_needs_neutral_control() {
        // Without a live ctrl_neutral the foreign leg is invalid this
        // cycle (§1.2): same picture, no Sni.
        let mut s = base();
        s.ctrl_neutral_alive = false;
        let mut a = addr(1, true, true);
        a.tcp_round1 = TcpOutcome::Ok;
        a.tls_real = vec![
            TlsRealOutcome::KilledPreCert,
            TlsRealOutcome::KilledPreCert,
        ];
        a.tls_foreign = vec![ForeignOutcome::Survived, ForeignOutcome::Survived];
        s.addrs = vec![a];
        assert_eq!(decide(&s), DiagnosisClass::Unknown);
    }

    #[test]
    fn sni_needs_two_agreeing_rounds() {
        // Foreign survived only once: no class on a single measurement.
        let mut s = base();
        let mut a = addr(1, true, true);
        a.tcp_round1 = TcpOutcome::Ok;
        a.tls_real = vec![
            TlsRealOutcome::KilledPreCert,
            TlsRealOutcome::KilledPreCert,
        ];
        a.tls_foreign = vec![ForeignOutcome::Survived];
        s.addrs = vec![a];
        assert_eq!(decide(&s), DiagnosisClass::Unknown);
    }

    // --- Ip --------------------------------------------------------------

    #[test]
    fn whole_union_dead_two_rounds_is_ip() {
        let mut s = base();
        let mut a = addr(1, true, false);
        a.tcp_round1 = TcpOutcome::Dead;
        a.tcp_round2 = TcpOutcome::Refused;
        let mut b = addr(2, false, true);
        b.tcp_round1 = TcpOutcome::Refused;
        b.tcp_round2 = TcpOutcome::Dead;
        s.addrs = vec![a, b];
        assert_eq!(decide(&s), DiagnosisClass::Ip);
    }

    #[test]
    fn ip_needs_second_round() {
        // One dead round is a transient, not a class (§1.3 row 4).
        let mut s = base();
        let mut a = addr(1, true, false);
        a.tcp_round1 = TcpOutcome::Dead;
        a.tcp_round2 = TcpOutcome::NotMeasured;
        s.addrs = vec![a];
        assert_eq!(decide(&s), DiagnosisClass::Unknown);
    }

    #[test]
    fn ip_needs_whole_union_dead() {
        let mut s = base();
        let mut a = addr(1, true, false);
        a.tcp_round1 = TcpOutcome::Dead;
        a.tcp_round2 = TcpOutcome::Dead;
        let mut b = addr(2, false, true);
        b.tcp_round1 = TcpOutcome::Dead;
        b.tcp_round2 = TcpOutcome::Ok; // came back in round 2
        s.addrs = vec![a, b];
        assert_eq!(decide(&s), DiagnosisClass::Unknown);
    }

    // --- ServerDown ------------------------------------------------------

    #[test]
    fn authenticated_5xx_twice_is_server_down() {
        let mut s = base();
        let mut a = addr(1, true, false);
        a.tcp_round1 = TcpOutcome::Ok;
        a.tls_real = vec![TlsRealOutcome::Http(503), TlsRealOutcome::Http(500)];
        s.addrs = vec![a];
        assert_eq!(decide(&s), DiagnosisClass::ServerDown);
    }

    #[test]
    fn five_xx_with_contradictory_repeat_is_unknown() {
        // One 5xx and one timeout: ServerDown demands both repeats
        // (§1.3 row 5), Open demands unanimous HTTP (§1.1 rule 1).
        // Neither is met — the honest outcome is Unknown.
        let mut s = base();
        let mut a = addr(1, true, false);
        a.tcp_round1 = TcpOutcome::Ok;
        a.tls_real = vec![TlsRealOutcome::Http(503), TlsRealOutcome::Timeout];
        s.addrs = vec![a];
        assert_eq!(decide(&s), DiagnosisClass::Unknown);
    }

    #[test]
    fn server_down_via_sys_https_statuses() {
        // Live scene S5 shape: the through path also reports Degraded,
        // but with 5xx statuses in both passes — that must not be
        // swallowed by Open's through-path leg.
        let mut s = base();
        s.sys_https = vec![
            SysPathOutcome::Degraded(Some(503)),
            SysPathOutcome::Degraded(Some(502)),
        ];
        let mut a = addr(1, true, false);
        a.tcp_round1 = TcpOutcome::Ok;
        a.tls_real = vec![TlsRealOutcome::Http(503), TlsRealOutcome::Http(503)];
        s.addrs = vec![a];
        assert_eq!(decide(&s), DiagnosisClass::ServerDown);
    }

    // --- Degraded reference ----------------------------------------------

    #[test]
    fn doh_operator_contradiction_disables_dns_class() {
        // Task counterexample: two DoH operators contradict each other
        // (disjoint sets, both channels alive) — the reference is not
        // trusted, evidence_grade=false, Dns unreachable even with a
        // textbook Dns picture otherwise.
        let mut s = base();
        s.ref_evidence_grade = false;
        s.ref_corroborated = false;
        let mut sys = addr(1, true, false);
        sys.tcp_round1 = TcpOutcome::Ok;
        sys.tls_real = vec![TlsRealOutcome::Timeout, TlsRealOutcome::Timeout];
        let mut refr = addr(2, false, false); // untrusted: not evidence members
        refr.tcp_round1 = TcpOutcome::Ok;
        refr.tls_real = vec![TlsRealOutcome::Http(200), TlsRealOutcome::Http(200)];
        s.addrs = vec![sys, refr];
        assert_eq!(decide(&s), DiagnosisClass::Unknown);
    }

    #[test]
    fn empty_signals_are_unknown() {
        let mut s = base();
        s.sys_https = vec![SysPathOutcome::NotMeasured];
        assert_eq!(decide(&s), DiagnosisClass::Unknown);
    }
}

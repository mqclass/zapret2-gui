//! nftables wiring for the Linux engine (NFQUEUE capture).
//!
//! On Windows the engine's root capture is WinDivert, driven by the
//! `--wf-tcp-out` / `--wf-udp-out` (winws2) and `--wf-tcp` / `--wf-udp`
//! (winws) root arguments the supervisor's builders emit. nfqws2 does
//! not know those arguments at all (verified against the pinned v1.0.6
//! binary: `--wf-tcp-out` is an "unrecognized option"); its capture is
//! an NFQUEUE fed by nftables rules. This module is the translation
//! layer:
//!
//! 1. [`rewrite_args_for_nfqws2`] strips the `--wf-*` root arguments
//!    from a built argument list, remembers the exact port sets they
//!    carried, and puts `--qnum=<queue>` in their place.
//! 2. [`build_nft_commands`] is the pure translation from those port
//!    sets to `nft` command lines (our own table `inet zapret2`, one
//!    output chain at mangle priority): every destination port in a
//!    wf set is queued to the engine, nothing else is. The port sets
//!    therefore can never capture wider than the Windows capture the
//!    builders asked for — the translation is argued token by token in
//!    the unit tests below.
//! 3. [`apply_rules`] / [`remove_rules`] execute the translation by
//!    invoking the `nft` binary with argument arrays (never a shell),
//!    resolving `nft` from the fixed sbin/bin directories so a writable
//!    PATH entry cannot shadow it for the root service.
//!
//! The `bypass` verdict on every queue rule is deliberate: if the
//! engine is not listening (dead, restarting), packets pass through
//! untouched instead of being dropped by a stale rule. Rule syntax
//! note: the verdict is spelled `queue num <n> bypass` — the
//! seemingly equivalent `queue to <n> bypass` is a syntax error in
//! nft 1.0.9 (verified against the real binary), because flags may
//! not follow the legacy `to` form.
//!
//! Queue number: 200, the same default zapret uses upstream. It is a
//! system-wide resource, so [`apply_rules`] refuses to install when a
//! foreign listener already holds the queue (checked via
//! /proc/net/nfnetlink_queue) instead of fighting over it.

use std::fs;
use std::io;
use std::process::Command;

use thiserror::Error;

/// NFQUEUE number the engine binds and the rules queue to.
pub const NFQUEUE_NUM: u16 = 200;

/// Our private nftables table (family `inet`: IPv4 + IPv6 together,
/// matching the wf filters, which are address-family agnostic).
pub const NFT_TABLE: &str = "zapret2";
const NFT_FAMILY: &str = "inet";
const NFT_CHAIN: &str = "output";

#[derive(Debug, Error)]
pub enum NftError {
    #[error("не удалось запустить nft: {0}")]
    SpawnFailed(#[source] io::Error),

    #[error("nft отклонил правила (код {status}): {stderr}")]
    Rejected { status: String, stderr: String },

    #[error("очередь NFQUEUE {0} уже занята другим слушателем (не zapret2)")]
    QueueBusy(u16),
}

/// Destination port sets extracted from the root `--wf-*` arguments.
/// Elements are single ports ("443") or ranges ("19294-19344"), in the
/// order the builders wrote them, deduplicated.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WfPortSets {
    pub tcp: Vec<String>,
    pub udp: Vec<String>,
}

impl WfPortSets {
    pub fn is_empty(&self) -> bool {
        self.tcp.is_empty() && self.udp.is_empty()
    }
}

/// One port-spec token is a port or an inclusive range, every part in
/// 1..=65535 (port 0 is not a destination port and is rejected).
/// Anything else (a stray token from hand-edited custom arguments) is
/// dropped: it must never reach the `nft` command line.
fn valid_port_token(token: &str) -> bool {
    let valid_part = |p: &str| {
        !p.is_empty()
            && p.len() <= 5
            && p.bytes().all(|b| b.is_ascii_digit())
            && p.parse::<u32>().is_ok_and(|v| (1..=65535).contains(&v))
    };
    match token.split_once('-') {
        None => valid_part(token),
        Some((lo, hi)) => {
            valid_part(lo)
                && valid_part(hi)
                && lo.parse::<u32>().unwrap_or(0) <= hi.parse::<u32>().unwrap_or(0)
        }
    }
}

fn merge_spec(sets: &mut Vec<String>, spec: &str) {
    for token in spec.split(',') {
        let token = token.trim();
        if valid_port_token(token) && !sets.iter().any(|t| t == token) {
            sets.push(token.to_string());
        }
    }
}

/// Scans an argument list for the root capture filters and merges
/// every port spec they carry. Recognizes both spellings:
/// `--wf-tcp-out=` / `--wf-udp-out=` (winws2 builders) and
/// `--wf-tcp=` / `--wf-udp=` (winws/Flowseal builders).
pub fn extract_wf_port_sets(args: &[String]) -> WfPortSets {
    let mut sets = WfPortSets::default();
    for arg in args {
        if let Some(spec) = arg
            .strip_prefix("--wf-tcp-out=")
            .or_else(|| arg.strip_prefix("--wf-tcp="))
        {
            merge_spec(&mut sets.tcp, spec);
        } else if let Some(spec) = arg
            .strip_prefix("--wf-udp-out=")
            .or_else(|| arg.strip_prefix("--wf-udp="))
        {
            merge_spec(&mut sets.udp, spec);
        }
    }
    sets
}

/// Result of rewriting a Windows-shaped argument list for nfqws2.
#[derive(Debug)]
pub struct RewrittenArgs {
    /// The argument list to actually spawn: no `--wf-*` anywhere,
    /// exactly one `--qnum`.
    pub args: Vec<String>,
    /// Port sets the removed `--wf-*` arguments carried (empty when
    /// the launch had no root filters, e.g. hand-written custom args).
    pub port_sets: WfPortSets,
    /// The queue the rules must target: the first parsable
    /// caller-supplied `--qnum` from the original list wins over our
    /// [`NFQUEUE_NUM`] default.
    pub queue_num: u16,
}

/// Strips the root `--wf-*` arguments nfqws2 would reject and makes
/// sure exactly one `--qnum` reaches the engine. If the list already
/// carries `--qnum` arguments, the first parsable one is kept (in
/// place) as the only one and its value is reported back so the rules
/// target the same queue; every other `--qnum` token — duplicates and
/// unparsable values alike — is dropped. If no parsable `--qnum` is
/// present, ours ([`NFQUEUE_NUM`]) is inserted where the first removed
/// filter stood (root parameters belong before the first section,
/// where the wf filters used to be).
pub fn rewrite_args_for_nfqws2(args: Vec<String>) -> RewrittenArgs {
    let port_sets = extract_wf_port_sets(&args);

    let mut out: Vec<String> = Vec::with_capacity(args.len() + 1);
    let mut insert_at: Option<usize> = None;
    let mut queue_num: Option<u16> = None;
    for arg in args {
        if arg.starts_with("--wf-") {
            if insert_at.is_none() {
                insert_at = Some(out.len());
            }
            continue;
        }
        if let Some(value) = arg.strip_prefix("--qnum=") {
            if queue_num.is_none() {
                if let Ok(n) = value.parse::<u16>() {
                    queue_num = Some(n);
                    out.push(arg);
                }
            }
            continue;
        }
        out.push(arg);
    }

    let queue_num = match queue_num {
        Some(n) => n,
        None => {
            let pos = insert_at.unwrap_or(0).min(out.len());
            out.insert(pos, format!("--qnum={}", NFQUEUE_NUM));
            NFQUEUE_NUM
        }
    };

    RewrittenArgs {
        args: out,
        port_sets,
        queue_num,
    }
}

/// nft set literal for a port list: `{ 80, 443, 19294-19344 }`.
/// Ranges keep their `a-b` spelling, which nft accepts inside sets.
fn port_set_expr(ports: &[String]) -> String {
    format!("{{ {} }}", ports.join(", "))
}

/// Pure translation: the full `nft` command sequence (argv arrays) that
/// installs capture for `sets` into our table. Table creation is NOT
/// included — see [`apply_rules`], which deletes any stale table
/// first; the sequence here assumes the table can be created fresh.
pub fn build_install_commands(sets: &WfPortSets, queue_num: u16) -> Vec<Vec<String>> {
    let mut cmds: Vec<Vec<String>> = Vec::new();
    let cmd = |parts: &[&str]| -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    };
    cmds.push(cmd(&["add", "table", NFT_FAMILY, NFT_TABLE]));
    cmds.push(cmd(&[
        "add",
        "chain",
        NFT_FAMILY,
        NFT_TABLE,
        NFT_CHAIN,
        "{",
        "type",
        "filter",
        "hook",
        "output",
        "priority",
        "mangle",
        ";",
        "policy",
        "accept",
        ";",
        "}",
    ]));
    if !sets.tcp.is_empty() {
        cmds.push(vec![
            "add".into(),
            "rule".into(),
            NFT_FAMILY.into(),
            NFT_TABLE.into(),
            NFT_CHAIN.into(),
            "tcp".into(),
            "dport".into(),
            port_set_expr(&sets.tcp),
            "queue".into(),
            "num".into(),
            queue_num.to_string(),
            "bypass".into(),
        ]);
    }
    if !sets.udp.is_empty() {
        cmds.push(vec![
            "add".into(),
            "rule".into(),
            NFT_FAMILY.into(),
            NFT_TABLE.into(),
            NFT_CHAIN.into(),
            "udp".into(),
            "dport".into(),
            port_set_expr(&sets.udp),
            "queue".into(),
            "num".into(),
            queue_num.to_string(),
            "bypass".into(),
        ]);
    }
    cmds
}

/// The single command that removes our whole table (and with it every
/// rule we installed). Deleting a missing table is an nft error the
/// callers deliberately tolerate.
pub fn build_delete_commands() -> Vec<Vec<String>> {
    vec![vec![
        "delete".into(),
        "table".into(),
        NFT_FAMILY.into(),
        NFT_TABLE.into(),
    ]]
}

/// Resolves the `nft` binary from fixed system directories (network
/// tools live in sbin). PATH is never consulted: the caller is a root
/// service, and a writable PATH entry shadowing `nft` would be a
/// privilege-escalation vector (same posture as platform::system32,
/// widened to the sbin directories where nft actually installs).
fn nft_binary() -> std::path::PathBuf {
    for dir in ["/usr/sbin", "/sbin", "/usr/bin", "/bin"] {
        let p = std::path::Path::new(dir).join("nft");
        if p.is_file() {
            return p;
        }
    }
    std::path::PathBuf::from("/usr/sbin/nft")
}

fn run_nft(argv: &[String]) -> Result<(), NftError> {
    let output = Command::new(nft_binary())
        .args(argv)
        .output()
        .map_err(NftError::SpawnFailed)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(NftError::Rejected {
            status: output.status.to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }
}

/// True when some process already listens on NFQUEUE `queue_num`,
/// per /proc/net/nfnetlink_queue. A missing file means the nfnetlink
/// queue subsystem has no bound queues at all (module not loaded), so
/// the queue is free. Our own engine never runs at the moment this is
/// checked (rules are applied before spawn), so any listener found is
/// foreign or a stale engine the caller chose not to clean up.
fn queue_is_busy(queue_num: u16) -> bool {
    let Ok(content) = fs::read_to_string("/proc/net/nfnetlink_queue") else {
        return false;
    };
    content.lines().any(|line| {
        line.split_whitespace()
            .next()
            .and_then(|n| n.parse::<u16>().ok())
            == Some(queue_num)
    })
}

/// (Re)installs capture rules for `sets`: any stale table of ours is
/// deleted first (a crashed svc leaves its table behind; the name is
/// ours alone, so deleting it can never touch foreign rules), then the
/// fresh ruleset is added. An empty `sets` just performs the cleanup —
/// the launch then captures nothing, exactly what its missing wf
/// filters said.
///
/// On any nft failure the half-installed table is deleted again and
/// the error is returned: the caller must refuse the engine start
/// rather than run an engine that captures half the intended traffic.
pub fn apply_rules(sets: &WfPortSets, queue_num: u16) -> Result<(), NftError> {
    // Clean our own stale state first; absence is the normal case.
    let _ = remove_rules();
    if sets.is_empty() {
        return Ok(());
    }
    if queue_is_busy(queue_num) {
        return Err(NftError::QueueBusy(queue_num));
    }
    for argv in build_install_commands(sets, queue_num) {
        if let Err(e) = run_nft(&argv) {
            let _ = remove_rules();
            return Err(e);
        }
    }
    Ok(())
}

/// Removes our table if it exists. Idempotent: a missing table is
/// success (the desired end state is already reached).
pub fn remove_rules() -> Result<(), NftError> {
    for argv in build_delete_commands() {
        match run_nft(&argv) {
            Ok(()) => {}
            // nft phrases a missing table as "No such file or
            // directory"; that is the state we want, not a failure.
            Err(NftError::Rejected { stderr, .. })
                if stderr.contains("No such file or directory") => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    /// The exact root-filter pair the winws2 builders emit for FULL:
    /// the nft port sets must equal these token for token, or capture
    /// on Linux would differ from Windows.
    #[test]
    fn extract_full_lua_and_ultimate_sets() {
        let args = v(&[
            "--debug=1",
            "--wf-tcp-out=80,443,2053,2083,2087,2096,8443",
            "--wf-udp-out=443,19294-19344,50000-65535",
            "--filter-tcp=443",
        ]);
        let sets = extract_wf_port_sets(&args);
        assert_eq!(
            sets.tcp,
            v(&["80", "443", "2053", "2083", "2087", "2096", "8443"])
        );
        assert_eq!(sets.udp, v(&["443", "19294-19344", "50000-65535"]));
    }

    /// The Flowseal builders use the spelling without `-out`.
    #[test]
    fn extract_flowseal_spelling() {
        let args = v(&[
            "--wf-tcp=80,443,2053,2083,2087,2096,8443",
            "--wf-udp=443,19294-19344,50000-65535",
        ]);
        let sets = extract_wf_port_sets(&args);
        assert_eq!(sets.tcp.len(), 7);
        assert_eq!(sets.udp.len(), 3);
    }

    /// GAME widens the root filters by the game spec (default
    /// 1024-65535): the union must show up in the capture sets.
    #[test]
    fn extract_game_widened_sets() {
        let args = v(&[
            "--wf-tcp-out=80,443,2053,2083,2087,2096,8443,1024-65535",
            "--wf-udp-out=443,19294-19344,50000-65535,1024-65535",
        ]);
        let sets = extract_wf_port_sets(&args);
        assert!(sets.tcp.contains(&"1024-65535".to_string()));
        assert!(sets.udp.contains(&"1024-65535".to_string()));
        assert_eq!(sets.tcp.len(), 8);
        assert_eq!(sets.udp.len(), 4);
    }

    /// Junk tokens from hand-edited custom args are dropped, never
    /// passed to nft; duplicates merge.
    #[test]
    fn extract_drops_junk_and_dedupes() {
        let args = v(&[
            "--wf-tcp-out=443,443,99999,80;rm,-5,70000-80000,abc,8443",
            "--wf-tcp-out=2053",
        ]);
        let sets = extract_wf_port_sets(&args);
        assert_eq!(sets.tcp, v(&["443", "8443", "2053"]));
    }

    /// Port 0 is outside the promised 1..=65535: rejected as a single
    /// port and as either bound of a range.
    #[test]
    fn extract_drops_port_zero() {
        assert!(!valid_port_token("0"));
        assert!(!valid_port_token("0-100"));
        assert!(!valid_port_token("100-0"));
        assert!(valid_port_token("1"));
        assert!(valid_port_token("1-65535"));
        let args = v(&["--wf-tcp-out=0,0-100,443", "--wf-udp-out=0"]);
        let sets = extract_wf_port_sets(&args);
        assert_eq!(sets.tcp, v(&["443"]));
        assert!(sets.udp.is_empty());
    }

    #[test]
    fn rewrite_strips_wf_and_inserts_qnum_at_filter_position() {
        let args = v(&[
            "--debug=1",
            "--wf-tcp-out=80,443",
            "--wf-udp-out=443",
            "--blob=discord_udp:@ACTIVE_DISCORD_UDP.bin",
            "--new=quic",
            "--filter-udp=443",
        ]);
        let out = rewrite_args_for_nfqws2(args);
        assert!(!out.args.iter().any(|a| a.starts_with("--wf-")));
        assert_eq!(
            out.args.iter().filter(|a| a.starts_with("--qnum=")).count(),
            1
        );
        assert_eq!(out.args[1], "--qnum=200");
        assert_eq!(out.queue_num, 200);
        assert_eq!(out.port_sets.tcp, v(&["80", "443"]));
        // Section arguments pass through untouched.
        assert!(out.args.iter().any(|a| a == "--new=quic"));
    }

    #[test]
    fn rewrite_keeps_caller_qnum() {
        let args = v(&["--qnum=300", "--wf-tcp-out=443", "--filter-tcp=443"]);
        let out = rewrite_args_for_nfqws2(args);
        assert_eq!(out.queue_num, 300);
        assert!(out.args.iter().any(|a| a == "--qnum=300"));
        assert!(!out.args.iter().any(|a| a == "--qnum=200"));
    }

    /// Duplicated hand-written `--qnum` arguments must not reach
    /// nfqws2: the first parsable one wins and is the only survivor;
    /// later duplicates and unparsable values are dropped.
    #[test]
    fn rewrite_dedupes_caller_qnum_first_parsable_wins() {
        let args = v(&[
            "--qnum=oops",
            "--wf-tcp-out=443",
            "--qnum=300",
            "--qnum=400",
            "--filter-tcp=443",
        ]);
        let out = rewrite_args_for_nfqws2(args);
        assert_eq!(out.queue_num, 300);
        let qnums: Vec<&String> = out
            .args
            .iter()
            .filter(|a| a.starts_with("--qnum="))
            .collect();
        assert_eq!(qnums, vec!["--qnum=300"]);
    }

    /// An unparsable `--qnum` does not count as caller-supplied: it is
    /// dropped and our default is inserted instead.
    #[test]
    fn rewrite_unparsable_qnum_falls_back_to_default() {
        let args = v(&["--qnum=oops", "--filter-tcp=443"]);
        let out = rewrite_args_for_nfqws2(args);
        assert_eq!(out.queue_num, 200);
        let qnums: Vec<&String> = out
            .args
            .iter()
            .filter(|a| a.starts_with("--qnum="))
            .collect();
        assert_eq!(qnums, vec!["--qnum=200"]);
    }

    #[test]
    fn rewrite_without_filters_still_pins_default_queue() {
        let args = v(&["--filter-tcp=443", "--lua-desync=fake"]);
        let out = rewrite_args_for_nfqws2(args);
        assert!(out.port_sets.is_empty());
        assert_eq!(out.args[0], "--qnum=200");
    }

    #[test]
    fn install_commands_shape() {
        let sets = WfPortSets {
            tcp: v(&["80", "443"]),
            udp: v(&["443", "50000-65535"]),
        };
        let cmds = build_install_commands(&sets, 200);
        assert_eq!(cmds.len(), 4);
        assert_eq!(cmds[0], v(&["add", "table", "inet", "zapret2"]));
        assert!(cmds[1].contains(&"mangle".to_string()));
        assert!(cmds[1].contains(&"output".to_string()));
        let tcp_rule = &cmds[2];
        assert_eq!(tcp_rule[5], "tcp");
        assert_eq!(tcp_rule[7], "{ 80, 443 }");
        assert_eq!(tcp_rule[8], "queue");
        assert_eq!(tcp_rule[9], "num");
        assert_eq!(tcp_rule[10], "200");
        assert_eq!(tcp_rule[11], "bypass");
        let udp_rule = &cmds[3];
        assert_eq!(udp_rule[5], "udp");
        assert_eq!(udp_rule[7], "{ 443, 50000-65535 }");
    }

    #[test]
    fn install_commands_empty_sets_create_no_rules() {
        let cmds = build_install_commands(&WfPortSets::default(), 200);
        assert_eq!(cmds.len(), 2); // table + chain only
        assert!(!cmds.iter().flatten().any(|a| a == "rule"));
    }

    #[test]
    fn delete_commands_remove_whole_table() {
        assert_eq!(
            build_delete_commands(),
            vec![v(&["delete", "table", "inet", "zapret2"])]
        );
    }
}

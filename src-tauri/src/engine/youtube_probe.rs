//! YouTube throttling: node host validation and cluster helpers.
//!
//! Some providers throttle *specific* googlevideo CDN nodes instead of
//! blocking YouTube outright (Flowseal/zapret-discord-youtube issues
//! #18725, #18271): the site opens, but the video stream from the unlucky
//! node crawls at ~17 KB/s. No desync strategy can fix that — the
//! connection is honest, its throughput is policed. The community
//! workaround is to point the throttled node hostnames at 127.0.0.2 in
//! hosts so the player fails fast and moves to a healthy node.
//!
//! This module owns the safety rules for that workaround:
//! - Only hostnames of the exact shape `rr<digits>---sn-<cluster>.googlevideo.com`
//!   may ever be blocked. No apex domains, no lookalikes.
//! - A throttled verdict for one node expands to its whole cluster
//!   (rr1..rr20), which is how the community lists are built.
//!
//! Below the pure helpers sits the network detector: it downloads a
//! public video page, pulls real `videoplayback` URLs out of
//! `ytInitialPlayerResponse`, range-downloads ~3 MiB per unique node and
//! classifies the sustained throughput. It only ever runs on the user's
//! machine, on explicit user request — throttling is a property of the
//! user's provider route, so measuring from anywhere else is pointless.
//! Honest outcomes: when YouTube does not hand out stream data (bot
//! wall, consent wall, datacenter IP), the result is `Unknown` and
//! nothing is blocked. Since 2026-11-12, unreachable nodes are blocked
//! too — but only when the same check found a live node (Dom.ru case,
//! Flowseal #18730: foreign GGC cut off, the local cluster fine); a
//! Fast+Unreachable mix is reported as `Mixed`, never as Fast.

use std::collections::HashSet;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// True when `host` is a googlevideo *node* hostname of the exact shape
/// `rr<digits>---sn-<cluster>.googlevideo.com`, where the cluster part is
/// non-empty lowercase alphanumerics and hyphens. This is the guard from
/// the feature design (`^rr\d+---sn-[a-z0-9-]+\.googlevideo\.com$`),
/// hand-rolled to avoid a regex dependency. Callers pass lowercase hosts
/// (list parsing and URL extraction both normalize).
pub fn is_googlevideo_node_host(host: &str) -> bool {
    let Some(rest) = host.strip_suffix(".googlevideo.com") else {
        return false;
    };
    let Some(rr) = rest.strip_prefix("rr") else {
        return false;
    };
    let Some((digits, cluster)) = rr.split_once("---sn-") else {
        return false;
    };
    !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && !cluster.is_empty()
        && cluster
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Returns the cluster identity of a node host: the `sn-<cluster>` part
/// shared by every rrN of the same cache cluster. `None` for hosts that
/// fail the guard.
pub fn cluster_prefix(host: &str) -> Option<String> {
    if !is_googlevideo_node_host(host) {
        return None;
    }
    let rest = host.strip_suffix(".googlevideo.com")?;
    let idx = rest.find("---")?;
    Some(rest[idx + 3..].to_string())
}

/// Expands a cluster prefix (`sn-<cluster>`, as returned by
/// [`cluster_prefix`]) to the full rr1..rr20 hostname set — the community
/// method: throttling is a property of the cluster, not of one server.
pub fn expand_cluster_prefix(cluster: &str) -> Vec<String> {
    (1..=20)
        .map(|i| format!("rr{}---{}.googlevideo.com", i, cluster))
        .collect()
}

/// Expands one node host to its whole cluster (rr1..rr20 including
/// itself). Empty for hosts that fail the guard.
pub fn expand_cluster(host: &str) -> Vec<String> {
    match cluster_prefix(host) {
        Some(cluster) => expand_cluster_prefix(&cluster),
        None => Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Detector
// ---------------------------------------------------------------------------

/// Throughput threshold: below 1.5 Mbit/s a stream cannot hold even
/// 720p — that is "throttled" for our purposes. At or above is "normal"
/// (feature design section 2).
pub const THROTTLE_THRESHOLD_BPS: f64 = 1_500_000.0;

/// Range-request size per node: ~3 MiB is enough for a sustained-rate
/// sample without burning the user's traffic.
pub const PROBE_RANGE_BYTES: u64 = 3_145_728;

/// Samples smaller than this cannot support a verdict either way.
const MIN_SAMPLE_BYTES: u64 = 64 * 1024;

/// At most this many unique nodes are measured per check.
pub const MAX_PROBE_HOSTS: usize = 6;

/// Per-host wall clock cap. A throttled stream at ~17 KB/s delivers
/// ~425 KB in this window — comfortably above the minimum sample.
const PER_HOST_TIMEOUT: Duration = Duration::from_secs(25);

/// Overall budget for one check (~3 minutes per the design), counted
/// from the very start of the check: the report_mapping lookup, the
/// watch-page fetches and every node probe share this one deadline, so
/// the worst case stays at 3 minutes, not 3 minutes plus page fetches.
const OVERALL_BUDGET: Duration = Duration::from_secs(180);

/// Time left until `deadline`, zero once it has passed.
fn remaining_until(deadline: Instant) -> Duration {
    deadline.saturating_duration_since(Instant::now())
}

/// Long-lived public videos used as stream sources. Any single one may
/// rot or get walled; the check tries them in order and reports
/// `Unknown` when none yields streaming data.
const TEST_VIDEO_IDS: &[&str] = &["aqz-KE-bpKQ", "dQw4w9WgXcQ", "LXb3EKWsInQ"];

const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

/// Per-node verdict of a throughput measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeVerdict {
    Fast,
    Throttled,
    Unreachable,
    Unknown,
    /// Aggregate-only: some nodes fast, some unreachable — never
    /// assigned to a single node (see [`aggregate`]). Added 2026-11-12
    /// so a check that found live nodes next to dead ones is not
    /// reported as Fast: the player still hangs on the dead ones.
    Mixed,
}

/// One measured node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YoutubeNodeProbe {
    pub host: String,
    pub verdict: NodeVerdict,
    /// Sustained throughput in Mbit/s (0 when nothing was downloaded).
    pub mbps: f64,
    pub bytes: u64,
}

/// Full result of one user-initiated check, as returned to the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YoutubeCheckResult {
    /// Aggregate verdict: Throttled if any node throttles; else Mixed
    /// when live nodes sit next to unreachable ones; else Fast if any
    /// node measured fast, else Unreachable / Unknown.
    pub outcome: NodeVerdict,
    pub nodes: Vec<YoutubeNodeProbe>,
    /// Node name from redirector.googlevideo.com/report_mapping —
    /// diagnostics only, never influences blocking.
    pub mapping_node: Option<String>,
    /// Whether the hosts lane toggle is on.
    pub lane_enabled: bool,
    /// Whether smart hosts is on (the lane lives in its section).
    pub smart_hosts_enabled: bool,
    /// Whether this check re-applied the hosts section.
    pub applied: bool,
    /// Lane entries in effect after the check (or last known count).
    pub blocked_now: usize,
    /// Hosts refresh failure while applying, if any.
    pub apply_error: Option<String>,
}

/// Classifies one throughput sample. Pure: the detector's verdict table
/// is unit-tested on synthetic samples, including the documented
/// real-world one (344064 bytes in 20 s ≈ 17 KB/s → Throttled).
pub fn classify_sample(bytes: u64, elapsed_secs: f64, network_failed: bool) -> NodeVerdict {
    if network_failed {
        return NodeVerdict::Unreachable;
    }
    if bytes < MIN_SAMPLE_BYTES || elapsed_secs <= 0.0 {
        return NodeVerdict::Unknown;
    }
    let bps = bytes as f64 * 8.0 / elapsed_secs;
    if bps < THROTTLE_THRESHOLD_BPS {
        NodeVerdict::Throttled
    } else {
        NodeVerdict::Fast
    }
}

/// Parses `redirector.googlevideo.com/report_mapping` output. Live
/// format (verified 2026-11-10): `<ip> => <node> (<prefix>)`, possibly
/// among other lines. Returns the assigned node name, diagnostics only.
pub fn parse_report_mapping(text: &str) -> Option<String> {
    for line in text.lines() {
        let Some((_, rest)) = line.split_once("=>") else {
            continue;
        };
        let node = rest
            .trim()
            .split(|c: char| c.is_whitespace() || c == '(')
            .next()
            .unwrap_or("");
        if !node.is_empty() {
            return Some(node.to_string());
        }
    }
    None
}

/// Extracts `videoplayback` URLs from a watch page's HTML.
/// `ytInitialPlayerResponse` embeds them with escaped slashes
/// (`https:\/\/…`) and `\u0026` separators; normalize first, then scan
/// for the marker. Junk and overlong candidates are dropped.
pub fn extract_videoplayback_urls(html: &str) -> Vec<String> {
    let normalized = html.replace("\\/", "/").replace("\\u0026", "&");
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for (idx, _) in normalized.match_indices("googlevideo.com/videoplayback") {
        let Some(start) = normalized[..idx].rfind("https://") else {
            continue;
        };
        let tail = &normalized[idx..];
        let end = tail
            .find(|c: char| c == '"' || c == '\'' || c == '<' || c.is_whitespace())
            .map(|e| idx + e)
            .unwrap_or(normalized.len());
        let url = &normalized[start..end];
        if url.len() <= 4096 && seen.insert(url.to_string()) {
            out.push(url.to_string());
        }
    }
    out
}

/// Host part of an https URL, without a URL-parser dependency.
fn url_host(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let host = rest.split('/').next()?;
    let host = host.split(':').next()?; // strip :port if present
    if host.is_empty() {
        None
    } else {
        Some(host.to_lowercase())
    }
}

/// Unique guard-valid node hosts with one sample URL each, in page
/// order, capped at [`MAX_PROBE_HOSTS`].
pub fn unique_node_urls(urls: &[String]) -> Vec<(String, String)> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<(String, String)> = Vec::new();
    for url in urls {
        let Some(host) = url_host(url) else { continue };
        if !is_googlevideo_node_host(&host) {
            continue;
        }
        if seen.insert(host.clone()) {
            out.push((host, url.clone()));
            if out.len() >= MAX_PROBE_HOSTS {
                break;
            }
        }
    }
    out
}

/// Merges fresh measurements into the stored detector records.
/// The latest verdict for a cluster replaces older records of the same
/// cluster in BOTH lists (a cluster is never recorded fast and
/// throttled at once — the same rule the lane resolver applies), and
/// records past the lane TTL are pruned.
///
/// Since 2026-11-12 an Unreachable host is recorded as throttled too —
/// but only when this same check found at least one Fast node: a dead
/// node next to a live one is a node the provider cut off, and blocking
/// it lets the player fail fast onto the live ones (Dom.ru, Flowseal
/// #18730: foreign GGC unreachable, the Yekaterinburg cluster fine).
/// When nothing measured fast — the whole check failed, or YouTube is
/// fully unreachable — Unreachable still changes nothing: blocking
/// everything would leave the player with nowhere to go. Unknown
/// never changes anything.
pub fn merge_measurements(
    throttled: &mut Vec<crate::platform::settings::YoutubeNodeRecord>,
    fast: &mut Vec<crate::platform::settings::YoutubeNodeRecord>,
    measured: &[(String, NodeVerdict)],
    now: u64,
) {
    use crate::platform::settings::YoutubeNodeRecord;
    let ttl = crate::platform::smart_hosts::YOUTUBE_NODE_TTL_SECS;
    throttled.retain(|r| now.saturating_sub(r.measured_at) < ttl);
    fast.retain(|r| now.saturating_sub(r.measured_at) < ttl);
    // Guard for the Unreachable rule, computed once from the whole
    // check so record order cannot flip it. Only guard-valid node
    // hosts count: a Fast verdict on a non-node host is not evidence
    // of a live node the player could fail over to.

    let any_fast = measured
        .iter()
        .any(|(h, v)| *v == NodeVerdict::Fast && cluster_prefix(h).is_some());
    for (host, verdict) in measured {
        let Some(cluster) = cluster_prefix(host) else {
            continue;
        };
        // An Unreachable host counts as throttled exactly when this
        // check found a live node somewhere (see the doc comment
        // above): same list, same TTL, same rr1..rr20 cluster
        // expansion by the lane resolver as a Throttled verdict.
        let verdict = match verdict {
            NodeVerdict::Unreachable if any_fast => NodeVerdict::Throttled,
            other => *other,
        };
        match verdict {
            NodeVerdict::Throttled => {
                fast.retain(|r| cluster_prefix(&r.host).as_deref() != Some(cluster.as_str()));
                throttled.retain(|r| r.host != *host);
                throttled.push(YoutubeNodeRecord {
                    host: host.clone(),
                    measured_at: now,
                });
            }
            NodeVerdict::Fast => {
                throttled.retain(|r| cluster_prefix(&r.host).as_deref() != Some(cluster.as_str()));
                fast.retain(|r| r.host != *host);
                fast.push(YoutubeNodeRecord {
                    host: host.clone(),
                    measured_at: now,
                });
            }
            _ => {}
        }
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

static PROBE_CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();

fn probe_client() -> &'static reqwest::Client {
    PROBE_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(PER_HOST_TIMEOUT)
            .connect_timeout(Duration::from_secs(6))
            .user_agent(BROWSER_UA)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

/// Best-effort report_mapping lookup; failure is silent (diagnostics).
/// Bounded by the check's overall deadline like every other stage.
async fn fetch_mapping_node(client: &reqwest::Client, deadline: Instant) -> Option<String> {
    let fetch = async {
        let resp = client
            .get("https://redirector.googlevideo.com/report_mapping")
            .send()
            .await
            .ok()?;
        resp.text().await.ok()
    };
    let text = tokio::time::timeout(remaining_until(deadline), fetch).await.ok()??;
    parse_report_mapping(&text)
}

/// Downloads one watch page and extracts its videoplayback URLs.
/// `None` on any failure (bot wall, consent wall, no streamingData) —
/// the caller moves on to the next test video. Bounded by the check's
/// overall deadline like every other stage.
async fn fetch_watch_urls(
    client: &reqwest::Client,
    video_id: &str,
    deadline: Instant,
) -> Option<Vec<String>> {
    let url = format!("https://www.youtube.com/watch?v={}", video_id);
    let fetch = async {
        let resp = client
            .get(&url)
            .header("Accept-Language", "ru-RU,ru;q=0.9,en;q=0.8")
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let html = resp.text().await.ok()?;
        let urls = extract_videoplayback_urls(&html);
        if urls.is_empty() {
            None
        } else {
            Some(urls)
        }
    };
    tokio::time::timeout(remaining_until(deadline), fetch)
        .await
        .ok()?
}

/// Decides whether a failed body read condemns the whole probe.
///
/// The per-host client timeout and a throttled node's drip feed race
/// each other: at the canonical ~17 KB/s the client timeout fires
/// mid-body with a perfectly good slow sample already in hand
/// (>= MIN_SAMPLE_BYTES). Such a sample is judged on its numbers via
/// [`classify_sample`], not written off as a network failure — writing
/// it off would mark the exact nodes this detector exists to find as
/// Unreachable and never record them. A failure before the minimum
/// sample says nothing about throughput: that one stays fatal.
fn read_failure_is_fatal(bytes: u64, read_failed: bool) -> bool {
    read_failed && bytes < MIN_SAMPLE_BYTES
}

/// Measures one node's sustained throughput with a ~3 MiB range
/// request against the original signed URL (hosts are never rewritten:
/// the signature is not known to survive that, and the design forbids
/// it). The verdict is computed from the bytes that actually arrived.
/// The measurement window ends at the earlier of the per-host cap and
/// the check's overall deadline.
async fn probe_host(client: &reqwest::Client, url: &str, deadline: Instant) -> YoutubeNodeProbe {
    let fallback_host = url_host(url).unwrap_or_default();
    let started = Instant::now();
    let limit = (started + PER_HOST_TIMEOUT).min(deadline);
    let send = client
        .get(url)
        .header("Range", format!("bytes=0-{}", PROBE_RANGE_BYTES - 1))
        .send();
    let mut resp = match tokio::time::timeout(remaining_until(limit), send).await {
        Ok(Ok(r)) => r,
        _ => {
            return YoutubeNodeProbe {
                host: fallback_host,
                verdict: NodeVerdict::Unreachable,
                mbps: 0.0,
                bytes: 0,
            };
        }
    };
    let status = resp.status();
    if status != reqwest::StatusCode::OK && status != reqwest::StatusCode::PARTIAL_CONTENT {
        return YoutubeNodeProbe {
            host: fallback_host,
            verdict: NodeVerdict::Unreachable,
            mbps: 0.0,
            bytes: 0,
        };
    }
    // The node that actually served the stream (after any redirect) is
    // the one the verdict belongs to.
    let host = resp
        .url()
        .host_str()
        .map(|h| h.to_lowercase())
        .filter(|h| is_googlevideo_node_host(h))
        .unwrap_or(fallback_host);

    let mut bytes: u64 = 0;
    let mut read_failed = false;
    while bytes < PROBE_RANGE_BYTES && Instant::now() < limit {
        match tokio::time::timeout(Duration::from_secs(5), resp.chunk()).await {
            Ok(Ok(Some(chunk))) => bytes += chunk.len() as u64,
            Ok(Ok(None)) => break, // clean EOF
            Ok(Err(_)) => {
                read_failed = true;
                break;
            }
            Err(_) => break, // 5 s without data: stalled; judge the sample
        }
    }
    let elapsed = started.elapsed().as_secs_f64();
    // A read error with a usable sample in hand (the client timeout
    // killing a drip feed) is judged by the sample, not as a failure.
    let verdict = classify_sample(bytes, elapsed, read_failure_is_fatal(bytes, read_failed));
    let mbps = if elapsed > 0.0 {
        bytes as f64 * 8.0 / elapsed / 1_000_000.0
    } else {
        0.0
    };
    YoutubeNodeProbe {
        host,
        verdict,
        mbps,
        bytes,
    }
}

fn aggregate(nodes: &[YoutubeNodeProbe]) -> NodeVerdict {
    if nodes.iter().any(|n| n.verdict == NodeVerdict::Throttled) {
        NodeVerdict::Throttled
    } else if nodes.iter().any(|n| n.verdict == NodeVerdict::Fast)
        && nodes.iter().any(|n| n.verdict == NodeVerdict::Unreachable)
    {
        // Live nodes next to dead ones: the player hangs on the dead
        // ones, so reporting Fast would be a lie.
        NodeVerdict::Mixed
    } else if nodes.iter().any(|n| n.verdict == NodeVerdict::Fast) {
        NodeVerdict::Fast
    } else if nodes.iter().any(|n| n.verdict == NodeVerdict::Unreachable) {
        NodeVerdict::Unreachable
    } else {
        NodeVerdict::Unknown
    }
}

/// Runs the full user-initiated check: fetch a video page, measure each
/// unique node, persist the verdicts, and — when the lane and smart
/// hosts are both on — re-apply the hosts section so fresh blocks take
/// effect immediately. With the lane off the check is diagnostic only:
/// measurements are recorded, hosts is never touched.
pub async fn run_youtube_throttle_check() -> YoutubeCheckResult {
    let settings = crate::platform::settings::load_settings();
    let lane_enabled = settings.youtube_throttle_enabled;
    let smart_hosts_enabled = settings.smart_hosts_enabled;
    let mut result = YoutubeCheckResult {
        outcome: NodeVerdict::Unknown,
        nodes: Vec::new(),
        mapping_node: None,
        lane_enabled,
        smart_hosts_enabled,
        applied: false,
        blocked_now: settings.youtube_throttle_blocked_count,
        apply_error: None,
    };

    let client = probe_client();
    // One deadline for the WHOLE check, from the very start: mapping,
    // page fetches and probes all share it. When it runs out mid-check
    // the result is honestly partial — the nodes measured so far, or
    // Unknown when nothing could be measured at all.
    let deadline = Instant::now() + OVERALL_BUDGET;
    result.mapping_node = fetch_mapping_node(client, deadline).await;

    let mut page_urls: Vec<String> = Vec::new();
    for id in TEST_VIDEO_IDS {
        if Instant::now() >= deadline {
            break;
        }
        if let Some(urls) = fetch_watch_urls(client, id, deadline).await {
            page_urls = urls;
            break;
        }
    }
    if page_urls.is_empty() {
        // Honest Unknown: YouTube gave us no stream data to measure.
        return result;
    }

    // Sequential on purpose: parallel downloads would distort each
    // other's throughput and the verdicts with it.
    let mut nodes: Vec<YoutubeNodeProbe> = Vec::new();
    for (_, url) in unique_node_urls(&page_urls) {
        if Instant::now() >= deadline {
            break;
        }
        nodes.push(probe_host(client, &url, deadline).await);
    }
    result.outcome = aggregate(&nodes);
    result.nodes = nodes;

    let measured: Vec<(String, NodeVerdict)> = result
        .nodes
        .iter()
        .map(|n| (n.host.clone(), n.verdict))
        .collect();
    let now = unix_now();
    if let Err(e) = crate::platform::settings::update_settings(|s| {
        merge_measurements(
            &mut s.youtube_throttled_nodes,
            &mut s.youtube_fast_nodes,
            &measured,
            now,
        );
    }) {
        eprintln!("youtube-probe: failed to persist measurements: {}", e);
    }

    if lane_enabled && smart_hosts_enabled {
        match crate::platform::smart_hosts::refresh_smart_hosts().await {
            Ok(st) => {
                result.applied = true;
                result.blocked_now = st.youtube_blocked_count;
            }
            Err(e) => {
                result.apply_error = Some(e);
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_accepts_node_hosts() {
        assert!(is_googlevideo_node_host(
            "rr7---sn-gvnuxaxjvh-n8ml.googlevideo.com"
        ));
        assert!(is_googlevideo_node_host("rr1---sn-aj5goc-5j.googlevideo.com"));
        assert!(is_googlevideo_node_host(
            "rr20---sn-gvnuxaxjvh-c35d.googlevideo.com"
        ));
    }

    #[test]
    fn guard_rejects_everything_else() {
        // Apex and ordinary service domains.
        assert!(!is_googlevideo_node_host("googlevideo.com"));
        assert!(!is_googlevideo_node_host("youtube.com"));
        assert!(!is_googlevideo_node_host("www.youtube.com"));
        // Lookalikes: our suffix must be a real dot-boundary suffix.
        assert!(!is_googlevideo_node_host(
            "rr1---sn-x.evil-googlevideo.com"
        ));
        assert!(!is_googlevideo_node_host(
            "rr1---sn-x.googlevideo.com.evil.com"
        ));
        assert!(!is_googlevideo_node_host("evilgooglevideo.com"));
        // Shape violations.
        assert!(!is_googlevideo_node_host("sn-x.googlevideo.com"));
        assert!(!is_googlevideo_node_host("rr---sn-x.googlevideo.com"));
        assert!(!is_googlevideo_node_host("rrx---sn-x.googlevideo.com"));
        assert!(!is_googlevideo_node_host("rr1---sn-.googlevideo.com"));
        assert!(!is_googlevideo_node_host(
            "rr1---sn-UPPER.googlevideo.com"
        ));
        assert!(!is_googlevideo_node_host(""));
    }

    #[test]
    fn cluster_roundtrip() {
        let host = "rr8---sn-gvnuxaxjvh-c35d.googlevideo.com";
        let cluster = cluster_prefix(host).unwrap();
        assert_eq!(cluster, "sn-gvnuxaxjvh-c35d");
        let all = expand_cluster(host);
        assert_eq!(all.len(), 20);
        assert_eq!(all[0], "rr1---sn-gvnuxaxjvh-c35d.googlevideo.com");
        assert_eq!(all[19], "rr20---sn-gvnuxaxjvh-c35d.googlevideo.com");
        assert!(all.iter().all(|h| is_googlevideo_node_host(h)));
        assert!(expand_cluster("youtube.com").is_empty());
        assert!(cluster_prefix("youtube.com").is_none());
    }

    #[test]
    fn classify_sample_verdicts() {
        // The documented real-world sample (Flowseal #18725): 344064
        // bytes in 20 s ≈ 17 KB/s — the stream is throttled to a crawl.
        assert_eq!(classify_sample(344_064, 20.0, false), NodeVerdict::Throttled);
        // Full 3 MiB range in 4 s ≈ 6.3 Mbit/s — healthy.
        assert_eq!(classify_sample(3_145_728, 4.0, false), NodeVerdict::Fast);
        // Same payload stretched over 20 s ≈ 1.26 Mbit/s — under the bar.
        assert_eq!(classify_sample(3_145_728, 20.0, false), NodeVerdict::Throttled);
        // Exactly at the 1.5 Mbit/s threshold counts as normal.
        assert_eq!(classify_sample(1_500_000 / 8 * 10, 10.0, false), NodeVerdict::Fast);
        // Transport failure is Unreachable regardless of the numbers.
        assert_eq!(classify_sample(0, 20.0, true), NodeVerdict::Unreachable);
        assert_eq!(classify_sample(3_145_728, 4.0, true), NodeVerdict::Unreachable);
        // A tiny clean sample cannot support a verdict.
        assert_eq!(classify_sample(10_000, 1.0, false), NodeVerdict::Unknown);
        assert_eq!(classify_sample(0, 1.0, false), NodeVerdict::Unknown);
    }

    #[test]
    fn read_failure_with_usable_sample_is_judged_by_numbers() {
        // The canonical drip feed: ~425 KB in 25 s (≈17 KB/s) and then
        // the per-host client timeout kills the read. The sample is
        // usable, so the verdict comes from its numbers: Throttled,
        // never Unreachable — otherwise the detector would write off
        // exactly the nodes it exists to record.
        assert!(!read_failure_is_fatal(425_000, true));
        assert_eq!(
            classify_sample(425_000, 25.0, read_failure_is_fatal(425_000, true)),
            NodeVerdict::Throttled
        );
        // A connection that delivered a fast full range and dropped at
        // the very end still judges Fast.
        assert_eq!(
            classify_sample(3_145_728, 4.0, read_failure_is_fatal(3_145_728, true)),
            NodeVerdict::Fast
        );
        // A failure before the minimum sample says nothing about
        // throughput and stays a network failure.
        assert!(read_failure_is_fatal(10_000, true));
        assert_eq!(
            classify_sample(10_000, 3.0, read_failure_is_fatal(10_000, true)),
            NodeVerdict::Unreachable
        );
        // Clean EOF is never a failure, whatever arrived.
        assert!(!read_failure_is_fatal(0, false));
        assert!(!read_failure_is_fatal(3_145_728, false));
    }

    #[test]
    fn report_mapping_parses() {
        let live = "203.0.113.7 => rr1---sn-gvnuxaxjvh-n8ml.googlevideo.com (203.0.113.0/24)\n";
        assert_eq!(
            parse_report_mapping(live).as_deref(),
            Some("rr1---sn-gvnuxaxjvh-n8ml.googlevideo.com")
        );
        // Node token may be a bare cache name; extra lines are skipped.
        assert_eq!(
            parse_report_mapping("junk line\n10.0.0.1 => cache07 (10.0.0.0/8)\n").as_deref(),
            Some("cache07")
        );
        assert_eq!(parse_report_mapping("no mapping here"), None);
        assert_eq!(parse_report_mapping(""), None);
        assert_eq!(parse_report_mapping("1.2.3.4 => \n"), None);
    }

    #[test]
    fn extract_and_unique_node_urls() {
        // Synthetic ytInitialPlayerResponse fragment: escaped slashes
        // and \u0026 separators, two formats on one node, one on
        // another, plus an unrelated googlevideo path that must be
        // ignored (not videoplayback).
        let html = r#"{"streamingData":{"adaptiveFormats":[{"url":"https:\/\/rr3---sn-aaaa-bb.googlevideo.com\/videoplayback?expire=1\u0026itag=137"},{"url":"https:\/\/rr3---sn-aaaa-bb.googlevideo.com\/videoplayback?expire=1\u0026itag=140"},{"url":"https:\/\/rr9---sn-cccc-dd.googlevideo.com\/videoplayback?expire=1\u0026itag=137"}],"other":"https:\/\/rr1---sn-aaaa-bb.googlevideo.com\/generate_204"}"#;
        let urls = extract_videoplayback_urls(html);
        assert_eq!(urls.len(), 3);
        let nodes = unique_node_urls(&urls);
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].0, "rr3---sn-aaaa-bb.googlevideo.com");
        assert_eq!(nodes[1].0, "rr9---sn-cccc-dd.googlevideo.com");
        // Non-googlevideo videoplayback URLs never pass the guard.
        let foreign = vec!["https://rr3---sn-aaaa-bb.example.com/videoplayback?x=1".to_string()];
        assert!(unique_node_urls(&foreign).is_empty());
    }

    #[test]
    fn merge_measurements_latest_verdict_wins() {
        use crate::platform::settings::YoutubeNodeRecord;
        let now = 1_000_000;
        let rec = |host: &str, ts: u64| YoutubeNodeRecord {
            host: host.to_string(),
            measured_at: ts,
        };
        let mut throttled = vec![rec("rr3---sn-aaaa-bb.googlevideo.com", now - 100)];
        let mut fast = vec![rec("rr1---sn-old-zz.googlevideo.com", 10)]; // long expired

        // A fresh Fast for a cluster-mate clears the throttled record.
        merge_measurements(
            &mut throttled,
            &mut fast,
            &[("rr9---sn-aaaa-bb.googlevideo.com".to_string(), NodeVerdict::Fast)],
            now,
        );
        assert!(throttled.is_empty());
        assert_eq!(fast.len(), 1); // expired one pruned, fresh one kept
        assert_eq!(fast[0].host, "rr9---sn-aaaa-bb.googlevideo.com");

        // A fresh Throttled clears the fast record the same way.
        merge_measurements(
            &mut throttled,
            &mut fast,
            &[("rr9---sn-aaaa-bb.googlevideo.com".to_string(), NodeVerdict::Throttled)],
            now,
        );
        assert!(fast.is_empty());
        assert_eq!(throttled.len(), 1);

        // Unreachable changes nothing when no node in the same check
        // measured fast; non-node hosts are ignored.
        merge_measurements(
            &mut throttled,
            &mut fast,
            &[
                ("rr9---sn-aaaa-bb.googlevideo.com".to_string(), NodeVerdict::Unreachable),
                ("youtube.com".to_string(), NodeVerdict::Throttled),
            ],
            now,
        );
        assert_eq!(throttled.len(), 1);
        assert!(fast.is_empty());
    }

    #[test]
    fn merge_measurements_blocks_unreachable_next_to_fast() {
        use crate::platform::settings::YoutubeNodeRecord;
        let now = 1_000_000;
        let rec = |host: &str, ts: u64| YoutubeNodeRecord {
            host: host.to_string(),
            measured_at: ts,
        };
        // The dead cluster was previously recorded fast: an Unreachable
        // verdict next to a live node must clear that, exactly like a
        // Throttled verdict would.
        let mut throttled = Vec::new();
        let mut fast = vec![rec("rr5---sn-dead-bb.googlevideo.com", now - 100)];

        merge_measurements(
            &mut throttled,
            &mut fast,
            &[
                ("rr1---sn-live-aa.googlevideo.com".to_string(), NodeVerdict::Fast),
                ("rr2---sn-dead-bb.googlevideo.com".to_string(), NodeVerdict::Unreachable),
            ],
            now,
        );
        assert_eq!(throttled.len(), 1);
        assert_eq!(throttled[0].host, "rr2---sn-dead-bb.googlevideo.com");
        assert_eq!(fast.len(), 1);
        assert_eq!(fast[0].host, "rr1---sn-live-aa.googlevideo.com");

        // Same cluster, latest verdict wins (measurement order): a
        // Fast host followed by an Unreachable host of the same
        // cluster ends throttled.
        merge_measurements(
            &mut throttled,
            &mut fast,
            &[
                ("rr1---sn-same-cc.googlevideo.com".to_string(), NodeVerdict::Fast),
                ("rr2---sn-same-cc.googlevideo.com".to_string(), NodeVerdict::Unreachable),
            ],
            now,
        );
        assert!(fast
            .iter()
            .all(|r| cluster_prefix(&r.host).as_deref() != Some("sn-same-cc")));
        assert_eq!(
            throttled
                .iter()
                .filter(|r| cluster_prefix(&r.host).as_deref() == Some("sn-same-cc"))
                .count(),
            1
        );
    }

    #[test]
    fn merge_measurements_unreachable_before_fast_still_blocked() {
        use crate::platform::settings::YoutubeNodeRecord;
        let now = 1_000_000;
        // Order inside the check does not matter: the guard is
        // computed from the whole check before the loop, so an
        // Unreachable listed before the Fast node is blocked all the
        // same.
        let mut throttled: Vec<YoutubeNodeRecord> = Vec::new();
        let mut fast: Vec<YoutubeNodeRecord> = Vec::new();
        merge_measurements(
            &mut throttled,
            &mut fast,
            &[
                ("rr7---sn-dead-dd.googlevideo.com".to_string(), NodeVerdict::Unreachable),
                ("rr8---sn-live-ee.googlevideo.com".to_string(), NodeVerdict::Fast),
            ],
            now,
        );
        assert_eq!(throttled.len(), 1);
        assert_eq!(throttled[0].host, "rr7---sn-dead-dd.googlevideo.com");
        assert_eq!(fast.len(), 1);
        assert_eq!(fast[0].host, "rr8---sn-live-ee.googlevideo.com");
    }

    #[test]
    fn merge_measurements_fast_on_non_node_host_does_not_arm_guard() {
        use crate::platform::settings::YoutubeNodeRecord;
        let now = 1_000_000;
        // A Fast verdict on a host that fails the node guard is not
        // evidence of a live node: it must not arm the Unreachable
        // rule (regression test for the guard-validity check).
        let mut throttled: Vec<YoutubeNodeRecord> = Vec::new();
        let mut fast: Vec<YoutubeNodeRecord> = Vec::new();
        merge_measurements(
            &mut throttled,
            &mut fast,
            &[
                ("youtube.com".to_string(), NodeVerdict::Fast),
                ("rr9---sn-dead-ff.googlevideo.com".to_string(), NodeVerdict::Unreachable),
            ],
            now,
        );
        assert!(throttled.is_empty());
        assert!(fast.is_empty());
    }

    #[test]
    fn merge_measurements_ignores_mixed_input() {
        use crate::platform::settings::YoutubeNodeRecord;
        let now = 1_000_000;
        // Mixed is aggregate-only and never reaches merge from the
        // detector; if it ever did, it must be a no-op, not a block.
        let mut throttled: Vec<YoutubeNodeRecord> = Vec::new();
        let mut fast: Vec<YoutubeNodeRecord> = Vec::new();
        merge_measurements(
            &mut throttled,
            &mut fast,
            &[
                ("rr1---sn-mix-aa.googlevideo.com".to_string(), NodeVerdict::Mixed),
                ("rr2---sn-live-bb.googlevideo.com".to_string(), NodeVerdict::Fast),
            ],
            now,
        );
        assert!(throttled.is_empty());
        assert_eq!(fast.len(), 1);
        assert_eq!(fast[0].host, "rr2---sn-live-bb.googlevideo.com");
    }

    #[test]
    fn merge_measurements_unreachable_alone_changes_nothing() {
        use crate::platform::settings::YoutubeNodeRecord;
        let now = 1_000_000;
        let rec = |host: &str, ts: u64| YoutubeNodeRecord {
            host: host.to_string(),
            measured_at: ts,
        };
        let mut throttled = vec![rec("rr3---sn-old-aa.googlevideo.com", now - 100)];
        let mut fast = vec![rec("rr1---sn-old-bb.googlevideo.com", now - 100)];

        // All-unreachable check: the guard (no Fast in this check)
        // leaves both lists untouched.
        merge_measurements(
            &mut throttled,
            &mut fast,
            &[
                ("rr1---sn-dead-aa.googlevideo.com".to_string(), NodeVerdict::Unreachable),
                ("rr2---sn-dead-bb.googlevideo.com".to_string(), NodeVerdict::Unreachable),
                ("rr3---sn-dead-cc.googlevideo.com".to_string(), NodeVerdict::Unknown),
            ],
            now,
        );
        assert_eq!(throttled.len(), 1);
        assert_eq!(throttled[0].host, "rr3---sn-old-aa.googlevideo.com");
        assert_eq!(fast.len(), 1);
        assert_eq!(fast[0].host, "rr1---sn-old-bb.googlevideo.com");

        // Throttled + Unreachable with no Fast: the throttled host is
        // recorded as before, the unreachable one is not.
        merge_measurements(
            &mut throttled,
            &mut fast,
            &[
                ("rr4---sn-slow-dd.googlevideo.com".to_string(), NodeVerdict::Throttled),
                ("rr5---sn-dead-ee.googlevideo.com".to_string(), NodeVerdict::Unreachable),
            ],
            now,
        );
        assert_eq!(throttled.len(), 2);
        assert!(throttled
            .iter()
            .any(|r| r.host == "rr4---sn-slow-dd.googlevideo.com"));
        assert!(!throttled
            .iter()
            .any(|r| r.host == "rr5---sn-dead-ee.googlevideo.com"));
    }

    #[test]
    fn aggregate_reports_mixed() {
        let probe = |host: &str, verdict: NodeVerdict| YoutubeNodeProbe {
            host: host.to_string(),
            verdict,
            mbps: 0.0,
            bytes: 0,
        };
        // The Dom.ru shape: live nodes next to dead ones is Mixed,
        // never Fast.
        assert_eq!(
            aggregate(&[
                probe("rr1---sn-live-aa.googlevideo.com", NodeVerdict::Fast),
                probe("rr2---sn-dead-bb.googlevideo.com", NodeVerdict::Unreachable),
            ]),
            NodeVerdict::Mixed
        );
        // One throttled node still dominates, even with dead ones.
        assert_eq!(
            aggregate(&[
                probe("rr1---sn-live-aa.googlevideo.com", NodeVerdict::Fast),
                probe("rr2---sn-slow-bb.googlevideo.com", NodeVerdict::Throttled),
                probe("rr3---sn-dead-cc.googlevideo.com", NodeVerdict::Unreachable),
            ]),
            NodeVerdict::Throttled
        );
        // Pure outcomes are unchanged.
        assert_eq!(
            aggregate(&[probe("rr1---sn-live-aa.googlevideo.com", NodeVerdict::Fast)]),
            NodeVerdict::Fast
        );
        assert_eq!(
            aggregate(&[probe(
                "rr1---sn-dead-aa.googlevideo.com",
                NodeVerdict::Unreachable
            )]),
            NodeVerdict::Unreachable
        );
        // Unreachable next to Unknown is still Unreachable: the
        // failure is real, the Unknown node only adds no data.
        assert_eq!(
            aggregate(&[
                probe("rr1---sn-dead-aa.googlevideo.com", NodeVerdict::Unreachable),
                probe("rr2---sn-myst-bb.googlevideo.com", NodeVerdict::Unknown),
            ]),
            NodeVerdict::Unreachable
        );
        assert_eq!(aggregate(&[]), NodeVerdict::Unknown);
        // Unknown is "no data", not a failure: Fast + Unknown stays Fast.
        assert_eq!(
            aggregate(&[
                probe("rr1---sn-live-aa.googlevideo.com", NodeVerdict::Fast),
                probe("rr2---sn-myst-aa.googlevideo.com", NodeVerdict::Unknown),
            ]),
            NodeVerdict::Fast
        );
    }
}

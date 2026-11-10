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
//! nothing is blocked.

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

/// Overall budget for one check (~3 minutes per the design).
const OVERALL_BUDGET: Duration = Duration::from_secs(170);

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
    /// Aggregate verdict: Throttled if any node throttles, else Fast if
    /// any node measured fast, else Unreachable / Unknown.
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
/// records past the lane TTL are pruned. Unreachable/Unknown hosts
/// change nothing.
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
    for (host, verdict) in measured {
        let Some(cluster) = cluster_prefix(host) else {
            continue;
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
async fn fetch_mapping_node(client: &reqwest::Client) -> Option<String> {
    let text = client
        .get("https://redirector.googlevideo.com/report_mapping")
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()?;
    parse_report_mapping(&text)
}

/// Downloads one watch page and extracts its videoplayback URLs.
/// `None` on any failure (bot wall, consent wall, no streamingData) —
/// the caller moves on to the next test video.
async fn fetch_watch_urls(client: &reqwest::Client, video_id: &str) -> Option<Vec<String>> {
    let url = format!("https://www.youtube.com/watch?v={}", video_id);
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
}

/// Measures one node's sustained throughput with a ~3 MiB range
/// request against the original signed URL (hosts are never rewritten:
/// the signature is not known to survive that, and the design forbids
/// it). The verdict is computed from the bytes that actually arrived.
async fn probe_host(client: &reqwest::Client, url: &str) -> YoutubeNodeProbe {
    let fallback_host = url_host(url).unwrap_or_default();
    let started = Instant::now();
    let mut resp = match client
        .get(url)
        .header("Range", format!("bytes=0-{}", PROBE_RANGE_BYTES - 1))
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => {
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
    let mut network_failed = false;
    while bytes < PROBE_RANGE_BYTES && started.elapsed() < PER_HOST_TIMEOUT {
        match tokio::time::timeout(Duration::from_secs(5), resp.chunk()).await {
            Ok(Ok(Some(chunk))) => bytes += chunk.len() as u64,
            Ok(Ok(None)) => break, // clean EOF
            Ok(Err(_)) => {
                network_failed = true;
                break;
            }
            Err(_) => break, // 5 s without data: stalled; judge the sample
        }
    }
    let elapsed = started.elapsed().as_secs_f64();
    let verdict = classify_sample(bytes, elapsed, network_failed);
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
    result.mapping_node = fetch_mapping_node(client).await;

    let mut page_urls: Vec<String> = Vec::new();
    for id in TEST_VIDEO_IDS {
        if let Some(urls) = fetch_watch_urls(client, id).await {
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
    let deadline = Instant::now() + OVERALL_BUDGET;
    let mut nodes: Vec<YoutubeNodeProbe> = Vec::new();
    for (_, url) in unique_node_urls(&page_urls) {
        if Instant::now() > deadline {
            break;
        }
        nodes.push(probe_host(client, &url).await);
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

        // Unreachable changes nothing; non-node hosts are ignored.
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
}

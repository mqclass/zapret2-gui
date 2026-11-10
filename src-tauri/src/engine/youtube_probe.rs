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
//! The network detector that measures node throughput lives here too
//! (added with the user-facing check); everything in this file section is
//! pure and unit-tested.

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
}

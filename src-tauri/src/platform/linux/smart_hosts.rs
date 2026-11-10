//! Smart Hosts: daily-merged system hosts file from community sources.
//!
//! Some blocks happen at the DNS level (poisoned answers, blocked resolvers)
//! and some services block by client IP (sanctions). Community projects
//! maintain hosts files that map such domains to working IPs / SNI proxies:
//! - MALW.link (https://github.com/ImMALWARE/dns.malw.link)
//! - GeoHide (https://dns.geohide.ru:8443/)
//!
//! This module fetches those lists over HTTPS, merges them (first enabled
//! source wins on conflicts), and writes the result into the system hosts
//! file (/etc/hosts) inside a clearly marked section. The user's own
//! entries are never touched: only our marked section is added, replaced,
//! or removed.
//!
//! Writing /etc/hosts needs root: in the Linux layout this module runs in
//! zapret2-svc (systemd, root). From an unprivileged process the write
//! simply fails with a permission error, which is surfaced as-is.
//!
//! Safety rules:
//! - HTTPS only, standard TLS verification (reqwest + rustls).
//! - Every entry is validated: parseable IP, sane domain syntax.
//! - `localhost` / loopback mappings from sources are dropped — the system
//!   owns those.
//! - The original hosts file is backed up once before the first write.
//! - `0.0.0.0` adblock entries are only applied when the user opts in.
//! - Sanity limits on download size and entry count.

use std::collections::HashSet;
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::platform::settings::{load_settings, update_settings};

/// Built-in hosts source.
#[derive(Debug, Clone, Copy)]
pub struct HostSource {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub url: &'static str,
}

pub const HOST_SOURCES: &[HostSource] = &[
    HostSource {
        id: "malw",
        name: "MALW.link",
        description: "DNS + SNI-прокси: обход IP-блокировок сервисов (ChatGPT, Spotify, Twitch и др.)",
        url: "https://raw.githubusercontent.com/ImMALWARE/dns.malw.link/master/hosts",
    },
    HostSource {
        id: "geohide",
        name: "GeoHide",
        description: "Ежедневно обновляемый hosts: рабочие IP и прокси GeoHide для заблокированных сервисов",
        url: "https://dns.geohide.ru:8443/static/hosts/ru/hosts",
    },
];

/// One parsed `IP domain` mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostEntry {
    pub ip: IpAddr,
    pub domain: String,
}

/// Status snapshot for the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmartHostsStatus {
    pub enabled: bool,
    pub sources: Vec<SmartHostsSourceInfo>,
    pub include_adblock: bool,
    pub last_update: Option<u64>,
    pub entry_count: usize,
    /// Source problems from the last refresh that still applied (e.g.
    /// the YouTube lane carried it while community sources were down).
    /// `None` after a fully clean refresh — failures are never silent.
    pub last_error: Option<String>,
    pub section_present: bool,
    pub backup_present: bool,
    /// YouTube-throttle lane toggle (its blocks live in this section).
    pub youtube_throttle_enabled: bool,
    /// YouTube lane entries written by the last successful apply.
    pub youtube_blocked_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmartHostsSourceInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub enabled: bool,
}

// ---------------------------------------------------------------------------
// Paths & markers
// ---------------------------------------------------------------------------

const SECTION_BEGIN: &str = "# >>> ZAPRET2-GUI SMART HOSTS >>>";
const SECTION_END: &str = "# <<< ZAPRET2-GUI SMART HOSTS <<<";
const BACKUP_SUFFIX: &str = "hosts.zapret2-gui.bak";

/// Max download size per source (10 MiB) — sanity cap.
const MAX_DOWNLOAD_BYTES: u64 = 10 * 1024 * 1024;
/// Max merged entries — sanity cap (resolvers slow down on huge files).
const MAX_ENTRIES: usize = 60_000;
/// Refresh interval: 24 hours.
pub const REFRESH_INTERVAL_SECS: u64 = 24 * 3600;

fn system_hosts_path() -> PathBuf {
    PathBuf::from("/etc/hosts")
}

fn backup_path() -> PathBuf {
    system_hosts_path()
        .parent()
        .map(|p| p.join(BACKUP_SUFFIX))
        .unwrap_or_else(|| PathBuf::from(BACKUP_SUFFIX))
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Fetch & parse
// ---------------------------------------------------------------------------

static HTTP_CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();

fn http_client() -> &'static reqwest::Client {
    HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("zapret2-gui-smart-hosts/1.0")
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

/// Downloads one source. Only `https://` URLs are accepted.
pub async fn fetch_source(url: &str) -> Result<String, String> {
    if !url.starts_with("https://") {
        return Err("Отклонён не-HTTPS источник".to_string());
    }
    let resp = http_client()
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Не удалось скачать {}: {}", url, e))?;
    if !resp.status().is_success() {
        return Err(format!("Источник {} вернул HTTP {}", url, resp.status()));
    }
    let len = resp.content_length().unwrap_or(0);
    if len > MAX_DOWNLOAD_BYTES {
        return Err(format!("Источник {} слишком большой ({} байт)", url, len));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("Ошибка чтения {}: {}", url, e))?;
    if bytes.len() as u64 > MAX_DOWNLOAD_BYTES {
        return Err(format!("Источник {} слишком большой", url));
    }
    String::from_utf8(bytes.to_vec()).map_err(|_| format!("Источник {} не в UTF-8", url))
}

/// Rough domain sanity check: labels of alphanumerics/hyphens, dots between.
fn is_sane_domain(d: &str) -> bool {
    if d.is_empty() || d.len() > 253 || d.starts_with('.') || d.ends_with('.') || d.contains("..") {
        return false;
    }
    d.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            && !label.starts_with('-')
            && !label.ends_with('-')
    })
}

/// Parses classic hosts format: `IP domain [# comment]`, one or more domains
/// per line. Returns validated entries.
pub fn parse_hosts(text: &str) -> Vec<HostEntry> {
    let mut out = Vec::new();
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Strip inline comments.
        let line = match line.find('#') {
            Some(i) => line[..i].trim(),
            None => line,
        };
        let mut parts = line.split_whitespace();
        let ip_raw = match parts.next() {
            Some(p) => p,
            None => continue,
        };
        // Must be a parseable IP address; keep it parsed so blocking
        // checks below compare values, not strings ("::" vs "0:0:0:0:0:0:0:0").
        let ip: IpAddr = match ip_raw.parse() {
            Ok(ip) => ip,
            Err(_) => continue,
        };
        for domain in parts {
            let domain = domain.to_lowercase();
            if is_sane_domain(&domain) {
                out.push(HostEntry { ip, domain });
            }
        }
        if out.len() >= MAX_ENTRIES {
            break;
        }
    }
    out
}

/// True for IPs used to *block* a domain: unspecified (0.0.0.0 / :: in any
/// textual form) or loopback (127.0.0.0/8 / ::1). Sources use these for
/// their adblock sections.
fn is_blocking_ip(ip: &IpAddr) -> bool {
    ip.is_unspecified() || ip.is_loopback()
}

/// Merges parsed sources in priority order: the first source that maps a
/// domain wins. When `include_adblock` is false, blocking entries
/// (0.0.0.0, ::, 127.0.0.1, ::1) are dropped. `localhost` entries from
/// sources are always dropped.
pub fn merge_entries(sources: Vec<Vec<HostEntry>>, include_adblock: bool) -> Vec<HostEntry> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut merged = Vec::new();
    for entries in sources {
        for e in entries {
            if e.domain == "localhost" || e.domain == "ip6-localhost" {
                continue;
            }
            if !include_adblock && is_blocking_ip(&e.ip) {
                continue;
            }
            if seen.insert(e.domain.clone()) {
                merged.push(e);
            }
            if merged.len() >= MAX_ENTRIES {
                return merged;
            }
        }
    }
    merged
}


// ---------------------------------------------------------------------------
// YouTube lane: deliberate blocks of throttled googlevideo nodes
// ---------------------------------------------------------------------------

/// Raw URL of the throttled-nodes list in our own repository: the list is
/// data and must be updatable without an app release.
pub const YOUTUBE_NODES_URL: &str =
    "https://raw.githubusercontent.com/mqclass/zapret2-gui/main/lists/youtube-bad-nodes.txt";

/// Bundled copy of the same list, used when the download fails or parses
/// empty.
const YOUTUBE_NODES_BUNDLED: &str = include_str!("../../../../lists/youtube-bad-nodes.txt");

/// Hosts target for throttled nodes: a loopback address where nothing
/// listens, so the player gets a fast refusal and picks another node.
pub const YOUTUBE_BLOCK_IP: IpAddr = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2));

/// Detector records (throttled blocks and fast verdicts alike) expire
/// after 7 days without re-confirmation and drop out at the next apply.
pub const YOUTUBE_NODE_TTL_SECS: u64 = 7 * 24 * 3600;

/// Parses the throttled-nodes list: one hostname per line, `#` starts a
/// comment. Only guard-matching googlevideo node hosts survive — this
/// file is downloaded at runtime, and downloaded data must never be able
/// to point arbitrary domains at 127.0.0.2 through our hosts section.
pub fn parse_node_list(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for raw_line in text.lines() {
        let line = match raw_line.find('#') {
            Some(i) => &raw_line[..i],
            None => raw_line,
        };
        let host = line.trim().to_lowercase();
        if host.is_empty() {
            continue;
        }
        if crate::engine::youtube_probe::is_googlevideo_node_host(&host)
            && seen.insert(host.clone())
        {
            out.push(host);
        }
    }
    out
}

/// Downloads the throttled-nodes list; on any failure falls back to the
/// copy bundled at build time.
async fn fetch_youtube_list_hosts() -> Vec<String> {
    match fetch_source(YOUTUBE_NODES_URL).await {
        Ok(text) => {
            let hosts = parse_node_list(&text);
            if !hosts.is_empty() {
                return hosts;
            }
            eprintln!("smart-hosts: youtube node list parsed empty, using the bundled copy");
        }
        Err(e) => {
            eprintln!(
                "smart-hosts: youtube node list download failed ({}), using the bundled copy",
                e
            );
        }
    }
    parse_node_list(YOUTUBE_NODES_BUNDLED)
}

/// Everything the YouTube lane needs, pre-extracted from settings so the
/// resolution logic stays pure and unit-testable.
pub struct YoutubeLaneInputs {
    /// Hosts from the external/bundled list (guard-validated).
    pub list_hosts: Vec<String>,
    /// Detector-confirmed throttled records: (host, measured_at unix).
    pub throttled: Vec<(String, u64)>,
    /// Detector-measured fast records: (host, measured_at unix).
    pub fast: Vec<(String, u64)>,
}

/// Resolves the YouTube lane to hosts entries.
///
/// Rules (feature design section 1):
/// - Sources unite: detector-found clusters plus community list hosts.
/// - A local measurement outranks the list: the freshest non-expired
///   verdict per cluster wins, and a cluster measured fast is never
///   blocked, even when the list names it.
/// - A throttled verdict blocks the whole cluster (rr1..rr20).
/// - Records older than [`YOUTUBE_NODE_TTL_SECS`] are ignored.
pub fn resolve_youtube_lane(inputs: &YoutubeLaneInputs, now: u64) -> Vec<HostEntry> {
    use crate::engine::youtube_probe::{cluster_prefix, expand_cluster_prefix};
    use std::collections::HashMap;

    // Freshest verdict per cluster: cluster -> (throttled, measured_at).
    let mut verdicts: HashMap<String, (bool, u64)> = HashMap::new();
    let mut note = |host: &str, measured_at: u64, throttled: bool| {
        if now.saturating_sub(measured_at) >= YOUTUBE_NODE_TTL_SECS {
            return;
        }
        let Some(cluster) = cluster_prefix(host) else {
            return;
        };
        match verdicts.get(&cluster) {
            Some((_, ts)) if *ts > measured_at => {}
            _ => {
                verdicts.insert(cluster, (throttled, measured_at));
            }
        }
    };
    for (host, ts) in &inputs.throttled {
        note(host, *ts, true);
    }
    for (host, ts) in &inputs.fast {
        note(host, *ts, false);
    }

    let mut seen: HashSet<String> = HashSet::new();
    let mut hosts: Vec<String> = Vec::new();
    // Detector-throttled clusters: block rr1..rr20 (community method).
    for (cluster, (throttled, _)) in &verdicts {
        if *throttled {
            for h in expand_cluster_prefix(cluster) {
                if seen.insert(h.clone()) {
                    hosts.push(h);
                }
            }
        }
    }
    // The measured host itself always joins its cluster's expansion:
    // the expansion covers rr1..rr20, but the detector may have
    // measured e.g. rr123 — a host outside that range would otherwise
    // see its cluster blocked while the one node proven throttled
    // stays reachable. Only fresh records whose cluster verdict
    // stands (throttled) are added.
    for (host, ts) in &inputs.throttled {
        if now.saturating_sub(*ts) >= YOUTUBE_NODE_TTL_SECS {
            continue;
        }
        let Some(cluster) = cluster_prefix(host) else {
            continue;
        };
        if !matches!(verdicts.get(&cluster), Some((true, _))) {
            continue;
        }
        if seen.insert(host.clone()) {
            hosts.push(host.clone());
        }
    }
    // Community list, minus clusters this machine measured fast.
    for h in &inputs.list_hosts {
        let Some(cluster) = cluster_prefix(h) else {
            continue;
        };
        if matches!(verdicts.get(&cluster), Some((false, _))) {
            continue;
        }
        if seen.insert(h.clone()) {
            hosts.push(h.clone());
        }
    }

    hosts.sort();
    hosts
        .into_iter()
        .map(|domain| HostEntry {
            ip: YOUTUBE_BLOCK_IP,
            domain,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Apply / remove
// ---------------------------------------------------------------------------

/// Renders our marked section.
fn render_section(entries: &[HostEntry], last_update: u64) -> String {
    let mut s = String::new();
    s.push_str(&format!("{}\n", SECTION_BEGIN));
    s.push_str(&format!(
        "# Умный hosts от zapret2-gui. Не редактируйте вручную — обновляется автоматически.\n"
    ));
    s.push_str(&format!("# Обновлено: {}\n", last_update));
    for e in entries {
        s.push_str(&format!("{} {}\n", e.ip, e.domain));
    }
    s.push_str(&format!("{}\n", SECTION_END));
    s
}

/// Splits file content into (before_section, after_section), dropping any
/// existing marked section. END is searched only after BEGIN so a stray
/// marker elsewhere can't cause runaway duplication.
fn strip_section(content: &str) -> (String, String) {
    let begin = match content.find(SECTION_BEGIN) {
        Some(b) => b,
        None => return (content.to_string(), String::new()),
    };
    let end = match content[begin..].find(SECTION_END) {
        Some(e) => begin + e,
        None => return (content.to_string(), String::new()),
    };
    let after_end = end + SECTION_END.len();
    // Also swallow one trailing newline so we don't accumulate blanks.
    let after = content[after_end..]
        .strip_prefix("\r\n")
        .unwrap_or(
            content[after_end..]
                .strip_prefix('\n')
                .unwrap_or(&content[after_end..]),
        );
    (content[..begin].to_string(), after.to_string())
}

/// Decodes hosts bytes without loss. Hosts files on Russian Windows are
/// often UTF-8 or cp1251 (Notepad's "ANSI"); a lossy UTF-8 decode would
/// mangle the user's Cyrillic comments on the first smart-hosts write, so
/// fall back to a full cp1251 decode when strict UTF-8 fails.
fn decode_hosts_bytes(bytes: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    bytes.iter().map(|&b| cp1251_char(b)).collect()
}

fn cp1251_char(b: u8) -> char {
    match b {
        0x00..=0x7F => b as char,
        // А..я occupy 0xC0..=0xFF contiguously in cp1251.
        0xC0..=0xFF => char::from_u32(0x0410 + (b as u32 - 0xC0)).unwrap_or('\u{FFFD}'),
        _ => CP1251_HIGH[(b - 0x80) as usize],
    }
}

/// cp1251 mapping for bytes 0x80..=0xBF (punctuation and extra letters).
const CP1251_HIGH: [char; 64] = [
    'Ђ', 'Ѓ', '‚', 'ѓ', '„', '…', '†', '‡', '€', '‰', 'Љ', '‹', 'Њ', 'Ќ', 'Ћ', 'Џ',
    'ђ', '‘', '’', '“', '”', '•', '–', '—', '\u{FFFD}', '™', 'љ', '›', 'њ', 'ќ', 'ћ', 'џ',
    '\u{00A0}', 'Ў', 'ў', 'Ј', '¤', 'Ґ', '¦', '§', 'Ё', '©', 'Є', '«', '¬', '\u{00AD}', '®', 'Ї',
    '°', '±', 'І', 'і', 'ґ', 'µ', '¶', '·', 'ё', '№', 'є', '»', 'ј', 'Ѕ', 'ѕ', 'ї',
];

/// Reads the current hosts file; missing file is treated as empty.
/// Non-UTF8 content (e.g. cp1251 comments) is decoded, never mangled.
fn read_hosts_file() -> Result<String, String> {
    let path = system_hosts_path();
    match std::fs::read(&path) {
        Ok(bytes) => Ok(decode_hosts_bytes(&bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("Не удалось прочитать hosts: {}", e)),
    }
}

/// Writes content atomically: temp file + rename.
fn write_hosts_file(content: &str) -> Result<(), String> {
    let path = system_hosts_path();
    let tmp = path.with_extension("tmp.zapret2");
    {
        let mut f = std::fs::File::create(&tmp)
            .map_err(|e| format!("Не удалось записать hosts (нужны права root — запись выполняет служба zapret2-svc): {}", e))?;
        f.write_all(content.as_bytes())
            .map_err(|e| format!("Не удалось записать hosts: {}", e))?;
        f.sync_all()
            .map_err(|e| format!("Не удалось записать hosts: {}", e))?;
    }
    std::fs::rename(&tmp, &path).map_err(|e| format!("Не удалось заменить hosts: {}", e))?;
    Ok(())
}

/// Creates a backup of the pristine hosts file once (before our first write).
/// The backup stores the original BYTES: whatever encoding the file was in,
/// the recovery copy must be byte-identical, not a decoded/re-encoded text.
fn ensure_backup() -> Result<(), String> {
    let bp = backup_path();
    if bp.exists() {
        return Ok(());
    }
    let bytes = std::fs::read(system_hosts_path()).unwrap_or_default();
    std::fs::write(&bp, bytes).map_err(|e| format!("Не удалось создать бэкап hosts: {}", e))?;
    Ok(())
}

/// Flushes the systemd-resolved DNS cache (best-effort; absent resolved
/// means there is no system cache to flush).
fn flush_dns_cache() {
    let resolvectl = crate::platform::system32("resolvectl");
    if resolvectl.is_file() {
        let _ = std::process::Command::new(resolvectl)
            .arg("flush-caches")
            .output();
    }
}

/// The persisted `last_error` for a refresh that is about to apply:
/// `None` when every enabled source fetched cleanly, otherwise the
/// source problems spelled out. Source failures must never be lost
/// just because the section applied — most importantly when the
/// YouTube lane carried a refresh whose community sources were all
/// unreachable or switched off in favour of detector records.
fn source_error_note(errors: &[String], any_source_parsed: bool) -> Option<String> {
    if errors.is_empty() {
        None
    } else if any_source_parsed {
        Some(format!("Не обновились источники: {}", errors.join("; ")))
    } else {
        Some(format!(
            "Не удалось обновить ни один источник: {}",
            errors.join("; ")
        ))
    }
}

/// Downloads, merges and applies all enabled sources.
pub async fn refresh_smart_hosts() -> Result<SmartHostsStatus, String> {
    let settings = load_settings();
    let enabled_ids: HashSet<String> = settings.smart_hosts_sources.iter().cloned().collect();

    let mut parsed: Vec<Vec<HostEntry>> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    for src in HOST_SOURCES.iter().filter(|s| enabled_ids.contains(s.id)) {
        match fetch_source(src.url).await {
            Ok(text) => parsed.push(parse_hosts(&text)),
            Err(e) => errors.push(format!("{}: {}", src.name, e)),
        }
    }

    // YouTube lane: deliberate 127.0.0.2 blocks of throttled googlevideo
    // nodes (see engine::youtube_probe). The lane is NOT passed through
    // merge_entries — these are intentional blocks and must not be eaten
    // by the adblock filter. It rides the same marked section, backup
    // and refresh as the community lists.
    //
    // The lane is resolved BEFORE the "no sources" bail-out below: a
    // non-empty lane (bundled/downloaded list and/or detector records)
    // is enough to apply the section on its own. That includes the
    // fully local configuration — both community sources switched off,
    // lane on — which needs no source download to succeed.
    let mut lane: Vec<HostEntry> = Vec::new();
    if settings.youtube_throttle_enabled {
        let inputs = YoutubeLaneInputs {
            list_hosts: fetch_youtube_list_hosts().await,
            throttled: settings
                .youtube_throttled_nodes
                .iter()
                .map(|r| (r.host.clone(), r.measured_at))
                .collect(),
            fast: settings
                .youtube_fast_nodes
                .iter()
                .map(|r| (r.host.clone(), r.measured_at))
                .collect(),
        };
        lane = resolve_youtube_lane(&inputs, unix_now());
    }

    if parsed.is_empty() && lane.is_empty() {
        return Err(if errors.is_empty() {
            "Нет включённых источников".to_string()
        } else {
            format!("Не удалось обновить ни один источник: {}", errors.join("; "))
        });
    }

    let any_source_parsed = !parsed.is_empty();
    let merged = merge_entries(parsed, settings.smart_hosts_include_adblock);
    if merged.is_empty() && lane.is_empty() {
        return Err("Источники не дали ни одной записи".to_string());
    }
    let lane_count = lane.len();
    let mut all = merged;
    all.extend(lane);

    apply_entries(&all)?;

    // Source failures ride along in last_error (cleared by the next
    // fully clean refresh): the apply succeeded, but the user must see
    // that the community lists behind it are stale or unreachable.
    let last_error = source_error_note(&errors, any_source_parsed);

    let now = unix_now();
    // Bookkeeping only: the hosts file is already updated at this point,
    // so a failed save must not turn a successful refresh into an error.
    if let Err(e) = update_settings(|s| {
        s.smart_hosts_last_update = Some(now);
        s.smart_hosts_entry_count = all.len();
        s.youtube_throttle_blocked_count = lane_count;
        s.smart_hosts_last_error = last_error.clone();
    }) {
        eprintln!("smart-hosts: applied, but failed to persist refresh state: {}", e);
    }

    if let Some(note) = &last_error {
        eprintln!("smart-hosts: applied with source problems: {}", note);
    }

    Ok(status())
}

/// Writes merged entries into the system hosts file (admin required).
fn apply_entries(entries: &[HostEntry]) -> Result<(), String> {
    let original = read_hosts_file()?;
    ensure_backup()?;
    let (before, after) = strip_section(&original);
    let now = unix_now();
    let mut new_content = before;
    if !new_content.is_empty() && !new_content.ends_with('\n') {
        new_content.push('\n');
    }
    new_content.push_str(&render_section(entries, now));
    if !after.is_empty() {
        // `after` already had its leading newline stripped; keep one blank line.
        new_content.push('\n');
        new_content.push_str(&after);
    }
    write_hosts_file(&new_content)?;
    flush_dns_cache();
    Ok(())
}

/// Removes our section from the hosts file, leaving everything else intact.
pub fn disable_smart_hosts() -> Result<SmartHostsStatus, String> {
    let original = read_hosts_file()?;
    let (before, after) = strip_section(&original);
    if original.contains(SECTION_BEGIN) {
        let mut new_content = before;
        if !after.is_empty() {
            if !new_content.is_empty() && !new_content.ends_with('\n') {
                new_content.push('\n');
            }
            new_content.push('\n');
            new_content.push_str(&after);
        }
        // Trim the trailing blank left by the removed section.
        let trimmed = new_content.trim_end().to_string() + "\n";
        write_hosts_file(&trimmed)?;
        flush_dns_cache();
    }
    update_settings(|s| {
        s.smart_hosts_enabled = false;
        s.smart_hosts_last_update = None;
        s.smart_hosts_entry_count = 0;
        s.smart_hosts_last_error = None;
        s.youtube_throttle_blocked_count = 0;
    })?;
    Ok(status())
}

/// Enables smart hosts: applies immediately (downloads fresh lists).
pub async fn enable_smart_hosts() -> Result<SmartHostsStatus, String> {
    update_settings(|s| {
        s.smart_hosts_enabled = true;
    })?;
    match refresh_smart_hosts().await {
        Ok(st) => Ok(st),
        Err(e) => {
            // Roll back the flag so UI and reality agree. A failed rollback
            // save is only logged: the refresh error is the one to surface.
            if let Err(save_err) = update_settings(|s| {
                s.smart_hosts_enabled = false;
            }) {
                eprintln!("smart-hosts: failed to roll back the enabled flag: {}", save_err);
            }
            Err(e)
        }
    }
}

/// Returns true when a refresh is due (enabled + older than 24h or never).
pub fn refresh_due() -> bool {
    let s = load_settings();
    if !s.smart_hosts_enabled {
        return false;
    }
    match s.smart_hosts_last_update {
        None => true,
        Some(ts) => unix_now().saturating_sub(ts) >= REFRESH_INTERVAL_SECS,
    }
}

/// Best-effort daily refresh; never fails the caller.
pub async fn maybe_refresh_smart_hosts() {
    if !refresh_due() {
        return;
    }
    if let Err(e) = refresh_smart_hosts().await {
        eprintln!("smart-hosts: scheduled refresh failed: {}", e);
    }
}

pub fn status() -> SmartHostsStatus {
    let s = load_settings();
    let enabled_ids: HashSet<&str> = s.smart_hosts_sources.iter().map(|x| x.as_str()).collect();
    let content = read_hosts_file().unwrap_or_default();
    SmartHostsStatus {
        enabled: s.smart_hosts_enabled,
        sources: HOST_SOURCES
            .iter()
            .map(|src| SmartHostsSourceInfo {
                id: src.id.to_string(),
                name: src.name.to_string(),
                description: src.description.to_string(),
                enabled: enabled_ids.contains(src.id),
            })
            .collect(),
        include_adblock: s.smart_hosts_include_adblock,
        last_update: s.smart_hosts_last_update,
        entry_count: s.smart_hosts_entry_count,
        last_error: s.smart_hosts_last_error.clone(),
        section_present: content.contains(SECTION_BEGIN),
        backup_present: backup_path().exists(),
        youtube_throttle_enabled: s.youtube_throttle_enabled,
        youtube_blocked_count: s.youtube_throttle_blocked_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic() {
        use std::net::IpAddr;
        let text = "# comment\n1.2.3.4 example.com www.example.com # inline\n\n5.6.7.8 other.org\nnot-an-ip bad.com\n1.2.3.4 bad..domain\n";
        let e = parse_hosts(text);
        assert_eq!(e.len(), 3);
        assert_eq!(e[0].domain, "example.com");
        assert_eq!(e[0].ip, "1.2.3.4".parse::<IpAddr>().unwrap());
        assert_eq!(e[1].domain, "www.example.com");
        assert_eq!(e[2].domain, "other.org");
    }

    #[test]
    fn merge_first_wins_and_filters() {
        use std::net::IpAddr;
        let v4 = |s: &str| s.parse::<IpAddr>().unwrap();
        let a = vec![
            HostEntry { ip: v4("1.1.1.1"), domain: "x.com".into() },
            HostEntry { ip: v4("0.0.0.0"), domain: "ads1.com".into() },
            HostEntry { ip: v4("0:0:0:0:0:0:0:0"), domain: "ads2.com".into() },
            HostEntry { ip: v4("127.0.0.1"), domain: "ads3.com".into() },
            HostEntry { ip: v4("::1"), domain: "ads4.com".into() },
            HostEntry { ip: v4("127.0.0.1"), domain: "localhost".into() },
        ];
        let b = vec![HostEntry { ip: v4("2.2.2.2"), domain: "x.com".into() }];
        let m = merge_entries(vec![a, b], false);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].ip, v4("1.1.1.1"));
        // with adblock included, all blocking forms survive
        let a2 = vec![
            HostEntry { ip: v4("0.0.0.0"), domain: "ads1.com".into() },
            HostEntry { ip: v4("127.0.0.1"), domain: "ads3.com".into() },
        ];
        let m2 = merge_entries(vec![a2], true);
        assert_eq!(m2.len(), 2);
    }

    #[test]
    fn strip_section_roundtrip() {
        let body = "127.0.0.1 localhost\n";
        let with = format!("{}a\n{}b\n{}\n", body, SECTION_BEGIN, SECTION_END);
        let (before, after) = strip_section(&with);
        assert!(before.contains("localhost"));
        assert!(!before.contains(SECTION_BEGIN));
        assert!(after.is_empty() || !after.contains(SECTION_END));
    }

    #[test]
    fn parse_node_list_filters_and_dedupes() {
        let text = "# comment\n\nrr1---sn-gvnuxaxjvh-n8ml.googlevideo.com\nRR2---sn-gvnuxaxjvh-n8ml.googlevideo.com\nrr1---sn-gvnuxaxjvh-n8ml.googlevideo.com\nyoutube.com\nrr3---sn-gvnuxaxjvh-n8ml.googlevideo.com # inline comment\ngarbage line\n1.2.3.4 rr4---sn-gvnuxaxjvh-n8ml.googlevideo.com\n";
        let hosts = parse_node_list(text);
        assert_eq!(
            hosts,
            vec![
                "rr1---sn-gvnuxaxjvh-n8ml.googlevideo.com",
                "rr2---sn-gvnuxaxjvh-n8ml.googlevideo.com",
                "rr3---sn-gvnuxaxjvh-n8ml.googlevideo.com",
            ]
        );
    }

    #[test]
    fn bundled_node_list_parses() {
        let hosts = parse_node_list(YOUTUBE_NODES_BUNDLED);
        assert_eq!(hosts.len(), 20);
        assert!(hosts.iter().all(|h| h.contains("sn-gvnuxaxjvh-n8ml")));
    }

    #[test]
    fn lane_list_entries_point_at_block_ip() {
        let inputs = YoutubeLaneInputs {
            list_hosts: vec!["rr5---sn-aaaa-bb.googlevideo.com".to_string()],
            throttled: vec![],
            fast: vec![],
        };
        let lane = resolve_youtube_lane(&inputs, 1_000_000);
        assert_eq!(lane.len(), 1);
        assert_eq!(lane[0].ip, YOUTUBE_BLOCK_IP);
        assert_eq!(lane[0].domain, "rr5---sn-aaaa-bb.googlevideo.com");
        // The lane bypasses the adblock filter by construction: the same
        // mapping fed through merge_entries with adblock off would be
        // dropped as a blocking entry. That is why refresh applies the
        // lane separately, never through merge_entries.
        assert!(merge_entries(vec![lane.clone()], false).is_empty());
        assert_eq!(merge_entries(vec![lane], true).len(), 1);
    }

    #[test]
    fn lane_detected_throttled_expands_cluster() {
        let now = 2_000_000;
        let inputs = YoutubeLaneInputs {
            list_hosts: vec![],
            throttled: vec![("rr3---sn-aaaa-bb.googlevideo.com".to_string(), now - 60)],
            fast: vec![],
        };
        let lane = resolve_youtube_lane(&inputs, now);
        assert_eq!(lane.len(), 20);
        assert!(lane
            .iter()
            .any(|e| e.domain == "rr20---sn-aaaa-bb.googlevideo.com"));
    }

    #[test]
    fn lane_measured_fast_vetoes_list_and_detection() {
        let now = 2_000_000;
        // Same cluster in the list AND freshly measured fast -> no block.
        let inputs = YoutubeLaneInputs {
            list_hosts: vec!["rr5---sn-aaaa-bb.googlevideo.com".to_string()],
            throttled: vec![],
            fast: vec![("rr9---sn-aaaa-bb.googlevideo.com".to_string(), now - 60)],
        };
        assert!(resolve_youtube_lane(&inputs, now).is_empty());
        // A fresh fast verdict also cancels an older throttled verdict.
        let inputs2 = YoutubeLaneInputs {
            list_hosts: vec![],
            throttled: vec![("rr3---sn-aaaa-bb.googlevideo.com".to_string(), now - 3600)],
            fast: vec![("rr9---sn-aaaa-bb.googlevideo.com".to_string(), now - 60)],
        };
        assert!(resolve_youtube_lane(&inputs2, now).is_empty());
        // Reverse freshness order: the block stands.
        let inputs3 = YoutubeLaneInputs {
            list_hosts: vec![],
            throttled: vec![("rr3---sn-aaaa-bb.googlevideo.com".to_string(), now - 60)],
            fast: vec![("rr9---sn-aaaa-bb.googlevideo.com".to_string(), now - 3600)],
        };
        assert_eq!(resolve_youtube_lane(&inputs3, now).len(), 20);
    }

    #[test]
    fn lane_measured_host_beyond_rr20_blocks_itself() {
        let now = 2_000_000;
        // The detector measured rr123: the rr1..rr20 expansion alone
        // would miss the one node actually proven throttled.
        let inputs = YoutubeLaneInputs {
            list_hosts: vec![],
            throttled: vec![("rr123---sn-aaaa-bb.googlevideo.com".to_string(), now - 60)],
            fast: vec![],
        };
        let lane = resolve_youtube_lane(&inputs, now);
        assert_eq!(lane.len(), 21);
        assert!(lane
            .iter()
            .any(|e| e.domain == "rr123---sn-aaaa-bb.googlevideo.com"));
        assert!(lane
            .iter()
            .any(|e| e.domain == "rr20---sn-aaaa-bb.googlevideo.com"));
        // A fresher fast verdict for the same cluster vetoes even the
        // measured host itself.
        let vetoed = YoutubeLaneInputs {
            list_hosts: vec![],
            throttled: vec![("rr123---sn-aaaa-bb.googlevideo.com".to_string(), now - 3600)],
            fast: vec![("rr9---sn-aaaa-bb.googlevideo.com".to_string(), now - 60)],
        };
        assert!(resolve_youtube_lane(&vetoed, now).is_empty());
    }

    #[test]
    fn source_error_note_keeps_failures_visible() {
        // Clean refresh: no note, whatever was applied.
        assert_eq!(source_error_note(&[], false), None);
        assert_eq!(source_error_note(&[], true), None);
        // Every source failed but the lane applied: the note says so.
        let all_dead = vec!["MALW.link: boom".to_string(), "GeoHide: boom".to_string()];
        let note = source_error_note(&all_dead, false).unwrap();
        assert!(note.contains("ни один источник"));
        assert!(note.contains("MALW.link"));
        // Partial failure: some lists are fresh, the note names the rest.
        let partial = vec!["GeoHide: boom".to_string()];
        let note = source_error_note(&partial, true).unwrap();
        assert!(note.contains("GeoHide"));
        assert!(!note.contains("ни один"));
    }

    #[test]
    fn lane_records_expire_after_ttl() {
        let now = 10 * YOUTUBE_NODE_TTL_SECS;
        let stale = now - YOUTUBE_NODE_TTL_SECS - 1;
        let inputs = YoutubeLaneInputs {
            list_hosts: vec!["rr5---sn-aaaa-bb.googlevideo.com".to_string()],
            throttled: vec![("rr3---sn-cccc-dd.googlevideo.com".to_string(), stale)],
            fast: vec![("rr9---sn-aaaa-bb.googlevideo.com".to_string(), stale)],
        };
        let lane = resolve_youtube_lane(&inputs, now);
        // Expired throttled record blocks nothing; expired fast record no
        // longer vetoes the list entry for its cluster.
        assert_eq!(lane.len(), 1);
        assert_eq!(lane[0].domain, "rr5---sn-aaaa-bb.googlevideo.com");
    }
}

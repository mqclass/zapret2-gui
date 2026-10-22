//! Smart Hosts: daily-merged system hosts file from community sources.
//!
//! Some blocks happen at the DNS level (poisoned answers, blocked resolvers)
//! and some services block by client IP (sanctions). Community projects
//! maintain hosts files that map such domains to working IPs / SNI proxies:
//! - MALW.link (https://github.com/ImMALWARE/dns.malw.link)
//! - GeoHide (https://dns.geohide.ru:8443/)
//!
//! This module fetches those lists over HTTPS, merges them (first enabled
//! source wins on conflicts), and writes the result into the Windows system
//! hosts file inside a clearly marked section. The user's own entries are
//! never touched: only our marked section is added, replaced, or removed.
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
use std::net::IpAddr;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::win32::settings::{load_settings, update_settings};

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
    pub section_present: bool,
    pub backup_present: bool,
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
/// Max merged entries — sanity cap (Windows DNS client slows down on huge files).
const MAX_ENTRIES: usize = 60_000;
/// Refresh interval: 24 hours.
pub const REFRESH_INTERVAL_SECS: u64 = 24 * 3600;

fn system_hosts_path() -> PathBuf {
    let sysdir = std::env::var("SystemRoot")
        .map(|r| PathBuf::from(r).join("System32"))
        .unwrap_or_else(|_| PathBuf::from(r"C:\Windows\System32"));
    sysdir.join("drivers").join("etc").join("hosts")
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
            .map_err(|e| format!("Не удалось записать hosts (нет прав администратора?): {}", e))?;
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

/// Flushes the Windows DNS resolver cache.
fn flush_dns_cache() {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let system32 = std::env::var("SystemRoot")
        .map(|r| PathBuf::from(r).join("System32"))
        .unwrap_or_else(|_| PathBuf::from(r"C:\Windows\System32"));
    let _ = std::process::Command::new(system32.join("ipconfig.exe"))
        .arg("/flushdns")
        .creation_flags(CREATE_NO_WINDOW)
        .output();
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
    if parsed.is_empty() {
        return Err(if errors.is_empty() {
            "Нет включённых источников".to_string()
        } else {
            format!("Не удалось обновить ни один источник: {}", errors.join("; "))
        });
    }

    let merged = merge_entries(parsed, settings.smart_hosts_include_adblock);
    if merged.is_empty() {
        return Err("Источники не дали ни одной записи".to_string());
    }

    apply_entries(&merged)?;

    let now = unix_now();
    update_settings(|s| {
        s.smart_hosts_last_update = Some(now);
        s.smart_hosts_entry_count = merged.len();
    });

    if !errors.is_empty() {
        // Partial success: some sources failed, but we applied the rest.
        // Surface it via the entry count note in logs; keep status green.
        eprintln!("smart-hosts: partial refresh, failed sources: {}", errors.join("; "));
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
    });
    Ok(status())
}

/// Enables smart hosts: applies immediately (downloads fresh lists).
pub async fn enable_smart_hosts() -> Result<SmartHostsStatus, String> {
    update_settings(|s| {
        s.smart_hosts_enabled = true;
    });
    match refresh_smart_hosts().await {
        Ok(st) => Ok(st),
        Err(e) => {
            // Roll back the flag so UI and reality agree.
            update_settings(|s| {
                s.smart_hosts_enabled = false;
            });
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
        section_present: content.contains(SECTION_BEGIN),
        backup_present: backup_path().exists(),
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
}

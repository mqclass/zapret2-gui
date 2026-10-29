//! DNS status and custom resolvers (Linux).
//!
//! What is real here in phase 1:
//! - The provider catalog and user-defined custom DNS entries — these are
//!   settings data, identical to the Windows build.
//! - Status reporting: the active servers are read from `/etc/resolv.conf`
//!   and the primary adapter is the default-route interface from
//!   `/proc/net/route`. What the file says is what gets reported.
//! - The Split-DNS custom domain list (settings-level CRUD).
//!
//! What is honestly unavailable:
//! - APPLYING a provider to the system. On Windows that is a netsh/
//!   PowerShell adapter change; on Linux it means owning resolv.conf /
//!   systemd-resolved / NetworkManager state, which phase 1 deliberately
//!   does not do (plan section 2). `set_system_dns` refuses with that reason.
//! - Split-DNS. NRPT is a Windows mechanism; its Linux analog is
//!   systemd-resolved per-domain routing, a separate piece of work. The
//!   NRPT entry points refuse, and split status always reports inactive —
//!   because it is.

use serde::{Deserialize, Serialize};
use crate::platform::settings::{load_settings, update_settings};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsProviderInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub primary: String,
    pub secondary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsStatus {
    pub current_provider: String,
    pub active_provider_id: String,
    pub active_servers: Vec<String>,
    pub current_dns: Vec<String>,
    pub adapter_name: String,
    pub available_providers: Vec<DnsProviderInfo>,
    pub dns_scope: String,
    pub split_dns_active: bool,
    pub split_dns_servers: Vec<String>,
}

pub fn get_available_providers() -> Vec<DnsProviderInfo> {
    let mut providers = vec![
        DnsProviderInfo {
            id: "system".into(),
            name: "По умолчанию (DHCP / Провайдер)".into(),
            description: "Стандартные DNS-сервера, выданные вашим роутером или провайдером".into(),
            primary: "DHCP".into(),
            secondary: "DHCP".into(),
        },
        DnsProviderInfo {
            id: "cloudflare".into(),
            name: "Cloudflare DNS (1.1.1.1)".into(),
            description: "Самый быстрый глобальный DNS с защитой приватности (DoH/DoT)".into(),
            primary: "1.1.1.1".into(),
            secondary: "1.0.0.1".into(),
        },
        DnsProviderInfo {
            id: "comss".into(),
            name: "Comss.one DNS (Анти-блокировка)".into(),
            description: "Специальный DNS для РФ с защитой от DNS-спуфинга и блокировок РКН".into(),
            primary: "92.223.109.31".into(),
            secondary: "91.217.137.37".into(),
        },
        DnsProviderInfo {
            id: "google".into(),
            name: "Google Public DNS (8.8.8.8)".into(),
            description: "Надежный высокоскоростной глобальный резолвер от Google".into(),
            primary: "8.8.8.8".into(),
            secondary: "8.8.4.4".into(),
        },
        DnsProviderInfo {
            id: "quad9".into(),
            name: "Quad9 Security (9.9.9.9)".into(),
            description: "Швейцарский безопасный DNS с блокировкой вредоносных фишинговых сайтов".into(),
            primary: "9.9.9.9".into(),
            secondary: "149.112.112.112".into(),
        },
    ];
    // Append user-defined DNS servers after the built-ins.
    for custom in load_settings().custom_dns {
        providers.push(DnsProviderInfo {
            id: format!("custom:{}", custom.id),
            name: format!("{} (свой)", custom.name),
            description: "Пользовательский DNS-сервер".into(),
            primary: custom.primary,
            secondary: if custom.secondary.is_empty() {
                "-".into()
            } else {
                custom.secondary
            },
        });
    }
    providers
}

/// Nameservers from `/etc/resolv.conf`, in file order. With
/// systemd-resolved this is typically the `127.0.0.53` stub — reported
/// as-is, because that IS the resolver every application uses.
fn resolv_conf_servers() -> Vec<String> {
    let content = match std::fs::read_to_string("/etc/resolv.conf") {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix("nameserver")?;
            let ip = rest.trim();
            if ip.is_empty() {
                None
            } else {
                Some(ip.to_string())
            }
        })
        .collect()
}

/// The interface holding the default route, from `/proc/net/route`
/// (destination 00000000). Falls back to the first non-loopback interface
/// in `/sys/class/net`, then "lo".
fn default_route_adapter() -> String {
    if let Ok(routes) = std::fs::read_to_string("/proc/net/route") {
        for line in routes.lines().skip(1) {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() >= 2 && fields[1] == "00000000" {
                return fields[0].to_string();
            }
        }
    }
    if let Ok(entries) = std::fs::read_dir("/sys/class/net") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name != "lo" {
                return name;
            }
        }
    }
    "lo".to_string()
}

/// Reports the primary adapter and the DNS servers the system actually
/// uses (resolv.conf). Linux has no per-adapter DNS in the Windows sense:
/// the resolver is system-wide, so the servers are the resolv.conf ones
/// regardless of which interface carries the route.
pub fn get_adapter_and_dns() -> (String, Vec<String>) {
    (default_route_adapter(), resolv_conf_servers())
}

/// Detects the primary active network adapter name (default-route iface).
pub fn detect_primary_adapter() -> Result<String, String> {
    let (adapter, _) = get_adapter_and_dns();
    Ok(adapter)
}

/// Returns the active DNS server addresses (resolv.conf nameservers).
pub fn get_active_dns_servers(_adapter: &str) -> Vec<String> {
    resolv_conf_servers()
}

pub const SPLIT_DNS_NAMESPACES: &[&str] = &[
    ".discord.gg",
    ".discord.com",
    ".discord.media",
    ".discordapp.com",
    ".discordapp.net",
    ".discordcdn.com",
    ".dis.gd",
    ".tenor.com",
    ".tenor.co",
    ".giphy.com",
    ".youtube.com",
    ".googlevideo.com",
    ".ytimg.com",
    ".ggpht.com",
    ".youtu.be",
    ".gvt1.com",
    ".instagram.com",
    ".cdninstagram.com",
    ".facebook.com",
    ".fbcdn.net",
    ".x.com",
    ".twitter.com",
    ".t.co",
    ".twimg.com",
    ".t.me",
    ".telegram.org",
    ".rutracker.org",
    ".ntc.party",
];

/// Split-DNS (NRPT) status. NRPT does not exist on Linux, so split-DNS is
/// never active in phase 1 — reported plainly, with no servers.
pub fn check_split_dns_status() -> (bool, Vec<String>) {
    (false, Vec::new())
}

/// Applies selective Split-DNS. NRPT is a Windows mechanism; the Linux
/// analog (systemd-resolved per-domain routing) is outside phase 1.
pub fn apply_split_dns_nrpt(_primary: &str, _secondary: &str) -> Result<(), String> {
    Err(
        "Split-DNS недоступен на Linux: NRPT — механизм Windows, а маршрутизация \
         DNS по доменам через systemd-resolved не входит в фазу 1 портирования."
            .to_string(),
    )
}

/// Validates a domain name for the Split-DNS custom list.
/// Accepts "example.com" or ".example.com"; rejects IPs, wildcards, ports, paths.
fn is_valid_domain(s: &str) -> bool {
    let s = s.trim().trim_start_matches('.');
    if s.is_empty() || s.len() > 253 {
        return false;
    }
    // Reject anything that looks like an IP address.
    if s.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) && s.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return false;
    }
    let labels: Vec<&str> = s.split('.').collect();
    if labels.len() < 2 {
        return false;
    }
    labels.iter().all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            && !label.starts_with('-')
            && !label.ends_with('-')
    })
}

/// Returns the built-in Split-DNS namespaces plus user custom domains.
pub fn get_split_dns_domains() -> Vec<String> {
    let mut domains: Vec<String> = SPLIT_DNS_NAMESPACES.iter().map(|s| s.to_string()).collect();
    domains.extend(load_settings().dns_split_custom_domains);
    domains
}

/// Adds a custom domain to the Split-DNS list (settings-level; the list
/// takes effect only where Split-DNS itself is supported).
pub fn add_split_dns_domain(domain: &str) -> Result<Vec<String>, String> {
    let domain = domain.trim().trim_start_matches('.').to_lowercase();
    if !is_valid_domain(&domain) {
        return Err(format!("Некорректное доменное имя: {}", domain));
    }
    // Don't add duplicates of built-in namespaces.
    let built_in: std::collections::HashSet<String> = SPLIT_DNS_NAMESPACES
        .iter()
        .map(|s| s.trim_start_matches('.').to_lowercase())
        .collect();
    if built_in.contains(&domain) {
        return Err("Этот домен уже есть во встроенном списке".into());
    }
    update_settings(|s| {
        if s.dns_split_custom_domains.len() >= 50 {
            s.dns_split_custom_domains.remove(0);
        }
        if !s.dns_split_custom_domains.contains(&domain) {
            s.dns_split_custom_domains.push(domain.clone());
        }
        s.dns_split_custom_domains.clone()
    })
    .map_err(|e| e.to_string())?;
    // If split-DNS is currently live, re-apply so the new domain takes
    // effect immediately. (Never live on Linux in phase 1: the check
    // above returns false, so this stays a no-op here.)
    if check_split_dns_status().0 {
        let provider = load_settings().dns_provider.clone();
        set_system_dns(&provider, Some("split"))?;
    }
    Ok(load_settings().dns_split_custom_domains)
}

/// Removes a custom domain from the Split-DNS list.
pub fn remove_split_dns_domain(domain: &str) -> Result<Vec<String>, String> {
    let domain = domain.trim().trim_start_matches('.').to_lowercase();
    update_settings(|s| {
        s.dns_split_custom_domains.retain(|d| d != &domain);
        s.dns_split_custom_domains.clone()
    })
    .map_err(|e| e.to_string())?;
    if check_split_dns_status().0 {
        let provider = load_settings().dns_provider.clone();
        set_system_dns(&provider, Some("split"))?;
    }
    Ok(load_settings().dns_split_custom_domains)
}

/// Clears the Split-DNS NRPT rule. There is never an NRPT rule on Linux,
/// so the postcondition ("no zapret split-DNS rule exists") trivially
/// holds and Ok is the truthful result.
pub fn clear_split_dns_nrpt() -> Result<(), String> {
    Ok(())
}

/// Retrieves the current DNS configuration status: the provider choice
/// from settings, the servers the resolver actually uses, and an always-
/// inactive split-DNS (NRPT does not exist here).
pub fn get_dns_status() -> Result<DnsStatus, String> {
    let settings = load_settings();
    let (adapter, active_servers) = get_adapter_and_dns();
    let (split_active, split_servers) = check_split_dns_status();

    let scope_str = match settings.dns_scope {
        crate::platform::settings::DnsScope::Split => "split",
        crate::platform::settings::DnsScope::Full => "full",
    };

    Ok(DnsStatus {
        current_provider: settings.dns_provider.clone(),
        active_provider_id: settings.dns_provider,
        active_servers: active_servers.clone(),
        current_dns: active_servers,
        adapter_name: adapter,
        available_providers: get_available_providers(),
        dns_scope: scope_str.into(),
        split_dns_active: split_active,
        split_dns_servers: split_servers,
    })
}

/// Custom DNS management: add / remove user-defined resolvers.
/// These are stored in settings and appear in get_available_providers()
/// with id "custom:<id>". Validation rejects anything that is not a
/// valid IPv4 address.
fn is_valid_ipv4(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 4 {
        return false;
    }
    parts.iter().all(|p| {
        !p.is_empty() && p.len() <= 3 && p.bytes().all(|b| b.is_ascii_digit()) && p.parse::<u8>().is_ok()
    })
}

pub fn add_custom_dns(name: &str, primary: &str, secondary: &str) -> Result<DnsStatus, String> {
    let name = name.trim();
    let primary = primary.trim();
    let secondary = secondary.trim();
    if name.is_empty() {
        return Err("Укажите название DNS-сервера".into());
    }
    if name.chars().count() > 60 {
        return Err("Название слишком длинное (максимум 60 символов)".into());
    }
    if !is_valid_ipv4(primary) {
        return Err(format!("Некорректный IP-адрес: {}", primary));
    }
    if !secondary.is_empty() && !is_valid_ipv4(secondary) {
        return Err(format!("Некорректный IP-адрес: {}", secondary));
    }
    if primary == secondary && !secondary.is_empty() {
        return Err("Основной и дополнительный адреса совпадают".into());
    }

    let id = format!(
        "custom-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    );
    let entry = crate::platform::settings::CustomDnsEntry {
        id: id.clone(),
        name: name.to_string(),
        primary: primary.to_string(),
        secondary: secondary.to_string(),
    };
    update_settings(|s| {
        // Cap at 10 custom entries to keep the list manageable.
        if s.custom_dns.len() >= 10 {
            s.custom_dns.remove(0);
        }
        s.custom_dns.push(entry.clone());
    })
    .map_err(|e| e.to_string())?;
    get_dns_status()
}

pub fn remove_custom_dns(id: &str) -> Result<DnsStatus, String> {
    let removed_active = update_settings(|s| {
        let was_active = s.dns_provider == format!("custom:{}", id);
        s.custom_dns.retain(|e| e.id != id);
        // If the removed entry was active, fall back to system DNS.
        if was_active {
            s.dns_provider = "system".into();
        }
        was_active
    })
    .map_err(|e| e.to_string())?;
    if removed_active {
        // Windows divergence, deliberate: the Windows build also resets
        // the network adapter to DHCP here. Linux phase 1 manages no
        // adapter state (see set_system_dns), so there is nothing to
        // reset — the settings fallback above is the whole operation.
        let _ = removed_active;
    }
    get_dns_status()
}

/// Applies a DNS provider to the system. NOT SUPPORTED on Linux in
/// phase 1: changing the system resolver means owning resolv.conf /
/// systemd-resolved / NetworkManager state, which the port deliberately
/// leaves alone for now (plan section 2). The provider id is still
/// validated so callers get the same "unknown provider" error as Windows.
pub fn set_system_dns(provider_id: &str, scope: Option<&str>) -> Result<DnsStatus, String> {
    let _ = scope;
    let providers = get_available_providers();
    if provider_id != "system"
        && provider_id != "dhcp"
        && !providers.iter().any(|p| p.id == provider_id)
    {
        return Err(format!("Unknown DNS provider ID: {}", provider_id));
    }
    Err(
        "Смена системного DNS на Linux пока не поддерживается (фаза 1): \
         приложение не изменяет resolv.conf и настройки systemd-resolved."
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_dns_must_match_apex_and_subdomains() {
        let rules: Vec<String> = SPLIT_DNS_NAMESPACES
            .iter()
            .flat_map(|ns| {
                let clean = ns.trim_start_matches('.');
                vec![
                    format!(".{}", clean),
                    clean.to_string(),
                ]
            })
            .collect();

        // Apex domains must be included verbatim
        assert!(rules.iter().any(|r| r == "discord.com"), "discord.com apex rule missing");
        assert!(rules.iter().any(|r| r == "youtube.com"), "youtube.com apex rule missing");
        assert!(rules.iter().any(|r| r == "x.com"), "x.com apex rule missing");
        assert!(rules.iter().any(|r| r == "t.me"), "t.me apex rule missing");
        assert!(rules.iter().any(|r| r == "rutracker.org"), "rutracker.org apex rule missing");

        // Subdomain suffix rules must also be present
        assert!(rules.iter().any(|r| r == ".discord.com"), ".discord.com suffix rule missing");
        assert!(rules.iter().any(|r| r == ".youtube.com"), ".youtube.com suffix rule missing");
        assert!(rules.iter().any(|r| r == ".x.com"), ".x.com suffix rule missing");
    }

    #[test]
    fn test_split_dns_namespaces_not_empty() {
        assert!(!SPLIT_DNS_NAMESPACES.is_empty());
        for ns in SPLIT_DNS_NAMESPACES {
            assert!(ns.starts_with('.'), "namespace should specify canonical suffix format");
        }
    }
}

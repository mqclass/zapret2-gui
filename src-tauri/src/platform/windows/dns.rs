//! Windows DNS Management & DNS Spoofing Protection (DoH / Secure DNS)
//!
//! Provides one-click configuration of trusted anti-censorship and anti-spoofing DNS resolvers
//! directly on active network interfaces using PowerShell DnsClient cmdlets:
//! - Cloudflare (1.1.1.1, 1.0.0.1)
//! - Google Public DNS (8.8.8.8, 8.8.4.4)
//! - Quad9 Security (9.9.9.9, 149.112.112.112)
//! - Comss.one (92.223.109.31, 91.217.137.37)
//! - Default DHCP / ISP DNS

use std::os::windows::process::CommandExt;
use std::process::Command;
use serde::{Deserialize, Serialize};
use crate::platform::settings::{load_settings, update_settings};

const CREATE_NO_WINDOW: u32 = 0x08000000;

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

/// Queries the primary active network adapter and its active DNS servers in a single silent PowerShell execution.
pub fn get_adapter_and_dns() -> (String, Vec<String>) {
    let script = r#"
        [Console]::OutputEncoding = [System.Text.Encoding]::UTF8;
        $r = Get-NetRoute -DestinationPrefix '0.0.0.0/0' -ErrorAction SilentlyContinue | Sort-Object RouteMetric | Select-Object -First 1;
        $alias = if ($r) {
            $a = Get-NetAdapter -InterfaceIndex $r.InterfaceIndex -ErrorAction SilentlyContinue;
            if ($a) { $a.Name } else { $r.InterfaceAlias }
        } else {
            $a = Get-NetAdapter | Where-Object Status -eq 'Up' | Sort-Object LinkSpeed -Descending | Select-Object -First 1;
            if ($a) { $a.Name } else { 'Ethernet' }
        };
        $dns = (Get-DnsClientServerAddress -InterfaceAlias $alias -AddressFamily IPv4 -ErrorAction SilentlyContinue).ServerAddresses;
        $dnsStr = if ($dns) { $dns -join ',' } else { '' };
        [string]::Format('{0}::{1}', $alias, $dnsStr)
    "#;

    let output = Command::new(crate::platform::system32(r"WindowsPowerShell\v1.0\powershell.exe"))
        .creation_flags(CREATE_NO_WINDOW)
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", script])
        .output();

    if let Ok(out) = output {
        let stdout = String::from_utf8_lossy(&out.stdout);
        let line = stdout.lines().filter(|l| !l.trim().is_empty()).last().unwrap_or("").trim();
        // SECURITY (L-7): split on the LAST "::" — the DNS part is a comma-joined
        // IPv4 list which can never contain "::", so any "::" must belong to
        // the adapter name. split_once would misattribute it.
        if let Some((adapter, dns_csv)) = line.rsplit_once("::") {
            let adapter_name = if adapter.trim().is_empty() {
                "Ethernet".to_string()
            } else {
                adapter.trim().to_string()
            };
            let servers = dns_csv
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            return (adapter_name, servers);
        }
    }

    ("Ethernet".into(), Vec::new())
}

/// Detects the primary active network adapter name (Ethernet or Wi-Fi).
pub fn detect_primary_adapter() -> Result<String, String> {
    let (adapter, _) = get_adapter_and_dns();
    Ok(adapter)
}

/// Queries active DNS server addresses on the primary adapter.
#[allow(dead_code)]
pub fn get_active_dns_servers(adapter: &str) -> Vec<String> {
    let script = format!(
        "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; $d = (Get-DnsClientServerAddress -InterfaceAlias '{}' -AddressFamily IPv4 -ErrorAction SilentlyContinue).ServerAddresses; if ($d) {{ $d -join ',' }} else {{ '' }}",
        escape_ps(adapter)
    );

    let output = Command::new(crate::platform::system32(r"WindowsPowerShell\v1.0\powershell.exe"))
        .creation_flags(CREATE_NO_WINDOW)
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &script])
        .output();

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let res = stdout.lines().filter(|l| !l.trim().is_empty()).last().unwrap_or("").trim().to_string();
            if res.is_empty() {
                Vec::new()
            } else {
                res.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
            }
        }
        Err(_) => Vec::new(),
    }
}

/// Escapes a value for safe embedding inside a PowerShell single-quoted string
/// literal (a single quote is escaped by doubling it).
fn escape_ps(value: &str) -> String {
    value.replace('\'', "''")
}

/// Runs a PowerShell script silently, returning an error when PowerShell itself
/// fails to launch or the script exits with a non-zero status.
fn run_powershell(script: &str) -> Result<(), String> {
    let output = Command::new(crate::platform::system32(r"WindowsPowerShell\v1.0\powershell.exe"))
        .creation_flags(CREATE_NO_WINDOW)
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", script])
        .output()
        .map_err(|e| format!("Failed to launch PowerShell: {}", e))?;

    if !output.status.success() {
        let err_msg = String::from_utf8_lossy(&output.stderr);
        return Err(format!("PowerShell error: {}", err_msg.trim()));
    }
    Ok(())
}

/// Resets the adapter to automatic (DHCP) DNS assignment.
fn reset_adapter_dns(adapter_esc: &str) -> Result<(), String> {
    let script = format!(
        "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; Set-DnsClientServerAddress -InterfaceAlias '{}' -ResetServerAddresses -ErrorAction Stop",
        adapter_esc
    );
    run_powershell(&script).map_err(|e| format!("Failed to reset adapter DNS: {}", e))
}

/// Assigns static DNS servers to the adapter.
fn set_adapter_dns(adapter_esc: &str, primary: &str, secondary: &str) -> Result<(), String> {
    let servers = if secondary.is_empty() {
        format!("'{}'", primary)
    } else {
        format!("'{}', '{}'", primary, secondary)
    };
    let script = format!(
        "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; Set-DnsClientServerAddress -InterfaceAlias '{}' -ServerAddresses @({}) -ErrorAction Stop",
        adapter_esc, servers
    );
    run_powershell(&script).map_err(|e| format!("Failed to apply DNS settings: {}", e))
}

/// Best-effort rollback of the adapter DNS to a previously snapshotted configuration.
/// An empty snapshot means the adapter was on DHCP → reset to automatic assignment.
fn rollback_adapter_dns(adapter_esc: &str, previous: &[String]) {
    let result = if previous.is_empty() {
        reset_adapter_dns(adapter_esc)
    } else {
        let list = previous
            .iter()
            .map(|s| format!("'{}'", escape_ps(s)))
            .collect::<Vec<_>>()
            .join(",");
        let script = format!(
            "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; Set-DnsClientServerAddress -InterfaceAlias '{}' -ServerAddresses @({}) -ErrorAction SilentlyContinue",
            adapter_esc, list
        );
        run_powershell(&script).map(|_| ())
    };
    let _ = result;
}

/// Target domain namespaces for Windows NRPT Split-DNS rules.
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

/// Checks if Windows NRPT Split-DNS rule for Zapret2 is currently active.
pub fn check_split_dns_status() -> (bool, Vec<String>) {
    let script = r#"
        [Console]::OutputEncoding = [System.Text.Encoding]::UTF8;
        $r = Get-DnsClientNrptRule -ErrorAction SilentlyContinue | Where-Object { $_.Comment -eq 'Zapret2_SplitDNS' };
        if ($r) {
            $servers = if ($r.NameServers) { $r.NameServers -join ',' } else { '' };
            [string]::Format('ACTIVE::{0}', $servers)
        } else {
            'INACTIVE::'
        }
    "#;

    let output = Command::new(crate::platform::system32(r"WindowsPowerShell\v1.0\powershell.exe"))
        .creation_flags(CREATE_NO_WINDOW)
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", script])
        .output();

    if let Ok(out) = output {
        let stdout = String::from_utf8_lossy(&out.stdout);
        let line = stdout.lines().filter(|l| !l.trim().is_empty()).last().unwrap_or("").trim();
        if let Some((status, srv_csv)) = line.split_once("::") {
            if status == "ACTIVE" {
                let servers: Vec<String> = srv_csv
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                return (true, servers);
            }
        }
    }

    (false, Vec::new())
}

/// Applies selective Split-DNS using Windows Name Resolution Policy Table (NRPT).
pub fn apply_split_dns_nrpt(primary: &str, secondary: &str) -> Result<(), String> {
    // Built-in namespaces + user-defined custom domains.
    // Re-validate custom domains here (defense in depth): settings are
    // user-writable and this script runs elevated.
    let custom: Vec<String> = load_settings()
        .dns_split_custom_domains
        .into_iter()
        .filter(|d| is_valid_domain(d))
        .collect();
    let ns_list = SPLIT_DNS_NAMESPACES
        .iter()
        .map(|s| s.to_string())
        .chain(custom.into_iter())
        .flat_map(|ns| {
            let clean = ns.trim_start_matches('.');
            vec![
                format!("'.{}'", clean),
                format!("'{}'", clean),
            ]
        })
        .collect::<Vec<_>>()
        .join(",");

    let ns_servers = if secondary.is_empty() {
        format!("'{}'", primary)
    } else {
        format!("'{}', '{}'", primary, secondary)
    };
    let script = format!(
        r#"
        [Console]::OutputEncoding = [System.Text.Encoding]::UTF8;
        $existing = Get-DnsClientNrptRule -ErrorAction SilentlyContinue | Where-Object {{ $_.Comment -eq 'Zapret2_SplitDNS' }};
        if ($existing) {{
            $existing | Remove-DnsClientNrptRule -Force -ErrorAction SilentlyContinue;
        }}
        $ns = @({});
        Add-DnsClientNrptRule -Namespace $ns -NameServers @({}) -Comment 'Zapret2_SplitDNS' -ErrorAction Stop;
        "#,
        ns_list, ns_servers
    );

    let output = Command::new(crate::platform::system32(r"WindowsPowerShell\v1.0\powershell.exe"))
        .creation_flags(CREATE_NO_WINDOW)
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &script])
        .output()
        .map_err(|e| format!("Failed to apply Split-DNS NRPT rule: {}", e))?;

    if !output.status.success() {
        let err_msg = String::from_utf8_lossy(&output.stderr);
        return Err(format!("PowerShell NRPT Split-DNS error: {}", err_msg.trim()));
    }

    Ok(())
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

/// Adds a custom domain to the Split-DNS list.
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
    // If split-DNS is currently live, re-apply so the new domain takes effect immediately.
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
    // If split-DNS is currently live, re-apply so the removal takes effect immediately.
    if check_split_dns_status().0 {
        let provider = load_settings().dns_provider.clone();
        set_system_dns(&provider, Some("split"))?;
    }
    Ok(load_settings().dns_split_custom_domains)
}

/// Clears Zapret2 Split-DNS NRPT rule.
pub fn clear_split_dns_nrpt() -> Result<(), String> {
    let script = r#"
        [Console]::OutputEncoding = [System.Text.Encoding]::UTF8;
        $existing = Get-DnsClientNrptRule -ErrorAction SilentlyContinue | Where-Object { $_.Comment -eq 'Zapret2_SplitDNS' };
        if ($existing) {
            $existing | Remove-DnsClientNrptRule -Force -ErrorAction SilentlyContinue;
        }
    "#;

    let _ = Command::new(crate::platform::system32(r"WindowsPowerShell\v1.0\powershell.exe"))
        .creation_flags(CREATE_NO_WINDOW)
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", script])
        .output();

    Ok(())
}

/// Retrieves the comprehensive DNS configuration status silently without console popup windows.
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
        current_dns: if split_active && !split_servers.is_empty() {
            split_servers.clone()
        } else {
            active_servers
        },
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
        // Actually reset the adapter to DHCP, not just the setting —
        // otherwise stale custom servers stay on the interface.
        return set_system_dns("system", None);
    }
    get_dns_status()
}

///
/// The previously configured DNS servers are snapshotted before any change; if any step
/// fails, the adapter is rolled back to them and the failure is reported. Settings are
/// only persisted on success, so the UI can never report success for a failed change.
pub fn set_system_dns(provider_id: &str, scope: Option<&str>) -> Result<DnsStatus, String> {
    let settings = load_settings();
    // Single PowerShell round-trip gives us both the adapter and the pre-change DNS snapshot.
    let (adapter, previous_dns) = get_adapter_and_dns();
    let adapter_esc = escape_ps(&adapter);
    let providers = get_available_providers();

    let effective_scope = match scope {
        Some("split") => crate::platform::settings::DnsScope::Split,
        Some("full") => crate::platform::settings::DnsScope::Full,
        _ => settings.dns_scope,
    };

    let apply_result: Result<(), String> = (|| {
        if provider_id == "system" || provider_id == "dhcp" {
            // Reset Split-DNS NRPT rule
            clear_split_dns_nrpt()?;
            if effective_scope == crate::platform::settings::DnsScope::Full {
                // Reset adapter to automatic DHCP assignment only in Full mode
                reset_adapter_dns(&adapter_esc)?;
            }
            Ok(())
        } else if let Some(target) = providers.iter().find(|p| p.id == provider_id) {
            // Custom entries store "-" as a display placeholder for an empty
            // secondary; never pass it to PowerShell as an address.
            // Re-validate (defense in depth): settings are user-writable.
            let primary = target.primary.as_str();
            if !is_valid_ipv4(primary) {
                return Err(format!("Invalid DNS address in settings: {}", primary));
            }
            let secondary = if target.secondary == "-" {
                ""
            } else if target.secondary.is_empty() || is_valid_ipv4(&target.secondary) {
                target.secondary.as_str()
            } else {
                return Err(format!("Invalid DNS address in settings: {}", target.secondary));
            };
            match effective_scope {
                crate::platform::settings::DnsScope::Split => {
                    // In Split mode: NRPT cleanly overlays on top of the adapter without destroying user's static/custom DNS
                    apply_split_dns_nrpt(primary, secondary)
                }
                crate::platform::settings::DnsScope::Full => {
                    // In Full mode: clear any Split NRPT rules and set adapter DNS
                    clear_split_dns_nrpt()?;
                    set_adapter_dns(&adapter_esc, primary, secondary)
                }
            }
        } else {
            Err(format!("Unknown DNS provider ID: {}", provider_id))
        }
    })();

    if let Err(e) = apply_result {
        // Best-effort rollback to the DNS configuration found before the change.
        rollback_adapter_dns(&adapter_esc, &previous_dns);
        let _ = clear_split_dns_nrpt();
        return Err(e);
    }

    // Flush Windows DNS resolver cache immediately and silently
    let _ = Command::new(crate::platform::system32("ipconfig.exe"))
        .creation_flags(CREATE_NO_WINDOW)
        .arg("/flushdns")
        .output();

    // Persist choice in settings only after the change actually succeeded.
    // Serialized vs. concurrent mutations (L-4): re-load inside the lock so a
    // concurrent settings write in between is not clobbered.
    let _ = update_settings(|s| {
        s.dns_provider = provider_id.to_string();
        s.dns_scope = effective_scope;
    });

    get_dns_status()
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

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
use crate::win32::settings::{load_settings, save_settings};

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
    vec![
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
    ]
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

    let output = Command::new("powershell")
        .creation_flags(CREATE_NO_WINDOW)
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", script])
        .output();

    if let Ok(out) = output {
        let stdout = String::from_utf8_lossy(&out.stdout);
        let line = stdout.lines().filter(|l| !l.trim().is_empty()).last().unwrap_or("").trim();
        if let Some((adapter, dns_csv)) = line.split_once("::") {
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
        adapter
    );

    let output = Command::new("powershell")
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

    let output = Command::new("powershell")
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
    let ns_list = SPLIT_DNS_NAMESPACES
        .iter()
        .flat_map(|ns| {
            let clean = ns.trim_start_matches('.');
            vec![
                format!("'.{}'", clean),
                format!("'{}'", clean),
            ]
        })
        .collect::<Vec<_>>()
        .join(",");

    let script = format!(
        r#"
        [Console]::OutputEncoding = [System.Text.Encoding]::UTF8;
        $existing = Get-DnsClientNrptRule -ErrorAction SilentlyContinue | Where-Object {{ $_.Comment -eq 'Zapret2_SplitDNS' }};
        if ($existing) {{
            $existing | Remove-DnsClientNrptRule -Force -ErrorAction SilentlyContinue;
        }}
        $ns = @({});
        Add-DnsClientNrptRule -Namespace $ns -NameServers @('{}', '{}') -Comment 'Zapret2_SplitDNS' -ErrorAction Stop;
        "#,
        ns_list, primary, secondary
    );

    let output = Command::new("powershell")
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

/// Clears Zapret2 Split-DNS NRPT rule.
pub fn clear_split_dns_nrpt() -> Result<(), String> {
    let script = r#"
        [Console]::OutputEncoding = [System.Text.Encoding]::UTF8;
        $existing = Get-DnsClientNrptRule -ErrorAction SilentlyContinue | Where-Object { $_.Comment -eq 'Zapret2_SplitDNS' };
        if ($existing) {
            $existing | Remove-DnsClientNrptRule -Force -ErrorAction SilentlyContinue;
        }
    "#;

    let _ = Command::new("powershell")
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
        crate::win32::settings::DnsScope::Split => "split",
        crate::win32::settings::DnsScope::Full => "full",
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

/// Sets the DNS resolver on the primary adapter or selective Split-DNS via NRPT and flushes DNS cache silently.
pub fn set_system_dns(provider_id: &str, scope: Option<&str>) -> Result<DnsStatus, String> {
    let mut settings = load_settings();
    let adapter = detect_primary_adapter().unwrap_or_else(|_| "Ethernet".into());
    let providers = get_available_providers();

    let effective_scope = match scope {
        Some("split") => crate::win32::settings::DnsScope::Split,
        Some("full") => crate::win32::settings::DnsScope::Full,
        _ => settings.dns_scope,
    };

    if provider_id == "system" || provider_id == "dhcp" {
        // Reset Split-DNS NRPT rule
        let _ = clear_split_dns_nrpt();
        if effective_scope == crate::win32::settings::DnsScope::Full {
            // Reset adapter to automatic DHCP assignment only in Full mode
            let script = format!(
                "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; Set-DnsClientServerAddress -InterfaceAlias '{}' -ResetServerAddresses -ErrorAction Stop",
                adapter
            );
            let _ = Command::new("powershell")
                .creation_flags(CREATE_NO_WINDOW)
                .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &script])
                .output();
        }
    } else if let Some(target) = providers.iter().find(|p| p.id == provider_id) {
        match effective_scope {
            crate::win32::settings::DnsScope::Split => {
                // In Split mode: NRPT cleanly overlays on top of the adapter without destroying user's static/custom DNS
                apply_split_dns_nrpt(&target.primary, &target.secondary)?;
            }
            crate::win32::settings::DnsScope::Full => {
                // In Full mode: clear any Split NRPT rules and set adapter DNS
                let _ = clear_split_dns_nrpt();
                let script = format!(
                    "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; Set-DnsClientServerAddress -InterfaceAlias '{}' -ServerAddresses ('{}', '{}') -ErrorAction Stop",
                    adapter, target.primary, target.secondary
                );
                let output = Command::new("powershell")
                    .creation_flags(CREATE_NO_WINDOW)
                    .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &script])
                    .output()
                    .map_err(|e| format!("Failed to apply DNS settings: {}", e))?;

                if !output.status.success() {
                    let err_msg = String::from_utf8_lossy(&output.stderr);
                    return Err(format!("PowerShell DNS configuration error: {}", err_msg));
                }
            }
        }
    } else {
        return Err(format!("Unknown DNS provider ID: {}", provider_id));
    }

    // Flush Windows DNS resolver cache immediately and silently
    let _ = Command::new("ipconfig")
        .creation_flags(CREATE_NO_WINDOW)
        .arg("/flushdns")
        .output();

    // Persist choice in settings
    settings.dns_provider = provider_id.to_string();
    settings.dns_scope = effective_scope;
    let _ = save_settings(settings);

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

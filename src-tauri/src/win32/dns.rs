//! Windows DNS Management & DNS Spoofing Protection (DoH / Secure DNS)
//!
//! Provides one-click configuration of trusted anti-censorship and anti-spoofing DNS resolvers
//! directly on active network interfaces using PowerShell DnsClient cmdlets:
//! - Cloudflare (1.1.1.1, 1.0.0.1)
//! - Google Public DNS (8.8.8.8, 8.8.4.4)
//! - Quad9 Security (9.9.9.9, 149.112.112.112)
//! - Comss.one (92.223.109.31, 91.217.137.37)
//! - Default DHCP / ISP DNS

use std::process::Command;
use serde::{Deserialize, Serialize};
use crate::win32::settings::{load_settings, save_settings};

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

/// Detects the primary active network adapter name (Ethernet or Wi-Fi) by lowest RouteMetric default gateway.
pub fn detect_primary_adapter() -> Result<String, String> {
    let script = r#"
        [Console]::OutputEncoding = [System.Text.Encoding]::UTF8;
        $r = Get-NetRoute -DestinationPrefix '0.0.0.0/0' -ErrorAction SilentlyContinue | Sort-Object RouteMetric | Select-Object -First 1;
        if ($r) {
            $a = Get-NetAdapter -InterfaceIndex $r.InterfaceIndex -ErrorAction SilentlyContinue;
            if ($a) { $a.Name } else { $r.InterfaceAlias }
        } else {
            $a = Get-NetAdapter | Where-Object Status -eq 'Up' | Sort-Object LinkSpeed -Descending | Select-Object -First 1;
            if ($a) { $a.Name } else { 'Ethernet' }
        }
    "#;
    
    let output = Command::new("powershell")
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .map_err(|e| format!("Failed to query network adapters: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let name = stdout.lines().filter(|l| !l.trim().is_empty()).last().unwrap_or("").trim().to_string();
    if name.is_empty() {
        Ok("Ethernet".into())
    } else {
        Ok(name)
    }
}

/// Queries active DNS server addresses on the primary adapter.
pub fn get_active_dns_servers(adapter: &str) -> Vec<String> {
    let script = format!(
        "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; $d = (Get-DnsClientServerAddress -InterfaceAlias '{}' -AddressFamily IPv4 -ErrorAction SilentlyContinue).ServerAddresses; if ($d) {{ $d -join ',' }} else {{ '' }}",
        adapter
    );

    let output = Command::new("powershell")
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", &script])
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

/// Retrieves the comprehensive DNS configuration status.
pub fn get_dns_status() -> Result<DnsStatus, String> {
    let settings = load_settings();
    let adapter = detect_primary_adapter().unwrap_or_else(|_| "Ethernet".into());
    let active_servers = get_active_dns_servers(&adapter);

    Ok(DnsStatus {
        current_provider: settings.dns_provider.clone(),
        active_provider_id: settings.dns_provider,
        active_servers: active_servers.clone(),
        current_dns: active_servers,
        adapter_name: adapter,
        available_providers: get_available_providers(),
    })
}

/// Sets the DNS resolver on the primary adapter and flushes DNS cache.
pub fn set_system_dns(provider_id: &str) -> Result<DnsStatus, String> {
    let adapter = detect_primary_adapter().unwrap_or_else(|_| "Ethernet".into());
    let providers = get_available_providers();

    if provider_id == "system" || provider_id == "dhcp" {
        // Reset to automatic DHCP assignment
        let script = format!(
            "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; Set-DnsClientServerAddress -InterfaceAlias '{}' -ResetServerAddresses -ErrorAction Stop",
            adapter
        );
        let _ = Command::new("powershell")
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", &script])
            .output();
    } else if let Some(target) = providers.iter().find(|p| p.id == provider_id) {
        let script = format!(
            "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; Set-DnsClientServerAddress -InterfaceAlias '{}' -ServerAddresses ('{}', '{}') -ErrorAction Stop",
            adapter, target.primary, target.secondary
        );
        let output = Command::new("powershell")
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .map_err(|e| format!("Failed to apply DNS settings: {}", e))?;

        if !output.status.success() {
            let err_msg = String::from_utf8_lossy(&output.stderr);
            return Err(format!("PowerShell DNS configuration error: {}", err_msg));
        }
    } else {
        return Err(format!("Unknown DNS provider ID: {}", provider_id));
    }

    // Flush Windows DNS resolver cache immediately
    let _ = Command::new("ipconfig").arg("/flushdns").output();

    // Persist choice in settings
    let mut settings = load_settings();
    settings.dns_provider = provider_id.to_string();
    let _ = save_settings(settings);

    get_dns_status()
}

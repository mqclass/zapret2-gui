use std::net::{IpAddr, Ipv6Addr};

use ipnet::IpNet;
use percent_encoding::percent_decode_str;
use url::Url;

pub(super) struct OutboundConfig {
    pub proxy: Option<ProxyConfig>,
    pub no_proxy: Option<NoProxy>,
}

pub(super) struct NoProxy {
    matchers: Vec<NoProxyMatcher>,
}

enum NoProxyMatcher {
    Address(IpAddr, u16),
    Network(IpNet),
    Host {
        suffix: String,
        port: u16,
        include_apex: bool,
    },
    Wildcard,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ProxyConfig {
    pub kind: ProxyKind,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ProxyKind {
    Http,
    Socks5 { remote_dns: bool },
}

impl OutboundConfig {
    pub fn from_sources(
        outbound_proxy: Option<&str>,
        no_proxy: Option<&str>,
        use_env: bool,
    ) -> Result<Self, String> {
        Self::from_sources_with_env(outbound_proxy, no_proxy, use_env, env_value)
    }

    pub(super) fn from_sources_with_env(
        outbound_proxy: Option<&str>,
        no_proxy: Option<&str>,
        use_env: bool,
        env_get: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, String> {
        let proxy = select_proxy(outbound_proxy, use_env, &env_get)?;

        let no_proxy = if proxy.is_some() {
            let no_proxy_source = select_no_proxy(no_proxy, use_env, &env_get);
            if let Some(no_proxy) = no_proxy_source.as_deref()
                && !no_proxy.is_empty()
            {
                validate_no_proxy(no_proxy)?;
            }
            no_proxy_source
                .filter(|no_proxy| !no_proxy.is_empty())
                .map(|no_proxy| NoProxy::from_validated(&no_proxy))
        } else {
            None
        };

        Ok(Self { proxy, no_proxy })
    }
}

impl NoProxy {
    pub(super) fn from_validated(raw: &str) -> Self {
        let matchers = raw
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(parse_no_proxy_matcher)
            .collect();
        Self { matchers }
    }

    pub fn matches(&self, target_host: &str, target_port: u16) -> bool {
        let host = target_host
            .strip_prefix('[')
            .and_then(|host| host.strip_suffix(']'))
            .unwrap_or(target_host);
        let target_ip = host.parse::<IpAddr>().ok();

        if target_ip.is_some_and(|ip| ip.is_loopback()) {
            return true;
        }

        self.matchers.iter().any(|matcher| match matcher {
            NoProxyMatcher::Address(ip, port) => {
                Some(*ip) == target_ip && (*port == 0 || *port == target_port)
            }
            NoProxyMatcher::Network(network) => target_ip.is_some_and(|ip| network.contains(&ip)),
            NoProxyMatcher::Host {
                suffix,
                port,
                include_apex,
            } => (*port == 0 || *port == target_port) && host_matches(host, suffix, *include_apex),
            NoProxyMatcher::Wildcard => true,
        })
    }
}

fn parse_no_proxy_matcher(entry: &str) -> NoProxyMatcher {
    if entry == "*" {
        return NoProxyMatcher::Wildcard;
    }
    if let Ok(network) = entry.parse::<IpNet>() {
        return NoProxyMatcher::Network(network);
    }

    let (host, port) = if let Some(bracketed) = entry.strip_prefix('[') {
        let end = bracketed.find(']').expect("validated bracketed IPv6");
        let host = &bracketed[..end];
        let rest = &bracketed[end + 1..];
        let port = rest
            .strip_prefix(':')
            .map(|port| port.parse().expect("validated NO_PROXY port"))
            .unwrap_or(0);
        (host, port)
    } else if entry.contains(':') {
        let (host, port) = entry.rsplit_once(':').expect("validated host:port");
        (host, port.parse().expect("validated NO_PROXY port"))
    } else {
        (entry, 0)
    };

    if let Ok(ip) = host.parse::<IpAddr>() {
        return NoProxyMatcher::Address(ip, port);
    }

    let (suffix, include_apex) = host
        .strip_prefix("*.")
        .or_else(|| host.strip_prefix('.'))
        .map_or((host, true), |suffix| (suffix, false));
    NoProxyMatcher::Host {
        suffix: suffix.to_string(),
        port,
        include_apex,
    }
}

fn host_matches(host: &str, suffix: &str, include_apex: bool) -> bool {
    if include_apex && host.eq_ignore_ascii_case(suffix) {
        return true;
    }
    let Some(start) = host.len().checked_sub(suffix.len()) else {
        return false;
    };
    start > 0
        && host.as_bytes().get(start - 1) == Some(&b'.')
        && host
            .get(start..)
            .is_some_and(|tail| tail.eq_ignore_ascii_case(suffix))
}

impl ProxyConfig {
    fn parse(raw: &str) -> Result<Self, String> {
        let url = Url::parse(raw).map_err(|_| "invalid outbound proxy URL".to_string())?;
        validate_proxy_url_shape(&url)?;

        let kind = match url.scheme().to_ascii_lowercase().as_str() {
            "http" => ProxyKind::Http,
            "socks5" => ProxyKind::Socks5 { remote_dns: false },
            "socks5h" => ProxyKind::Socks5 { remote_dns: true },
            "https" => {
                return Err(
                    "HTTPS outbound proxy URLs are not supported; use an http:// CONNECT proxy"
                        .to_string(),
                );
            }
            scheme => return Err(format!("unsupported outbound proxy scheme: {scheme}")),
        };

        let host = url
            .host_str()
            .ok_or_else(|| "outbound proxy URL must include a host".to_string())?
            .to_string();
        let port = url.port().unwrap_or_else(|| default_port(kind));

        let username = decode_userinfo(url.username())?;
        let password = url.password().map(decode_required_userinfo).transpose()?;
        if username.is_none() && password.is_some() {
            return Err("proxy URL password requires a username".to_string());
        }

        Ok(Self {
            kind,
            host,
            port,
            username,
            password,
        })
    }

    pub fn summary(&self) -> String {
        let scheme = match self.kind {
            ProxyKind::Http => "http",
            ProxyKind::Socks5 { remote_dns: false } => "socks5",
            ProxyKind::Socks5 { remote_dns: true } => "socks5h",
        };
        let authority = authority(&self.host, self.port);
        if self.username.is_some() {
            format!("{scheme}://user:***@{authority}")
        } else {
            format!("{scheme}://{authority}")
        }
    }
}

pub(super) fn authority(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

fn default_port(kind: ProxyKind) -> u16 {
    match kind {
        ProxyKind::Http => 80,
        ProxyKind::Socks5 { .. } => 1080,
    }
}

fn decode_userinfo(value: &str) -> Result<Option<String>, String> {
    if value.is_empty() {
        Ok(None)
    } else {
        decode_required_userinfo(value).map(Some)
    }
}

fn decode_required_userinfo(value: &str) -> Result<String, String> {
    percent_decode_str(value)
        .decode_utf8()
        .map(|value| value.into_owned())
        .map_err(|_| "proxy URL userinfo is not valid UTF-8".to_string())
}

fn select_proxy(
    explicit: Option<&str>,
    use_env: bool,
    env_get: &impl Fn(&str) -> Option<String>,
) -> Result<Option<ProxyConfig>, String> {
    if let Some(value) = explicit {
        let value = value.trim();
        if !value.is_empty() {
            if is_direct_marker(value) {
                return Ok(None);
            }
            return ProxyConfig::parse(value).map(Some);
        }
    }

    if !use_env {
        return Ok(None);
    }

    select_env_proxy([
        ("HTTPS_PROXY", env_get("HTTPS_PROXY")),
        ("https_proxy", env_get("https_proxy")),
        ("ALL_PROXY", env_get("ALL_PROXY")),
        ("all_proxy", env_get("all_proxy")),
        ("HTTP_PROXY", env_get("HTTP_PROXY")),
        ("http_proxy", env_get("http_proxy")),
    ])
}

fn select_env_proxy(
    values: impl IntoIterator<Item = (&'static str, Option<String>)>,
) -> Result<Option<ProxyConfig>, String> {
    let mut first_error = None;

    for (name, value) in values {
        let Some(value) = value else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        if is_direct_marker(value) {
            return Ok(None);
        }

        match ProxyConfig::parse(value) {
            Ok(proxy) => return Ok(Some(proxy)),
            Err(err) => {
                first_error.get_or_insert_with(|| format!("{name}: {err}"));
            }
        }
    }

    match first_error {
        Some(err) => Err(format!(
            "no supported outbound proxy URL found in environment ({err})"
        )),
        None => Ok(None),
    }
}

fn select_no_proxy(
    explicit: Option<&str>,
    use_env: bool,
    env_get: &impl Fn(&str) -> Option<String>,
) -> Option<String> {
    if let Some(value) = explicit {
        return Some(value.trim().to_string());
    }

    if !use_env {
        return None;
    }

    first_non_empty([env_get("NO_PROXY"), env_get("no_proxy")])
}

fn first_non_empty(values: impl IntoIterator<Item = Option<String>>) -> Option<String> {
    values
        .into_iter()
        .flatten()
        .map(|value| value.trim().to_string())
        .find(|value| !value.is_empty())
}

fn env_value(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn is_direct_marker(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "direct" | "none" | "off"
    )
}

fn validate_proxy_url_shape(url: &Url) -> Result<(), String> {
    if !matches!(url.path(), "" | "/") || url.query().is_some() || url.fragment().is_some() {
        return Err(
            "outbound proxy URL must not include a path, query string or fragment".to_string(),
        );
    }
    Ok(())
}

fn validate_no_proxy(raw: &str) -> Result<(), String> {
    for entry in raw
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        validate_no_proxy_entry(entry)?;
    }

    Ok(())
}

fn validate_no_proxy_entry(entry: &str) -> Result<(), String> {
    if entry == "*" {
        return Ok(());
    }
    if entry.contains("://") {
        return Err("NO_PROXY entries must not include a URL scheme".to_string());
    }
    if entry.parse::<IpNet>().is_ok() {
        return Ok(());
    }
    if entry.starts_with('[') {
        return validate_bracketed_ipv6_no_proxy(entry);
    }
    if entry.contains('/') {
        return Err("invalid NO_PROXY CIDR entry".to_string());
    }
    if entry.contains(':') {
        let Some((host, port)) = entry.rsplit_once(':') else {
            return Err("invalid NO_PROXY entry".to_string());
        };
        if host.contains(':') {
            return Err("IPv6 NO_PROXY entries with ports must use brackets".to_string());
        }
        validate_hostname_no_proxy(host)?;
        validate_no_proxy_port(port)?;
        return Ok(());
    }

    validate_hostname_no_proxy(entry)
}

fn validate_bracketed_ipv6_no_proxy(entry: &str) -> Result<(), String> {
    let Some(end) = entry.find(']') else {
        return Err("invalid bracketed IPv6 NO_PROXY entry".to_string());
    };
    entry[1..end]
        .parse::<Ipv6Addr>()
        .map_err(|_| "invalid bracketed IPv6 NO_PROXY entry".to_string())?;
    let rest = &entry[end + 1..];
    if rest.is_empty() {
        return Ok(());
    }
    let Some(port) = rest.strip_prefix(':') else {
        return Err("invalid bracketed IPv6 NO_PROXY entry".to_string());
    };
    validate_no_proxy_port(port)
}

fn validate_no_proxy_port(port: &str) -> Result<(), String> {
    if port.is_empty() || port.parse::<u16>().is_err() {
        return Err("invalid NO_PROXY entry port".to_string());
    }
    Ok(())
}

fn validate_hostname_no_proxy(host: &str) -> Result<(), String> {
    let host = host
        .strip_prefix("*.")
        .or_else(|| host.strip_prefix('.'))
        .unwrap_or(host);
    if host.is_empty()
        || host.contains(['/', '[', ']', ':'])
        || host.split('.').any(|label| {
            label.is_empty()
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
    {
        return Err("invalid NO_PROXY host entry".to_string());
    }
    Ok(())
}

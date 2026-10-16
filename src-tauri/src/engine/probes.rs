//! Asynchronous High-Precision Network Diagnostics & DPI Evasion Probes
//!
//! # Architecture
//! Continuously evaluates real-time latency (DNS lookup, TLS/HTTP TTFB)
//! across critical services without saturating network connections:
//! 1. YouTube Web & 4K Streaming (`youtube.com:443`, `googlevideo.com:443`)
//! 2. Discord Gateway API (`discord.com:443`, `gateway.discord.gg:443`)
//! 3. Discord Voice Infrastructure (`voice.discord.gg:443`)
//! 4. Telegram Web & Media CDN (`web.telegram.org:443`)
//! 5. X / Twitter API & Media (`x.com:443`)
//! 6. Cloudflare Global Network (`1.1.1.1:443` / `cloudflare.com:443`)
//! 7. Custom Target Ping: User-specified domain or URL probing.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio::net::lookup_host;
use tokio::time::{sleep, timeout};

/// Operational status of a probed endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProbeStatus {
    Operational,
    Degraded,
    Blocked,
    Checking,
}

/// Detailed latency and connection metrics for a service endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeMetric {
    pub target: String,
    pub status: ProbeStatus,
    pub latency_ms: Option<u64>,
    pub dns_ms: Option<u64>,
    pub tcp_ms: Option<u64>,
    pub http_status: Option<u16>,
    pub packet_loss_pct: f32,
    pub last_checked_epoch: u64,
    pub ip_address: Option<String>,
    pub message: String,
}

/// Comprehensive diagnostics snapshot covering all probed targets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticsReport {
    pub youtube: ProbeMetric,
    pub discord: ProbeMetric,
    pub discord_voice: ProbeMetric,
    pub telegram: ProbeMetric,
    pub twitter: ProbeMetric,
    pub cloudflare: ProbeMetric,
    pub timestamp: u64,
}

/// Strategy benchmark benchmark evaluation for automated tuning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyBenchmarkResult {
    pub strategy_id: String,
    pub strategy_name: String,
    pub working: bool,
    pub average_ping_ms: Option<u64>,
    pub youtube_status: ProbeStatus,
    pub discord_status: ProbeStatus,
    pub summary: String,
}

impl Default for DiagnosticsReport {
    fn default() -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Self {
            youtube: Self::empty_metric("YouTube (4K Video)", now),
            discord: Self::empty_metric("Discord Gateway", now),
            discord_voice: Self::empty_metric("Discord Voice", now),
            telegram: Self::empty_metric("Telegram Web", now),
            twitter: Self::empty_metric("X (Twitter)", now),
            cloudflare: Self::empty_metric("Cloudflare Global", now),
            timestamp: now,
        }
    }
}

impl DiagnosticsReport {
    fn empty_metric(name: &str, now: u64) -> ProbeMetric {
        ProbeMetric {
            target: name.into(),
            status: ProbeStatus::Checking,
            latency_ms: None,
            dns_ms: None,
            tcp_ms: None,
            http_status: None,
            packet_loss_pct: 0.0,
            last_checked_epoch: now,
            ip_address: None,
            message: "Инициализация проверки...".into(),
        }
    }
}

/// Diagnostics service managing active network probes.
pub struct DiagnosticsService {
    report: Arc<RwLock<DiagnosticsReport>>,
    client: reqwest::Client,
    /// Redirect-limited client for IPC-supplied (untrusted) probe targets (M-6).
    untrusted_client: reqwest::Client,
}

impl DiagnosticsService {
    /// Creates a new instance with optimized timeouts and safe connection pooling.
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(3500))
            .connect_timeout(Duration::from_millis(2500))
            .danger_accept_invalid_certs(false)
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .build()
            .unwrap_or_default();

        // SECURITY (M-6): untrusted probe targets get at most 2 redirects so a
        // malicious redirect chain cannot be used to bounce requests around.
        let untrusted_client = reqwest::Client::builder()
            .timeout(Duration::from_millis(3500))
            .connect_timeout(Duration::from_millis(2500))
            .redirect(reqwest::redirect::Policy::limited(2))
            .danger_accept_invalid_certs(false)
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .build()
            .unwrap_or_default();

        Self {
            report: Arc::new(RwLock::new(DiagnosticsReport::default())),
            client,
            untrusted_client,
        }
    }

    /// Retrieves current report snapshot.
    pub fn get_report(&self) -> DiagnosticsReport {
        self.report.read().clone()
    }

    /// High-precision probe that measures DNS lookup, TCP connection time, and HTTP response.
    pub async fn measure_endpoint(
        &self,
        name: &str,
        host: &str,
        port: u16,
        url: &str,
    ) -> ProbeMetric {
        self.measure_endpoint_with(&self.client, name, host, port, url)
            .await
    }

    /// Same as [`Self::measure_endpoint`], but with an explicitly supplied HTTP
    /// client (used for untrusted targets with a redirect-limited client, M-6).
    async fn measure_endpoint_with(
        &self,
        client: &reqwest::Client,
        name: &str,
        host: &str,
        port: u16,
        url: &str,
    ) -> ProbeMetric {
        let now_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // 1. DNS Resolution Phase
        let dns_start = Instant::now();
        let resolve_addr = format!("{}:{}", host, port);
        let dns_res = timeout(Duration::from_millis(2000), lookup_host(&resolve_addr)).await;

        let (first_addr, dns_time_ms) = match dns_res {
            Ok(Ok(mut addrs)) => {
                let duration = dns_start.elapsed().as_millis() as u64;
                (addrs.next(), Some(duration))
            }
            _ => (None, None),
        };

        let target_addr: SocketAddr = match first_addr {
            Some(addr) => addr,
            None => {
                return ProbeMetric {
                    target: name.into(),
                    status: ProbeStatus::Blocked,
                    latency_ms: None,
                    dns_ms: None,
                    tcp_ms: None,
                    http_status: None,
                    packet_loss_pct: 100.0,
                    last_checked_epoch: now_epoch,
                    ip_address: None,
                    message: "Ошибка DNS: домен не резолвится (DNS-спуфинг или таймаут)".into(),
                };
            }
        };

        let ip_str = target_addr.ip().to_string();

        // 2. HTTPS Phase — a single connection (TCP + TLS + HTTP) via the pooled client.
        //    NOTE: the previous explicit raw TcpStream::connect phase was removed — it
        //    doubled every probe connection for marginal diagnostic value.
        let http_start = Instant::now();
        let http_res = client.get(url).send().await;
        let http_time_ms = http_start.elapsed().as_millis() as u64;

        match http_res {
            Ok(resp) => {
                let status_code = resp.status().as_u16();
                let is_ok = resp.status().is_success() || status_code == 204 || status_code == 301 || status_code == 302;
                let total_rtt = http_time_ms;

                let (status, msg) = if is_ok {
                    if total_rtt > 450 {
                        (ProbeStatus::Degraded, format!("Работает с задержкой (RTT: {}мс, HTTP {})", total_rtt, status_code))
                    } else {
                        (ProbeStatus::Operational, format!("Отличный отклик (RTT: {}мс)", total_rtt))
                    }
                } else {
                    (ProbeStatus::Degraded, format!("Ответ HTTP {} (RTT: {}мс)", status_code, total_rtt))
                };

                ProbeMetric {
                    target: name.into(),
                    status,
                    latency_ms: Some(total_rtt),
                    dns_ms: dns_time_ms,
                    tcp_ms: None,
                    http_status: Some(status_code),
                    packet_loss_pct: if is_ok { 0.0 } else { 20.0 },
                    last_checked_epoch: now_epoch,
                    ip_address: Some(ip_str),
                    message: msg,
                }
            }
            Err(e) => {
                ProbeMetric {
                    target: name.into(),
                    status: ProbeStatus::Blocked,
                    latency_ms: None,
                    dns_ms: dns_time_ms,
                    tcp_ms: None,
                    http_status: None,
                    packet_loss_pct: 100.0,
                    last_checked_epoch: now_epoch,
                    ip_address: Some(ip_str),
                    message: format!("Соединение отклонено или заблокировано (TCP/TLS/DPI): {}", e),
                }
            }
        }
    }

    /// Probes YouTube and GoogleVideo endpoints.
    pub async fn probe_youtube(&self) -> ProbeMetric {
        self.measure_endpoint(
            "YouTube / GoogleVideo",
            "www.youtube.com",
            443,
            "https://www.youtube.com/generate_204",
        ).await
    }

    /// Probes Discord Gateway.
    pub async fn probe_discord(&self) -> ProbeMetric {
        self.measure_endpoint(
            "Discord Gateway",
            "gateway.discord.gg",
            443,
            "https://discord.com/api/v10/gateway",
        ).await
    }

    /// Probes Discord Voice infrastructure.
    pub async fn probe_discord_voice(&self) -> ProbeMetric {
        self.measure_endpoint(
            "Discord Voice",
            "discord.media",
            443,
            "https://discord.com/api/v10/voice/regions",
        ).await
    }

    /// Probes Telegram Web & API.
    pub async fn probe_telegram(&self) -> ProbeMetric {
        self.measure_endpoint(
            "Telegram Web",
            "web.telegram.org",
            443,
            "https://web.telegram.org",
        ).await
    }

    /// Probes X / Twitter.
    pub async fn probe_twitter(&self) -> ProbeMetric {
        self.measure_endpoint(
            "X (Twitter)",
            "x.com",
            443,
            "https://x.com",
        ).await
    }

    /// Probes Cloudflare Global Network.
    pub async fn probe_cloudflare(&self) -> ProbeMetric {
        self.measure_endpoint(
            "Cloudflare Global",
            "1.1.1.1",
            443,
            "https://1.1.1.1",
        ).await
    }

    /// User-initiated custom ping for a domain or URL.
    ///
    /// SECURITY (M-6): this is reachable via IPC, so it must not become a blind
    /// SSRF oracle running with admin rights: HTTPS only, no private/loopback/
    /// link-local targets (including a DNS-rebinding check), max 2 redirects.
    pub async fn probe_custom(&self, target_input: &str) -> ProbeMetric {
        let clean = target_input.trim();
        let now_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let blocked = |msg: &str| ProbeMetric {
            target: clean.into(),
            status: ProbeStatus::Blocked,
            latency_ms: None,
            dns_ms: None,
            tcp_ms: None,
            http_status: None,
            packet_loss_pct: 100.0,
            last_checked_epoch: now_epoch,
            ip_address: None,
            message: msg.into(),
        };

        if clean.is_empty() {
            return blocked("Пустая цель для пробы");
        }
        if clean.starts_with("http://") {
            return blocked("HTTP запрещён для произвольных проб — используйте HTTPS");
        }
        let url_str = if clean.starts_with("https://") {
            clean.to_string()
        } else {
            let hostport = clean.split('/').next().unwrap_or(clean);
            format!("https://{}", hostport)
        };
        let parsed = match reqwest::Url::parse(&url_str) {
            Ok(u) if u.scheme() == "https" => u,
            _ => return blocked("Некорректный URL — разрешён только HTTPS"),
        };
        let host = parsed.host_str().unwrap_or_default().to_string();
        if host.is_empty() {
            return blocked("Не указан хост цели");
        }
        let port = parsed.port_or_known_default().unwrap_or(443);

        // 1. IP literals must be publicly routable.
        if let Ok(ip) = host.parse::<std::net::IpAddr>() {
            if !is_public_ip(&ip) {
                return blocked("Приватные и локальные IP запрещены для проб");
            }
        } else {
            // 2. Block localhost-style hostnames outright.
            let h = host.to_lowercase();
            if h == "localhost"
                || h.ends_with(".localhost")
                || h.ends_with(".local")
                || h.ends_with(".internal")
                || h.ends_with(".invalid")
            {
                return blocked("Локальные хосты запрещены для проб");
            }
            // 3. DNS-rebinding guard: resolve and require every returned
            // address to be publicly routable.
            match timeout(
                Duration::from_millis(2000),
                lookup_host(format!("{}:{}", host, port)),
            )
            .await
            {
                Ok(Ok(addrs)) => {
                    let mut any = false;
                    for a in addrs {
                        any = true;
                        if !is_public_ip(&a.ip()) {
                            return blocked("Хост резолвится в приватный IP — проба запрещена");
                        }
                    }
                    if !any {
                        return blocked("Хост не резолвится в IP-адреса");
                    }
                }
                _ => return blocked("Ошибка DNS при проверке цели пробы"),
            }
        }

        self.measure_endpoint_with(&self.untrusted_client, &host, &host, port, &url_str)
            .await
    }

    /// Runs all diagnostic probes concurrently and updates current report.
    pub async fn run_probes_now(&self) -> DiagnosticsReport {
        let (yt, disc, disc_v, tg, tw, cf) = tokio::join!(
            self.probe_youtube(),
            self.probe_discord(),
            self.probe_discord_voice(),
            self.probe_telegram(),
            self.probe_twitter(),
            self.probe_cloudflare()
        );

        let now_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let new_report = DiagnosticsReport {
            youtube: yt,
            discord: disc,
            discord_voice: disc_v,
            telegram: tg,
            twitter: tw,
            cloudflare: cf,
            timestamp: now_epoch,
        };

        *self.report.write() = new_report.clone();
        new_report
    }

    /// Starts the continuous background polling loop (every 30 seconds).
    pub fn start_polling_loop<F>(self: Arc<Self>, emit_callback: F)
    where
        F: Fn(DiagnosticsReport) + Send + Sync + 'static,
    {
        tokio::spawn(async move {
            loop {
                let report = self.run_probes_now().await;
                emit_callback(report);
                sleep(Duration::from_secs(30)).await;
            }
        });
    }
}

/// Returns true only for publicly routable IP addresses (M-6 SSRF guard).
/// Uses explicit range checks (no nightly-only `is_global`) so the result does
/// not depend on the toolchain version.
fn is_public_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let o = v4.octets();
            if o[0] == 10 {
                return false;
            } // 10.0.0.0/8
            if o[0] == 172 && (16..=31).contains(&o[1]) {
                return false;
            } // 172.16.0.0/12
            if o[0] == 192 && o[1] == 168 {
                return false;
            } // 192.168.0.0/16
            if o[0] == 127 {
                return false;
            } // 127.0.0.0/8 loopback
            if o[0] == 169 && o[1] == 254 {
                return false;
            } // 169.254.0.0/16 link-local
            if o[0] >= 224 {
                return false;
            } // 224.0.0.0/4 multicast + 240.0.0.0/4 reserved
            if o[0] == 0 {
                return false;
            } // 0.0.0.0/8 ("this network")
            if o[0] == 100 && (64..=127).contains(&o[1]) {
                return false;
            } // 100.64.0.0/10 CGNAT
            // Documentation / TEST-NET ranges
            if (o[0] == 192 && o[1] == 0 && o[2] == 2)
                || (o[0] == 198 && o[1] == 51 && o[2] == 100)
                || (o[0] == 203 && o[1] == 0 && o[2] == 113)
            {
                return false;
            }
            true
        }
        std::net::IpAddr::V6(v6) => {
            if v6.is_loopback() || v6.is_multicast() || v6.is_unspecified() {
                return false;
            }
            let seg = v6.segments();
            if (seg[0] & 0xfe00) == 0xfc00 {
                return false;
            } // fc00::/7 unique local
            if (seg[0] & 0xffc0) == 0xfe80 {
                return false;
            } // fe80::/10 link-local
            if seg[0] == 0x2001 && seg[1] == 0x0db8 {
                return false;
            } // 2001:db8::/32 documentation
            true
        }
    }
}

/// Splits a `host`, `host:port`, `[::1]` or `[::1]:port` input into (host, port).
/// IPv6-aware: a bare IPv6 literal (multiple colons, no brackets) is never split.
fn parse_host_port(input: &str) -> (String, u16) {
    let input = input.trim();
    if input.is_empty() {
        return ("google.com".to_string(), 443);
    }
    if let Some(rest) = input.strip_prefix('[') {
        // Bracketed IPv6 literal, optionally followed by :port
        if let Some(end) = rest.find(']') {
            let host = format!("[{}]", &rest[..end]);
            let port = rest[end + 1..]
                .strip_prefix(':')
                .and_then(|p| p.parse::<u16>().ok())
                .unwrap_or(443);
            return (host, port);
        }
        return (input.to_string(), 443);
    }
    // Only split on ':' when there is exactly one (hostname / IPv4 + port)
    if input.matches(':').count() == 1 {
        if let Some((h, p)) = input.split_once(':') {
            if !h.is_empty() {
                if let Ok(port) = p.parse::<u16>() {
                    return (h.to_string(), port);
                }
            }
        }
    }
    (input.to_string(), 443)
}

#[cfg(test)]
mod tests {
    use super::parse_host_port;

    #[test]
    fn test_parse_host_port() {
        assert_eq!(parse_host_port("example.com"), ("example.com".to_string(), 443));
        assert_eq!(parse_host_port("example.com:8080"), ("example.com".to_string(), 8080));
        assert_eq!(parse_host_port("1.2.3.4:53"), ("1.2.3.4".to_string(), 53));
        assert_eq!(parse_host_port("[::1]"), ("[::1]".to_string(), 443));
        assert_eq!(parse_host_port("[::1]:5353"), ("[::1]".to_string(), 5353));
        // Bare IPv6 literal must not be split on colons
        assert_eq!(parse_host_port("::1"), ("::1".to_string(), 443));
        assert_eq!(parse_host_port("example.com:notaport"), ("example.com:notaport".to_string(), 443));
    }
}

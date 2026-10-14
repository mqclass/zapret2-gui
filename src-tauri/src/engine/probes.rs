//! Asynchronous High-Precision Network Diagnostics & DPI Evasion Probes
//!
//! # Architecture
//! Continuously evaluates real-time latency (DNS lookup, TCP handshake, TLS/HTTP TTFB)
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
use tokio::net::{lookup_host, TcpStream};
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

        Self {
            report: Arc::new(RwLock::new(DiagnosticsReport::default())),
            client,
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

        // 2. TCP 3-Way Handshake Phase (SYN -> SYN/ACK)
        let tcp_start = Instant::now();
        let tcp_connect_res = timeout(Duration::from_millis(2500), TcpStream::connect(target_addr)).await;

        let tcp_time_ms = match tcp_connect_res {
            Ok(Ok(_stream)) => Some(tcp_start.elapsed().as_millis() as u64),
            Ok(Err(e)) => {
                return ProbeMetric {
                    target: name.into(),
                    status: ProbeStatus::Blocked,
                    latency_ms: None,
                    dns_ms: dns_time_ms,
                    tcp_ms: None,
                    http_status: None,
                    packet_loss_pct: 100.0,
                    last_checked_epoch: now_epoch,
                    ip_address: Some(ip_str),
                    message: format!("Сброс соединения TCP RST (блокировка ТСПУ): {}", e),
                };
            }
            Err(_) => {
                return ProbeMetric {
                    target: name.into(),
                    status: ProbeStatus::Blocked,
                    latency_ms: None,
                    dns_ms: dns_time_ms,
                    tcp_ms: None,
                    http_status: None,
                    packet_loss_pct: 100.0,
                    last_checked_epoch: now_epoch,
                    ip_address: Some(ip_str),
                    message: "Таймаут TCP соединения (пакеты отброшены провайдером / Blackhole)".into(),
                };
            }
        };

        // 3. HTTP / TLS Handshake Phase (GET / HEAD)
        let http_start = Instant::now();
        let http_res = self.client.get(url).send().await;
        let http_time_ms = http_start.elapsed().as_millis() as u64;

        match http_res {
            Ok(resp) => {
                let status_code = resp.status().as_u16();
                let is_ok = resp.status().is_success() || status_code == 204 || status_code == 301 || status_code == 302;
                let total_rtt = tcp_time_ms.unwrap_or(0) + http_time_ms;

                let (status, msg) = if is_ok {
                    if total_rtt > 450 {
                        (ProbeStatus::Degraded, format!("Работает с задержкой (RTT: {}мс, HTTP {})", total_rtt, status_code))
                    } else {
                        (ProbeStatus::Operational, format!("Отличный отклик (RTT: {}мс, TCP: {}мс)", total_rtt, tcp_time_ms.unwrap_or(0)))
                    }
                } else {
                    (ProbeStatus::Degraded, format!("Ответ HTTP {} (RTT: {}мс)", status_code, total_rtt))
                };

                ProbeMetric {
                    target: name.into(),
                    status,
                    latency_ms: Some(total_rtt),
                    dns_ms: dns_time_ms,
                    tcp_ms: tcp_time_ms,
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
                    tcp_ms: tcp_time_ms,
                    http_status: None,
                    packet_loss_pct: 100.0,
                    last_checked_epoch: now_epoch,
                    ip_address: Some(ip_str),
                    message: format!("TLS/DPI блокировка: {}", e),
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
            "voice.discord.gg",
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

    /// User-initiated custom ping for ANY domain or URL.
    pub async fn probe_custom(&self, target_input: &str) -> ProbeMetric {
        let clean = target_input.trim();
        let (host, port, url) = if clean.starts_with("http://") || clean.starts_with("https://") {
            let parsed = reqwest::Url::parse(clean).unwrap_or_else(|_| reqwest::Url::parse("https://google.com").unwrap());
            let h = parsed.host_str().unwrap_or("google.com").to_string();
            let p = parsed.port_or_known_default().unwrap_or(443);
            (h, p, clean.to_string())
        } else {
            let h = clean.split('/').next().unwrap_or(clean).to_string();
            (h.clone(), 443, format!("https://{}", h))
        };

        self.measure_endpoint(&host, &host, port, &url).await
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

    /// Starts the continuous background polling loop (every 7 seconds).
    pub fn start_polling_loop<F>(self: Arc<Self>, emit_callback: F)
    where
        F: Fn(DiagnosticsReport) + Send + Sync + 'static,
    {
        tokio::spawn(async move {
            loop {
                let report = self.run_probes_now().await;
                emit_callback(report);
                sleep(Duration::from_secs(7)).await;
            }
        });
    }
}

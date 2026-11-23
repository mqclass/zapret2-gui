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

use std::cmp::Ordering;
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

/// Deep body probe: how far past the handshake a probe must read to
/// prove the flow survives the ~16 KB TCP freeze. The engine's own
/// failure detector watches only the first 32 KB (maxseq=32768), so
/// a probe that stops at headers is blind exactly where the freeze
/// lives — it calls a frozen path Operational while the browser
/// stalls. Target 48 KB, pass at 32 KB.
pub const DEEP_PROBE_TARGET_BYTES: u64 = 48 * 1024;
pub const DEEP_PROBE_PASS_BYTES: u64 = 32 * 1024;
/// No chunk for this long mid-body means the flow stalled (matches
/// the youtube_probe node prober's stall window).
pub const DEEP_PROBE_CHUNK_TIMEOUT: Duration = Duration::from_secs(5);
/// HTTP phase of a deep probe (headers + body) must finish inside
/// this. The budget starts AFTER the DNS phase (up to 2 s), so the
/// worst wall time of a deep probe is ~12 s, not 10.
pub const DEEP_PROBE_DEADLINE: Duration = Duration::from_secs(10);

/// Outcome of one body download past the freeze zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyVerdict {
    /// At least `pass_bytes` arrived: the flow survives the freeze zone.
    Passed,
    /// The server closed the body cleanly below `pass_bytes`: a short
    /// body, not a stall — the check cannot judge this URL.
    ShortBody,
    /// No data (stall, error, or deadline) before `pass_bytes`.
    Stalled,
}

/// One measured body download: the bytes that actually arrived plus
/// the verdict. The caller decides what the sample means.
#[derive(Debug, Clone)]
pub struct BodySample {
    pub bytes_downloaded: u64,
    pub verdict: BodyVerdict,
}

/// Optional body phase for [`DiagnosticsService::measure_endpoint_with`]:
/// after headers arrive, download the start of the body past the
/// freeze zone before calling the endpoint healthy.
#[derive(Debug, Clone, Copy)]
pub struct BodyCheck {
    pub target_bytes: u64,
    pub pass_bytes: u64,
    /// No chunk for this long mid-body means the flow stalled.
    pub chunk_timeout: Duration,
}

/// Reads the start of `resp`'s body until `target_bytes`, the overall
/// `deadline`, a clean EOF, or a stall (no chunk for `chunk_timeout`).
/// Pure measurement: the caller decides what the sample means. The
/// response is dropped afterwards — a half-read body is not returned
/// to the pool, which is fine for a probe. Chunk loop mirrors the
/// youtube_probe node prober (`probe_host`).
pub(crate) async fn download_body_sample(
    mut resp: reqwest::Response,
    target_bytes: u64,
    pass_bytes: u64,
    chunk_timeout: Duration,
    deadline: Instant,
) -> BodySample {
    // The verdict logic below assumes target > pass: with
    // target <= pass, Passed would be unreachable and every sample
    // would read Stalled (fail-closed, but surprising).
    debug_assert!(target_bytes > pass_bytes);
    let mut bytes: u64 = 0;
    let mut eof = false;
    while bytes < target_bytes && Instant::now() < deadline {
        match timeout(chunk_timeout, resp.chunk()).await {
            Ok(Ok(Some(chunk))) => bytes += chunk.len() as u64,
            Ok(Ok(None)) => {
                eof = true;
                break;
            }
            // Read error, or no chunk within the window: stalled.
            _ => break,
        }
    }
    // The loop exits only via: target reached (bytes >= pass, since
    // target > pass), clean EOF, stall/error, or deadline. A partial
    // sample with no EOF is a stall by construction — it must fail
    // closed, never read as a pass.
    let verdict = if bytes >= pass_bytes {
        BodyVerdict::Passed
    } else if eof {
        BodyVerdict::ShortBody
    } else {
        BodyVerdict::Stalled
    };
    BodySample {
        bytes_downloaded: bytes,
        verdict,
    }
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

/// Real per-strategy probe result: the engine was actually started on this
/// strategy and live endpoints were probed through it. Unlike the old
/// estimator, nothing here is derived from another strategy's numbers:
/// each service status and latency was measured under this strategy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyProbeResult {
    pub strategy_id: String,
    pub strategy_name: String,
    pub youtube_status: ProbeStatus,
    pub youtube_ms: Option<u64>,
    pub discord_status: ProbeStatus,
    pub discord_ms: Option<u64>,
    pub voice_status: ProbeStatus,
    pub voice_ms: Option<u64>,
    /// Operational counts 2, Degraded 1, Blocked/Checking 0, summed over the
    /// three services (0-6). Higher is better; average latency breaks ties.
    pub score: u32,
    pub average_ping_ms: Option<u64>,
    /// False when the engine never reached Running for this strategy: the
    /// service statuses then mean "unreachable", not "blocked by DPI".
    pub engine_running: bool,
    /// Set when the strategy could not be probed at all (unknown id, engine
    /// failed to start). The caller must show it instead of the statuses.
    pub error: Option<String>,
}

/// Lightweight YouTube-only screen result: phase 1 of the two-phase
/// auto-tune. Same engine discipline as StrategyProbeResult (the engine
/// was really started on this strategy and the result carries whether it
/// stayed Running), but only YouTube was measured, so a screen pass is
/// cheaper than a full probe. Discord/voice are NOT measured here and
/// must never be inferred from this result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyScreenResult {
    pub strategy_id: String,
    pub strategy_name: String,
    pub youtube_status: ProbeStatus,
    pub youtube_ms: Option<u64>,
    /// False when the engine never reached Running for this strategy.
    pub engine_running: bool,
    /// Set when the strategy could not be screened at all (unknown id,
    /// engine failed to start). The caller must show it instead of the
    /// status.
    pub error: Option<String>,
}

impl StrategyScreenResult {
    /// Viable for the finals: the engine really ran, nothing failed, and
    /// YouTube was reachable (operational or degraded) through it. A
    /// blocked YouTube means the strategy does not help the main pain
    /// point, however well Discord might do.
    pub fn is_viable(&self) -> bool {
        self.engine_running
            && self.error.is_none()
            && matches!(
                self.youtube_status,
                ProbeStatus::Operational | ProbeStatus::Degraded
            )
    }
}

/// Screen-phase rank: reachable YouTube first, lower latency breaks
/// ties. Blocked/Checking sort last — they are never finalists, but the
/// order stays total so the ranking is stable and testable.
///
/// Canonical ordering, mirrored by the frontend auto-tune loop
/// (AutoTuneModal.tsx): the loop runs in the frontend, so Rust cannot
/// own the phase transition — but the rules live here, pinned by unit
/// tests, and the mirror is a transliteration of these three functions.
#[allow(dead_code)]
pub fn screen_status_rank(status: ProbeStatus) -> u8 {
    match status {
        ProbeStatus::Operational => 0,
        ProbeStatus::Degraded => 1,
        _ => 2,
    }
}

#[allow(dead_code)]
pub fn compare_screen_results(a: &StrategyScreenResult, b: &StrategyScreenResult) -> Ordering {
    screen_status_rank(a.youtube_status)
        .cmp(&screen_status_rank(b.youtube_status))
        .then_with(|| {
            a.youtube_ms
                .unwrap_or(u64::MAX)
                .cmp(&b.youtube_ms.unwrap_or(u64::MAX))
        })
}

/// Finalists for phase 2: viable screens only, best first, capped at
/// `limit`. Returns owned results in ranked order.
#[allow(dead_code)]
pub fn select_finalists(
    results: &[StrategyScreenResult],
    limit: usize,
) -> Vec<StrategyScreenResult> {
    let mut ranked: Vec<StrategyScreenResult> = results
        .iter()
        .filter(|r| r.is_viable())
        .cloned()
        .collect();
    ranked.sort_by(compare_screen_results);
    ranked.truncate(limit);
    ranked
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
        self.measure_endpoint_with(&self.client, name, host, port, url, None)
            .await
    }

    /// Same measurement through the redirect-limited untrusted client
    /// (M-6). The block classifier uses it for the through-path signal
    /// of user-supplied targets; built-in targets keep the main client
    /// via their dedicated probe methods.
    pub async fn measure_endpoint_untrusted(
        &self,
        name: &str,
        host: &str,
        port: u16,
        url: &str,
    ) -> ProbeMetric {
        self.measure_endpoint_with(&self.untrusted_client, name, host, port, url, None)
            .await
    }

    /// Same as [`Self::measure_endpoint`], but with an explicitly supplied HTTP
    /// client (used for untrusted targets with a redirect-limited client, M-6).
    /// When `body` is `Some`, a successful header response is followed by
    /// a body download past the freeze zone before the endpoint is
    /// called healthy (see [`BodyCheck`]).
    async fn measure_endpoint_with(
        &self,
        client: &reqwest::Client,
        name: &str,
        host: &str,
        port: u16,
        url: &str,
        body: Option<BodyCheck>,
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
        // The client's 3.5 s total timeout would cut a body phase
        // short, so a deep probe gets its own per-request budget.
        let mut req = client.get(url);
        if body.is_some() {
            req = req.timeout(DEEP_PROBE_DEADLINE);
        }
        let http_res = req.send().await;
        let http_time_ms = http_start.elapsed().as_millis() as u64;

        match http_res {
            Ok(resp) => {
                let status_code = resp.status().as_u16();
                let is_ok = resp.status().is_success() || status_code == 204 || status_code == 301 || status_code == 302;
                let total_rtt = http_time_ms;

                // Deep phase: headers alone cannot see the ~16 KB TCP
                // freeze, so read the body past 32 KB before calling
                // the endpoint healthy. A short body (clean EOF below
                // 32 KB) cannot judge the path and leaves the header
                // verdict standing. The response is consumed here;
                // nothing below uses it (status was copied out first).
                let mut body_sample: Option<BodySample> = None;
                if is_ok {
                    if let Some(check) = body {
                        body_sample = Some(
                            download_body_sample(
                                resp,
                                check.target_bytes,
                                check.pass_bytes,
                                check.chunk_timeout,
                                http_start + DEEP_PROBE_DEADLINE,
                            )
                            .await,
                        );
                    }
                }
                let stalled = matches!(
                    body_sample.as_ref().map(|s| s.verdict),
                    Some(BodyVerdict::Stalled)
                );

                let (status, msg) = if stalled {
                    let kb = body_sample
                        .as_ref()
                        .map(|s| s.bytes_downloaded / 1024)
                        .unwrap_or(0);
                    let msg = if kb == 0 {
                        "Соединение устанавливается, но загрузка не начинается — похоже на заморозку потока"
                            .to_string()
                    } else {
                        format!(
                            "Соединение устанавливается, но загрузка виснет после {} КБ — похоже на заморозку потока",
                            kb
                        )
                    };
                    (ProbeStatus::Degraded, msg)
                } else if is_ok {
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
                    packet_loss_pct: if stalled {
                        50.0
                    } else if is_ok {
                        0.0
                    } else {
                        20.0
                    },
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
    pub async fn probe_youtube(&self) -> ProbeMetric {        self.measure_endpoint(
            "YouTube / GoogleVideo",
            "www.youtube.com",
            443,
            "https://www.youtube.com/generate_204",
        ).await
    }

    /// YouTube probe with a body phase: the plain probe stops at
    /// headers (generate_204 has no body at all), so it cannot see
    /// the ~16 KB TCP freeze. The deep probe fetches the homepage
    /// and reads past 32 KB; a stall below that marks the endpoint
    /// Degraded, which the watchdog treats as degradation. Used by
    /// refresh_probes (watchdog ticks, manual refresh) and by the
    /// auto-tune finals — NOT by the 30 s polling loop, which keeps
    /// the light probe (a 48 KB homepage fetch every 30 s would be
    /// ~140 MB/day and risks tripping YouTube's bot defenses).
    pub async fn probe_youtube_deep(&self) -> ProbeMetric {
        self.measure_endpoint_with(
            &self.client,
            "YouTube / GoogleVideo",
            "www.youtube.com",
            443,
            "https://www.youtube.com/",
            Some(BodyCheck {
                target_bytes: DEEP_PROBE_TARGET_BYTES,
                pass_bytes: DEEP_PROBE_PASS_BYTES,
                chunk_timeout: DEEP_PROBE_CHUNK_TIMEOUT,
            }),
        )
        .await
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

        self.measure_endpoint_with(&self.untrusted_client, &host, &host, port, &url_str, None)
            .await
    }

    /// Runs all diagnostic probes concurrently and updates current report.
    /// Light probes only: the 30 s polling loop uses this.
    pub async fn run_probes_now(&self) -> DiagnosticsReport {
        self.run_probes_inner(false).await
    }

    /// Same, but YouTube runs the deep body probe: the watchdog and
    /// manual refreshes see the ~16 KB TCP freeze the light probe
    /// cannot. Costs one homepage fetch (≈48 KB + one chunk, ≤12 s
    /// wall: 2 s DNS + 10 s HTTP) per run.
    pub async fn run_probes_now_deep(&self) -> DiagnosticsReport {
        self.run_probes_inner(true).await
    }

    async fn run_probes_inner(&self, deep_youtube: bool) -> DiagnosticsReport {
        // One async block so both probe variants share a future type.
        let yt = async {
            if deep_youtube {
                self.probe_youtube_deep().await
            } else {
                self.probe_youtube().await
            }
        };
        let (yt, disc, disc_v, tg, tw, cf) = tokio::join!(
            yt,
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
///
/// `pub(crate)`: the block classifier (engine::block_classifier) applies the
/// same guard to every address it is about to connect to, including addresses
/// coming from the *system* resolver (§3.3 of the classifier design).
pub(crate) fn is_public_ip(ip: &std::net::IpAddr) -> bool {
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
            // IPv4-mapped (::ffff:a.b.c.d) and IPv4-compatible
            // (::a.b.c.d) addresses embed an IPv4 address: judge them
            // by the IPv4 rules above. Without this, ::ffff:192.168.0.1
            // would sail through the v6 range checks below as "public"
            // while connecting into the LAN all the same.
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public_ip(&std::net::IpAddr::V4(v4));
            }
            let seg = v6.segments();
            if seg[..6] == [0, 0, 0, 0, 0, 0] {
                // IPv4-compatible form. :: and ::1 also land here and
                // are rejected by the IPv4 rules (0.0.0.0/8).
                let v4 = std::net::Ipv4Addr::new(
                    (seg[6] >> 8) as u8,
                    seg[6] as u8,
                    (seg[7] >> 8) as u8,
                    seg[7] as u8,
                );
                return is_public_ip(&std::net::IpAddr::V4(v4));
            }
            if v6.is_loopback() || v6.is_multicast() || v6.is_unspecified() {
                return false;
            }
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
    use std::time::{Duration, Instant};

    use super::{
        download_body_sample, is_public_ip, parse_host_port, select_finalists, BodyCheck,
        BodyVerdict, DiagnosticsService, ProbeStatus, StrategyScreenResult,
    };

    /// Behaviors for the local test HTTP server (one connection).
    enum TestBody {
        /// Send `n` bytes at once, then close (Content-Length: n).
        Full(usize),
        /// Send `n` bytes, then hold the connection open without
        /// closing (Content-Length claims 1 MiB): the probe must
        /// notice the stall itself, not a close.
        StallAfter(usize),
    }

    async fn spawn_test_server(body: TestBody) -> std::net::SocketAddr {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                // Read the request (until end of headers).
                let mut buf = [0u8; 4096];
                let mut req = Vec::new();
                loop {
                    match socket.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => {
                            req.extend_from_slice(&buf[..n]);
                            if req.windows(4).any(|w| w == b"\r\n\r\n") {
                                break;
                            }
                        }
                    }
                }
                match body {
                    TestBody::Full(n) => {
                        let payload = vec![b'x'; n];
                        let head = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            n
                        );
                        let _ = socket.write_all(head.as_bytes()).await;
                        let _ = socket.write_all(&payload).await;
                    }
                    TestBody::StallAfter(n) => {
                        let payload = vec![b'x'; n];
                        let head = "HTTP/1.1 200 OK\r\nContent-Length: 1048576\r\n\r\n";
                        let _ = socket.write_all(head.as_bytes()).await;
                        let _ = socket.write_all(&payload).await;
                        tokio::time::sleep(Duration::from_secs(60)).await;
                    }
                }
            }
        });
        addr
    }

    async fn sample_from(addr: std::net::SocketAddr) -> super::BodySample {
        let client = reqwest::Client::new();
        let resp = client
            .get(format!("http://{addr}/"))
            .send()
            .await
            .unwrap();
        download_body_sample(
            resp,
            48 * 1024,
            32 * 1024,
            Duration::from_millis(200),
            Instant::now() + Duration::from_secs(5),
        )
        .await
    }

    #[tokio::test]
    async fn test_body_sample_passes_fast_body() {
        let addr = spawn_test_server(TestBody::Full(64 * 1024)).await;
        let sample = sample_from(addr).await;
        assert_eq!(sample.verdict, BodyVerdict::Passed);
        assert!(sample.bytes_downloaded >= 32 * 1024);
    }

    #[tokio::test]
    async fn test_body_sample_stalls_on_drip() {
        // The freeze shape: 16 KB arrive, then nothing — no close,
        // no error. The sample must say Stalled with exactly the
        // bytes that arrived, not hang and not pass.
        let addr = spawn_test_server(TestBody::StallAfter(16 * 1024)).await;
        let sample = sample_from(addr).await;
        assert_eq!(sample.verdict, BodyVerdict::Stalled);
        assert_eq!(sample.bytes_downloaded, 16 * 1024);
    }

    #[tokio::test]
    async fn test_body_sample_short_body_is_inconclusive() {
        let addr = spawn_test_server(TestBody::Full(4 * 1024)).await;
        let sample = sample_from(addr).await;
        assert_eq!(sample.verdict, BodyVerdict::ShortBody);
        assert_eq!(sample.bytes_downloaded, 4 * 1024);
    }

    fn test_body_check() -> BodyCheck {
        BodyCheck {
            target_bytes: 48 * 1024,
            pass_bytes: 32 * 1024,
            chunk_timeout: Duration::from_millis(200),
        }
    }

    async fn deep_metric_from(addr: std::net::SocketAddr) -> super::ProbeMetric {
        let diag = DiagnosticsService::new();
        diag.measure_endpoint_with(
            &diag.client,
            "test",
            "127.0.0.1",
            addr.port(),
            &format!("http://{addr}/"),
            Some(test_body_check()),
        )
        .await
    }

    #[tokio::test]
    async fn test_deep_probe_marks_freeze_degraded() {
        // Headers arrive fine (200) but the body freezes at 16 KB:
        // the header-only verdict would be Operational — the deep
        // probe must say Degraded so the watchdog escalates.
        let addr = spawn_test_server(TestBody::StallAfter(16 * 1024)).await;
        let metric = deep_metric_from(addr).await;
        assert_eq!(metric.status, ProbeStatus::Degraded);
        assert!(metric.message.contains("виснет"), "{}", metric.message);
        assert_eq!(metric.http_status, Some(200));
    }

    #[tokio::test]
    async fn test_deep_probe_zero_bytes_message() {
        // Headers arrive, the body never starts: the message must
        // not read "stalls after 0 KB".
        let addr = spawn_test_server(TestBody::StallAfter(0)).await;
        let metric = deep_metric_from(addr).await;
        assert_eq!(metric.status, ProbeStatus::Degraded);
        assert!(
            metric.message.contains("не начинается"),
            "{}",
            metric.message
        );
    }

    #[tokio::test]
    async fn test_deep_probe_passes_healthy_body() {
        let addr = spawn_test_server(TestBody::Full(64 * 1024)).await;
        let metric = deep_metric_from(addr).await;
        assert_eq!(metric.status, ProbeStatus::Operational);
    }

    #[tokio::test]
    async fn test_deep_probe_short_body_keeps_header_verdict() {
        // A 4 KB body that ends cleanly cannot judge the path: the
        // header verdict stands, no false Degraded.
        let addr = spawn_test_server(TestBody::Full(4 * 1024)).await;
        let metric = deep_metric_from(addr).await;
        assert_eq!(metric.status, ProbeStatus::Operational);
    }

    fn screen(id: &str, status: ProbeStatus, ms: Option<u64>) -> StrategyScreenResult {
        StrategyScreenResult {
            strategy_id: id.to_string(),
            strategy_name: id.to_string(),
            youtube_status: status,
            youtube_ms: ms,
            engine_running: true,
            error: None,
        }
    }

    #[test]
    fn test_screen_result_serializes_snake_case() {
        // The frontend reads these exact field names; a rename breaks the
        // modal silently (undefined statuses, no crash).
        let r = screen("yt4k", ProbeStatus::Operational, Some(84));
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["strategy_id"], "yt4k");
        assert_eq!(v["strategy_name"], "yt4k");
        assert_eq!(v["youtube_status"], "operational");
        assert_eq!(v["youtube_ms"], 84);
        assert_eq!(v["engine_running"], true);
        assert!(v["error"].is_null());
        // And it round-trips.
        let back: StrategyScreenResult = serde_json::from_value(v).unwrap();
        assert_eq!(back.strategy_id, "yt4k");
        assert!(back.is_viable());
    }

    #[test]
    fn test_select_finalists_ranks_status_then_latency() {
        let input = vec![
            screen("slow-op", ProbeStatus::Operational, Some(300)),
            screen("blocked", ProbeStatus::Blocked, None),
            screen("fast-op", ProbeStatus::Operational, Some(90)),
            screen("degraded", ProbeStatus::Degraded, Some(50)),
            screen("checking", ProbeStatus::Checking, None),
            StrategyScreenResult {
                engine_running: false,
                ..screen("dead-engine", ProbeStatus::Operational, Some(10))
            },
            StrategyScreenResult {
                error: Some("boom".to_string()),
                ..screen("errored", ProbeStatus::Operational, Some(5))
            },
        ];
        let out = select_finalists(&input, 8);
        let ids: Vec<&str> = out.iter().map(|r| r.strategy_id.as_str()).collect();
        // Viable only; operational first by latency, then degraded.
        // Blocked/Checking, a dead engine and an errored screen are out,
        // even though two of them carry the lowest latencies.
        assert_eq!(ids, vec!["fast-op", "slow-op", "degraded"]);
    }

    #[test]
    fn test_select_finalists_caps_at_limit_and_keeps_best() {
        let input: Vec<StrategyScreenResult> = (0..10)
            .map(|i| {
                screen(
                    &format!("cand-{i}"),
                    ProbeStatus::Operational,
                    Some(100 + i * 10),
                )
            })
            .collect();
        let out = select_finalists(&input, 8);
        assert_eq!(out.len(), 8);
        assert_eq!(out[0].strategy_id, "cand-0");
        assert_eq!(out[7].strategy_id, "cand-7");
    }

    #[test]
    fn test_select_finalists_empty_when_nothing_viable() {
        let input = vec![
            screen("blocked", ProbeStatus::Blocked, None),
            screen("checking", ProbeStatus::Checking, None),
            StrategyScreenResult {
                engine_running: false,
                ..screen("dead", ProbeStatus::Operational, Some(10))
            },
        ];
        assert!(select_finalists(&input, 8).is_empty());
    }

    #[test]
    fn test_screen_viability_needs_engine_and_reachable_youtube() {
        assert!(screen("ok", ProbeStatus::Operational, Some(90)).is_viable());
        assert!(screen("ok", ProbeStatus::Degraded, Some(90)).is_viable());
        assert!(!screen("ok", ProbeStatus::Blocked, None).is_viable());
        assert!(!screen("ok", ProbeStatus::Checking, None).is_viable());
        assert!(!StrategyScreenResult {
            engine_running: false,
            ..screen("ok", ProbeStatus::Operational, Some(90))
        }
        .is_viable());
        assert!(!StrategyScreenResult {
            error: Some("x".to_string()),
            ..screen("ok", ProbeStatus::Operational, Some(90))
        }
        .is_viable());
    }

    #[test]
    fn test_is_public_ip_ipv4_embedded_in_ipv6() {
        let check = |s: &str| is_public_ip(&s.parse::<std::net::IpAddr>().unwrap());
        // IPv4-mapped forms of private/loopback addresses are NOT
        // public, even though their v6 shape matches no v6 deny range.
        assert!(!check("::ffff:192.168.0.1"));
        assert!(!check("::ffff:127.0.0.1"));
        assert!(!check("::ffff:10.0.0.5"));
        // IPv4-compatible forms get the same IPv4 judgement.
        assert!(!check("::192.168.0.1"));
        assert!(!check("::127.0.0.1"));
        // Embedded public addresses stay public in both forms.
        assert!(check("::ffff:8.8.8.8"));
        assert!(check("::8.8.8.8"));
        // Plain v6 behaviour is unchanged.
        assert!(check("2606:4700:4700::1111"));
        assert!(!check("fe80::1"));
        assert!(!check("fd00::1"));
    }

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

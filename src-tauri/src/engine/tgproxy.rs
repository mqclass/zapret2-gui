//! Native Telegram MTProto→WebSocket bridge, embedded in the supervisor.
//!
//! Instead of downloading and supervising an external TgWsProxy executable,
//! the bridge from `tg-ws-proxy-rs` (MIT) runs in-process as a Tokio task:
//!
//! ```text
//! Telegram Desktop → MTProto (127.0.0.1:port) → bridge task → WebSocket (TLS) → Telegram DC
//! ```
//!
//! The WebSocket transport is the actual circumvention: traffic looks like
//! ordinary HTTPS to `web.telegram.org` and passes DPI that blocks raw
//! MTProto or Telegram IP ranges. The local MTProto listener is just the
//! interface Telegram clients know how to speak.
//!
//! The frontend keeps talking to the same IPC surface (`get_tgproxy_status`,
//! `start_tgproxy`, `stop_tgproxy`, `set_tgproxy_config`,
//! `regenerate_tgproxy_secret`); the "download binary" step is gone because
//! there is nothing to download anymore.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::{Mutex, RwLock};
use rand::rngs::OsRng;
use rand::RngCore;
use crate::engine::events::{EventSink, SinkSlot};

use crate::engine::supervisor::{Supervisor, SupervisorState};

/// Loopback bind address — the default. The bridge never listens on more
/// than the user explicitly allows.
pub const TGWS_DEFAULT_HOST: &str = "127.0.0.1";
/// Bind address when LAN access is enabled (phones on the local network).
pub const TGWS_LAN_HOST: &str = "0.0.0.0";
/// Default MTProto listen port (Telegram clients default to 1443 for proxies).
pub const TGWS_DEFAULT_PORT: u16 = 1443;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TgProxyStatus {
    pub state: SupervisorState,
    pub host: String,
    pub port: u16,
    /// 32-hex-char MTProto secret; empty when not generated yet.
    pub secret: String,
    /// Ready-to-use `tg://proxy?...` deep link (empty when no secret yet).
    pub tg_link: String,
    pub uptime_secs: u64,
    pub last_error: Option<String>,
    /// Connection mode: "auto", "cfproxy" or "direct".
    pub mode: String,
    /// Whether LAN access is enabled (bridge bound to 0.0.0.0).
    pub lan_enabled: bool,
    /// This machine's LAN IPv4 address (for the phone QR code); None when
    /// LAN mode is off or the address could not be determined.
    pub lan_ip: Option<String>,
    /// Stale-secret pattern live right now: at least
    /// [`STALE_BAD_HANDSHAKE_THRESHOLD`] bad handshakes inside
    /// [`STALE_BAD_HANDSHAKE_WINDOW`]. Most likely a Telegram client still
    /// configured with an old tg:// link. Cleared by a good handshake or a
    /// bridge restart. Serde defaults keep older svc replies decodable.
    #[serde(default)]
    pub stale_secret_detected: bool,
    /// The bridge start() found a stored-but-unusable secret and generated
    /// a fresh one, so old tg:// links stopped working. This is NOT set on
    /// a first run (no secret was ever stored). Cleared by a good handshake
    /// or an explicit secret change.
    #[serde(default)]
    pub secret_rotated: bool,
    /// Bad handshakes inside the current window (diagnostics, not a secret).
    #[serde(default)]
    pub bad_handshake_count: usize,
    /// Monotonic rotation generation for this process (see BridgeHealth).
    #[serde(default)]
    pub rotation_count: u64,
    /// Monotonic stale-episode generation for this process (see BridgeHealth).
    #[serde(default)]
    pub stale_trip_count: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum TgProxyError {
    #[error("bridge error: {0}")]
    Bridge(String),
    #[error("bridge is already running")]
    AlreadyRunning,
    #[error("port must be 1..=65535, got {0}")]
    BadPort(u16),
    #[error("secret must be exactly 32 hex chars (0-9, a-f)")]
    BadSecret,
    #[error("unknown connection mode: {0} (expected auto, cfproxy or direct)")]
    BadMode(String),
    #[error("stop the bridge before changing its config")]
    ConfigLocked,
    #[error("settings: {0}")]
    Settings(String),
}

/// Valid connection modes for the bridge. Unknown values fall back to
/// "auto" — a corrupt settings file must never break the bridge.
pub fn normalize_mode(raw: &str) -> &'static str {
    match raw.trim().to_lowercase().as_str() {
        "cfproxy" => "cfproxy",
        "direct" => "direct",
        _ => "auto",
    }
}

/// Strict validation for user input (the mode picker).
fn validate_mode(raw: &str) -> Result<&'static str, TgProxyError> {
    match raw.trim().to_lowercase().as_str() {
        "auto" => Ok("auto"),
        "cfproxy" => Ok("cfproxy"),
        "direct" => Ok("direct"),
        other => Err(TgProxyError::BadMode(other.to_string())),
    }
}

/// Stale-secret detection: how many bad handshakes inside what window
/// count as a pattern, not an accident.
///
/// Why 5 in 60 s: a single typo or a casual port probe produces one or two
/// attempts and must never raise a banner. A real Telegram client with an
/// old secret retries persistently (the 27.10 case produced dozens within
/// seconds), so 5 trips within seconds while staying far above probe
/// noise. The window slides: bursts older than 60 s stop counting.
///
/// Known limit, by design: a client that tries fewer than 5 times in
/// 60 s never trips the banner. That is the price of not crying wolf
/// on probes and typos. The residual gap is narrow: such a slow client
/// is invisible to the app (the connection check uses the current
/// secret, so it passes, and per-attempt bad lines are suppressed at
/// any rate), and only the user's Telegram client itself shows the
/// failure. Lowering the threshold would trade false banners for
/// slow-client coverage, which is worse than the gap.
const STALE_BAD_HANDSHAKE_THRESHOLD: usize = 5;
const STALE_BAD_HANDSHAKE_WINDOW: Duration = Duration::from_secs(60);

/// In-memory stale-secret state for one bridge process.
///
/// Lives on the manager, not in settings: it describes the current run
/// (recent handshakes, whether this start rotated the secret). It never
/// touches the secret or the bridge config — detection only observes and
/// reports. A good handshake proves a client with the current secret
/// connected, so it clears both warnings; a bridge restart re-arms the
/// counter from zero.
struct BridgeHealth {
    /// Timestamps of bad handshakes inside the current window.
    bad_times: Mutex<VecDeque<Instant>>,
    /// Threshold reached in the current window.
    stale: RwLock<bool>,
    /// This process's start() replaced a stored-but-unusable secret.
    rotated: RwLock<bool>,
    /// Monotonic rotation generation for this process. Never reset by
    /// reset_run() or a good handshake: the frontend re-arms a dismissed
    /// rotation banner when this changes, even if `rotated` never left
    /// true (dismiss → second DPAPI loss → second rotation).
    rotation_count: RwLock<u64>,
    /// Monotonic stale-episode generation for this process. Incremented
    /// exactly when a fresh stale pattern trips (false -> true), never
    /// reset by reset_run() or a good handshake: the frontend re-arms a
    /// dismissed stale banner when this changes, even if the poll missed
    /// the false gap between two episodes (good handshake + retrip
    /// inside one 5 s poll would otherwise look like true -> true).
    stale_trip_count: RwLock<u64>,
}

impl BridgeHealth {
    fn new() -> Self {
        Self {
            bad_times: Mutex::new(VecDeque::new()),
            stale: RwLock::new(false),
            rotated: RwLock::new(false),
            rotation_count: RwLock::new(0),
            stale_trip_count: RwLock::new(0),
        }
    }

    /// Records one bad handshake. Returns (became_stale, count_in_window).
    fn record_bad_handshake(&self) -> (bool, usize) {
        let mut times = self.bad_times.lock();
        let now = Instant::now();
        times.retain(|t| now.duration_since(*t) <= STALE_BAD_HANDSHAKE_WINDOW);
        times.push_back(now);
        let count = times.len();
        let mut stale = self.stale.write();
        if count >= STALE_BAD_HANDSHAKE_THRESHOLD && !*stale {
            *stale = true;
            *self.stale_trip_count.write() += 1;
            return (true, count);
        }
        (false, count)
    }

    /// Records one good handshake. Returns true when a live warning
    /// (stale pattern or unresolved rotation) was cleared by it.
    fn record_good_handshake(&self) -> bool {
        self.bad_times.lock().clear();
        let was_stale = *self.stale.read();
        let was_rotated = *self.rotated.read();
        if was_stale || was_rotated {
            *self.stale.write() = false;
            *self.rotated.write() = false;
            true
        } else {
            false
        }
    }

    /// Fresh counting for a new bridge run. Rotation deliberately survives:
    /// a stop/start does not fix the user's stale client.
    fn reset_run(&self) {
        self.bad_times.lock().clear();
        *self.stale.write() = false;
    }

    fn snapshot(&self) -> (bool, bool, usize, u64, u64) {
        let mut times = self.bad_times.lock();
        let now = Instant::now();
        times.retain(|t| now.duration_since(*t) <= STALE_BAD_HANDSHAKE_WINDOW);
        (
            *self.stale.read(),
            *self.rotated.read(),
            times.len(),
            *self.rotation_count.read(),
            *self.stale_trip_count.read(),
        )
    }
}

/// `tracing` layer funneling the bridge library's diagnostics into the
/// supervisor log buffer.
///
/// m2: without a subscriber the library's WS errors and fallback-ladder
/// decisions are invisible (it logs via `tracing`). The library also prints
/// the MTProto secret in its startup banner, so every message is scrubbed
/// against the current secret by exact match before it reaches our logs
/// (which users can export as a diagnostic package).
///
/// Stale-secret detection hooks in here because the library reports
/// handshakes ONLY via tracing: there is no callback API, and forking the
/// vendored library for a counter would be worse than matching its two
/// stable debug strings. Per-event bad-handshake lines are the log flood
/// from the 27.10 case, so they are counted and suppressed; only state
/// transitions reach the log (one line each). Good-handshake lines are kept:
/// they are infrequent and are the evidence that the bridge itself is fine.
struct BridgeLogLayer {
    supervisor: Arc<Supervisor>,
    redact: Arc<RwLock<String>>,
    health: Arc<BridgeHealth>,
}

struct MsgVisitor {
    message: String,
}

impl tracing::field::Visit for MsgVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        }
    }
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        }
    }
}

impl<S> tracing_subscriber::Layer<S> for BridgeLogLayer
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut visitor = MsgVisitor {
            message: String::new(),
        };
        event.record(&mut visitor);
        if visitor.message.is_empty() {
            return;
        }
        // Stale-secret detection: the library's only handshake signals.
        // A bad handshake cannot be attributed to the user's client (any
        // TCP speaker on the loopback port trips it), which is exactly why
        // a single one is never enough — only the thresholded pattern
        // raises the UI banner, and only transitions are logged.
        // Match on the two reason markers, not the full vendor sentence:
        // the label prefix and the parenthetical punctuation are the
        // parts most likely to move on a vendor bump. The pair is
        // narrower than either marker alone, and a stray match still
        // needs 5 hits in 60 s to raise anything (and only a state
        // transition is logged), so the blast radius stays small. If
        // the vendor rewords the reasons themselves, detection silently
        // stops matching: re-check this predicate on every vendor bump.
        let text = visitor.message.as_str();
        if text.contains("bad handshake")
            && (text.contains("wrong secret") || text.contains("reserved prefix"))
        {
            let (became_stale, count) = self.health.record_bad_handshake();
            if became_stale {
                self.supervisor.log(
                    "warn",
                    &format!(
                        "[tg-bridge] Похоже, Telegram стучится со старым секретом: {count} неудачных подключений за 60 секунд. Обновите ссылку в Telegram."
                    ),
                );
            }
            return;
        }
        let mut warning_cleared = false;
        if visitor.message.contains("handshake ok:") {
            warning_cleared = self.health.record_good_handshake();
        }
        let secret = self.redact.read();
        let msg = if secret.is_empty() {
            visitor.message
        } else {
            visitor.message.replace(secret.as_str(), "[redacted]")
        };
        let level = match *event.metadata().level() {
            tracing::Level::ERROR => "error",
            tracing::Level::WARN => "warn",
            _ => "info",
        };
        self.supervisor.log(level, &format!("[tg-bridge] {msg}"));
        if warning_cleared {
            self.supervisor.log(
                "info",
                "[tg-bridge] Подключение с актуальным секретом: предупреждение о старой ссылке снято",
            );
        }
    }
}

/// Manages the embedded MTProto→WebSocket bridge lifecycle.
///
/// The bridge itself runs as a Tokio task inside this process; `stop()`
/// signals it through a oneshot channel and awaits its completion, so no
/// orphaned state can survive us.
pub struct TgProxyManager {
    /// Serializes start()/stop() so they can never interleave.
    lifecycle: Arc<tokio::sync::Mutex<()>>,
    state: Arc<RwLock<SupervisorState>>,
    start_time: Arc<RwLock<Option<Instant>>>,
    last_error: Arc<RwLock<Option<String>>>,
    /// Latest `tg://proxy?...` link reported by the bridge on bind.
    tg_link: Arc<RwLock<String>>,
    bridge_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    shutdown_tx: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    /// Current MTProto secret, mirrored for log redaction (see BridgeLogLayer).
    log_redact: Arc<RwLock<String>>,
    /// Stale-secret detection state (see BridgeHealth). Same lifetime
    /// reasoning as log_redact: the manager is a process singleton, so the
    /// Once-installed log layer keeps pointing at the right object.
    health: Arc<BridgeHealth>,
    sink: SinkSlot,
    supervisor: Arc<Supervisor>,
}

impl TgProxyManager {
    pub fn new(supervisor: Arc<Supervisor>) -> Self {
        Self {
            lifecycle: Arc::new(tokio::sync::Mutex::new(())),
            state: Arc::new(RwLock::new(SupervisorState::Stopped)),
            start_time: Arc::new(RwLock::new(None)),
            last_error: Arc::new(RwLock::new(None)),
            tg_link: Arc::new(RwLock::new(String::new())),
            bridge_task: Arc::new(Mutex::new(None)),
            shutdown_tx: Arc::new(Mutex::new(None)),
            log_redact: Arc::new(RwLock::new(String::new())),
            health: Arc::new(BridgeHealth::new()),
            sink: SinkSlot::new(),
            supervisor,
        }
    }

    /// Installs the UI event sink.
    pub fn set_event_sink(&self, sink: Arc<dyn EventSink>) {
        self.sink.set(sink);
    }

    fn emit_state(&self, state: SupervisorState) {
        self.sink.emit_tgproxy_state(state);
    }

    fn is_valid_secret(s: &str) -> bool {
        s.len() == 32 && s.chars().all(|c| c.is_ascii_hexdigit())
    }

    fn generate_secret() -> String {
        let mut buf = [0u8; 16];
        OsRng.fill_bytes(&mut buf);
        let mut out = String::with_capacity(32);
        for b in buf {
            out.push_str(&format!("{:02x}", b));
        }
        out
    }

    fn normalize_port(raw: u16) -> u16 {
        if raw == 0 {
            TGWS_DEFAULT_PORT
        } else {
            raw
        }
    }

    /// Finds a free TCP port, starting from the preferred one. If the
    /// preferred port is busy (another app, a stale bridge), walks up to 20
    /// ports ahead instead of failing. The chosen port is persisted so the
    /// UI and the tg:// link always show the real one.
    fn find_free_port(host: &str, preferred: u16) -> u16 {
        for p in preferred..preferred.saturating_add(20) {
            if p == 0 {
                continue;
            }
            if std::net::TcpListener::bind(format!("{host}:{p}")).is_ok() {
                return p;
            }
            if p == 65535 {
                break;
            }
        }
        preferred
    }

    /// Detects this machine's LAN IPv4 address via the standard UDP trick:
    /// "connect" a datagram socket (no packets are sent) and read the local
    /// address the OS would use. Returns None when offline.
    fn detect_lan_ip() -> Option<String> {
        let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
        sock.connect("8.8.8.8:80").ok()?;
        let ip = sock.local_addr().ok()?.ip();
        match ip {
            std::net::IpAddr::V4(v4) if !v4.is_loopback() => Some(v4.to_string()),
            _ => None,
        }
    }

    fn bind_host(lan: bool) -> &'static str {
        if lan {
            TGWS_LAN_HOST
        } else {
            TGWS_DEFAULT_HOST
        }
    }

    /// Builds the bridge configuration through the library's own CLI parser,
    /// so all of its defaults (timeouts, pool sizes, fallbacks) stay exactly
    /// as the upstream project ships them — we only override what the user
    /// controls: port, bind host, secret and connection mode.
    ///
    /// The mode pins the upstream tier order: "auto" leaves the library's
    /// default ladder alone (CF-proxy → TCP fallback; the direct WS tier is
    /// skipped when no --dc-ip targets are configured); "cfproxy" tries
    /// Cloudflare first; "direct" skips Cloudflare entirely. A pinned list
    /// never falls through to unlisted tiers — that is the point of pinning.
    fn bridge_config(
        host: &str,
        port: u16,
        secret: &str,
        mode: &str,
    ) -> Result<tg_ws_proxy_rs::config::Config, TgProxyError> {
        // M1 hardening: the library declares `env = "TG_..."` on ~40 options.
        // try_parse_from would silently inherit them from our process
        // environment (TG_SKIP_TLS_VERIFY, TG_CHECK, TG_CF_DOMAIN, ...),
        // changing bridge behavior behind the user's back. Nothing else in
        // this app consumes TG_*, so drop them before parsing.
        let tg_keys: Vec<String> = std::env::vars()
            .map(|(k, _)| k)
            .filter(|k| k.starts_with("TG_"))
            .collect();
        for k in tg_keys {
            std::env::remove_var(k);
        }
        use clap::Parser;
        let mut args = vec![
            "zapret2-tgbridge".to_string(),
            "--port".to_string(),
            port.to_string(),
            "--host".to_string(),
            host.to_string(),
            "--secret".to_string(),
            secret.to_string(),
        ];
        // Pin the tier order for non-default modes. Media connections keep
        // the default ladder (pinning media too aggressively breaks calls).
        //
        // --default-domains enables the cfproxy tier via the community CF
        // domain list. Without it the cfproxy tier is silently skipped in
        // auto mode (leaving only DPI-blocked direct TCP), and in cfproxy
        // mode the bridge refuses to start at all (pinned to an unconfigured
        // tier). "direct" intentionally skips Cloudflare entirely.
        match normalize_mode(mode) {
            "cfproxy" => {
                args.push("--default-domains".to_string());
                args.push("--pinned-upstream".to_string());
                args.push("cfproxy,ws,tcp".to_string());
            }
            "direct" => {
                args.push("--pinned-upstream".to_string());
                args.push("ws,tcp".to_string());
            }
            _ => {
                args.push("--default-domains".to_string());
            }
        }
        tg_ws_proxy_rs::config::Config::try_parse_from(args)
            .map_err(|e| TgProxyError::Bridge(format!("invalid bridge config: {e}")))
    }

    pub fn get_status(&self) -> TgProxyStatus {
        let settings = crate::win32::settings::load_settings();
        let port = Self::normalize_port(settings.tgproxy_port);
        let secret = settings.tgproxy_secret.clone();
        let uptime_secs = self
            .start_time
            .read()
            .map(|t| t.elapsed().as_secs())
            .unwrap_or(0);
        let lan_enabled = settings.tgproxy_lan;
        let link_host = Self::bind_host(lan_enabled);
        // The library generates the canonical tg:// link on bind; fall back
        // to a locally built one when the bridge has not bound yet.
        let tg_link = {
            let link = self.tg_link.read();
            if link.is_empty() && Self::is_valid_secret(&secret) {
                format!(
                    "tg://proxy?server={}&port={}&secret={}",
                    link_host, port, secret
                )
            } else {
                link.clone()
            }
        };
        let host = Self::bind_host(lan_enabled);
        let (stale_secret_detected, secret_rotated, bad_handshake_count, rotation_count, stale_trip_count) =
            self.health.snapshot();
        TgProxyStatus {
            state: *self.state.read(),
            host: host.to_string(),
            port,
            secret,
            tg_link,
            uptime_secs,
            last_error: self.last_error.read().clone(),
            mode: normalize_mode(&settings.tgproxy_mode).to_string(),
            lan_enabled,
            lan_ip: if lan_enabled {
                Self::detect_lan_ip()
            } else {
                None
            },
            stale_secret_detected,
            secret_rotated,
            bad_handshake_count,
            rotation_count,
            stale_trip_count,
        }
    }

    /// Starts the embedded bridge. Generates and persists a secret on first
    /// run so the user never has to think about it.
    pub async fn start(self: Arc<Self>) -> Result<(), TgProxyError> {
        let _lifecycle_guard = self.lifecycle.lock().await;
        {
            let state = self.state.read();
            if *state == SupervisorState::Running || *state == SupervisorState::Starting {
                return Err(TgProxyError::AlreadyRunning);
            }
        }

        // Ensure a valid secret exists (persisted via settings; DPAPI at rest
        // is handled transparently by the settings layer).
        let settings = crate::win32::settings::load_settings();
        let mut secret = settings.tgproxy_secret.clone();
        // A fresh counting run: handshakes from a previous run must not
        // combine with the new one. Rotation deliberately survives restarts.
        self.health.reset_run();
        if !Self::is_valid_secret(&secret) {
            // Rotation vs first run: a stored-but-unusable value means old
            // tg:// links just died (e.g. DPAPI unprotect failed after a
            // profile move); an absent value means the user never had one
            // and must not be scared with a rotation warning.
            let had_stored_secret =
                crate::win32::settings::stored_tgproxy_secret_present();
            secret = Self::generate_secret();
            let secret_clone = secret.clone();
            crate::win32::settings::update_settings(|s| {
                s.tgproxy_secret = secret_clone;
            })
            .map_err(|e| TgProxyError::Settings(e.to_string()))?;
            if had_stored_secret {
                *self.health.rotated.write() = true;
                // Monotonic generation: a second rotation must re-arm a
                // dismissed banner even though `rotated` never left true.
                *self.health.rotation_count.write() += 1;
                self.supervisor.log(
                    "warn",
                    "Секрет Telegram-моста был утерян (например, после переноса профиля на другого пользователя или машину) и создан заново. Старые ссылки в Telegram больше не работают.",
                );
            } else {
                self.supervisor.log("info", "Сгенерирован новый MTProto-secret");
            }
        }
        let host = Self::bind_host(settings.tgproxy_lan);
        let mut port = Self::normalize_port(settings.tgproxy_port);
        // If the preferred port is busy, pick a free one nearby instead of
        // failing. Persist it so the UI and tg:// link show the real port.
        let free = Self::find_free_port(host, port);
        if free != port {
            self.supervisor.log(
                "info",
                &format!("Порт {port} занят, мост использует {free}"),
            );
            port = free;
            crate::win32::settings::update_settings(|s| {
                s.tgproxy_port = port;
            })
            .map_err(|e| TgProxyError::Settings(e.to_string()))?;
        }

        // m2: route the bridge's tracing diagnostics into our log buffer.
        // Installed once per process; the layer holds an Arc to `log_redact`
        // so secret rotations are picked up without reinstalling.
        *self.log_redact.write() = secret.clone();
        Self::install_bridge_logging(
            self.supervisor.clone(),
            self.log_redact.clone(),
            self.health.clone(),
        );

        // M2: a config error must never leave us stuck in `Starting` with no
        // task running — build it before touching the state machine.
        let config = Self::bridge_config(host, port, &secret, &settings.tgproxy_mode)?;

        *self.state.write() = SupervisorState::Starting;
        *self.last_error.write() = None;
        *self.tg_link.write() = String::new();
        self.emit_state(SupervisorState::Starting);
        self.supervisor.log(
            "info",
            &format!("Запуск встроенного Telegram-моста на {host}:{port}…"),
        );

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        *self.shutdown_tx.lock() = Some(shutdown_tx);
        *self.start_time.write() = Some(Instant::now());

        let this = self.clone();
        let handle = tokio::spawn(async move {
            let tg_link = this.tg_link.clone();
            let state = this.state.clone();
            let last_error = this.last_error.clone();
            let supervisor = this.supervisor.clone();

            // on_listen is FnOnce: hand it its own Arc clones so the originals
            // stay usable in the match below after run_with_listen returns.
            let cb_tg_link = tg_link.clone();
            let cb_state = state.clone();
            let cb_supervisor = supervisor.clone();
            let cb_this = this.clone();

            let result = tg_ws_proxy_rs::server::run_with_listen(
                config,
                async move {
                    let _ = shutdown_rx.await;
                },
                move |info| {
                    *cb_tg_link.write() = info.tg_link.clone();
                    // m5: stop() may have fired while we were still binding —
                    // don't flash `Running` on the way to `Stopped`.
                    let mut st = cb_state.write();
                    if *st == SupervisorState::Starting {
                        *st = SupervisorState::Running;
                        drop(st);
                        cb_this.emit_state(SupervisorState::Running);
                    }
                    cb_supervisor.log(
                        "info",
                        &format!(
                            "Telegram-мост слушает {} — введите параметры в Telegram-клиенте",
                            info.addr
                        ),
                    );
                },
            )
            .await;

            // The task ended. A deliberate stop() owns the state transition
            // (it set Stopping before awaiting us); anything else is either
            // an unexpected death or a bind failure — surface it honestly.
            match result {
                Ok(()) => {
                    let mut st = state.write();
                    if *st == SupervisorState::Starting || *st == SupervisorState::Running {
                        *st = SupervisorState::Stopped;
                    }
                }
                Err(e) => {
                    let msg = format!("{e}");
                    supervisor.log("error", &format!("Telegram-мост: {msg}"));
                    *last_error.write() = Some(msg);
                    let mut st = state.write();
                    if *st != SupervisorState::Stopping && *st != SupervisorState::Stopped {
                        *st = SupervisorState::Error;
                        this.emit_state(SupervisorState::Error);
                    }
                }
            }
        });

        *self.bridge_task.lock() = Some(handle);

        // Persist the "enabled" intent so it survives app restarts.
        let _ = crate::win32::settings::update_settings(|s| s.tgproxy_enabled = true);
        Ok(())
    }

    /// Stops the bridge: signals shutdown, awaits the task, then marks stopped.
    /// Never holds a lock across `.await` — the future must stay `Send` for
    /// Tauri commands.
    pub async fn stop(&self) -> Result<(), TgProxyError> {
        self.stop_impl(true).await
    }

    /// Stops the bridge WITHOUT clearing the persisted enabled-intent.
    /// Used when the bridge migrates into the svc: the intent stays on, the
    /// svc copy autostarts from the same flag, and the user's toggle is not
    /// silently flipped by an internal handover.
    pub async fn stop_transient(&self) -> Result<(), TgProxyError> {
        self.stop_impl(false).await
    }

    async fn stop_impl(&self, persist_intent: bool) -> Result<(), TgProxyError> {
        let _lifecycle_guard = self.lifecycle.lock().await;
        {
            let state = self.state.read();
            if *state == SupervisorState::Stopped {
                return Ok(());
            }
        }
        *self.state.write() = SupervisorState::Stopping;
        self.emit_state(SupervisorState::Stopping);
        self.supervisor.log("info", "Остановка Telegram-моста…");

        // Signal shutdown first, then take the task handle — each guarded by
        // its own short critical section, never across an await point.
        let tx = self.shutdown_tx.lock().take();
        if let Some(tx) = tx {
            let _ = tx.send(());
        }
        let task = self.bridge_task.lock().take();
        if let Some(mut task) = task {
            // A wedged bridge task must not hang stop() (and with it the svc
            // shutdown and update install): give it a bounded grace period,
            // then abort.
            if tokio::time::timeout(std::time::Duration::from_secs(5), &mut task)
                .await
                .is_err()
            {
                task.abort();
                let _ = task.await;
            }
        }

        *self.start_time.write() = None;
        *self.tg_link.write() = String::new();
        *self.state.write() = SupervisorState::Stopped;
        self.emit_state(SupervisorState::Stopped);
        // A stopped bridge cannot produce handshakes; the next start
        // re-arms from zero. Rotation survives: stopping does not fix
        // the user's stale client.
        self.health.reset_run();

        if persist_intent {
            let _ = crate::win32::settings::update_settings(|s| s.tgproxy_enabled = false);
        }
        self.supervisor.log("info", "Telegram-мост остановлен");
        Ok(())
    }

    /// Installs the global tracing subscriber exactly once. Our app does
    /// not use `tracing` itself, so this only affects the bridge library's
    /// diagnostics — nothing else starts logging unexpectedly.
    fn install_bridge_logging(
        supervisor: Arc<Supervisor>,
        redact: Arc<RwLock<String>>,
        health: Arc<BridgeHealth>,
    ) {
        use tracing_subscriber::layer::SubscriberExt;
        static INSTALLED: std::sync::Once = std::sync::Once::new();
        INSTALLED.call_once(|| {
            let layer = BridgeLogLayer {
                supervisor,
                redact,
                health,
            };
            let subscriber = tracing_subscriber::registry().with(layer);
            let _ = tracing::subscriber::set_global_default(subscriber);
        });
    }

    fn ensure_config_change_allowed(&self) -> Result<(), TgProxyError> {
        match *self.state.read() {
            SupervisorState::Stopped | SupervisorState::Error => Ok(()),
            _ => Err(TgProxyError::ConfigLocked),
        }
    }

    /// Updates port/secret. Refused while running — the bridge reads config
    /// only at startup, so a silent no-op would be worse than an explicit error.
    pub fn set_config(&self, port: u16, secret: String) -> Result<(), TgProxyError> {
        self.ensure_config_change_allowed()?;
        if port == 0 {
            return Err(TgProxyError::BadPort(port));
        }
        let secret = secret.trim().to_lowercase();
        if !Self::is_valid_secret(&secret) {
            return Err(TgProxyError::BadSecret);
        }
        crate::win32::settings::update_settings(|settings| {
            settings.tgproxy_port = port;
            settings.tgproxy_secret = secret.clone();
        })
        .map_err(|e| TgProxyError::Settings(e.to_string()))?;
        *self.log_redact.write() = secret;
        // A deliberate secret change supersedes any rotation warning: the
        // user set this secret themselves, nothing was silently lost.
        *self.health.rotated.write() = false;
        Ok(())
    }

    /// Updates the connection mode ("auto", "cfproxy", "direct").
    /// Refused while running — the bridge reads the mode only at startup.
    pub fn set_mode(&self, mode: String) -> Result<(), TgProxyError> {
        self.ensure_config_change_allowed()?;
        let mode = validate_mode(&mode)?.to_string();
        crate::win32::settings::update_settings(|settings| {
            settings.tgproxy_mode = mode.clone();
        })
        .map_err(|e| TgProxyError::Settings(e.to_string()))?;
        Ok(())
    }

    /// Toggles LAN access (bind to 0.0.0.0 so phones on the local network
    /// can use the bridge). Refused while running.
    pub fn set_lan(&self, enabled: bool) -> Result<(), TgProxyError> {
        self.ensure_config_change_allowed()?;
        crate::win32::settings::update_settings(|settings| {
            settings.tgproxy_lan = enabled;
        })
        .map_err(|e| TgProxyError::Settings(e.to_string()))?;
        Ok(())
    }

    /// Honest end-to-end test of the running bridge: TCP connect to the
    /// listener → obfuscated MTProto handshake → real `req_pq_multi` → the
    /// reply must decrypt to Telegram's `resPQ`. Only the DC's answer proves
    /// the full chain (listener, chosen upstream tier, DC) works.
    ///
    /// Returns the DC round-trip time, or a human-readable failure naming
    /// the broken step.
    pub async fn test_connection(&self) -> Result<std::time::Duration, TgProxyError> {
        if *self.state.read() != SupervisorState::Running {
            return Err(TgProxyError::Bridge(
                "мост не запущен — сначала включите его".to_string(),
            ));
        }
        let settings = crate::win32::settings::load_settings();
        let port = Self::normalize_port(settings.tgproxy_port);
        let secret = settings.tgproxy_secret.clone();
        if !Self::is_valid_secret(&secret) {
            return Err(TgProxyError::BadSecret);
        }
        let mut raw = [0u8; 16];
        for (i, chunk) in secret.as_bytes().chunks(2).enumerate() {
            raw[i] =
                u8::from_str_radix(std::str::from_utf8(chunk).unwrap_or("00"), 16).unwrap_or(0);
        }
        let addr: std::net::SocketAddr = format!("{TGWS_DEFAULT_HOST}:{port}")
            .parse()
            .map_err(|e| TgProxyError::Bridge(format!("bad listener address: {e}")))?;
        // DC 2: a representative data centre, same as the library's own
        // --check-listener probe. dc_idx 0 is not a documented "pick for me".
        match tg_ws_proxy_rs::check::probe_bridge_listener(
            addr,
            &raw,
            2,
            std::time::Duration::from_secs(15),
        )
        .await
        {
            tg_ws_proxy_rs::check::BridgeProbeResult::Ok(rtt) => {
                self.supervisor.log(
                    "info",
                    &format!("Проверка моста: Telegram ответил за {} мс", rtt.as_millis()),
                );
                Ok(rtt)
            }
            tg_ws_proxy_rs::check::BridgeProbeResult::Fail(e) => {
                Err(TgProxyError::Bridge(format!("проверка не прошла: {e}")))
            }
        }
    }

    /// Generates a fresh CSPRNG secret and persists it.
    pub fn regenerate_secret(&self) -> Result<String, TgProxyError> {
        self.ensure_config_change_allowed()?;
        let secret = Self::generate_secret();
        let secret_clone = secret.clone();
        crate::win32::settings::update_settings(|settings| {
            settings.tgproxy_secret = secret_clone;
        })
        .map_err(|e| TgProxyError::Settings(e.to_string()))?;
        *self.log_redact.write() = secret.clone();
        // Same reasoning as set_config: an explicit regeneration is not a loss.
        *self.health.rotated.write() = false;
        Ok(secret)
    }
}

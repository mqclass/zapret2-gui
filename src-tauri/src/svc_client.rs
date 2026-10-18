//! GUI-side client for `zapret2-svc.exe`.
//!
//! [`SvcClient`] speaks the JSON-lines protocol over the named pipe. Two
//! connections are used: one for sequential request/response commands, one
//! for the event subscription. [`SupervisorBackend`] abstracts "local
//! in-process supervisor" vs "remote svc", so Tauri commands don't care.

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::windows::named_pipe::ClientOptions;
use tokio::sync::Mutex;

use crate::engine::{LogEntry, Supervisor, SupervisorStatus, TgProxyManager, TgProxyStatus};
use crate::svc_ipc::{ClientMessage, ServerMessage, SvcEvent, PROTOCOL_VERSION, SVC_PIPE_NAME};

/// Connected client to a running svc instance. Commands are serialized
/// through a mutex: one in-flight request at a time.
pub struct SvcClient {
    io: Mutex<BufReader<tokio::net::windows::named_pipe::NamedPipeClient>>,
    /// Last successfully fetched status + when. Returned on transient pipe
    /// failures so the UI doesn't fabricate an "Error" state (which prompts
    /// users to hammer the start button). Expires after 30s so a dead svc
    /// can't masquerade as "running" forever.
    last_good: Mutex<Option<(SupervisorStatus, std::time::Instant)>>,
}

impl SvcClient {
    pub async fn connect() -> io::Result<Self> {
        let client = ClientOptions::new().open(SVC_PIPE_NAME)?;
        Ok(Self {
            io: Mutex::new(BufReader::new(client)),
            last_good: Mutex::new(None),
        })
    }

    /// Version handshake + liveness check. Returns the svc's atomic status
    /// snapshot; rejects the peer on protocol mismatch instead of
    /// miscommunicating silently.
    pub async fn hello(&self) -> io::Result<SupervisorStatus> {
        match self
            .roundtrip(&ClientMessage::Hello {
                version: PROTOCOL_VERSION,
            })
            .await?
        {
            ServerMessage::Welcome { version, status } if version == PROTOCOL_VERSION => Ok(status),
            ServerMessage::Response { ok: false, error, .. } => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                error.unwrap_or_else(|| "hello rejected".to_string()),
            )),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "bad hello response",
            )),
        }
    }

    async fn roundtrip(&self, msg: &ClientMessage) -> io::Result<ServerMessage> {
        let mut io = self.io.lock().await;
        let mut line = serde_json::to_string(msg)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        line.push('\n');
        io.get_mut().write_all(line.as_bytes()).await?;
        io.get_mut().flush().await?;
        let mut resp = String::new();
        let n = io.read_line(&mut resp).await?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "svc closed the connection",
            ));
        }
        serde_json::from_str(&resp)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    fn into_response(msg: ServerMessage) -> Result<ServerMessage, String> {
        match msg {
            ServerMessage::Response { ok: true, .. } => Ok(msg),
            ServerMessage::Response {
                ok: false, error, ..
            } => Err(error.unwrap_or_else(|| "svc error".to_string())),
            ServerMessage::Welcome { .. } => Err("unexpected welcome".to_string()),
            ServerMessage::Event(_) => Err("unexpected event".to_string()),
        }
    }

    pub async fn get_status(&self) -> Result<SupervisorStatus, String> {
        let msg = self
            .roundtrip(&ClientMessage::GetStatus)
            .await
            .map_err(|e| e.to_string())?;
        let status = match Self::into_response(msg)? {
            ServerMessage::Response { status: Some(s), .. } => s,
            _ => return Err("svc returned no status".to_string()),
        };
        // Cache the good read for transient pipe failures.
        *self.last_good.lock().await = Some((status.clone(), std::time::Instant::now()));
        Ok(status)
    }

    /// Last known-good status, if fresh (under 30s old).
    pub async fn last_known_status(&self) -> Option<SupervisorStatus> {
        let guard = self.last_good.lock().await;
        guard.as_ref().and_then(|(s, t)| {
            if t.elapsed() < std::time::Duration::from_secs(30) {
                Some(s.clone())
            } else {
                None
            }
        })
    }

    pub async fn start(
        &self,
        strategy_id: Option<String>,
        custom_args: Option<Vec<String>>,
    ) -> Result<(), String> {
        let msg = self
            .roundtrip(&ClientMessage::Start {
                strategy_id,
                custom_args,
            })
            .await
            .map_err(|e| e.to_string())?;
        Self::into_response(msg).map(|_| ())
    }

    pub async fn stop(&self) -> Result<(), String> {
        let msg = self
            .roundtrip(&ClientMessage::Stop)
            .await
            .map_err(|e| e.to_string())?;
        Self::into_response(msg).map(|_| ())
    }

    pub async fn get_logs(&self, limit: usize) -> Result<Vec<LogEntry>, String> {
        let msg = self
            .roundtrip(&ClientMessage::GetLogs { limit })
            .await
            .map_err(|e| e.to_string())?;
        match Self::into_response(msg)? {
            ServerMessage::Response { logs: Some(l), .. } => Ok(l),
            _ => Err("svc returned no logs".to_string()),
        }
    }

    pub async fn clear_logs(&self) -> Result<(), String> {
        let msg = self
            .roundtrip(&ClientMessage::ClearLogs)
            .await
            .map_err(|e| e.to_string())?;
        Self::into_response(msg).map(|_| ())
    }

    pub async fn tgproxy_start(&self) -> Result<(), String> {
        let msg = self
            .roundtrip(&ClientMessage::TgproxyStart)
            .await
            .map_err(|e| e.to_string())?;
        Self::into_response(msg).map(|_| ())
    }

    pub async fn tgproxy_stop(&self) -> Result<(), String> {
        let msg = self
            .roundtrip(&ClientMessage::TgproxyStop)
            .await
            .map_err(|e| e.to_string())?;
        Self::into_response(msg).map(|_| ())
    }

    pub async fn tgproxy_get_status(&self) -> Result<TgProxyStatus, String> {
        let msg = self
            .roundtrip(&ClientMessage::TgproxyGetStatus)
            .await
            .map_err(|e| e.to_string())?;
        match Self::into_response(msg)? {
            ServerMessage::Response {
                tgproxy_status: Some(s),
                ..
            } => Ok(s),
            _ => Err("svc returned no tgproxy status".to_string()),
        }
    }

    pub async fn tgproxy_set_config(&self, port: u16, secret: String) -> Result<(), String> {
        let msg = self
            .roundtrip(&ClientMessage::TgproxySetConfig { port, secret })
            .await
            .map_err(|e| e.to_string())?;
        Self::into_response(msg).map(|_| ())
    }

    pub async fn tgproxy_regenerate_secret(&self) -> Result<String, String> {
        let msg = self
            .roundtrip(&ClientMessage::TgproxyRegenerateSecret)
            .await
            .map_err(|e| e.to_string())?;
        match Self::into_response(msg)? {
            ServerMessage::Response {
                secret: Some(s), ..
            } => Ok(s),
            _ => Err("svc returned no secret".to_string()),
        }
    }

    pub async fn tgproxy_set_mode(&self, mode: String) -> Result<(), String> {
        let msg = self
            .roundtrip(&ClientMessage::TgproxySetMode { mode })
            .await
            .map_err(|e| e.to_string())?;
        Self::into_response(msg).map(|_| ())
    }

    /// End-to-end test through the bridge. Returns the DC round-trip time.
    pub async fn tgproxy_test_connection(&self) -> Result<std::time::Duration, String> {
        let msg = self
            .roundtrip(&ClientMessage::TgproxyTestConnection)
            .await
            .map_err(|e| e.to_string())?;
        match Self::into_response(msg)? {
            ServerMessage::Response {
                rtt_ms: Some(ms), ..
            } => Ok(std::time::Duration::from_millis(ms)),
            _ => Err("svc returned no rtt".to_string()),
        }
    }

    pub async fn tgproxy_set_lan(&self, enabled: bool) -> Result<(), String> {
        let msg = self
            .roundtrip(&ClientMessage::TgproxySetLan { enabled })
            .await
            .map_err(|e| e.to_string())?;
        Self::into_response(msg).map(|_| ())
    }

    /// Asks the svc to stop its engine and terminate. Used by the
    /// updater/installer so the svc executable can be replaced.
    pub async fn shutdown(&self) -> Result<(), String> {
        let msg = self
            .roundtrip(&ClientMessage::Shutdown)
            .await
            .map_err(|e| e.to_string())?;
        Self::into_response(msg).map(|_| ())
    }

    /// Subscribes on a dedicated connection and re-emits every [`SvcEvent`]
    /// as a Tauri frontend event using the exact channel names the UI
    /// already listens to — the frontend needs no changes.
    ///
    /// Process-wide single-flight: concurrent attach paths share one forwarder.
    pub async fn forward_events(app: AppHandle) {
        if FORWARDER_RUNNING.swap(true, Ordering::SeqCst) {
            return;
        }
        Self::forward_events_inner(app).await;
        FORWARDER_RUNNING.store(false, Ordering::SeqCst);
    }

    async fn forward_events_inner(app: AppHandle) {
        let client = match ClientOptions::new().open(SVC_PIPE_NAME) {
            Ok(c) => c,
            Err(_) => return,
        };
        let mut io = BufReader::new(client);
        let mut sub = serde_json::to_string(&ClientMessage::Subscribe).unwrap_or_default();
        sub.push('\n');
        // Best-effort subscribe; if it fails the loop below just ends.
        let _ = io.get_mut().write_all(sub.as_bytes()).await;
        let _ = io.get_mut().flush().await;

        loop {
            let mut line = String::new();
            match io.read_line(&mut line).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let msg: ServerMessage = match serde_json::from_str(&line) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let ev = match msg {
                ServerMessage::Event(e) => e,
                _ => continue,
            };
            match ev {
                SvcEvent::LogBatch { batch } => {
                    let _ = app.emit("log-batch", batch);
                }
                SvcEvent::StateChanged { state } => {
                    let _ = app.emit("state-changed", state);
                }
                SvcEvent::ProbeUpdate { report } => {
                    let _ = app.emit("probe-update", report);
                }
                SvcEvent::LogsCleared => {
                    let _ = app.emit("logs-cleared", ());
                }
                SvcEvent::TgproxyState { state } => {
                    let _ = app.emit("tgproxy-state", state);
                }
            }
        }
    }
}

/// Process-wide guard: only one event-forwarder task may run at a time.
/// Prevents duplicate Tauri emissions when several attach paths race
/// (setup probe vs. Start button vs. tray).
static FORWARDER_RUNNING: AtomicBool = AtomicBool::new(false);

/// Where the supervisor engine actually runs.
#[derive(Clone)]
pub enum SupervisorBackend {
    /// In-process (classic mode): the Tauri process owns the Job Object.
    Local(Arc<Supervisor>),
    /// Attached to a running `zapret2-svc.exe` over the named pipe.
    Remote(Arc<SvcClient>),
}

impl SupervisorBackend {
    pub async fn get_status(&self) -> SupervisorStatus {
        match self {
            Self::Local(s) => s.get_status(),
            Self::Remote(c) => match c.get_status().await {
                Ok(s) => s,
                // Transient pipe failure: report last known state instead of
                // fabricating "Error" (which prompts users to hammer start).
                Err(_) => {
                    c.last_known_status().await.unwrap_or(SupervisorStatus {
                        state: crate::engine::SupervisorState::Error,
                        pid: None,
                        active_strategy: None,
                        active_transport: None,
                        binary_path: None,
                        uptime_secs: 0,
                        total_logs_buffered: 0,
                    })
                }
            },
        }
    }

    pub async fn start(
        &self,
        strategy_id: Option<String>,
        custom_args: Option<Vec<String>>,
    ) -> Result<(), String> {
        match self {
            Self::Local(s) => s.clone().start(strategy_id, custom_args).await.map_err(|e| e.to_string()),
            Self::Remote(c) => c.start(strategy_id, custom_args).await,
        }
    }

    pub async fn stop(&self) -> Result<(), String> {
        match self {
            Self::Local(s) => s.stop().await.map_err(|e| e.to_string()),
            Self::Remote(c) => c.stop().await,
        }
    }

    pub async fn get_all_logs(&self) -> Vec<LogEntry> {
        match self {
            Self::Local(s) => s.get_all_logs(),
            Self::Remote(c) => c.get_logs(1000).await.unwrap_or_default(),
        }
    }

    pub async fn clear_logs(&self) {
        match self {
            Self::Local(s) => s.clear_logs(),
            Self::Remote(c) => {
                let _ = c.clear_logs().await;
            }
        }
    }

    /// True when commands go over the pipe.
    pub fn is_remote(&self) -> bool {
        matches!(self, Self::Remote(_))
    }

    /// Opens the log folder in Explorer. In remote mode the svc writes to the
    /// same per-user location when run as the same user (the common case:
    /// the GUI auto-elevates, so both sides resolve identically).
    pub fn open_logs_folder(&self) -> Result<(), String> {
        match self {
            Self::Local(s) => s.open_logs_folder(),
            Self::Remote(_) => crate::engine::FileLogger::new()
                .open_in_explorer()
                .map_err(|e| e.to_string()),
        }
    }

    /// Absolute path of the active log file (same caveat as above).
    pub fn get_log_file_path(&self) -> String {
        match self {
            Self::Local(s) => s.get_log_file_path(),
            Self::Remote(_) => crate::engine::FileLogger::new()
                .get_log_file_path()
                .to_string_lossy()
                .to_string(),
        }
    }
}

/// Where the Telegram MTProto→WebSocket bridge actually runs.
///
/// Mirrors [`SupervisorBackend`]: the bridge moved into `zapret2-svc.exe`,
/// so a GUI crash no longer kills it. When no svc binary is available
/// (dev builds), the bridge falls back to running in-process.
#[derive(Clone)]
pub enum TgproxyBackend {
    /// In-process: the Tauri process owns the bridge task.
    Local(Arc<TgProxyManager>),
    /// The bridge runs inside `zapret2-svc.exe`; commands go over the pipe.
    Remote(Arc<SvcClient>),
}

impl TgproxyBackend {
    pub async fn get_status(&self) -> TgProxyStatus {
        match self {
            Self::Local(m) => m.get_status(),
            // Status is polled from the UI on a timer; never fail the poll
            // on a dead pipe — report Error and let the re-attach path heal.
            Self::Remote(c) => c.tgproxy_get_status().await.unwrap_or(TgProxyStatus {
                state: crate::engine::SupervisorState::Error,
                host: String::new(),
                port: 0,
                secret: String::new(),
                tg_link: String::new(),
                uptime_secs: 0,
                last_error: Some("svc unreachable".to_string()),
                mode: "auto".to_string(),
                lan_enabled: false,
                lan_ip: None,
            }),
        }
    }

    pub async fn start(&self) -> Result<(), String> {
        match self {
            Self::Local(m) => m.clone().start().await.map_err(|e| e.to_string()),
            Self::Remote(c) => c.tgproxy_start().await,
        }
    }

    pub async fn stop(&self) -> Result<(), String> {
        match self {
            Self::Local(m) => m.stop().await.map_err(|e| e.to_string()),
            Self::Remote(c) => c.tgproxy_stop().await,
        }
    }

    pub async fn set_config(&self, port: u16, secret: String) -> Result<(), String> {
        match self {
            Self::Local(m) => m.set_config(port, secret).map_err(|e| e.to_string()),
            Self::Remote(c) => c.tgproxy_set_config(port, secret).await,
        }
    }

    pub async fn regenerate_secret(&self) -> Result<String, String> {
        match self {
            Self::Local(m) => m.regenerate_secret().map_err(|e| e.to_string()),
            Self::Remote(c) => c.tgproxy_regenerate_secret().await,
        }
    }

    pub async fn set_mode(&self, mode: String) -> Result<(), String> {
        match self {
            Self::Local(m) => m.set_mode(mode).map_err(|e| e.to_string()),
            Self::Remote(c) => c.tgproxy_set_mode(mode).await,
        }
    }

    pub async fn test_connection(&self) -> Result<std::time::Duration, String> {
        match self {
            Self::Local(m) => m.test_connection().await.map_err(|e| e.to_string()),
            Self::Remote(c) => c.tgproxy_test_connection().await,
        }
    }

    pub async fn set_lan(&self, enabled: bool) -> Result<(), String> {
        match self {
            Self::Local(m) => m.set_lan(enabled).map_err(|e| e.to_string()),
            Self::Remote(c) => c.tgproxy_set_lan(enabled).await,
        }
    }

    /// True when bridge commands go over the pipe.
    pub fn is_remote(&self) -> bool {
        matches!(self, Self::Remote(_))
    }
}

/// Locate `zapret2-svc.exe`: next to the current executable, in the Tauri
/// resources dir, or (dev) next to it via the target dir layout.
pub fn find_svc_exe() -> Option<PathBuf> {
    let current = std::env::current_exe().ok()?;
    let dir = current.parent()?;
    for candidate in [
        dir.join("zapret2-svc.exe"),
        dir.join("resources").join("zapret2-svc.exe"),
    ] {
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Spawn the svc detached so it outlives this GUI process.
///
/// `CREATE_BREAKAWAY_FROM_JOB` is the critical flag: if this GUI ever runs
/// inside a Job Object, the child must not inherit it — otherwise killing the
/// GUI's job would take the svc (and the bypass) down with it, defeating the
/// entire split. No-op when no job is present.
pub fn spawn_svc_detached(path: &PathBuf) -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x00000008;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x01000000;
    std::process::Command::new(path)
        .creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    Ok(())
}

/// Synchronously asks a running svc to shut down, then waits for its pipe
/// to vanish. For the updater/installer: the svc exe cannot be replaced
/// while the process is alive. Best-effort and silent when no svc runs.
///
/// Must be called from a blocking thread (e.g. inside `spawn_blocking`).
pub fn shutdown_svc_blocking() {
    let handle = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(_) => return,
    };
    let _ = tokio::task::block_in_place(|| {
        handle.block_on(async {
            let client = match SvcClient::connect().await {
                Ok(c) => c,
                Err(_) => return, // no svc running
            };
            let _ = client.shutdown().await;
            for _ in 0..50 {
                tokio::time::sleep(Duration::from_millis(200)).await;
                if SvcClient::connect().await.is_err() {
                    break;
                }
            }
        })
    });
}

/// True when the backend needs (re-)attachment: local mode with a svc
/// binary available, or remote mode with a dead pipe (svc died under us).
pub async fn backend_needs_attach(backend: &SupervisorBackend) -> bool {
    match backend {
        SupervisorBackend::Local(_) => find_svc_exe().is_some(),
        SupervisorBackend::Remote(c) => c.hello().await.is_err(),
    }
}

/// Process-wide mutex serializing svc attach: without it, two concurrent
/// `start_supervisor` calls can both miss the fast-path connect and both
/// spawn `zapret2-svc.exe`, each with its own Supervisor → duplicate engines.
static ATTACH_MUTEX: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();

fn attach_mutex() -> &'static tokio::sync::Mutex<()> {
    ATTACH_MUTEX.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Attach to a running svc, spawning it first if necessary. Polls the pipe
/// until the svc answers or the timeout expires.
pub async fn ensure_svc_attached(timeout: Duration) -> Result<Arc<SvcClient>, String> {
    // Serialize check-then-spawn so concurrent callers can't both decide to spawn.
    let _guard = attach_mutex().lock().await;
    // Fast path: svc already running.
    if let Ok(client) = SvcClient::connect().await {
        if client.hello().await.is_ok() {
            return Ok(Arc::new(client));
        }
    }
    // Spawn it detached.
    let path = find_svc_exe().ok_or_else(|| "zapret2-svc.exe not found".to_string())?;
    spawn_svc_detached(&path).map_err(|e| format!("cannot start svc: {e}"))?;
    // Wait for the pipe.
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Ok(client) = SvcClient::connect().await {
            if client.hello().await.is_ok() {
                return Ok(Arc::new(client));
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("svc did not answer in time".to_string());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

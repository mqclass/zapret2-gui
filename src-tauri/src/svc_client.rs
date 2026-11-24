//! GUI-side client for `zapret2-svc`.
//!
//! [`SvcClient`] speaks the JSON-lines protocol over the svc transport:
//! the named pipe on Windows, a Unix socket on Linux. Two
//! connections are used: one for sequential request/response commands, one
//! for the event subscription. [`SupervisorBackend`] abstracts "local
//! in-process supervisor" vs "remote svc", so Tauri commands don't care.

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::AppHandle;
use tauri::Emitter;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
#[cfg(windows)]
use tokio::net::windows::named_pipe::ClientOptions;
use tokio::sync::Mutex;

use crate::engine::{LogEntry, Supervisor, SupervisorStatus, TgProxyManager, TgProxyStatus};
use crate::svc_ipc::{ClientMessage, ServerMessage, SvcEvent, APP_VERSION, PROTOCOL_VERSION};
#[cfg(windows)]
use crate::svc_ipc::SVC_PIPE_NAME;

/// The byte-stream transport to the svc: a named pipe on Windows, a Unix
/// stream on Linux (socket path resolved by
/// [`svc_socket_path`](crate::svc_ipc::svc_socket_path), the same
/// function the server binds through). The protocol logic in this module
/// is written against the alias and shared; only opening the stream is
/// platform-specific (`open_svc_stream`).
// `#[allow(dead_code)]` on the transport pieces below (and on the
// client-only attach/version helpers further down): both binaries
// include this module, but the svc binary itself never acts as a client,
// so in that binary's build they read as dead — same idiom as the
// platform layer's own allow(dead_code) markers.
#[cfg(windows)]
#[allow(dead_code)]
type SvcStream = tokio::net::windows::named_pipe::NamedPipeClient;
#[cfg(target_os = "linux")]
#[allow(dead_code)]
type SvcStream = tokio::net::UnixStream;

/// Opens the svc byte stream (see [`SvcStream`]).
#[cfg(windows)]
#[allow(dead_code)]
async fn open_svc_stream() -> io::Result<SvcStream> {
    ClientOptions::new().open(SVC_PIPE_NAME)
}

/// Opens the svc byte stream (see [`SvcStream`]).
///
/// Connect failures are immediate, never hangs: a missing socket file is
/// `NotFound`, a stale one `ConnectionRefused`, and a socket owned by
/// another user (mode 0660) `PermissionDenied` — all surface as a plain
/// connect error, and the GUI falls back to its in-process Local backend
/// exactly as it does for a missing svc on Windows.
#[cfg(target_os = "linux")]
#[allow(dead_code)]
async fn open_svc_stream() -> io::Result<SvcStream> {
    tokio::net::UnixStream::connect(crate::svc_ipc::svc_socket_path()).await
}

/// Liveness probe for the svc endpoint: `true` while the svc is still running,
/// `false` once it has exited and its endpoint is gone. Quit waits on this so it
/// never force-kills a svc that is still mid-shutdown.
///
/// "Gone" is platform-specific. On Windows the pipe name vanishes when the
/// process exits, so only `NotFound` proves it left — a busy pipe
/// (`ERROR_PIPE_BUSY`, every instance held while a wedged runtime can't accept)
/// still means the process is alive, and must not be mistaken for a clean exit
/// or Quit would skip its force-kill sweep. On Linux the socket file lingers
/// after exit, so any connect error (stale `ConnectionRefused`, or `NotFound`)
/// means the listener is gone. A probe that times out counts as alive.
pub async fn svc_is_alive() -> bool {
    match tokio::time::timeout(Duration::from_millis(400), open_svc_stream()).await {
        Ok(Ok(_)) => true,
        Ok(Err(e)) => {
            #[cfg(windows)]
            {
                e.kind() != io::ErrorKind::NotFound
            }
            #[cfg(not(windows))]
            {
                let _ = e;
                false
            }
        }
        Err(_) => true,
    }
}

/// Connected client to a running svc instance. Commands are serialized
/// through a mutex: one in-flight request at a time.
pub struct SvcClient {
    io: Mutex<BufReader<SvcStream>>,
    /// Set when a roundtrip times out: the connection may still deliver the
    /// late response, which would be misread as the answer to the NEXT
    /// request. A desynchronized client refuses further roundtrips instead
    /// of corrupting the protocol; recovery is a fresh attach.
    desynced: AtomicBool,
    /// Last successfully fetched status + when. Returned on transient pipe
    /// failures so the UI doesn't fabricate an "Error" state (which prompts
    /// users to hammer the start button). Expires after 30s so a dead svc
    /// can't masquerade as "running" forever.
    /// The allow is load-bearing: the svc binary compiles this module but
    /// never acts as a client, so it never reads the field (the GUI
    /// binary does, via `last_known_status`).
    #[allow(dead_code)]
    last_good: Mutex<Option<(SupervisorStatus, std::time::Instant)>>,
}

/// Result of the Hello handshake: the svc's atomic status snapshot plus
/// the svc binary's own application version. `app_version` is "" when
/// the svc predates the field (1.4.0 and older): such a peer is stale
/// by definition (see [`svc_is_stale`]).
#[allow(dead_code)]
pub struct HelloReply {
    pub status: SupervisorStatus,
    pub app_version: String,
}

impl SvcClient {
    pub async fn connect() -> io::Result<Self> {
        let client = open_svc_stream().await?;
        Ok(Self {
            io: Mutex::new(BufReader::new(client)),
            desynced: AtomicBool::new(false),
            last_good: Mutex::new(None),
        })
    }

    /// Version handshake + liveness check. Returns the svc's atomic status
    /// snapshot and its application version; rejects the peer on protocol
    /// mismatch instead of miscommunicating silently.
    pub async fn hello(&self) -> io::Result<HelloReply> {
        match self
            .roundtrip(&ClientMessage::Hello {
                version: PROTOCOL_VERSION,
            })
            .await?
        {
            ServerMessage::Welcome {
                version,
                app_version,
                status,
            } if version == PROTOCOL_VERSION => Ok(HelloReply {
                status,
                app_version,
            }),
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

    /// One request/response exchange with the svc.
    ///
    /// The exchange itself is bounded by a 30s timeout (see roundtrip_io):
    /// without it a single wedged svc operation would hold the io mutex
    /// forever and block every command and status poll queued behind it.
    /// Waiting for the mutex is NOT part of the timeout, so a caller queued
    /// behind a slow-but-healthy operation is never failed (or worse, marks
    /// the connection desynchronized) for an exchange it never made.
    async fn roundtrip(&self, msg: &ClientMessage) -> io::Result<ServerMessage> {
        if self.desynced.load(Ordering::SeqCst) {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "svc connection desynchronized after a timeout; re-attach required",
            ));
        }
        self.roundtrip_io(msg).await
    }

    async fn roundtrip_io(&self, msg: &ClientMessage) -> io::Result<ServerMessage> {
        let mut io = self.io.lock().await;
        // The timer starts only now that the lock is held and the exchange
        // is really ours: on expiry we HAVE written a request whose reply
        // may still arrive late and be misread as the next response, so the
        // connection is genuinely desynchronized and gets poisoned.
        match tokio::time::timeout(
            std::time::Duration::from_secs(30),
            self.exchange(&mut io, msg),
        )
        .await
        {
            Ok(res) => res,
            Err(_) => {
                self.desynced.store(true, Ordering::SeqCst);
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "svc did not answer within 30s",
                ))
            }
        }
    }

    async fn exchange(
        &self,
        io: &mut BufReader<SvcStream>,
        msg: &ClientMessage,
    ) -> io::Result<ServerMessage> {
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

    /// One transport-agnostic implementation: the subscription rides the
    /// same `open_svc_stream` as the command connection, so Windows and
    /// Linux forward events identically.
    async fn forward_events_inner(app: AppHandle) {
        let client = match open_svc_stream().await {
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
                        active_custom_args: false,
                        active_transport: None,
                        binary_path: None,
                        uptime_secs: 0,
                        total_logs_buffered: 0,
                        last_engine_error: None,
                        app_split_configured_mode: None,
                        app_split_applied_mode: None,
                        app_split_enforced: false,
                        app_split_not_enforced_reason: None,
                        split_tunnel_applied_mode: None,
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
    /// the GUI auto-elevates, so both sides resolve identically). Resolves
    /// the directory directly instead of constructing a throwaway logger
    /// (no icacls re-hardening, no extra session line in the log file).
    pub fn open_logs_folder(&self) -> Result<std::path::PathBuf, String> {
        match self {
            Self::Local(s) => s.open_logs_folder(),
            Self::Remote(_) => crate::engine::FileLogger::open_log_dir_in_explorer(),
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
    /// Ahead of a migration into the svc: if the bridge currently runs
    /// in-process, stop it transiently (persisted enabled-intent untouched).
    /// Two bridges must never coexist: the svc autostarts its own copy from
    /// the persisted flag the moment it spawns, and if the local listener
    /// still holds the port, the svc copy is pushed to a fallback port and
    /// the local one becomes invisible and unmanageable. Returns true when
    /// a running bridge was stopped, so the caller can restart it locally
    /// if the migration does not happen after all.
    pub async fn stop_local_for_migration(&self) -> bool {
        if let Self::Local(m) = self {
            let running = !matches!(
                m.get_status().state,
                crate::engine::SupervisorState::Stopped | crate::engine::SupervisorState::Error
            );
            if running {
                let _ = m.stop_transient().await;
                return true;
            }
        }
        false
    }

    /// Restarts the in-process bridge after a migration that fell through.
    pub async fn restart_local(&self) {
        if let Self::Local(m) = self {
            let _ = m.clone().start().await;
        }
    }

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
                link_secret: String::new(),
                tg_link: String::new(),
                uptime_secs: 0,
                last_error: Some("svc unreachable".to_string()),
                mode: "auto".to_string(),
                lan_enabled: false,
                lan_ip: None,
                stale_secret_detected: false,
                secret_rotated: false,
                bad_handshake_count: 0,
                rotation_count: 0,
                stale_trip_count: 0,
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

/// The svc executable's file name on this platform.
#[cfg(windows)]
#[allow(dead_code)]
const SVC_EXE_NAME: &str = "zapret2-svc.exe";
/// The svc executable's file name on this platform.
#[cfg(target_os = "linux")]
#[allow(dead_code)]
const SVC_EXE_NAME: &str = "zapret2-svc";

/// Locate the svc executable: next to the current executable, or in the
/// Tauri resources dir.
pub fn find_svc_exe() -> Option<PathBuf> {
    let current = std::env::current_exe().ok()?;
    let dir = current.parent()?;
    for candidate in [
        dir.join(SVC_EXE_NAME),
        dir.join("resources").join(SVC_EXE_NAME),
    ] {
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Spawn the svc detached so it outlives this GUI process.
///
/// Windows: `CREATE_BREAKAWAY_FROM_JOB` is the critical flag: if this GUI
/// ever runs inside a Job Object, the child must not inherit it —
/// otherwise killing the GUI's job would take the svc (and the bypass)
/// down with it, defeating the entire split. No-op when no job is present.
///
/// Linux: the equivalent detachment is a new session (`process_group(0)`
/// → setsid in the child) plus closed stdio: the svc keeps no controlling
/// terminal and no process-group ties to the GUI, so a GUI death cannot
/// take it down — and the svc's own supervisor keeps the engine safe via
/// its process-group registry.
pub fn spawn_svc_detached(path: &PathBuf) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x00000008;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x01000000;
        let base = DETACHED_PROCESS | CREATE_NO_WINDOW;
        let spawn_with = |flags: u32| {
            std::process::Command::new(path)
                .creation_flags(flags)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
        };
        // Prefer breakaway so the svc outlives any job the GUI sits in. When
        // the GUI is inside a job that forbids breakaway, CreateProcess fails
        // with ERROR_ACCESS_DENIED; retry without the flag so the svc still
        // starts (it keeps its own KILL_ON_JOB_CLOSE job for the engine).
        match spawn_with(base | CREATE_BREAKAWAY_FROM_JOB) {
            Ok(_) => Ok(()),
            Err(e) if e.raw_os_error() == Some(5) => {
                spawn_with(base)?;
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        std::process::Command::new(path)
            .process_group(0)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        Ok(())
    }
}

/// Parsed `major.minor.patch` application version, for numeric comparison.
/// No semver crate in the manifest — this is ~20 lines by hand, and the
/// only versions it ever sees are crate versions (`1.4.0`, `1.5.0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
struct AppVersion {
    major: u32,
    minor: u32,
    patch: u32,
    /// True when the version carried a `-suffix` or `+suffix`. Strict
    /// semver ignores build metadata (`+build`) for precedence; we
    /// treat any suffix as a pre-release, so a suffixed build precedes
    /// the bare release of the same core.
    prerelease: bool,
}

/// Parses `major.minor.patch` with an optional `-suffix`/`+suffix`.
/// Strict on purpose: anything else (missing component, a fourth
/// component, a non-numeric part, empty input) is `None`, and an
/// unparseable svc version is treated as stale — failing toward the
/// restart is safe, because the restart replaces the odd binary with
/// this GUI's own shipped one.
#[allow(dead_code)]
fn parse_app_version(s: &str) -> Option<AppVersion> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (core, prerelease) = match s.find(|c| c == '-' || c == '+') {
        Some(i) => (&s[..i], true),
        None => (s, false),
    };
    let mut it = core.split('.');
    let major: u32 = it.next()?.parse().ok()?;
    let minor: u32 = it.next()?.parse().ok()?;
    let patch: u32 = it.next()?.parse().ok()?;
    if it.next().is_some() {
        return None;
    }
    Some(AppVersion {
        major,
        minor,
        patch,
        prerelease,
    })
}

/// Semver-order comparison: numeric core first; on equal cores a
/// pre-release build precedes the bare release. Suffix *contents* are
/// not compared (`rc1` vs `rc2` counts as equal) — shipped versions
/// never carry suffixes, and dev builds are not worth arbitrating.
#[allow(dead_code)]
fn cmp_app_version(a: AppVersion, b: AppVersion) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    (a.major, a.minor, a.patch)
        .cmp(&(b.major, b.minor, b.patch))
        .then_with(|| match (a.prerelease, b.prerelease) {
            (true, false) => Less,
            (false, true) => Greater,
            _ => Equal,
        })
}

/// True when the svc behind `svc_version` must be replaced by this GUI's
/// own version: the svc is older than the GUI, or its version is
/// missing/empty/unparseable — a 1.4.0-or-older svc sends no version at
/// all, and that is exactly the stale case. A svc equal to or newer
/// than the GUI is never downgraded.
///
/// An unparseable *GUI* version cannot happen (it is the crate version),
/// but if it did, the answer is "not stale": without a version we can
/// trust, a restart would be a blind guess.
#[allow(dead_code)]
pub fn svc_is_stale(svc_version: &str, gui_version: &str) -> bool {
    match (
        parse_app_version(svc_version),
        parse_app_version(gui_version),
    ) {
        (Some(svc), Some(gui)) => cmp_app_version(svc, gui) == std::cmp::Ordering::Less,
        (None, _) => true,
        (Some(_), None) => false,
    }
}

/// Waits until the svc process is gone, up to `timeout`. Used after a
/// graceful shutdown so a fresh spawn never races the old process still
/// holding the single-instance mutex and the pipe. Returns false on
/// timeout — the caller must not spawn then.
///
/// Probes with [`svc_is_alive`], not a bare connect: on Windows a busy
/// pipe (`ERROR_PIPE_BUSY`, every instance held while a wedged runtime
/// can't accept) still means the process is alive, and mistaking it for
/// a clean exit would spawn a second svc next to the old one.
#[allow(dead_code)]
async fn wait_for_svc_gone(timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if !svc_is_alive().await {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
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
            let _ = wait_for_svc_gone(Duration::from_secs(10)).await;
        })
    });
}

/// True when the backend needs (re-)attachment: local mode with a svc
/// binary available, or remote mode with a dead pipe (svc died under us),
/// or remote mode with a stale svc (an older binary left over from a
/// manual zip upgrade — the attach path restarts it, see
/// [`ensure_svc_attached`]).
pub async fn backend_needs_attach(backend: &SupervisorBackend) -> bool {
    match backend {
        SupervisorBackend::Local(_) => find_svc_exe().is_some(),
        SupervisorBackend::Remote(c) => match c.hello().await {
            Ok(reply) => svc_is_stale(&reply.app_version, APP_VERSION),
            Err(_) => true,
        },
    }
}

/// Process-wide mutex serializing svc attach: without it, two concurrent
/// `start_supervisor` calls can both miss the fast-path connect and both
/// spawn `zapret2-svc.exe`, each with its own Supervisor → duplicate engines.
static ATTACH_MUTEX: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();

fn attach_mutex() -> &'static tokio::sync::Mutex<()> {
    ATTACH_MUTEX.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Polls the svc endpoint until a client answers Hello or the timeout
/// expires. Returns the client and its Hello reply (the caller checks
/// the version — a hello that answers is not yet an attachable svc).
#[allow(dead_code)]
async fn poll_svc_hello(timeout: Duration) -> Result<(Arc<SvcClient>, HelloReply), String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Ok(client) = SvcClient::connect().await {
            if let Ok(reply) = client.hello().await {
                return Ok((Arc::new(client), reply));
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("svc did not answer in time".to_string());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Restarts the bypass under the fresh svc when the stale one was running
/// it, with the same strategy. Best-effort: a failed restore is logged,
/// never fatal to the attach itself — the user can always press Start,
/// and failing the attach would drop a healthy fresh svc for a Local
/// fallback. The camouflage transport needs no special handling: the
/// fresh svc re-applies it from settings on start.
///
/// Custom-args launches are not restored: their strategy id is a
/// placeholder, and guessing would start the wrong engine.
#[allow(dead_code)]
async fn restore_engine_after_restart(fresh: &SvcClient, old_status: &SupervisorStatus) {
    if old_status.state != crate::engine::SupervisorState::Running {
        return;
    }
    if old_status.active_custom_args {
        eprintln!(
            "svc restart: engine was running with custom args; leaving the bypass off (the recorded strategy id is a placeholder)"
        );
        return;
    }
    if let Some(strategy_id) = old_status.active_strategy.clone() {
        match fresh.start(Some(strategy_id.clone()), None).await {
            Ok(()) => eprintln!("svc restart: engine restored with strategy {strategy_id}"),
            Err(e) => eprintln!(
                "svc restart: failed to restore engine with strategy {strategy_id}: {e}"
            ),
        }
    } else {
        eprintln!(
            "svc restart: engine was running but no strategy id was recorded; leaving the bypass off"
        );
    }
}

/// Replaces a stale svc with a fresh instance of this GUI's own version.
///
/// `stale` is the connected client to the outdated svc and `reply` its
/// Hello answer (its status snapshot restores a running engine
/// afterwards). Callers must hold the attach mutex: shutdown + respawn
/// is a check-then-act sequence no second attacher may interleave with.
///
/// Exactly one restart attempt: if the fresh svc still reports a stale
/// version, the attach fails with an error instead of looping.
#[allow(dead_code)]
async fn restart_stale_svc(
    stale: &SvcClient,
    reply: HelloReply,
    timeout: Duration,
) -> Result<Arc<SvcClient>, String> {
    eprintln!(
        "svc version mismatch: svc is {:?}, gui is {APP_VERSION}; restarting svc",
        reply.app_version
    );
    // Resolve the replacement binary BEFORE killing the old instance:
    // with a broken install (missing exe, antivirus quarantine) there
    // is nothing to spawn, and a dead svc with no replacement is
    // strictly worse than a stale one still running.
    let path = find_svc_exe().ok_or_else(|| "zapret2-svc.exe not found".to_string())?;
    // Graceful shutdown of the old instance. On failure the old process
    // may still hold the single-instance mutex: never spawn a second svc
    // next to it — fail cleanly and let the next attach retry.
    stale
        .shutdown()
        .await
        .map_err(|e| format!("cannot shut down stale svc: {e}"))?;
    if !wait_for_svc_gone(Duration::from_secs(10)).await {
        return Err(
            "stale svc did not exit in time; refusing to spawn a second instance".to_string(),
        );
    }
    spawn_svc_detached(&path).map_err(|e| format!("cannot start svc: {e}"))?;
    let (fresh, fresh_reply) = poll_svc_hello(timeout).await?;
    if svc_is_stale(&fresh_reply.app_version, APP_VERSION) {
        return Err(format!(
            "fresh svc still reports stale version {:?} (gui is {APP_VERSION}); refusing to attach",
            fresh_reply.app_version
        ));
    }
    eprintln!(
        "svc restarted: version {:?} (gui {APP_VERSION})",
        fresh_reply.app_version
    );
    restore_engine_after_restart(&fresh, &reply.status).await;
    Ok(fresh)
}

/// Attach to a running svc if one is already up, restarting it first
/// when its version is stale. Never spawns: returns `Ok(None)` when no
/// svc is running (no endpoint at all). Returns `Err` naming the io
/// error kind when the endpoint exists but is unreachable (e.g. a 0660
/// socket owned by another user) or the hello handshake fails — the
/// caller logs it, so a failed attach is diagnosable instead of silent
/// (issue #9). The startup probe uses this; [`ensure_svc_attached`] is
/// the spawning counterpart.
///
/// Takes the attach mutex, like `ensure_svc_attached`: without it, the
/// startup probe could shut a stale svc down while the autostart path
/// is mid-attach, and both would spawn.
#[allow(dead_code)]
pub async fn attach_to_running_svc(
    timeout: Duration,
) -> Result<Option<Arc<SvcClient>>, String> {
    let _guard = attach_mutex().lock().await;
    let client = match SvcClient::connect().await {
        Ok(c) => c,
        Err(e) => {
            // No endpoint at all is the normal case (fresh start before
            // the svc ever ran): stay silent, the caller keeps Local.
            if e.kind() == io::ErrorKind::NotFound {
                return Ok(None);
            }
            // The endpoint exists but is unreachable: a 0660 socket owned
            // by another user (PermissionDenied), a stale socket
            // (ConnectionRefused), or a busy pipe on Windows. Name the
            // kind: the caller logs it, so a failed attach is diagnosable
            // instead of silent (issue #9).
            return Err(format!(
                "cannot connect to the svc endpoint ({}): {} [kind: {:?}]",
                svc_endpoint_label(),
                e,
                e.kind()
            ));
        }
    };
    match client.hello().await {
        Ok(reply) => {
            if svc_is_stale(&reply.app_version, APP_VERSION) {
                return restart_stale_svc(&client, reply, timeout)
                    .await
                    .map(Some);
            }
            Ok(Some(Arc::new(client)))
        }
        Err(e) => Err(format!(
            "svc endpoint answered but the hello handshake failed: {e} [kind: {:?}]",
            e.kind()
        )),
    }
}

/// Human-readable label for the svc endpoint, for attach diagnostics.
#[cfg(windows)]
fn svc_endpoint_label() -> String {
    SVC_PIPE_NAME.to_string()
}

/// Human-readable label for the svc endpoint, for attach diagnostics.
#[cfg(target_os = "linux")]
fn svc_endpoint_label() -> String {
    crate::svc_ipc::svc_socket_path().display().to_string()
}

/// Attach to a running svc, spawning it first if necessary. Polls the pipe
/// until the svc answers or the timeout expires. A running svc whose
/// version is stale is restarted once (see [`restart_stale_svc`]); the
/// function never returns a stale client.
pub async fn ensure_svc_attached(timeout: Duration) -> Result<Arc<SvcClient>, String> {
    // Serialize check-then-spawn so concurrent callers can't both decide to spawn.
    let _guard = attach_mutex().lock().await;
    // Fast path: svc already running.
    if let Ok(client) = SvcClient::connect().await {
        if let Ok(reply) = client.hello().await {
            if svc_is_stale(&reply.app_version, APP_VERSION) {
                return restart_stale_svc(&client, reply, timeout).await;
            }
            return Ok(Arc::new(client));
        }
    }
    // Spawn it detached.
    let path = find_svc_exe().ok_or_else(|| "zapret2-svc.exe not found".to_string())?;
    spawn_svc_detached(&path).map_err(|e| format!("cannot start svc: {e}"))?;
    // Wait for the pipe.
    let (client, reply) = poll_svc_hello(timeout).await?;
    if svc_is_stale(&reply.app_version, APP_VERSION) {
        // The spawn came from this GUI's own directory, so a stale
        // answer means the install itself is inconsistent (an old
        // svc.exe left in resources/): fail closed, never attach to it.
        return Err(format!(
            "spawned svc reports stale version {:?} (gui is {APP_VERSION}); the installation is inconsistent - the svc binary at {} is old. Please reinstall the application; refusing to attach",
            reply.app_version,
            path.display()
        ));
    }
    Ok(client)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_when_svc_older() {
        assert!(svc_is_stale("1.4.0", "1.5.0"));
        assert!(svc_is_stale("1.4.9", "1.5.0"));
        assert!(svc_is_stale("0.9.9", "1.0.0"));
    }

    #[test]
    fn not_stale_when_equal_or_newer() {
        assert!(!svc_is_stale("1.5.0", "1.5.0"));
        assert!(!svc_is_stale("1.5.1", "1.5.0"));
        assert!(!svc_is_stale("1.6.0", "1.5.0"));
        assert!(!svc_is_stale("2.0.0", "1.9.9"));
    }

    #[test]
    fn comparison_is_numeric_not_lexicographic() {
        // Lexicographic order would call 1.10.0 older than 1.9.0.
        assert!(!svc_is_stale("1.10.0", "1.9.0"));
        assert!(svc_is_stale("1.9.0", "1.10.0"));
        assert!(!svc_is_stale("1.5.10", "1.5.9"));
    }

    #[test]
    fn stale_when_version_missing_or_unparseable() {
        // The 1.4.0 upgrade case: an old svc sends no version at all.
        assert!(svc_is_stale("", "1.5.0"));
        assert!(svc_is_stale("   ", "1.5.0"));
        assert!(svc_is_stale("garbage", "1.5.0"));
        assert!(svc_is_stale("1.4", "1.5.0"));
        assert!(svc_is_stale("1.4.0.1", "1.5.0"));
        assert!(svc_is_stale("1.x.0", "1.5.0"));
        assert!(svc_is_stale("v1.4.0", "1.5.0"));
    }

    #[test]
    fn prerelease_suffix_precedes_the_bare_release() {
        // Semver: 1.5.0-rc1 < 1.5.0, so an rc svc is stale under a
        // release GUI; a release svc is never stale under an rc GUI.
        assert!(svc_is_stale("1.5.0-rc1", "1.5.0"));
        assert!(svc_is_stale("1.5.0+build", "1.5.0"));
        assert!(!svc_is_stale("1.5.0", "1.5.0-rc1"));
        // Suffix contents are not compared: two suffixed builds with
        // equal cores count as equal.
        assert!(!svc_is_stale("1.5.0-rc1", "1.5.0-rc2"));
        // The numeric core still decides first.
        assert!(svc_is_stale("1.4.0-rc1", "1.5.0"));
        assert!(!svc_is_stale("1.6.0-rc1", "1.5.0"));
    }

    #[test]
    fn whitespace_is_trimmed() {
        assert!(svc_is_stale(" 1.4.0 ", "1.5.0"));
        assert!(!svc_is_stale(" 1.5.0 ", "1.5.0"));
    }

    #[test]
    fn comparator_edge_cases() {
        // Leading zeros parse as the same number.
        assert!(!svc_is_stale("01.4.0", "1.4.0"));
        assert!(svc_is_stale("01.4.0", "1.5.0"));
        // A component that overflows u32 fails the parse: stale, even
        // against a tiny GUI version.
        assert!(svc_is_stale("4294967296.0.0", "1.5.0"));
        assert!(svc_is_stale("4294967296.0.0", "0.0.1"));
        // An empty suffix still counts as a pre-release marker.
        assert!(svc_is_stale("1.4.0-", "1.4.0"));
    }

    #[test]
    fn unparseable_gui_version_never_triggers_a_restart() {
        // Cannot happen in practice (the GUI version is the crate
        // version), but if it did, restarting would be a blind guess.
        assert!(!svc_is_stale("1.5.0", "garbage"));
        // An unparseable svc version is stale regardless.
        assert!(svc_is_stale("garbage", "garbage"));
    }
}

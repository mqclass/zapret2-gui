//! IPC server for `zapret2-svc`.
//!
//! Listens on [`SVC_PIPE_NAME`](crate::svc_ipc::SVC_PIPE_NAME) (Windows
//! named pipe) or on the Unix socket from
//! [`svc_socket_path`](crate::svc_ipc::svc_socket_path) (Linux), and
//! speaks the [`ClientMessage`](crate::svc_ipc::ClientMessage) /
//! [`ServerMessage`](crate::svc_ipc::ServerMessage) JSON-lines protocol.
//! The per-connection session (`handle_client`) is written against
//! generic async streams and shared by both transports; only the
//! listener setup is platform-specific.
//!
//! [`PipeSink`] is the [`EventSink`](crate::engine::events::EventSink)
//! implementation the svc installs on its engine: every engine event is
//! fanned out to all currently subscribed pipe clients.

use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
#[cfg(windows)]
use tokio::net::windows::named_pipe::NamedPipeServer;
use tokio::sync::broadcast;
use tokio::sync::Mutex;
#[cfg(windows)]
use windows::core::PCWSTR;
#[cfg(windows)]
use windows::Win32::Foundation::{LocalFree, INVALID_HANDLE_VALUE};
#[cfg(windows)]
use windows::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
#[cfg(windows)]
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
#[cfg(windows)]
use windows::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
#[cfg(windows)]
use windows::Win32::System::Pipes::{
    CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

use crate::engine::{EventSink, LogEntry, Supervisor, SupervisorState, TgProxyManager};
use crate::svc_ipc::{ClientMessage, ServerMessage, PROTOCOL_VERSION};
#[cfg(windows)]
use crate::svc_ipc::SVC_PIPE_NAME;
use crate::svc_ipc::SvcEvent;
use crate::engine::DiagnosticsReport;

/// Broadcasts engine events to subscribed pipe clients.
pub struct PipeSink {
    tx: broadcast::Sender<SvcEvent>,
}

impl PipeSink {
    pub fn new(tx: broadcast::Sender<SvcEvent>) -> Self {
        Self { tx }
    }
}

impl EventSink for PipeSink {
    fn emit_log_batch(&self, batch: &[LogEntry]) {
        let _ = self.tx.send(SvcEvent::LogBatch {
            batch: batch.to_vec(),
        });
    }
    fn emit_state_changed(&self, state: SupervisorState) {
        let _ = self.tx.send(SvcEvent::StateChanged { state });
    }
    fn emit_probe_update(&self, report: &DiagnosticsReport) {
        let _ = self.tx.send(SvcEvent::ProbeUpdate {
            report: report.clone(),
        });
    }
    fn emit_logs_cleared(&self) {
        let _ = self.tx.send(SvcEvent::LogsCleared);
    }
    fn emit_tgproxy_state(&self, state: SupervisorState) {
        let _ = self.tx.send(SvcEvent::TgproxyState { state });
    }
}

/// Per-connection handler: JSON-lines request/response, plus an optional
/// event subscription streaming [`SvcEvent`]s back.
///
/// Transport-agnostic: the Windows named pipe and the Linux Unix socket
/// both hand their accepted stream here, so the protocol session exists
/// exactly once — there is no second implementation to drift.
async fn handle_client<S>(
    stream: S,
    supervisor: Arc<Supervisor>,
    tgproxy: Arc<TgProxyManager>,
    event_tx: broadcast::Sender<SvcEvent>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
{
    let (reader, writer) = tokio::io::split(stream);
    let mut lines = BufReader::new(reader).lines();
    let writer = Arc::new(Mutex::new(writer));
    let mut event_rx: Option<broadcast::Receiver<SvcEvent>> = None;

    loop {
        tokio::select! {
            line = lines.next_line() => {
                let line = match line {
                    Ok(Some(l)) => l,
                    _ => break, // EOF or error: client is gone
                };
                if line.trim().is_empty() {
                    continue;
                }
                let msg: ClientMessage = match serde_json::from_str(&line) {
                    Ok(m) => m,
                    Err(e) => {
                        let _ = write_msg(&writer, &ServerMessage::err(format!("bad command: {e}"))).await;
                        continue;
                    }
                };
                match msg {
                    ClientMessage::Hello { version } => {
                        if version != PROTOCOL_VERSION {
                            let _ = write_msg(
                                &writer,
                                &ServerMessage::err(format!(
                                    "protocol version mismatch: client={version}, svc={PROTOCOL_VERSION}"
                                )),
                            )
                            .await;
                        } else {
                            let _ = write_msg(
                                &writer,
                                &ServerMessage::Welcome {
                                    version: PROTOCOL_VERSION,
                                    status: supervisor.get_status(),
                                },
                            )
                            .await;
                        }
                    }
                    ClientMessage::GetStatus => {
                        let _ = write_msg(&writer, &ServerMessage::Response {
                            ok: true,
                            error: None,
                            status: Some(supervisor.get_status()),
                            logs: None,
                            tgproxy_status: None,
                            secret: None,
                            rtt_ms: None,
                        }).await;
                    }
                    ClientMessage::Start { strategy_id, custom_args } => {
                        let r = supervisor.clone().start(strategy_id, custom_args).await;
                        let reply = match r {
                            Ok(()) => ServerMessage::ok(),
                            Err(e) => ServerMessage::err(e.to_string()),
                        };
                        let _ = write_msg(&writer, &reply).await;
                    }
                    ClientMessage::Stop => {
                        let r = supervisor.stop().await;
                        let reply = match r {
                            Ok(()) => ServerMessage::ok(),
                            Err(e) => ServerMessage::err(e.to_string()),
                        };
                        let _ = write_msg(&writer, &reply).await;
                    }
                    ClientMessage::GetLogs { limit } => {
                        let mut logs = supervisor.get_all_logs();
                        if logs.len() > limit {
                            logs = logs.split_off(logs.len() - limit);
                        }
                        let _ = write_msg(&writer, &ServerMessage::Response {
                            ok: true,
                            error: None,
                            status: None,
                            logs: Some(logs),
                            tgproxy_status: None,
                            secret: None,
                            rtt_ms: None,
                        }).await;
                    }
                    ClientMessage::ClearLogs => {
                        supervisor.clear_logs();
                        let _ = write_msg(&writer, &ServerMessage::ok()).await;
                    }
                    ClientMessage::Subscribe => {
                        if event_rx.is_none() {
                            event_rx = Some(event_tx.subscribe());
                        }
                        let _ = write_msg(&writer, &ServerMessage::ok()).await;
                    }
                    ClientMessage::Shutdown => {
                        let _ = write_msg(&writer, &ServerMessage::ok()).await;
                        // Graceful: stop the bridge and the engine (Job Object
                        // kills winws2), then terminate. The updater waits for
                        // the pipe to vanish before replacing files.
                        let _ = tgproxy.stop().await;
                        let _ = supervisor.stop().await;
                        std::process::exit(0);
                    }
                    ClientMessage::TgproxyStart => {
                        let r = tgproxy.clone().start().await;
                        let reply = match r {
                            Ok(()) => ServerMessage::ok(),
                            Err(e) => ServerMessage::err(e.to_string()),
                        };
                        let _ = write_msg(&writer, &reply).await;
                    }
                    ClientMessage::TgproxyStop => {
                        let r = tgproxy.stop().await;
                        let reply = match r {
                            Ok(()) => ServerMessage::ok(),
                            Err(e) => ServerMessage::err(e.to_string()),
                        };
                        let _ = write_msg(&writer, &reply).await;
                    }
                    ClientMessage::TgproxyGetStatus => {
                        let _ = write_msg(&writer, &ServerMessage::Response {
                            ok: true,
                            error: None,
                            status: None,
                            logs: None,
                            tgproxy_status: Some(tgproxy.get_status()),
                            secret: None,
                            rtt_ms: None,
                        }).await;
                    }
                    ClientMessage::TgproxySetConfig { port, secret } => {
                        let reply = match tgproxy.set_config(port, secret) {
                            Ok(()) => ServerMessage::ok(),
                            Err(e) => ServerMessage::err(e.to_string()),
                        };
                        let _ = write_msg(&writer, &reply).await;
                    }
                    ClientMessage::TgproxyRegenerateSecret => {
                        let reply = match tgproxy.regenerate_secret() {
                            Ok(secret) => ServerMessage::Response {
                                ok: true,
                                error: None,
                                status: None,
                                logs: None,
                                tgproxy_status: None,
                                secret: Some(secret),
                                rtt_ms: None,
                            },
                            Err(e) => ServerMessage::err(e.to_string()),
                        };
                        let _ = write_msg(&writer, &reply).await;
                    }
                    ClientMessage::TgproxySetMode { mode } => {
                        let reply = match tgproxy.set_mode(mode) {
                            Ok(()) => ServerMessage::ok(),
                            Err(e) => ServerMessage::err(e.to_string()),
                        };
                        let _ = write_msg(&writer, &reply).await;
                    }
                    ClientMessage::TgproxyTestConnection => {
                        let reply = match tgproxy.test_connection().await {
                            Ok(rtt) => ServerMessage::Response {
                                ok: true,
                                error: None,
                                status: None,
                                logs: None,
                                tgproxy_status: None,
                                secret: None,
                                rtt_ms: Some(rtt.as_millis() as u64),
                            },
                            Err(e) => ServerMessage::err(e.to_string()),
                        };
                        let _ = write_msg(&writer, &reply).await;
                    }
                    ClientMessage::TgproxySetLan { enabled } => {
                        let reply = match tgproxy.set_lan(enabled) {
                            Ok(()) => ServerMessage::ok(),
                            Err(e) => ServerMessage::err(e.to_string()),
                        };
                        let _ = write_msg(&writer, &reply).await;
                    }
                }
            }
            // Forward broadcast events on the subscribed receiver, if any.
            ev = async {
                match event_rx.as_mut() {
                    Some(rx) => Some(rx.recv().await),
                    None => std::future::pending().await,
                }
            } => {
                match ev {
                    Some(Ok(ev)) => {
                        if write_msg(&writer, &ServerMessage::Event(ev)).await.is_err() {
                            break;
                        }
                    }
                    // Slow client: drop the missed events, keep the subscription alive.
                    Some(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
                    // Channel closed or no subscription: done.
                    _ => break,
                }
            }
        }
    }
}

async fn write_msg<W>(
    writer: &Arc<Mutex<W>>,
    msg: &ServerMessage,
) -> std::io::Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut line = serde_json::to_string(msg).unwrap_or_else(|_| r#"{"type":"response","ok":false,"error":"encode error"}"#.to_string());
    line.push('\n');
    let mut w = writer.lock().await;
    w.write_all(line.as_bytes()).await?;
    w.flush().await
}

/// Creates one pipe instance with a restrictive DACL: only SYSTEM, the
/// Administrators group, and the owning user may connect. Without this, the
/// default descriptor would let any local process drive an elevated
/// supervisor (start/stop the bypass, read the log stream).
///
/// The SDDL is deliberately small and auditable:
/// `D:P` (protected DACL) + full access for SY / BA / OW.
#[cfg(windows)]
fn create_secure_pipe() -> std::io::Result<NamedPipeServer> {
    unsafe {
        // SDDL: D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;OW)
        let sddl: Vec<u16> = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;OW)"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let name: Vec<u16> = SVC_PIPE_NAME
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let mut sd = PSECURITY_DESCRIPTOR::default();
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            1, // SDDL_REVISION_1
            &mut sd,
            None,
        )
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::PermissionDenied, e.to_string()))?;

        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd.0,
            bInheritHandle: false.into(),
        };

        let handle = CreateNamedPipeW(
            PCWSTR(name.as_ptr()),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
            PIPE_UNLIMITED_INSTANCES,
            65536,
            65536,
            0,
            Some(&sa),
        );
        // The pipe captured the descriptor; free our copy.
        let _ = LocalFree(windows::Win32::Foundation::HLOCAL(sd.0));

        if handle == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error());
        }
        NamedPipeServer::from_raw_handle(handle.0 as *mut std::ffi::c_void)
    }
}

/// Runs the pipe server until the process exits. Each loop iteration creates a
/// fresh pipe instance, waits for one client, and hands it to its own task —
/// the standard multi-client named-pipe pattern.
#[cfg(windows)]
pub async fn run_pipe_server(
    supervisor: Arc<Supervisor>,
    tgproxy: Arc<TgProxyManager>,
    event_tx: broadcast::Sender<SvcEvent>,
) {
    use std::time::Duration;

    loop {
        let server = match create_secure_pipe() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("svc: cannot create named pipe {SVC_PIPE_NAME}: {e}");
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };
        match server.connect().await {
            Ok(()) => {
                let sup = supervisor.clone();
                let tgp = tgproxy.clone();
                let tx = event_tx.clone();
                tokio::spawn(async move {
                    handle_client(server, sup, tgp, tx).await;
                });
            }
            Err(e) => {
                eprintln!("svc: pipe accept error: {e}");
                // `server` is dropped here; the loop creates a fresh instance.
            }
        }
    }
}

/// Runs the Unix-socket server until the process exits (Linux).
///
/// Access model — the filesystem analogue of the Windows pipe DACL: the
/// socket file is chmodded to 0660 right after binding, so only its
/// owner (the svc user, root under systemd) and its group can connect.
/// Unlike a DACL this is not per-user granular; who may drive the
/// supervisor is decided by socket file ownership/permissions, which
/// the administrator controls via the runtime directory.
///
/// Stale-socket handling: a leftover socket file from a crashed svc is
/// indistinguishable from a live one by name alone, so an existing file
/// is probed with a real connect. Only when the probe fails (nobody
/// listening) is the file unlinked and rebound; a live listener is
/// never taken over — this server parks with a clear message instead
/// (single-instance is normally enforced earlier, by the flock claim
/// in `main`; this is the belt-and-braces guard for custom socket
/// paths via `ZAPRET2_SVC_SOCKET`).
#[cfg(target_os = "linux")]
pub async fn run_pipe_server(
    supervisor: Arc<Supervisor>,
    tgproxy: Arc<TgProxyManager>,
    event_tx: broadcast::Sender<SvcEvent>,
) {
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;

    let path = crate::svc_ipc::svc_socket_path();
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            // Under systemd the RuntimeDirectory already exists; this is
            // for dev runs with a custom ZAPRET2_SVC_SOCKET. Best-effort:
            // a real failure surfaces at bind time below.
            let _ = std::fs::create_dir_all(parent);
        }
    }
    if path.exists() {
        match std::os::unix::net::UnixStream::connect(&path) {
            Ok(_probe) => {
                // A live listener owns this path. Dropping `_probe`
                // disconnects; the other svc keeps serving. Park rather
                // than unlink a working endpoint out from under it.
                eprintln!(
                    "svc: {} is already served by a live listener; \
                     this instance will not take it over",
                    path.display()
                );
                std::future::pending::<()>().await;
                return;
            }
            Err(_) => {
                // Stale socket (or a stray regular file): safe to remove.
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    let listener = match tokio::net::UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("svc: cannot bind unix socket {}: {e}", path.display());
            std::future::pending::<()>().await;
            return;
        }
    };
    // Restrict the endpoint before accepting anyone (see module docs):
    // owner + group only, mirroring the Windows DACL's intent.
    if let Err(e) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o660)) {
        eprintln!(
            "svc: cannot chmod 0660 {}: {e}; socket stays at umask defaults",
            path.display()
        );
    }
    eprintln!("svc: listening on unix socket {}", path.display());

    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                // SO_PEERCRED via tokio: who is on the other end. The
                // socket's 0660 mode is the gate; this log line is the
                // audit trail for it.
                match stream.peer_cred() {
                    Ok(cred) => eprintln!(
                        "svc: unix client connected: uid={} gid={} pid={}",
                        cred.uid(),
                        cred.gid(),
                        cred.pid()
                            .map(|p| p.to_string())
                            .unwrap_or_else(|| "?".to_string())
                    ),
                    Err(e) => eprintln!("svc: unix client connected (peer cred unavailable: {e})"),
                }
                let sup = supervisor.clone();
                let tgp = tgproxy.clone();
                let tx = event_tx.clone();
                tokio::spawn(async move {
                    handle_client(stream, sup, tgp, tx).await;
                });
            }
            Err(e) => {
                eprintln!("svc: unix accept error: {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

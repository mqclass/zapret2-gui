//! Named-pipe server for `zapret2-svc.exe`.
//!
//! Listens on [`SVC_PIPE_NAME`](crate::svc_ipc::SVC_PIPE_NAME), speaks the
//! [`ClientMessage`](crate::svc_ipc::ClientMessage) /
//! [`ServerMessage`](crate::svc_ipc::ServerMessage) JSON-lines protocol.
//!
//! [`PipeSink`] is the [`EventSink`](crate::engine::events::EventSink)
//! implementation the svc installs on its engine: every engine event is
//! fanned out to all currently subscribed pipe clients.

use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::windows::named_pipe::NamedPipeServer;
use tokio::sync::{broadcast, Mutex};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{LocalFree, INVALID_HANDLE_VALUE};
use windows::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
use windows::Win32::System::Pipes::{
    CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

use crate::engine::{EventSink, LogEntry, Supervisor, SupervisorState};
use crate::svc_ipc::{
    ClientMessage, ServerMessage, SvcEvent, PROTOCOL_VERSION, SVC_PIPE_NAME,
};
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
async fn handle_client(
    server: NamedPipeServer,
    supervisor: Arc<Supervisor>,
    event_tx: broadcast::Sender<SvcEvent>,
) {
    let (reader, writer) = tokio::io::split(server);
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
                        // Graceful: stop the engine (Job Object kills winws2),
                        // then terminate. The updater waits for the pipe to
                        // vanish before replacing files.
                        let _ = supervisor.stop().await;
                        std::process::exit(0);
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

async fn write_msg(
    writer: &Arc<Mutex<tokio::io::WriteHalf<NamedPipeServer>>>,
    msg: &ServerMessage,
) -> std::io::Result<()> {
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
fn create_secure_pipe() -> std::io::Result<NamedPipeServer> {
    use std::os::windows::io::FromRawHandle;

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
pub async fn run_pipe_server(supervisor: Arc<Supervisor>, event_tx: broadcast::Sender<SvcEvent>) {
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
                let tx = event_tx.clone();
                tokio::spawn(async move {
                    handle_client(server, sup, tx).await;
                });
            }
            Err(e) => {
                eprintln!("svc: pipe accept error: {e}");
                // `server` is dropped here; the loop creates a fresh instance.
            }
        }
    }
}

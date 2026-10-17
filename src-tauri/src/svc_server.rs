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
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio::sync::{broadcast, Mutex};

use crate::engine::{EventSink, LogEntry, Supervisor, SupervisorState};
use crate::svc_ipc::{
    ClientMessage, ServerMessage, SvcEvent, SVC_PIPE_NAME,
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
                    ClientMessage::Ping => {
                        let _ = write_msg(&writer, &ServerMessage::ok()).await;
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
                }
            }
            // Forward broadcast events on the subscribed receiver, if any.
            ev = async {
                match event_rx.as_mut() {
                    Some(rx) => rx.recv().await.map_err(|_| ()),
                    None => std::future::pending().await,
                }
            } => {
                if let Ok(ev) = ev {
                    if write_msg(&writer, &ServerMessage::Event(ev)).await.is_err() {
                        break;
                    }
                } else {
                    break; // broadcast channel closed
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

/// Runs the pipe server until the process exits. Each loop iteration creates a
/// fresh pipe instance, waits for one client, and hands it to its own task —
/// the standard multi-client named-pipe pattern.
pub async fn run_pipe_server(supervisor: Arc<Supervisor>, event_tx: broadcast::Sender<SvcEvent>) {
    use std::time::Duration;

    loop {
        let server = match ServerOptions::new().create(SVC_PIPE_NAME) {
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

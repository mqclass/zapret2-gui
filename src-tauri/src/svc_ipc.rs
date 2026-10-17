//! IPC protocol between `zapret2-svc.exe` (headless supervisor) and GUI clients.
//!
//! Transport: Windows named pipe `\\.\pipe\zapret2-svc`, newline-delimited JSON.
//! Every line is one [`ClientMessage`] (GUI -> svc) or one [`ServerMessage`]
//! (svc -> GUI). The event variants mirror [`EventSink`](crate::engine::events::EventSink)
//! one-to-one, so the svc can reuse the engine without translation layers.
//!
//! Security note: the pipe is local-machine only. Any local process can
//! connect and *observe* state; *mutating* commands are still gated by the
//! supervisor's own admin check (`Supervisor::start` refuses without
//! elevation). This matches the threat model of a single-user workstation.

use serde::{Deserialize, Serialize};

use crate::engine::{DiagnosticsReport, LogEntry, SupervisorState, SupervisorStatus};

/// Well-known pipe name for the supervisor service.
pub const SVC_PIPE_NAME: &str = r"\\.\pipe\zapret2-svc";
/// Named mutex used for single-instance enforcement of the svc process.
pub const SVC_SINGLE_INSTANCE_MUTEX: &str = "Global\\zapret2-svc-single-instance";

/// A command sent by a GUI client to the service.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum ClientMessage {
    /// Health check.
    Ping,
    /// Full supervisor snapshot.
    GetStatus,
    /// Start the bypass engine.
    Start {
        strategy_id: Option<String>,
        custom_args: Option<Vec<String>>,
    },
    /// Stop the bypass engine.
    Stop,
    /// Last N log entries from the ring buffer.
    GetLogs { limit: usize },
    /// Clear the in-memory log buffer.
    ClearLogs,
    /// Start receiving [`ServerMessage::Event`] lines on this connection.
    Subscribe,
}

/// A reply or pushed event from the service.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ServerMessage {
    /// Response to a command. `ok: false` carries a human-readable error.
    Response {
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<SupervisorStatus>,
        #[serde(skip_serializing_if = "Option::is_none")]
        logs: Option<Vec<LogEntry>>,
    },
    /// Pushed to subscribed connections; mirrors `EventSink`.
    Event(SvcEvent),
}

/// Broadcast event; 1:1 with `EventSink` methods.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum SvcEvent {
    LogBatch { batch: Vec<LogEntry> },
    StateChanged { state: SupervisorState },
    ProbeUpdate { report: DiagnosticsReport },
    LogsCleared,
    TgproxyState { state: SupervisorState },
}

impl ServerMessage {
    pub fn ok() -> Self {
        Self::Response {
            ok: true,
            error: None,
            status: None,
            logs: None,
        }
    }

    pub fn err(msg: impl Into<String>) -> Self {
        Self::Response {
            ok: false,
            error: Some(msg.into()),
            status: None,
            logs: None,
        }
    }
}

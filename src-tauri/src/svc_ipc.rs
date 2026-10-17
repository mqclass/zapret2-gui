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

use crate::engine::{DiagnosticsReport, LogEntry, SupervisorState, SupervisorStatus, TgProxyStatus};

/// Well-known pipe name for the supervisor service.
pub const SVC_PIPE_NAME: &str = r"\\.\pipe\zapret2-svc";
/// Named mutex used for single-instance enforcement of the svc process.
pub const SVC_SINGLE_INSTANCE_MUTEX: &str = "Global\\zapret2-svc-single-instance";
/// IPC protocol version. Bumped on any breaking change; the Hello handshake
/// rejects mismatched peers instead of miscommunicating silently.
///
/// v2: Telegram-bridge commands (`tgproxy-*`) — the bridge moved from the
/// GUI process into the svc, so it survives GUI crashes like the bypass
/// engine does.
pub const PROTOCOL_VERSION: u32 = 2;

/// A command sent by a GUI client to the service.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum ClientMessage {
    /// Version handshake. The server rejects unknown versions instead of
    /// miscommunicating; the reply carries an atomic status snapshot.
    Hello { version: u32 },
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
    /// Graceful shutdown: stop the engine and terminate the svc process.
    /// Used by the updater/installer so the exe can be replaced.
    Shutdown,
    /// Start the embedded Telegram MTProto→WebSocket bridge.
    TgproxyStart,
    /// Stop the Telegram bridge.
    TgproxyStop,
    /// Full Telegram-bridge snapshot (state, port, secret, tg:// link).
    TgproxyGetStatus,
    /// Update bridge port/secret. Refused while the bridge is running.
    TgproxySetConfig { port: u16, secret: String },
    /// Generate a fresh MTProto secret, persist it, return it.
    /// Refused while the bridge is running.
    TgproxyRegenerateSecret,
    /// Set the bridge connection mode ("auto", "cfproxy", "direct").
    /// Refused while the bridge is running.
    TgproxySetMode { mode: String },
    /// End-to-end test: handshake through the listener to the DC.
    /// Returns the DC round-trip time in `rtt_ms`.
    TgproxyTestConnection,
    /// Toggle LAN access (bind 0.0.0.0 for phones). Refused while running.
    TgproxySetLan { enabled: bool },
}

/// A reply or pushed event from the service.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ServerMessage {
    /// Answer to the Hello handshake: protocol version + atomic snapshot.
    Welcome {
        version: u32,
        status: SupervisorStatus,
    },
    /// Response to a command. `ok: false` carries a human-readable error.
    Response {
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<SupervisorStatus>,
        #[serde(skip_serializing_if = "Option::is_none")]
        logs: Option<Vec<LogEntry>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tgproxy_status: Option<TgProxyStatus>,
        #[serde(skip_serializing_if = "Option::is_none")]
        secret: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        rtt_ms: Option<u64>,
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
            tgproxy_status: None,
            secret: None,
            rtt_ms: None,
        }
    }

    pub fn err(msg: impl Into<String>) -> Self {
        Self::Response {
            ok: false,
            error: Some(msg.into()),
            status: None,
            logs: None,
            tgproxy_status: None,
            secret: None,
            rtt_ms: None,
        }
    }
}

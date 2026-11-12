//! IPC protocol between `zapret2-svc.exe` (headless supervisor) and GUI clients.
//!
//! Transport: Windows named pipe `\\.\pipe\zapret2-svc`; on Linux, a Unix
//! socket (see [`svc_socket_path`]). Either way the wire format is the
//! same: newline-delimited JSON.
//! Every line is one [`ClientMessage`] (GUI -> svc) or one [`ServerMessage`]
//! (svc -> GUI). The event variants mirror [`EventSink`](crate::engine::events::EventSink)
//! one-to-one, so the svc can reuse the engine without translation layers.
//!
//! Security note: the pipe is local-machine only. Any local process can
//! connect and *observe* state; *mutating* commands are still gated by the
//! supervisor's own admin check (`Supervisor::start` refuses without
//! elevation). This matches the threat model of a single-user workstation.

use serde::{Deserialize, Serialize};

use crate::engine::{
    DiagnosticsReport, LogEntry, SingBoxStatus, SupervisorState, SupervisorStatus, TgProxyStatus,
    TunnelState,
};

/// Well-known pipe name for the supervisor service.
pub const SVC_PIPE_NAME: &str = r"\\.\pipe\zapret2-svc";
/// Default path of the supervisor Unix socket on Linux. `/run/zapret2` is
/// the systemd `RuntimeDirectory` of zapret2-svc.service: systemd creates
/// it (owned by the service user, root) when the unit starts and removes
/// it when the unit stops.
#[cfg(target_os = "linux")]
pub const SVC_SOCKET_DEFAULT_PATH: &str = "/run/zapret2/svc.sock";
/// Environment variable overriding the svc socket path on Linux
/// (tests and unprivileged dev runs).
#[cfg(target_os = "linux")]
pub const SVC_SOCKET_ENV_VAR: &str = "ZAPRET2_SVC_SOCKET";

/// The one source of truth for the svc socket location on Linux: the
/// server binds and every client connects through this resolution, so
/// the two sides can never drift apart.
///
/// `ZAPRET2_SVC_SOCKET` wins when set and non-empty; otherwise the
/// systemd runtime path above. Access model: after binding, the svc
/// chmods the socket file to 0660 — the filesystem-permissions analogue
/// of the Windows pipe DACL. Who may drive the supervisor is decided by
/// who can open this file.
#[cfg(target_os = "linux")]
pub fn svc_socket_path() -> std::path::PathBuf {
    match std::env::var_os(SVC_SOCKET_ENV_VAR) {
        Some(p) if !p.is_empty() => std::path::PathBuf::from(p),
        _ => std::path::PathBuf::from(SVC_SOCKET_DEFAULT_PATH),
    }
}
/// Named mutex used for single-instance enforcement of the svc process.
pub const SVC_SINGLE_INSTANCE_MUTEX: &str = "Global\\zapret2-svc-single-instance";
/// IPC protocol version. Bumped on any breaking change; the Hello handshake
/// rejects mismatched peers instead of miscommunicating silently.
///
/// v2: Telegram-bridge commands (`tgproxy-*`) — the bridge moved from the
/// GUI process into the svc, so it survives GUI crashes like the bypass
/// engine does.
/// v3: WARP tunnel commands (`tunnel-*`), the `TunnelState` event, and
/// the `tunnel` field of `SupervisorStatus` — the tunnel lives in the
/// svc next to the desync engine, so the GUI drives it over the pipe.
pub const PROTOCOL_VERSION: u32 = 3;

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
    /// Start the WARP tunnel (sing-box sidecar). Sets the durable
    /// `warp_enabled` switch; restarts the desync with the WARP
    /// launch args first when it runs without them.
    TunnelStart,
    /// Stop the WARP tunnel and turn the `warp_enabled` switch off.
    /// The desync engine is untouched.
    TunnelStop,
    /// Full tunnel snapshot. The updater uses it when the GUI is
    /// attached to the svc: the tunnel lives in the svc there, so
    /// the local manager is Stopped and cannot see it.
    TunnelGetStatus,
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
        tunnel_status: Option<SingBoxStatus>,
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
    TunnelState { state: TunnelState },
}

impl ServerMessage {
    pub fn ok() -> Self {
        Self::Response {
            ok: true,
            error: None,
            status: None,
            logs: None,
            tgproxy_status: None,
            tunnel_status: None,
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
            tunnel_status: None,
            secret: None,
            rtt_ms: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_version_is_3() {
        // v3: tunnel commands + TunnelState event + the tunnel field
        // of SupervisorStatus. A bump here is deliberate - the Hello
        // handshake rejects mismatched peers, so both sides must
        // move together (GUI and svc ship in one package).
        assert_eq!(PROTOCOL_VERSION, 3);
    }

    #[test]
    fn tunnel_commands_serialize_kebab_case() {
        assert_eq!(
            serde_json::to_string(&ClientMessage::TunnelStart).unwrap(),
            r#"{"cmd":"tunnel-start"}"#
        );
        assert_eq!(
            serde_json::to_string(&ClientMessage::TunnelStop).unwrap(),
            r#"{"cmd":"tunnel-stop"}"#
        );
        assert_eq!(
            serde_json::to_string(&ClientMessage::TunnelGetStatus).unwrap(),
            r#"{"cmd":"tunnel-get-status"}"#
        );
        // And parse back.
        assert!(matches!(
            serde_json::from_str::<ClientMessage>(r#"{"cmd":"tunnel-start"}"#).unwrap(),
            ClientMessage::TunnelStart
        ));
        assert!(matches!(
            serde_json::from_str::<ClientMessage>(r#"{"cmd":"tunnel-get-status"}"#).unwrap(),
            ClientMessage::TunnelGetStatus
        ));
    }

    #[test]
    fn tunnel_state_event_round_trips() {
        let event = SvcEvent::TunnelState {
            state: TunnelState::Running,
        };
        let line = serde_json::to_string(&ServerMessage::Event(event)).unwrap();
        assert_eq!(
            line,
            r#"{"type":"event","event":"tunnel-state","state":"running"}"#
        );
        let back: ServerMessage = serde_json::from_str(&line).unwrap();
        assert!(matches!(
            back,
            ServerMessage::Event(SvcEvent::TunnelState {
                state: TunnelState::Running
            })
        ));
    }

    #[test]
    fn response_with_tunnel_status_round_trips() {
        let msg = ServerMessage::Response {
            ok: true,
            error: None,
            status: None,
            logs: None,
            tgproxy_status: None,
            tunnel_status: Some(SingBoxStatus {
                state: TunnelState::Degraded,
                pid: Some(4242),
                engine_version: Some("sing-box version 1.14.3-extended-2.7.3".to_string()),
                clash_api_port: Some(54321),
                warmup_delay_ms: None,
                last_error: Some("warp exit did not answer".to_string()),
            }),
            secret: None,
            rtt_ms: None,
        };
        let line = serde_json::to_string(&msg).unwrap();
        let back: ServerMessage = serde_json::from_str(&line).unwrap();
        match back {
            ServerMessage::Response {
                tunnel_status: Some(s),
                ..
            } => {
                assert_eq!(s.state, TunnelState::Degraded);
                assert_eq!(s.pid, Some(4242));
                assert_eq!(s.clash_api_port, Some(54321));
            }
            other => panic!("unexpected round-trip result: {other:?}"),
        }
    }

    #[test]
    fn old_snapshot_without_tunnel_field_still_parses() {
        // A v2-era SupervisorStatus JSON has no `tunnel` key: the
        // field defaults to the stopped snapshot instead of failing
        // the whole status read.
        let json = r#"{"state":"stopped","pid":null,"active_strategy":null,"active_transport":null,"binary_path":null,"uptime_secs":0,"total_logs_buffered":0,"last_engine_error":null}"#;
        let status: SupervisorStatus = serde_json::from_str(json).unwrap();
        assert_eq!(status.tunnel.state, TunnelState::Stopped);
        assert_eq!(status.tunnel.pid, None);
    }

    #[test]
    fn hello_rejects_unknown_versions_at_parse_level() {
        // The handshake compares versions numerically; a v2 client
        // talking to a v3 svc gets the mismatch error, not a silent
        // misread. The parse itself must accept any version number
        // so the server can reject it with a clear message.
        let msg: ClientMessage = serde_json::from_str(r#"{"cmd":"hello","version":2}"#).unwrap();
        assert!(matches!(msg, ClientMessage::Hello { version: 2 }));
        assert_ne!(2, PROTOCOL_VERSION);
    }
}

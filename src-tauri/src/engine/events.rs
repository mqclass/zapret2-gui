//! UI event sink abstraction.
//!
//! The supervisor engine produces UI-facing events (log batches, state changes,
//! probe updates). Historically these were emitted straight into Tauri via
//! `AppHandle`. That hard-codes the engine to the Tauri process, which is
//! exactly what the svc split removes: `zapret2-svc.exe` runs the same engine
//! headless and broadcasts events to GUI clients over a named pipe instead.
//!
//! [`EventSink`] is the single seam: the engine talks to `Arc<dyn EventSink>`,
//! and each host process installs its own implementation.

use std::sync::Arc;

use parking_lot::RwLock;
use tauri::{AppHandle, Emitter};

use super::probes::DiagnosticsReport;
use super::singbox::TunnelState;
use super::supervisor::{LogEntry, SupervisorState};

/// Destination for engine UI events. All methods are fire-and-forget and must
/// never block the engine or panic.
pub trait EventSink: Send + Sync {
    /// A micro-batch of freshly captured engine log lines.
    fn emit_log_batch(&self, batch: &[LogEntry]);
    /// The supervisor lifecycle state changed.
    fn emit_state_changed(&self, state: SupervisorState);
    /// Fresh network probe diagnostics.
    fn emit_probe_update(&self, report: &DiagnosticsReport);
    /// The in-memory log buffer was cleared.
    fn emit_logs_cleared(&self);
    /// The Telegram bridge state changed.
    fn emit_tgproxy_state(&self, state: SupervisorState);
    /// The WARP tunnel state changed (stage 5). Emitted by
    /// SingBoxManager on every transition; the GUI listens on the
    /// "tunnel-state" channel, the svc forwards it as
    /// [`SvcEvent::TunnelState`](crate::svc_ipc::SvcEvent::TunnelState).
    fn emit_tunnel_state(&self, state: TunnelState);
}

/// Drops every event. Used before a real sink is installed and in contexts
/// where nobody listens.
pub struct NullSink;

impl EventSink for NullSink {
    fn emit_log_batch(&self, _batch: &[LogEntry]) {}
    fn emit_state_changed(&self, _state: SupervisorState) {}
    fn emit_probe_update(&self, _report: &DiagnosticsReport) {}
    fn emit_logs_cleared(&self) {}
    fn emit_tgproxy_state(&self, _state: SupervisorState) {}
    fn emit_tunnel_state(&self, _state: TunnelState) {}
}

/// Forwards events into a Tauri frontend via `AppHandle::emit`.
/// Preserves the exact channel names the UI already listens to.
pub struct TauriSink {
    handle: AppHandle,
}

impl TauriSink {
    pub fn new(handle: AppHandle) -> Self {
        Self { handle }
    }
}

impl EventSink for TauriSink {
    fn emit_log_batch(&self, batch: &[LogEntry]) {
        let _ = self.handle.emit("log-batch", batch);
    }
    fn emit_state_changed(&self, state: SupervisorState) {
        let _ = self.handle.emit("state-changed", state);
    }
    fn emit_probe_update(&self, report: &DiagnosticsReport) {
        let _ = self.handle.emit("probe-update", report);
    }
    fn emit_logs_cleared(&self) {
        let _ = self.handle.emit("logs-cleared", ());
    }
    fn emit_tgproxy_state(&self, state: SupervisorState) {
        let _ = self.handle.emit("tgproxy-state", state);
    }
    fn emit_tunnel_state(&self, state: TunnelState) {
        let _ = self.handle.emit("tunnel-state", state);
    }
}

/// Swappable holder: the engine reads the current sink on every emit, so a
/// host can replace the destination at runtime (e.g. GUI attaching to an
/// already-running service).
#[derive(Clone)]
pub struct SinkSlot {
    inner: Arc<RwLock<Arc<dyn EventSink>>>,
}

impl SinkSlot {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(Arc::new(NullSink))),
        }
    }

    pub fn set(&self, sink: Arc<dyn EventSink>) {
        *self.inner.write() = sink;
    }

    pub fn emit_log_batch(&self, batch: &[LogEntry]) {
        self.inner.read().emit_log_batch(batch);
    }
    pub fn emit_state_changed(&self, state: SupervisorState) {
        self.inner.read().emit_state_changed(state);
    }
    pub fn emit_probe_update(&self, report: &DiagnosticsReport) {
        self.inner.read().emit_probe_update(report);
    }
    pub fn emit_logs_cleared(&self) {
        self.inner.read().emit_logs_cleared();
    }
    pub fn emit_tgproxy_state(&self, state: SupervisorState) {
        self.inner.read().emit_tgproxy_state(state);
    }
    pub fn emit_tunnel_state(&self, state: TunnelState) {
        self.inner.read().emit_tunnel_state(state);
    }
}

impl Default for SinkSlot {
    fn default() -> Self {
        Self::new()
    }
}

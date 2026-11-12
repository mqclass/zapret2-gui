#![allow(unused_imports)]
pub mod events;
pub mod imported_strategies;
pub mod logger;
pub mod managed;
pub mod probes;
// The classifier's entry points live in the GUI binary (commands.rs);
// the svc binary compiles the shared engine tree but never invokes it,
// so without this the whole API surface reads as dead code there —
// the same treatment as other GUI-only items in this tree.
#[allow(dead_code)]
pub mod block_classifier;
pub mod speedtest;
pub mod supervisor;
pub mod youtube_probe;
pub mod tgproxy;
pub mod transport;
// WARP mode, stage 4: the winws2 tunnel-exclusion filter and the
// api.cloudflareclient.com desync recipe. Called from the supervisor
// (certain engine) and from the transport framework (best-effort
// engine detection), so no dead-code allowance needed.
pub mod tunnel_capture;
// WARP mode, stage 2: the sing-box config generator. Nothing calls it
// yet - stage 3 (SingBoxManager) wires it up - so without this the
// whole API surface reads as dead code in both binaries.
#[allow(dead_code)]
pub mod singbox_config;
// WARP mode, stage 3: the Clash API client and the sing-box sidecar
// manager. Stage 4 wires the start side into the supervisor lifecycle
// and stage 5 exposes the tunnel over IPC; until then most of the API
// surface reads as dead code in both binaries.
#[allow(dead_code)]
pub mod clash_api;
#[allow(dead_code)]
pub mod singbox;
pub use events::{EventSink, NullSink, SinkSlot, TauriSink};
pub use logger::FileLogger;
pub use managed::ManagedSidecar;
pub use probes::{DiagnosticsReport, DiagnosticsService, ProbeMetric, ProbeStatus, StrategyProbeResult};
pub use speedtest::{run_youtube_speedtest, SpeedtestResult};
pub use supervisor::{LogEntry, Supervisor, SupervisorState, SupervisorStatus};
pub use tgproxy::{TgProxyManager, TgProxyStatus};
pub use transport::{
    apply_active_transport, global_registry, validate_transport_config_json, DomainFrontingTransport,
    Transport, TransportAvailability, TransportConfig, TransportDescriptor, TransportError,
    TransportRegistry, TransportView,
};
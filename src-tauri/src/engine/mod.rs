#![allow(unused_imports)]
pub mod events;
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
pub mod tgproxy;
pub mod transport;
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
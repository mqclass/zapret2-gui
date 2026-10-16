#![allow(unused_imports)]
pub mod logger;
pub mod managed;
pub mod probes;
pub mod speedtest;
pub mod supervisor;
<<<<<<< HEAD
pub mod tgproxy;
=======
pub mod transport;
>>>>>>> feature/transport-framework

pub use logger::FileLogger;
pub use managed::ManagedSidecar;
pub use probes::{DiagnosticsReport, DiagnosticsService, ProbeMetric, ProbeStatus, StrategyBenchmarkResult};
pub use speedtest::{run_youtube_speedtest, SpeedtestResult};
pub use supervisor::{LogEntry, Supervisor, SupervisorState, SupervisorStatus};
<<<<<<< HEAD
pub use tgproxy::{TgProxyManager, TgProxyStatus};
=======
pub use transport::{
    apply_active_transport, global_registry, validate_transport_config_json, DomainFrontingTransport,
    Transport, TransportAvailability, TransportConfig, TransportDescriptor, TransportError,
    TransportRegistry, TransportView,
};
>>>>>>> feature/transport-framework

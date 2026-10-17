//! Outbound TCP connector with optional proxy support.
//!
//! All network paths that open an outgoing TCP connection should go through
//! this module. That keeps direct WS, Cloudflare, Worker, TCP fallback, checks
//! and startup fetches consistent when an environment only has internet through
//! a local or corporate proxy.

mod config;
mod connector;
#[cfg(test)]
mod tests;

pub use connector::{OutboundConnector, OutboundError};

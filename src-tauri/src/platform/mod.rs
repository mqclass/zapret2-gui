//! Platform abstraction layer.
//!
//! All OS-specific code lives behind this facade so the engine, the Tauri
//! commands and the svc stay platform-neutral and refer to
//! `crate::platform::...` only. Each OS gets a submodule with the same
//! surface; the facade re-exports the active one.
//!
//! Today: Windows (`platform/windows`, formerly `win32/`).
//! In progress: Linux (`platform/linux`) — zapret2 ships a native Linux
//! engine (`nfqws2`), so the port adds an implementation, not a rewrite.

#[cfg(windows)]
pub mod windows;

#[cfg(windows)]
pub use windows::*;

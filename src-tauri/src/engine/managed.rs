//! Registry hook for auxiliary managed processes ("sidecars").
//!
//! The self-update pipeline renames the whole installation directory, so it
//! must not run while *any* managed process still executes from inside that
//! directory — not just the main bypass engine. Each feature that spawns a
//! long-lived child implements [`ManagedSidecar`] and registers its manager
//! in [`AppState::sidecars`](crate::commands::AppState); then `install_update`
//! fails closed until every registered sidecar reports stopped, and the
//! frontend update flow surfaces the conflict before the swap is attempted.
//!
//! (The Telegram bridge used to be such a sidecar; since `feature/tg-native-
//! bridge` it runs in-process as a Tokio task, so the registry is currently
//! empty — kept for future external helpers.)

/// A long-lived auxiliary process managed by the application.
pub trait ManagedSidecar: Send + Sync {
    /// Stable human-readable name, used in logs and error messages.
    fn name(&self) -> &'static str;
    /// `true` unless the sidecar is fully stopped. Any non-stopped state
    /// (Starting / Running / Stopping / Error) counts as active, because the
    /// process may still hold files inside the installation directory.
    fn is_active(&self) -> bool;
}

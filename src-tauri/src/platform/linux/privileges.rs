//! Privilege model on Linux.
//!
//! There is no UAC here and no relaunch-elevated flow: by design (phase 1)
//! the GUI runs as the plain user, and the privileged half — zapret2-svc —
//! runs as root under its systemd unit. This module therefore answers the
//! one question the neutral code actually asks (`is_elevated`, the real
//! euid check) and refuses elevation requests with a clear reason instead
//! of pretending a sudo/pkexec dance exists.
//!
//! Why the GUI must NOT self-elevate: the engine needs root for
//! NFQUEUE/nftables, and that root lives in the svc. A root GUI would only
//! widen the attack surface (WebKitGTK as root) for zero engine benefit.

use std::io;
use thiserror::Error;

/// Errors originating from privilege validation and elevation requests.
/// The first five variants mirror the Windows `PrivilegeError`; the last
/// one carries the Linux refusals (neutral code only ever stringifies
/// these errors, never matches on variants).
#[derive(Debug, Error)]
pub enum PrivilegeError {
    #[error("Failed to open process token for query: {0}")]
    OpenTokenFailed(#[source] io::Error),

    #[error("Failed to retrieve token elevation information: {0}")]
    GetTokenInfoFailed(#[source] io::Error),

    #[error("Failed to determine current executable path: {0}")]
    CurrentExeResolutionFailed(#[source] io::Error),

    #[error("UAC privilege elevation was rejected or cancelled by the user")]
    ElevationCancelledByUser,

    #[error("ShellExecuteExW relaunch failed: {0}")]
    ShellExecuteFailed(#[source] io::Error),

    #[error("Privilege elevation is not available on Linux: {0}")]
    NotSupportedOnLinux(String),
}

/// Checks whether the current process runs with root privileges
/// (effective uid == 0) — the Linux form of the Windows token-elevation
/// query. Under systemd the svc gets `true`; the GUI normally gets `false`.
pub fn is_elevated() -> Result<bool, PrivilegeError> {
    Ok(unsafe { libc::geteuid() } == 0)
}

/// Relaunching elevated does not exist on Linux by design (see module
/// docs): always an honest refusal, never a silent no-op success.
pub fn relaunch_elevated() -> Result<(), PrivilegeError> {
    Err(PrivilegeError::NotSupportedOnLinux(
        "the GUI runs unprivileged; the bypass engine lives in zapret2-svc, \
         which systemd starts as root"
            .to_string(),
    ))
}

/// Verifies root privileges for processes that genuinely need them.
///
/// Ok when already root (the svc under systemd). Otherwise a refusal that
/// names the intended model: callers like zapret2-svc exit on this error,
/// which is correct — a user-launched svc could not drive NFQUEUE anyway.
pub fn ensure_elevated() -> Result<(), PrivilegeError> {
    if is_elevated()? {
        Ok(())
    } else {
        Err(PrivilegeError::NotSupportedOnLinux(
            "this process must run as root; on Linux zapret2-svc gets root \
             from its systemd unit, and the GUI intentionally stays unprivileged"
                .to_string(),
        ))
    }
}

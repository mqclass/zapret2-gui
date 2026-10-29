//! De-elevated process spawning (Linux).
//!
//! The Windows module exists because the GUI runs elevated there and the
//! Telegram sidecar must not inherit the admin token. On Linux the model
//! is inverted by design: the GUI never runs elevated, and the privileged
//! half (zapret2-svc under systemd) has nothing to de-elevate INTO a user
//! session from this API. Accordingly:
//!
//! - [`spawn_current_token`] is the real, only spawn path: a plain spawn
//!   in its own process group — the child is already least-privilege.
//! - [`spawn_deelevated`] fails closed with an explicit error, exactly
//!   like the Windows module refuses every failure: there is no elevated
//!   token to drop here, and silently spawning elevated would be worse
//!   than refusing.
//!
//! (In the current architecture the bridge runs in-process via
//! `engine::tgproxy`; no neutral code calls this module. It is kept at
//! surface parity so the platform layer stays complete.)

use std::io;
use std::os::unix::io::RawFd;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, ExitStatus};
use std::sync::{Arc, Mutex};

/// De-elevation failures. Always fail-closed: the caller must refuse to
/// start the child rather than fall back to a privileged spawn.
#[derive(Debug, thiserror::Error)]
pub enum DeelevateError {
    #[error("{0}")]
    Msg(String),
}

impl DeelevateError {
    fn msg(text: impl Into<String>) -> Self {
        DeelevateError::Msg(text.into())
    }
}

/// A spawned child process.
///
/// Mirrors the subset of the child API the Windows `RawChild` exposes
/// (`id`, `start_kill`, `try_wait`, `wait`), backed by a shared
/// `std::process::Child` so the async `wait` can run on a blocking thread
/// while other handles keep access.
pub struct RawChild {
    child: Arc<Mutex<Child>>,
    pid: u32,
}

impl RawChild {
    pub fn id(&self) -> Option<u32> {
        Some(self.pid)
    }

    /// No process HANDLE exists on Unix; a pid is not an fd and pretending
    /// otherwise would hand callers a number that closes something else.
    /// Always `None` — identity on Linux is the pid ([`RawChild::id`]).
    pub fn raw_handle(&self) -> Option<RawFd> {
        None
    }

    /// Best-effort kill (mirrors `Child::start_kill`).
    pub fn start_kill(&mut self) -> io::Result<()> {
        self.child
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "child lock poisoned"))?
            .kill()
    }

    /// Non-blocking exit check (mirrors `Child::try_wait`).
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "child lock poisoned"))?
            .try_wait()
    }

    /// Waits for process exit on a blocking thread (mirrors `Child::wait`).
    pub async fn wait(&mut self) -> io::Result<ExitStatus> {
        let child = self.child.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = child
                .lock()
                .map_err(|_| io::Error::new(io::ErrorKind::Other, "child lock poisoned"))?;
            guard.wait()
        })
        .await
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("wait task failed: {e}")))?
    }
}

/// Spawns `exe` de-elevated with the interactive user's token.
///
/// Does not exist on Linux: the caller is either the unprivileged GUI
/// (nothing to drop) or the root svc (dropping into a desktop session is
/// the systemd unit's business, not a spawn flag). Fail-closed refusal,
/// per this module's contract.
pub fn spawn_deelevated(
    _exe: &Path,
    _args: &[&str],
    _workdir: &Path,
) -> Result<RawChild, DeelevateError> {
    Err(DeelevateError::msg(
        "de-elevated spawn is unavailable on Linux: the GUI runs unprivileged \
         and zapret2-svc runs as root under systemd — there is no elevated \
         token to drop",
    ))
}

/// Spawns `exe` with the current process's credentials — on Linux this is
/// the normal spawn path: the child is already least-privilege. The child
/// gets its own process group (setsid semantics via `process_group(0)`),
/// like every supervised spawn in this codebase.
pub fn spawn_current_token(
    exe: &Path,
    args: &[&str],
    workdir: &Path,
) -> Result<RawChild, DeelevateError> {
    let mut cmd = std::process::Command::new(exe);
    cmd.args(args);
    cmd.current_dir(workdir);
    cmd.process_group(0);
    let child = cmd
        .spawn()
        .map_err(|e| DeelevateError::msg(format!("spawn {}: {e}", exe.display())))?;
    let pid = child.id();
    Ok(RawChild {
        child: Arc::new(Mutex::new(child)),
        pid,
    })
}

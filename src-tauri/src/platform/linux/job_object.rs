//! Process-group isolation for zapret2 engine processes (Linux).
//!
//! The Linux equivalent of the Windows Job Object with
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. There is no kernel handle to close,
//! so the guarantee is assembled from three parts:
//!
//! 1. Every supervised engine is spawned with `setsid` (see
//!    `platform::process::spawn_process`): it leads its own process group,
//!    and `prctl(PR_SET_PDEATHSIG, SIGKILL)` makes the kernel kill the
//!    leader if the supervisor process dies in any way — the direct analog
//!    of the job dying with its owner.
//! 2. This registry remembers every assigned group id; `terminate` and
//!    `Drop` `killpg(SIGKILL)` them all, so helpers the engine itself may
//!    have forked die with it — the analog of terminating the job.
//! 3. `Drop` runs on orderly supervisor teardown (svc shutdown, GUI exit),
//!    covering the graceful paths; the pdeathsig covers the violent ones.
//!
//! Honest limit: a grandchild that calls `setsid` itself escapes its group
//! and survives `terminate` — the same class of escape a process breaking
//! away from a Windows job has. Nothing in the zapret2 engine does this.

use std::collections::HashSet;
use std::io;
use std::sync::Mutex;
use thiserror::Error;

/// Errors originating from Linux process-group supervision operations.
/// Variant names deliberately parallel the Windows `JobObjectError` so
/// `ProcessError::JobError` reads the same on both platforms.
#[allow(dead_code)]
#[derive(Debug, Error)]
pub enum JobObjectError {
    #[error("Failed to create process-group registry: {0}")]
    CreateFailed(#[source] io::Error),

    #[error("Failed to configure process-group supervision: {0}")]
    ConfigureLimitsFailed(#[source] io::Error),

    #[error("Failed to assign process PID {pid} to process-group supervision: {source}")]
    AssignProcessFailed {
        pid: u32,
        #[source]
        source: io::Error,
    },

    #[error("Failed to open process PID {pid} for process-group assignment: {source}")]
    OpenProcessFailed {
        pid: u32,
        #[source]
        source: io::Error,
    },

    #[error("Failed to terminate process group with exit code {exit_code}: {source}")]
    TerminateFailed {
        exit_code: u32,
        #[source]
        source: io::Error,
    },

    #[error("Process-group registry is invalid or already closed")]
    InvalidHandle,
}

/// Thread-safe registry of supervised process groups.
///
/// The group-leader equivalent of the Windows Job Object guard: created
/// empty, processes are assigned as they are spawned, and dropping the
/// registry (or the hosting process terminating) kills every member group.
#[derive(Debug)]
pub struct JobObject {
    /// Process-group ids under supervision (pgid == leader pid for every
    /// process we spawn, thanks to `setsid`).
    groups: Mutex<HashSet<i32>>,
}

// The registry is plain data behind a Mutex; it is shareable exactly like
// the Windows HANDLE wrapper.
unsafe impl Send for JobObject {}
unsafe impl Sync for JobObject {}

#[allow(dead_code)]
impl JobObject {
    /// Creates an empty process-group registry.
    pub fn create() -> Result<Self, JobObjectError> {
        Ok(Self {
            groups: Mutex::new(HashSet::new()),
        })
    }

    /// Assigns a spawned process to this registry by PID.
    ///
    /// The process must be a process-group leader (our spawn path uses
    /// `setsid`, so pgid == pid). The PID is re-verified to exist, mirroring
    /// the Windows `OpenProcess` step: assigning a dead or reused PID must
    /// fail loudly instead of registering a stranger's group.
    pub fn assign_pid(&self, pid: u32) -> Result<(), JobObjectError> {
        // Confirm the process exists and learn its real group id.
        let pgid = unsafe { libc::getpgid(pid as i32) };
        if pgid < 0 {
            return Err(JobObjectError::OpenProcessFailed {
                pid,
                source: io::Error::last_os_error(),
            });
        }
        // Safety: our spawn path guarantees the child leads its own group.
        // If that ever failed, the child's group could be OUR group — and
        // registering it would let terminate() kill the supervisor itself.
        if pgid == unsafe { libc::getpgid(0) } {
            return Err(JobObjectError::AssignProcessFailed {
                pid,
                source: io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "process did not get its own group (setsid failed?)",
                ),
            });
        }
        let mut groups = self
            .groups
            .lock()
            .map_err(|_| JobObjectError::InvalidHandle)?;
        // Prune groups that no longer exist (Windows forgets dead job
        // members automatically; a recycled pgid must never be killed on
        // behalf of a long-dead engine).
        groups.retain(|&g| unsafe { libc::kill(-g, 0) } == 0 || io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH));
        groups.insert(pgid);
        Ok(())
    }

    /// Forcefully terminates all processes in every supervised group
    /// (SIGKILL to each process group).
    pub fn terminate(&self, exit_code: u32) -> Result<(), JobObjectError> {
        let groups: Vec<i32> = match self.groups.lock() {
            Ok(g) => g.iter().copied().collect(),
            Err(_) => return Err(JobObjectError::InvalidHandle),
        };
        let mut last_err: Option<io::Error> = None;
        for pgid in groups {
            // Negative pid targets the whole process group. ESRCH simply
            // means the group already exited — not a failure.
            let rc = unsafe { libc::kill(-pgid, libc::SIGKILL) };
            if rc != 0 {
                let e = io::Error::last_os_error();
                if e.raw_os_error() != Some(libc::ESRCH) {
                    last_err = Some(e);
                }
            }
        }
        // Everything supervised is dead or dying now; forget the groups so
        // a recycled pgid can never be killed for a stale registration
        // (mirrors Windows, where dead processes leave the job on their own).
        if let Ok(mut g) = self.groups.lock() {
            g.clear();
        }
        match last_err {
            Some(source) => Err(JobObjectError::TerminateFailed { exit_code, source }),
            None => Ok(()),
        }
    }

    /// The registry has no kernel handle; it is valid for its whole lifetime.
    #[inline]
    pub fn is_valid(&self) -> bool {
        self.groups.lock().is_ok()
    }
}

impl Drop for JobObject {
    fn drop(&mut self) {
        // KILL_ON_JOB_CLOSE analog: the registry going away kills every
        // group still assigned to it.
        let _ = self.terminate(1);
    }
}

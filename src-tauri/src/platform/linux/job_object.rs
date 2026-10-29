//! Process-group isolation for zapret2 engine processes (Linux).
//!
//! The Linux equivalent of the Windows Job Object with
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. There is no kernel handle to close,
//! so the guarantee is assembled from three parts:
//!
//! 1. Every supervised engine is spawned with `setsid` (see
//!    `platform::process::spawn_process`): it leads its own process group,
//!    and `prctl(PR_SET_PDEATHSIG, SIGKILL)` arms a kernel kill of the
//!    LEADER ONLY if the supervisor process dies.
//! 2. This registry remembers every assigned group id together with the
//!    leader's identity (its `/proc` start time, captured at assignment);
//!    `terminate` and `Drop` re-verify that identity and only then
//!    `killpg(SIGKILL)` the group, so helpers the engine itself may have
//!    forked die with it — the analog of terminating the job. A group id
//!    whose leader no longer matches the captured identity was recycled
//!    by the kernel for a stranger's process group: it is forgotten,
//!    never killed.
//! 3. `Drop` runs on orderly supervisor teardown (svc shutdown, GUI exit),
//!    covering the graceful paths with whole-group kills.
//!
//! Honest limits:
//! - On a VIOLENT supervisor death (SIGKILL, crash) no `Drop` runs, and
//!   the pdeathsig reaches only the group leader. Helpers the leader
//!   forked survive that path; the next svc start cleans up stray engine
//!   processes by image scan (`kill_stray_winws2_processes`). Windows
//!   covers this path fully (the job dies with its owner); Linux does
//!   not, and this module does not pretend otherwise.
//! - A grandchild that calls `setsid` itself escapes its group and
//!   survives `terminate` — the same class of escape a process breaking
//!   away from a Windows job has. Nothing in the zapret2 engine does
//!   this.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;
use thiserror::Error;

/// Identity of a supervised group leader, captured at assignment time.
///
/// A pgid alone is not an identity: the kernel recycles pids, and a group
/// id whose engine died long ago can reappear as a stranger's group.
/// Before any `killpg`, the leader (pid == pgid) must still match what
/// was captured here, or the registration is stale and is dropped
/// without signalling anything.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LeaderIdentity {
    /// `/proc/<pid>/stat` field 22 (start time in clock ticks since
    /// boot). Two processes with the same recycled pid practically
    /// never share a start time.
    StartTime(u64),
    /// Fallback when the stat file cannot be parsed: the leader's
    /// executable path from `/proc/<pid>/exe`.
    ExePath(PathBuf),
}

/// Reads the leader start time (field 22) from `/proc/<pid>/stat`.
///
/// The `comm` field in parentheses can itself contain spaces and
/// parentheses, so parsing starts after the LAST ')': the remaining
/// whitespace-separated tokens begin at field 3, which puts field 22
/// at offset 19.
fn read_leader_start_time(pid: u32) -> Option<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let tail = stat.rsplit(')').next()?;
    tail.split_whitespace().nth(19)?.parse().ok()
}

/// Reads the leader executable path from `/proc/<pid>/exe`, trimming
/// the " (deleted)" suffix the kernel appends for a replaced binary
/// (the same running image, so the identity is unchanged).
fn read_leader_exe_path(pid: u32) -> Option<PathBuf> {
    let exe = fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    let name = exe.to_string_lossy();
    let trimmed = name.strip_suffix(" (deleted)").unwrap_or(&name);
    Some(PathBuf::from(trimmed.to_string()))
}

/// Captures the identity of the process currently leading group `pgid`
/// (leader pid == pgid for everything we spawn). None when the process
/// is already gone or exposes neither a start time nor an exe path.
fn capture_leader_identity(pgid: i32) -> Option<LeaderIdentity> {
    let pid = pgid as u32;
    if let Some(start) = read_leader_start_time(pid) {
        return Some(LeaderIdentity::StartTime(start));
    }
    read_leader_exe_path(pid).map(LeaderIdentity::ExePath)
}

/// True when the process now leading group `pgid` is still the one
/// `expected` was captured from. Unverifiable (process gone, /proc
/// unreadable, identity source vanished) is NOT a match: an
/// unverifiable group is never signalled.
fn leader_identity_matches(pgid: i32, expected: &LeaderIdentity) -> bool {
    match capture_leader_identity(pgid) {
        Some(current) => &current == expected,
        None => false,
    }
}

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
    /// process we spawn, thanks to `setsid`), each with the leader
    /// identity captured at assignment (see [`LeaderIdentity`]).
    groups: Mutex<HashMap<i32, LeaderIdentity>>,
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
            groups: Mutex::new(HashMap::new()),
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
        // Capture the leader's identity NOW: terminate() will only
        // killpg this group while the leader still matches it, so a
        // pgid recycled after the engine died can never be killed on
        // behalf of a long-dead registration. A leader we cannot
        // identify cannot be supervised safely — fail the assignment
        // (the caller kills the just-spawned child on this error).
        let identity = capture_leader_identity(pgid).ok_or_else(|| {
            JobObjectError::AssignProcessFailed {
                pid,
                source: io::Error::new(
                    io::ErrorKind::NotFound,
                    "cannot capture group leader identity from /proc",
                ),
            }
        })?;
        let mut groups = self
            .groups
            .lock()
            .map_err(|_| JobObjectError::InvalidHandle)?;
        // Prune registrations whose leader process is gone entirely
        // (Windows forgets dead job members automatically). Identity
        // re-verification at kill time is the real recycled-pgid
        // guard; this only keeps the map from growing stale entries.
        groups.retain(|&g, _| unsafe { libc::kill(g, 0) } == 0 || io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH));
        groups.insert(pgid, identity);
        Ok(())
    }

    /// Forcefully terminates all processes in every supervised group
    /// (SIGKILL to each process group).
    ///
    /// Each group is killed ONLY if its leader still matches the
    /// identity captured at assignment. A group whose leader changed
    /// (the engine died and the kernel recycled the pgid) or that can
    /// no longer be verified is forgotten without being signalled.
    pub fn terminate(&self, exit_code: u32) -> Result<(), JobObjectError> {
        let groups: Vec<(i32, LeaderIdentity)> = match self.groups.lock() {
            Ok(g) => g.iter().map(|(&pgid, id)| (pgid, id.clone())).collect(),
            Err(_) => return Err(JobObjectError::InvalidHandle),
        };
        let mut last_err: Option<io::Error> = None;
        for (pgid, identity) in groups {
            if !leader_identity_matches(pgid, &identity) {
                // Stale or recycled registration: never signal it.
                continue;
            }
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
        // Everything supervised is dead, dying, or proven not ours now;
        // forget the registrations so a recycled pgid can never be
        // killed for a stale registration later (mirrors Windows, where
        // dead processes leave the job on their own).
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

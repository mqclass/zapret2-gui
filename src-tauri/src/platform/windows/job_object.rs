//! Windows Job Object Management for zapret2 Process Tree Isolation
//!
//! # Architecture & Failure Mode Prevention
//! When the GUI crashes, is forcefully killed via Task Manager, or panics, standard
//! Windows child processes survive as orphans. If `winws2.exe` remains running without
//! gracefully closing its WinDivert packet filter handles, WinDivert locks the Windows
//! Filtering Platform (WFP) callout drivers, completely cutting off all host network traffic.
//!
//! By binding `winws2.exe` to a Windows Job Object configured with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`,
//! the Windows NT kernel itself guarantees that when the supervisor process handle closes
//! (under ANY termination condition), all child processes associated with the job object
//! are instantaneously terminated by the kernel.

use std::ffi::c_void;
use std::io;
use std::mem::size_of;
use std::os::windows::io::RawHandle;
use thiserror::Error;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject, TerminateJobObject,
    JobObjectExtendedLimitInformation, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
};

/// Errors originating from Windows Job Object operations.
#[allow(dead_code)]
#[derive(Debug, Error)]
pub enum JobObjectError {
    #[error("Failed to create Windows Job Object: {0}")]
    CreateFailed(#[source] io::Error),

    #[error("Failed to configure Job Object limits (JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE): {0}")]
    ConfigureLimitsFailed(#[source] io::Error),

    #[error("Failed to assign process (HANDLE: {handle:?}) to Job Object: {source}")]
    AssignProcessFailed {
        handle: isize,
        #[source]
        source: io::Error,
    },

    #[error("Failed to open process PID {pid} for Job Object assignment: {source}")]
    OpenProcessFailed {
        pid: u32,
        #[source]
        source: io::Error,
    },

    #[error("Failed to terminate Job Object with exit code {exit_code}: {source}")]
    TerminateFailed {
        exit_code: u32,
        #[source]
        source: io::Error,
    },

    #[error("Job Object handle is invalid or already closed")]
    InvalidHandle,
}

/// Thread-safe RAII guard encapsulating a Windows Job Object.
///
/// Configured with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. When this guard is dropped
/// or the hosting process terminates, the Windows kernel forcefully terminates all
/// member processes.
#[derive(Debug)]
pub struct JobObject {
    handle: HANDLE,
}

// Windows kernel handles are thread-safe and can be shared across threads.
unsafe impl Send for JobObject {}
unsafe impl Sync for JobObject {}

#[allow(dead_code)]
impl JobObject {
    /// Creates and configures a new Windows Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
    ///
    /// # Errors
    /// Returns `JobObjectError::CreateFailed` if `CreateJobObjectW` fails.
    /// Returns `JobObjectError::ConfigureLimitsFailed` if setting extended limits fails.
    pub fn create() -> Result<Self, JobObjectError> {
        let handle = unsafe { CreateJobObjectW(None, None) }.map_err(|e| {
            JobObjectError::CreateFailed(io::Error::from_raw_os_error(e.code().0))
        })?;

        if handle.is_invalid() || handle.0.is_null() {
            return Err(JobObjectError::CreateFailed(io::Error::last_os_error()));
        }

        // Configure Job Object limits: Kill all assigned processes when the Job Object handle is closed.
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

        let set_res = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const c_void,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };

        if let Err(e) = set_res {
            unsafe {
                let _ = CloseHandle(handle);
            }
            return Err(JobObjectError::ConfigureLimitsFailed(
                io::Error::from_raw_os_error(e.code().0),
            ));
        }

        Ok(Self { handle })
    }

    /// Assigns an existing Windows process handle to this Job Object.
    ///
    /// The process must have `PROCESS_SET_QUOTA` and `PROCESS_TERMINATE` access rights.
    pub fn assign_process(&self, process_handle: HANDLE) -> Result<(), JobObjectError> {
        if self.handle.is_invalid() || self.handle.0.is_null() {
            return Err(JobObjectError::InvalidHandle);
        }

        unsafe { AssignProcessToJobObject(self.handle, process_handle) }.map_err(|e| {
            JobObjectError::AssignProcessFailed {
                handle: process_handle.0 as isize,
                source: io::Error::from_raw_os_error(e.code().0),
            }
        })
    }

    /// Assigns a process identified by its raw handle (`RawHandle`) to this Job Object.
    ///
    /// Useful for integrating directly with `tokio::process::Child::raw_handle()`
    /// or `std::process::Child::as_raw_handle()`.
    pub fn assign_raw_handle(&self, raw: RawHandle) -> Result<(), JobObjectError> {
        if raw.is_null() {
            return Err(JobObjectError::InvalidHandle);
        }
        let handle = HANDLE(raw as *mut c_void);
        self.assign_process(handle)
    }

    /// Opens a process by its PID and assigns it to this Job Object.
    ///
    /// Requests `PROCESS_SET_QUOTA | PROCESS_TERMINATE | PROCESS_ASSIGN_TO_JOB`.
    pub fn assign_pid(&self, pid: u32) -> Result<(), JobObjectError> {
        let process_handle = unsafe {
            OpenProcess(
                PROCESS_SET_QUOTA | PROCESS_TERMINATE,
                false,
                pid,
            )
        }
        .map_err(|e| JobObjectError::OpenProcessFailed {
            pid,
            source: io::Error::from_raw_os_error(e.code().0),
        })?;

        // Ensure process handle is closed after assignment
        let assign_result = self.assign_process(process_handle);
        unsafe {
            let _ = CloseHandle(process_handle);
        }

        assign_result
    }

    /// Forcefully terminates all processes currently associated with this Job Object.
    pub fn terminate(&self, exit_code: u32) -> Result<(), JobObjectError> {
        if self.handle.is_invalid() || self.handle.0.is_null() {
            return Err(JobObjectError::InvalidHandle);
        }

        unsafe { TerminateJobObject(self.handle, exit_code) }.map_err(|e| {
            JobObjectError::TerminateFailed {
                exit_code,
                source: io::Error::from_raw_os_error(e.code().0),
            }
        })
    }

    /// Returns the raw Windows `HANDLE` value of the Job Object.
    #[inline]
    pub fn raw_handle(&self) -> RawHandle {
        self.handle.0 as RawHandle
    }

    /// Checks if the Job Object handle is currently valid.
    #[inline]
    pub fn is_valid(&self) -> bool {
        !self.handle.is_invalid() && !self.handle.0.is_null()
    }
}

impl Drop for JobObject {
    fn drop(&mut self) {
        if !self.handle.is_invalid() && !self.handle.0.is_null() {
            unsafe {
                let _ = CloseHandle(self.handle);
            }
        }
    }
}

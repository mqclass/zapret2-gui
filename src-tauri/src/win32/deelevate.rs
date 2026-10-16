//! De-elevated process spawning for the Telegram MTProxy sidecar (M-3).
//!
//! The GUI runs elevated (WinDivert needs Administrator), but the tg-ws-proxy
//! sidecar only listens on `127.0.0.1:1443` — it has no business holding the
//! admin token. A compromised upstream binary (see the supply-chain analysis
//! in `engine::tgproxy`) would otherwise execute as Administrator.
//!
//! When the GUI is elevated we duplicate the primary token of the interactive
//! user's shell (`explorer.exe`, which always runs as the plain non-elevated
//! user) and spawn via `CreateProcessAsUserW`, so the sidecar runs at medium
//! integrity as the plain user. When the GUI itself is NOT elevated there is
//! nothing to drop and the child is spawned with the current token via
//! `CreateProcessW`.
//!
//! Fail-closed: every error here refuses the spawn — there is deliberately no
//! fallback to an elevated child.

use std::ffi::c_void;
use std::io;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::os::windows::process::ExitStatusExt;
use std::path::Path;
use std::process::ExitStatus;

use windows::core::{PCWSTR, PWSTR, w};
use windows::Win32::Foundation::{CloseHandle, GetLastError, BOOL, HANDLE, LUID};
use windows::Win32::Security::{
    AdjustTokenPrivileges, DuplicateTokenEx, LookupPrivilegeValueW, LUID_AND_ATTRIBUTES,
    SE_PRIVILEGE_ENABLED, SecurityImpersonation, TokenPrimary, TOKEN_ADJUST_PRIVILEGES,
    TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
    TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    CreateProcessAsUserW, CreateProcessW, GetCurrentProcess, GetExitCodeProcess, OpenProcess,
    OpenProcessToken, TerminateProcess, WaitForSingleObject, CREATE_NEW_PROCESS_GROUP,
    CREATE_NO_WINDOW, INFINITE, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION,
    PROCESS_QUERY_LIMITED_INFORMATION, STARTUPINFOW,
};

/// De-elevation failures. Always fail-closed: the caller must refuse to start
/// the sidecar rather than fall back to an elevated child.
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

/// `GetExitCodeProcess` sentinel for "still running" (winbase.h `STILL_ACTIVE`).
const STILL_ACTIVE: u32 = 259;

/// A spawned child process owned through a raw Windows handle.
///
/// Mirrors the subset of the `tokio::process::Child` API used by
/// `TgProxyManager`, so the de-elevated and current-token spawn paths share a
/// single type downstream.
pub struct RawChild {
    handle: OwnedHandle,
    pid: u32,
}

impl RawChild {
    pub fn id(&self) -> Option<u32> {
        Some(self.pid)
    }

    pub fn raw_handle(&self) -> Option<RawHandle> {
        Some(self.handle.as_raw_handle())
    }

    fn win_handle(&self) -> HANDLE {
        HANDLE(self.handle.as_raw_handle() as *mut c_void)
    }

    /// Best-effort kill (mirrors `Child::start_kill`).
    pub fn start_kill(&mut self) -> io::Result<()> {
        unsafe { TerminateProcess(self.win_handle(), 1) }
            .map_err(|e| io::Error::from_raw_os_error(e.code().0))?;
        Ok(())
    }

    /// Non-blocking exit check (mirrors `Child::try_wait`).
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        let mut code: u32 = 0;
        unsafe { GetExitCodeProcess(self.win_handle(), &mut code) }
            .map_err(|e| io::Error::from_raw_os_error(e.code().0))?;
        if code == STILL_ACTIVE {
            Ok(None)
        } else {
            Ok(Some(ExitStatus::from_raw(code)))
        }
    }

    /// Waits for process exit on a blocking thread (mirrors `Child::wait`).
    pub async fn wait(&mut self) -> io::Result<ExitStatus> {
        // `usize` is `Send + 'static`; the `&mut self` borrow held across the
        // await keeps the owned handle alive for the whole wait, so
        // reconstructing the HANDLE from the raw value is sound.
        let raw = self.handle.as_raw_handle() as usize;
        tokio::task::spawn_blocking(move || unsafe {
            let handle = HANDLE(raw as *mut c_void);
            WaitForSingleObject(handle, INFINITE);
            let mut code: u32 = 0;
            GetExitCodeProcess(handle, &mut code)
                .map_err(|e| io::Error::from_raw_os_error(e.code().0))?;
            Ok(ExitStatus::from_raw(code))
        })
        .await
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("wait task failed: {e}")))?
    }
}

/// Enables one privilege (e.g. `SeAssignPrimaryTokenPrivilege`) in the current
/// process token. Elevated administrators hold these disabled by default;
/// `CreateProcessAsUserW` requires them enabled.
fn enable_privilege(name: PCWSTR) -> Result<(), DeelevateError> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )
        .map_err(|e| DeelevateError::msg(format!("OpenProcessToken: {e}")))?;

        let mut luid = LUID::default();
        let null_system = PCWSTR(std::ptr::null());
        let lookup = LookupPrivilegeValueW(null_system, name, &mut luid);
        if let Err(e) = lookup {
            let _ = CloseHandle(token);
            return Err(DeelevateError::msg(format!("LookupPrivilegeValueW: {e}")));
        }

        let tp = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        let adjusted =
            AdjustTokenPrivileges(token, BOOL::from(false), Some(&tp as *const _), 0, None, None);
        // `AdjustTokenPrivileges` reports partial failure via GetLastError,
        // not via its return value.
        let last_err = GetLastError();
        let _ = CloseHandle(token);
        adjusted.map_err(|e| DeelevateError::msg(format!("AdjustTokenPrivileges: {e}")))?;
        if last_err.0 != 0 {
            return Err(DeelevateError::msg(format!(
                "privilege not held by process token (GetLastError={})",
                last_err.0
            )));
        }
        Ok(())
    }
}

/// Duplicates a primary token for the interactive user from their shell
/// (`explorer.exe`). The shell always runs as the plain (non-elevated) user,
/// so the duplicated token carries no Administrator group.
fn duplicated_shell_token() -> Result<HANDLE, DeelevateError> {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
            .map_err(|e| DeelevateError::msg(format!("CreateToolhelp32Snapshot: {e}")))?;

        let mut entry = PROCESSENTRY32W::default();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut explorer_pid: Option<u32> = None;
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                if String::from_utf16_lossy(&entry.szExeFile[..len])
                    .eq_ignore_ascii_case("explorer.exe")
                {
                    explorer_pid = Some(entry.th32ProcessID);
                    break;
                }
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);

        let pid = explorer_pid.ok_or_else(|| {
            DeelevateError::msg(
                "explorer.exe not found — cannot determine the interactive user token",
            )
        })?;

        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, BOOL::from(false), pid)
            .map_err(|e| DeelevateError::msg(format!("OpenProcess(explorer): {e}")))?;
        let mut token = HANDLE::default();
        let opened = OpenProcessToken(process, TOKEN_DUPLICATE | TOKEN_QUERY, &mut token);
        let _ = CloseHandle(process);
        opened.map_err(|e| DeelevateError::msg(format!("OpenProcessToken(explorer): {e}")))?;

        let mut primary = HANDLE::default();
        let duplicated = DuplicateTokenEx(
            token,
            TOKEN_ASSIGN_PRIMARY | TOKEN_DUPLICATE | TOKEN_QUERY,
            None,
            SecurityImpersonation,
            TokenPrimary,
            &mut primary,
        );
        let _ = CloseHandle(token);
        duplicated.map_err(|e| DeelevateError::msg(format!("DuplicateTokenEx: {e}")))?;
        Ok(primary)
    }
}

/// Builds the mutable command line `"exe" arg1 arg2 …` required by the
/// `CreateProcess*` family (`lpCommandLine` may be written to by the callee,
/// so it must be a private mutable buffer).
fn build_command_line(exe: &Path, args: &[&str]) -> Vec<u16> {
    let mut s = format!("\"{}\"", exe.display());
    for a in args {
        s.push(' ');
        s.push_str(a);
    }
    let mut wide: Vec<u16> = s.encode_utf16().collect();
    wide.push(0);
    wide
}

/// NUL-terminated UTF-16 for a path (`PCWSTR` parameters).
fn wide_null(path: &Path) -> Vec<u16> {
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);
    wide
}

fn spawn_common(
    exe: &Path,
    args: &[&str],
    workdir: &Path,
    token: Option<HANDLE>,
) -> Result<RawChild, DeelevateError> {
    unsafe {
        let mut cmdline = build_command_line(exe, args);
        let exe_w = wide_null(exe);
        let workdir_w = wide_null(workdir);

        let mut si = STARTUPINFOW::default();
        si.cb = size_of::<STARTUPINFOW>() as u32;
        let mut pi = PROCESS_INFORMATION::default();

        let flags: PROCESS_CREATION_FLAGS = CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP;
        let created = match token {
            Some(t) => CreateProcessAsUserW(
                t,
                PCWSTR(exe_w.as_ptr()),
                PWSTR(cmdline.as_mut_ptr()),
                None,
                None,
                BOOL::from(false),
                flags,
                None,
                PCWSTR(workdir_w.as_ptr()),
                &si,
                &mut pi,
            ),
            None => CreateProcessW(
                PCWSTR(exe_w.as_ptr()),
                PWSTR(cmdline.as_mut_ptr()),
                None,
                None,
                BOOL::from(false),
                flags,
                None,
                PCWSTR(workdir_w.as_ptr()),
                &si,
                &mut pi,
            ),
        };
        if let Err(e) = created {
            // Don't leak the duplicated user token on spawn failure.
            if let Some(t) = token {
                let _ = CloseHandle(t);
            }
            return Err(DeelevateError::msg(format!(
                "CreateProcess{}W: {e}",
                if token.is_some() { "AsUser" } else { "" }
            )));
        }

        let _ = CloseHandle(pi.hThread);
        if let Some(t) = token {
            let _ = CloseHandle(t);
        }
        let handle = OwnedHandle::from_raw_handle(pi.hProcess.0 as *mut c_void);
        Ok(RawChild {
            handle,
            pid: pi.dwProcessId,
        })
    }
}

/// Spawns `exe` de-elevated with the interactive user's token.
///
/// Call only when the GUI itself is elevated. Fail-closed: any failure
/// refuses the spawn — there is no elevated fallback.
pub fn spawn_deelevated(
    exe: &Path,
    args: &[&str],
    workdir: &Path,
) -> Result<RawChild, DeelevateError> {
    enable_privilege(*w!("SeAssignPrimaryTokenPrivilege")).map_err(|e| {
        DeelevateError::msg(format!("cannot enable SeAssignPrimaryTokenPrivilege: {e}"))
    })?;
    enable_privilege(*w!("SeIncreaseQuotaPrivilege")).map_err(|e| {
        DeelevateError::msg(format!("cannot enable SeIncreaseQuotaPrivilege: {e}"))
    })?;
    let token = duplicated_shell_token()?;
    spawn_common(exe, args, workdir, Some(token))
}

/// Spawns `exe` with the current process token (for a non-elevated GUI —
/// there is nothing to de-elevate, the child is already least-privilege).
pub fn spawn_current_token(
    exe: &Path,
    args: &[&str],
    workdir: &Path,
) -> Result<RawChild, DeelevateError> {
    spawn_common(exe, args, workdir, None)
}

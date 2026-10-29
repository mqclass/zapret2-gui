//! UAC & Windows Process Privilege Elevation Management
//!
//! # Purpose
//! The zapret2 supervisor utilizes the WinDivert kernel driver (`WinDivert64.sys`)
//! to intercept and mutate network packets in the Windows Filtering Platform (WFP).
//! Initializing, registering, and attaching WinDivert WFP callout drivers strictly
//! requires Windows Administrator privileges.
//!
//! This module provides verification of token elevation (`GetTokenInformation`) and
//! seamless self-elevation via the Windows Shell `runas` verb (`ShellExecuteExW`).

use std::ffi::c_void;
use std::io;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use thiserror::Error;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{
    GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW};

/// Errors originating from UAC privilege validation and self-elevation.
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
}

/// Checks if the current process is running with elevated Administrator privileges.
///
/// Queries the primary token of the current process using `GetTokenInformation(TokenElevation)`.
/// Returns `Ok(true)` if running elevated, or `Ok(false)` if running in standard user mode.
pub fn is_elevated() -> Result<bool, PrivilegeError> {
    let mut token_handle = HANDLE::default();

    unsafe {
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token_handle)
    }
    .map_err(|e| PrivilegeError::OpenTokenFailed(io::Error::from_raw_os_error(e.code().0)))?;

    let mut elevation = TOKEN_ELEVATION::default();
    let mut return_length = 0u32;

    let query_res = unsafe {
        GetTokenInformation(
            token_handle,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut c_void),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut return_length,
        )
    };

    // Always release the token handle to prevent handle leakage
    unsafe {
        let _ = CloseHandle(token_handle);
    }

    query_res.map_err(|e| {
        PrivilegeError::GetTokenInfoFailed(io::Error::from_raw_os_error(e.code().0))
    })?;

    Ok(elevation.TokenIsElevated != 0)
}

/// Formats command-line arguments according to Windows quoting conventions.
fn format_command_line_args(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            if arg.contains(' ') || arg.contains('\t') || arg.contains('"') || arg.is_empty() {
                let mut escaped = String::from("\"");
                let mut backslashes = 0;
                for ch in arg.chars() {
                    match ch {
                        '\\' => backslashes += 1,
                        '"' => {
                            for _ in 0..(backslashes * 2 + 1) {
                                escaped.push('\\');
                            }
                            escaped.push('"');
                            backslashes = 0;
                        }
                        _ => {
                            for _ in 0..backslashes {
                                escaped.push('\\');
                            }
                            backslashes = 0;
                            escaped.push(ch);
                        }
                    }
                }
                for _ in 0..(backslashes * 2) {
                    escaped.push('\\');
                }
                escaped.push('"');
                escaped
            } else {
                arg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Relaunches the current application with elevated privileges using the `runas` verb.
///
/// If UAC elevation is granted, the new elevated process starts and this unprivileged
/// process immediately terminates via `std::process::exit(0)`.
/// If the user denies the UAC prompt, returns `PrivilegeError::ElevationCancelledByUser`.
pub fn relaunch_elevated() -> Result<(), PrivilegeError> {
    let current_exe: PathBuf = std::env::current_exe()
        .map_err(PrivilegeError::CurrentExeResolutionFailed)?;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let formatted_args = format_command_line_args(&args);

    let exe_utf16: Vec<u16> = current_exe
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let verb_utf16: Vec<u16> = "runas\0".encode_utf16().collect();

    let params_utf16: Vec<u16> = formatted_args
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let working_dir: PathBuf = std::env::current_dir()
        .unwrap_or_else(|_| {
            current_exe
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."))
        });

    let dir_utf16: Vec<u16> = working_dir
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut sei = SHELLEXECUTEINFOW::default();
    sei.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
    sei.fMask = SEE_MASK_NOASYNC;
    sei.lpVerb = PCWSTR(verb_utf16.as_ptr());
    sei.lpFile = PCWSTR(exe_utf16.as_ptr());
    sei.lpDirectory = PCWSTR(dir_utf16.as_ptr());
    if !formatted_args.is_empty() {
        sei.lpParameters = PCWSTR(params_utf16.as_ptr());
    }
    sei.nShow = 1; // SW_SHOWNORMAL

    let exec_res = unsafe { ShellExecuteExW(&mut sei) };

    match exec_res {
        Ok(_) => {
            // Elevated instance launched successfully; terminate the unprivileged instance immediately
            std::process::exit(0);
        }
        Err(e) => {
            // ERROR_CANCELLED (0x800704C7 or 1223) indicates the user pressed "No" on the UAC prompt
            let code = e.code().0 as u32;
            if code == 1223 || code == 0x800704C7 {
                Err(PrivilegeError::ElevationCancelledByUser)
            } else {
                Err(PrivilegeError::ShellExecuteFailed(
                    io::Error::from_raw_os_error(e.code().0),
                ))
            }
        }
    }
}

/// Verifies elevation and automatically relaunches with Administrator privileges if unprivileged.
///
/// Intended for use in `main()` before GUI initialization.
pub fn ensure_elevated() -> Result<(), PrivilegeError> {
    if !is_elevated()? {
        relaunch_elevated()?;
    }
    Ok(())
}

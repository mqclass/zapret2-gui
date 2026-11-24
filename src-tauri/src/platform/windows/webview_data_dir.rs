//! WebView2 user-data directory for the interactive session user.
//!
//! Issue #8: when the app is launched from an ordinary account with an
//! admin password ("run as administrator"), the process token belongs to
//! the admin account, but WebView2 always runs as the signed-in session
//! user and cannot write into the admin profile. The default data folder
//! (`%LOCALAPPDATA%\com.zapret2.supervisor\EBWebView`, resolved from the
//! process token) then fails with "could not create the data directory".
//!
//! This module detects the mismatch (token user != session user, compared
//! by SID) and resolves the session user's LOCALAPPDATA instead. It
//! returns `None` for a normal launch (same user) and on any failure —
//! the caller then keeps Tauri's default, so a failure here can never
//! break a working launch. The old admin-profile data is not migrated:
//! the first fixed launch starts a fresh profile under the session user.
//!
//! windows 0.58 notes (got wrong once — CI caught it, see below):
//! - `PSID` is a tuple struct `PSID(pub *mut c_void)`, not a raw pointer:
//!   construct it, never `as`-cast to it; test null via `.0.is_null()`.
//! - `EqualSid` returns `Result<()>` (Ok = equal), not BOOL.
//! - `OpenProcessToken` lives in `Win32::System::Threading`, and
//!   `ConvertSidToStringSidW` in `Win32::Security::Authorization`,
//!   not in `Win32::Security`.

use std::ffi::c_void;
use std::path::PathBuf;

use windows::core::{w, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{
    EqualSid, GetLengthSid, GetTokenInformation, LookupAccountNameW, TokenUser, PSID,
    SID_NAME_USE, TOKEN_QUERY, TOKEN_USER,
};
use windows::Win32::System::Registry::{
    RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ,
};
use windows::Win32::System::RemoteDesktop::{
    WTSFreeMemory, WTSQuerySessionInformationW, WTSUserName, WTS_CURRENT_SESSION,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// App identifier, mirroring tauri.conf.json: the default WebView2 data
/// folder is `%LOCALAPPDATA%\<identifier>\EBWebView`. Keep in sync.
const APP_IDENTIFIER: &str = "com.zapret2.supervisor";

/// A SID byte buffer as a `PSID` (the struct wraps a raw pointer, so
/// this is only a re-interpretation — the bytes must stay alive).
/// Both consumers (`EqualSid`, `ConvertSidToStringSidW`) only read the
/// SID, so the `*mut` here is never written through.
fn psid(bytes: &[u8]) -> PSID {
    PSID(bytes.as_ptr() as *mut c_void)
}

/// WebView2 data folder for the interactive session user, when the
/// process token belongs to a different account. `None` for a normal
/// launch (token user == session user) and on any failure.
pub fn session_user_data_dir() -> Option<PathBuf> {
    let session_sid = session_user_sid()?;
    let token_sid = process_token_sid()?;
    // Same user: the default data folder is already the session user's.
    // EqualSid returns Err both when the SIDs differ and on a genuine
    // error; either way we redirect, which is the safe direction
    // (WebView2 runs as the session user regardless).
    let same = unsafe { EqualSid(psid(&session_sid), psid(&token_sid)).is_ok() };
    if same {
        return None;
    }
    let sid_string = sid_to_string(&session_sid)?;
    let profile = profile_path_for_sid(&sid_string)?;
    Some(
        profile
            .join("AppData")
            .join("Local")
            .join(APP_IDENTIFIER)
            .join("EBWebView"),
    )
}

/// SID of the user signed into the current interactive session, via the
/// session username (WTS) resolved to a SID (LookupAccountName).
fn session_user_sid() -> Option<Vec<u8>> {
    unsafe {
        let mut buffer = PWSTR::null();
        let mut bytes: u32 = 0;
        WTSQuerySessionInformationW(
            HANDLE::default(), // NULL = current server
            WTS_CURRENT_SESSION,
            WTSUserName,
            &mut buffer,
            &mut bytes,
        )
        .ok()?;
        if buffer.is_null() || bytes == 0 {
            // Non-null but empty buffer: still owned by us, free it.
            if !buffer.is_null() {
                WTSFreeMemory(buffer.0 as *mut _);
            }
            return None;
        }
        // Null-terminated UTF-16 username, `bytes` counts the NUL too.
        let raw = std::slice::from_raw_parts(buffer.0, bytes as usize / 2);
        let end = raw.iter().position(|&c| c == 0).unwrap_or(raw.len());
        let username = String::from_utf16_lossy(&raw[..end]).trim().to_string();
        WTSFreeMemory(buffer.0 as *mut _);
        if username.is_empty() {
            return None;
        }
        // WTS returns a bare username and LookupAccountName resolves
        // local-first: a domain session user whose name collides with a
        // local account would resolve to the wrong SID. Fail-safe (the
        // redirect just doesn't happen, as before the fix); the robust
        // alternative (WTSQueryUserToken) needs SE_TCB_NAME.
        lookup_sid(&username)
    }
}

/// SID bytes of the current process token user.
fn process_token_sid() -> Option<Vec<u8>> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).ok()?;
        // Two-call pattern: first call returns the TOKEN_USER size.
        let mut needed: u32 = 0;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
        if needed == 0 {
            let _ = CloseHandle(token);
            return None;
        }
        let mut buf = vec![0u8; needed as usize];
        let ok = GetTokenInformation(
            token,
            TokenUser,
            Some(buf.as_mut_ptr() as *mut _),
            needed,
            &mut needed,
        );
        let _ = CloseHandle(token);
        ok.ok()?;
        // `buf` is a byte buffer: its alignment is not guaranteed, so
        // read the TOKEN_USER out unaligned instead of dereferencing.
        // The API guarantees `needed >= sizeof(TOKEN_USER)`; check it
        // anyway so the read below cannot run past the buffer.
        if buf.len() < std::mem::size_of::<TOKEN_USER>() {
            return None;
        }
        let user = std::ptr::read_unaligned(buf.as_ptr() as *const TOKEN_USER);
        let sid = user.User.Sid;
        if sid.0.is_null() {
            return None;
        }
        let len = GetLengthSid(sid) as usize;
        // The SID data follows the TOKEN_USER header inside `buf`.
        if len == 0 || std::mem::size_of::<TOKEN_USER>() + len > buf.len() {
            return None;
        }
        Some(std::slice::from_raw_parts(sid.0 as *const u8, len).to_vec())
    }
}

/// Resolves an account name to its SID bytes (two-call pattern).
fn lookup_sid(account: &str) -> Option<Vec<u8>> {
    unsafe {
        let name = HSTRING::from(account);
        let mut sid_len: u32 = 0;
        let mut domain_len: u32 = 0;
        let mut sid_type = SID_NAME_USE(0);
        // First call: sizes only (fails with insufficient buffer).
        let _ = LookupAccountNameW(
            PCWSTR::null(),
            PCWSTR(name.as_ptr()),
            PSID(std::ptr::null_mut()),
            &mut sid_len,
            PWSTR::null(),
            &mut domain_len,
            &mut sid_type,
        );
        if sid_len == 0 {
            return None;
        }
        let mut sid = vec![0u8; sid_len as usize];
        let mut domain = vec![0u16; domain_len as usize];
        LookupAccountNameW(
            PCWSTR::null(),
            PCWSTR(name.as_ptr()),
            PSID(sid.as_mut_ptr() as *mut c_void),
            &mut sid_len,
            PWSTR(domain.as_mut_ptr()),
            &mut domain_len,
            &mut sid_type,
        )
        .ok()?;
        sid.truncate(sid_len as usize);
        Some(sid)
    }
}

/// SID bytes to their string form (`S-1-5-21-...`), for the ProfileList
/// registry key.
fn sid_to_string(sid: &[u8]) -> Option<String> {
    unsafe {
        let mut out = PWSTR::null();
        ConvertSidToStringSidW(psid(sid), &mut out).ok()?;
        if out.is_null() {
            return None;
        }
        // SID strings are short (well under 256 chars); scan for the NUL.
        let mut len = 0;
        while len < 256 && *out.0.add(len) != 0 {
            len += 1;
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(out.0, len));
        let _ = LocalFree(HLOCAL(out.0 as *mut c_void));
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }
}

/// Profile directory for a SID, from the ProfileList registry key.
/// `ProfileImagePath` is REG_EXPAND_SZ (`%SystemDrive%\Users\...`), so
/// the value is read expanded.
fn profile_path_for_sid(sid_string: &str) -> Option<PathBuf> {
    let subkey = HSTRING::from(format!(
        r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList\{sid_string}"
    ));
    unsafe {
        let mut buf = [0u16; 512];
        let mut len = (buf.len() * 2) as u32;
        let status = RegGetValueW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(subkey.as_ptr()),
            w!("ProfileImagePath"),
            RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut len),
        );
        if status.is_ok() {
            let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            let s = String::from_utf16_lossy(&buf[..end]).trim().to_string();
            if !s.is_empty() {
                return Some(PathBuf::from(s));
            }
        }
        None
    }
}

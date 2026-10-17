//! WebView2 runtime pre-flight check.
//!
//! The UI is rendered by the system WebView2 (Edge/Chromium) engine. If it is
//! missing or ancient, Tauri cannot create the window and the user gets a raw
//! framework error. This module runs before the Tauri builder and shows a
//! plain-language message instead:
//!
//! - WebView2 missing entirely -> error dialog, offers to open the Microsoft
//!   download page, then exits (there is no UI to fall back to).
//! - WebView2 present but older than MIN_WEBVIEW2_VERSION -> warning dialog
//!   with a choice to continue anyway or quit and update.
//!
//! The version is read from the EdgeUpdate registry keys where the WebView2
//! Runtime registers itself (machine-wide and per-user).

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Registry::{
    RegGetValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_EXPAND_SZ,
    RRF_RT_REG_SZ,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, MB_ICONERROR, MB_ICONWARNING, MB_OKCANCEL, MB_YESNO, IDOK, IDYES,
    MESSAGEBOX_STYLE, SW_SHOWNORMAL,
};

/// WebView2 Runtime client product ID as registered by the EdgeUpdate service.
const WEBVIEW2_CLIENT_ID: &str = "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";
/// Recommended floor: 120.x. Not a documented hard requirement of wry/Tauri v2
/// (the runtime itself works from ~92), just a sane recency line: anything older
/// is years behind evergreen and deserves a nudge. The warning is soft — the
/// user may always continue.
const MIN_WEBVIEW2_VERSION: (u32, u32, u32, u32) = (120, 0, 0, 0);
const DOWNLOAD_URL: &str = "https://developer.microsoft.com/microsoft-edge/webview2/";
/// Process exit code when WebView2 is missing entirely.
const EXIT_NO_WEBVIEW2: i32 = 3;
/// Process exit code when the user declines to continue with an outdated WebView2.
const EXIT_WEBVIEW2_DECLINED: i32 = 4;

fn registry_version(hkey: HKEY, subkey: &HSTRING) -> Option<String> {
    unsafe {
        let mut buf = [0u16; 64];
        let mut len = (buf.len() * 2) as u32;
        let status = RegGetValueW(
            hkey,
            PCWSTR(subkey.as_ptr()),
            w!("pv"),
            RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut len),
        );
        if status.is_ok() {
            let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            let s = String::from_utf16_lossy(&buf[..end]).trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
        None
    }
}

/// Returns the installed WebView2 Runtime version string, e.g. "128.0.2739.42".
pub fn installed_webview2_version() -> Option<String> {
    let base = format!(
        "SOFTWARE\\Microsoft\\EdgeUpdate\\Clients\\{}",
        WEBVIEW2_CLIENT_ID
    );
    let wow = format!(
        "SOFTWARE\\WOW6432Node\\Microsoft\\EdgeUpdate\\Clients\\{}",
        WEBVIEW2_CLIENT_ID
    );
    for (hkey, path) in [
        (HKEY_LOCAL_MACHINE, base.as_str()),
        (HKEY_LOCAL_MACHINE, wow.as_str()),
        (HKEY_CURRENT_USER, base.as_str()),
    ] {
        if let Some(v) = registry_version(hkey, &HSTRING::from(path)) {
            return Some(v);
        }
    }
    None
}

fn parse_version(s: &str) -> Option<(u32, u32, u32, u32)> {
    let mut parts = s.split('.');
    let get = |p: Option<&str>| p?.trim().parse::<u32>().ok();
    Some((
        get(parts.next())?,
        get(parts.next())?,
        get(parts.next())?,
        get(parts.next())?,
    ))
}

fn msg_box(title: &HSTRING, text: &HSTRING, style: MESSAGEBOX_STYLE) -> i32 {
    unsafe {
        MessageBoxW(
            HWND(std::ptr::null_mut()),
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            style,
        )
        .0
    }
}

fn open_download_page() {
    unsafe {
        let url = HSTRING::from(DOWNLOAD_URL);
        let _ = ShellExecuteW(
            HWND(std::ptr::null_mut()),
            w!("open"),
            PCWSTR(url.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// Pre-flight WebView2 check. Returns only when the app may continue;
/// otherwise shows a user-facing dialog and terminates the process.
pub fn ensure_webview2() {
    match installed_webview2_version() {
        None => {
            let title = HSTRING::from("zapret2-gui — нужен WebView2");
            let text = HSTRING::from(
                "Для работы интерфейса нужен Microsoft Edge WebView2 Runtime, \
                 но он не найден на этом компьютере.\n\n\
                 Нажмите OK, чтобы открыть страницу загрузки Microsoft. \
                 Установите WebView2 и запустите приложение снова.",
            );
            if msg_box(&title, &text, MB_OKCANCEL | MB_ICONERROR) == IDOK.0 {
                open_download_page();
            }
            std::process::exit(EXIT_NO_WEBVIEW2);
        }
        Some(ver) => {
            let too_old = parse_version(&ver)
                .map(|v| v < MIN_WEBVIEW2_VERSION)
                .unwrap_or(true);
            if too_old {
                let title = HSTRING::from("zapret2-gui — устаревший WebView2");
                let text = HSTRING::from(format!(
                    "Найден WebView2 версии {}, но для стабильной работы интерфейса \
                     рекомендуется {}.0 или новее.\n\n\
                     Продолжить всё равно?",
                    ver, MIN_WEBVIEW2_VERSION.0
                ));
                if msg_box(&title, &text, MB_YESNO | MB_ICONWARNING) != IDYES.0 {
                    std::process::exit(EXIT_WEBVIEW2_DECLINED);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parses() {
        assert_eq!(parse_version("128.0.2739.42"), Some((128, 0, 2739, 42)));
        assert_eq!(parse_version(" 120.0.0.0 "), Some((120, 0, 0, 0)));
        assert!(parse_version("abc").is_none());
        assert!(parse_version("120.0").is_none());
    }

    #[test]
    fn version_compares() {
        assert!((128, 0, 2739, 42) >= MIN_WEBVIEW2_VERSION);
        assert!((119, 9, 9, 9) < MIN_WEBVIEW2_VERSION);
    }
}

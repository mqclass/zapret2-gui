//! Web runtime pre-flight check (Linux).
//!
//! The Windows build checks for the WebView2 runtime because it is a
//! separate system component that can be missing. On Linux the web
//! engine (WebKitGTK) is a shared library the binary links against at
//! build time: if it were missing, this process would not be running at
//! all — the loader refuses to start it. There is therefore no runtime
//! to pre-flight and nothing to offer a download for; the check exists
//! (and passes trivially) so `main()` stays platform-neutral.

/// Returns the installed WebView2 Runtime version string.
///
/// Always `None` on Linux: there is no WebView2 and no separately
/// versioned web runtime to report. Kept for surface parity with the
/// Windows module.
pub fn installed_webview2_version() -> Option<String> {
    None
}

/// Pre-flight web runtime check. Returns immediately: WebKitGTK is
/// linked into the binary (see module docs), so reaching this line is
/// itself the proof the runtime is present.
pub fn ensure_webview2() {}

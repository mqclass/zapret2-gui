//! Platform abstraction layer.
//!
//! All OS-specific code lives behind this facade so the engine, the Tauri
//! commands and the svc stay platform-neutral and refer to
//! `crate::platform::...` only. Each OS gets a submodule with the same
//! surface; the facade re-exports the active one.
//!
//! Today: Windows (`platform/windows`, formerly `win32/`).
//! In progress: Linux (`platform/linux`) — zapret2 ships a native Linux
//! engine (`nfqws2`), so the port adds an implementation, not a rewrite.

pub mod windivert_common;

#[cfg(windows)]
pub mod windows;

#[cfg(windows)]
pub use windows::*;

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_os = "linux")]
pub use linux::*;

/// Default game-filter port range: the whole high-port space. This is the
/// default of Flowseal's `GameFilterTCPRange` / `GameFilterUDPRange` and of
/// the program's own `game_filter_tcp` / `game_filter_udp` settings.
pub const DEFAULT_GAME_FILTER_RANGE: &str = "1024-65535";

/// Validates a user-supplied port specification: a comma-separated list of
/// ports (`N`) and ranges (`N-M`), each number within 0..=65535 and each
/// range non-inverted (`N <= M`). Anything else — empty, too long, bad
/// characters, an out-of-range or inverted element — falls back to the
/// default range, so an invalid value from user-writable settings can never
/// reach the engine's port lists (its parser rejects the same inputs).
///
/// Single source of truth: the supervisor's `sanitize_port_spec` delegates
/// here, and the custom-profile parsers use it through
/// `expand_game_filter_vars`.
pub fn sanitize_port_spec(raw: &str) -> String {
    fn valid_port(s: &str) -> Option<u32> {
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        s.parse::<u32>().ok().filter(|&n| n <= 65535)
    }
    let ok = !raw.is_empty()
        && raw.len() <= 64
        && raw.split(',').all(|elem| match elem.split_once('-') {
            Some((from, to)) => match (valid_port(from), valid_port(to)) {
                (Some(f), Some(t)) => f <= t,
                _ => false,
            },
            None => valid_port(elem).is_some(),
        });
    if ok {
        raw.to_string()
    } else {
        DEFAULT_GAME_FILTER_RANGE.to_string()
    }
}

/// Expands Flowseal's game-filter variables in a piece of engine argument
/// text: `%GameFilterTCP%` / `%GameFilterUDP%` become the (sanitized)
/// game-filter port ranges. Flowseal .bat files define these variables in
/// service.bat, so a pasted command line carries them literally (issue #6);
/// expanding them here lets imported strategies launch. Text without the
/// variables is returned unchanged.
pub fn expand_game_filter_vars(text: &str, game_filter_tcp: &str, game_filter_udp: &str) -> String {
    if !text.contains("%GameFilter") {
        return text.to_string();
    }
    let tcp = sanitize_port_spec(game_filter_tcp);
    let udp = sanitize_port_spec(game_filter_udp);
    text.replace("%GameFilterTCP%", &tcp)
        .replace("%GameFilterUDP%", &udp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_port_spec_accepts_valid_ranges() {
        assert_eq!(sanitize_port_spec("1024-65535"), "1024-65535");
        assert_eq!(
            sanitize_port_spec("80,443,27015-27030"),
            "80,443,27015-27030"
        );
        assert_eq!(sanitize_port_spec("12"), "12");
    }

    #[test]
    fn test_sanitize_port_spec_falls_back_on_invalid() {
        assert_eq!(sanitize_port_spec(""), "1024-65535");
        assert_eq!(sanitize_port_spec("12;rm -rf"), "1024-65535");
        assert_eq!(sanitize_port_spec("80 443"), "1024-65535");
        assert_eq!(sanitize_port_spec(&"1".repeat(65)), "1024-65535");
        // Character-valid but semantically rejected by the engine's port
        // parser: out of range, inverted, or empty elements.
        assert_eq!(sanitize_port_spec("65536"), "1024-65535");
        assert_eq!(sanitize_port_spec("80-79"), "1024-65535");
        assert_eq!(sanitize_port_spec("80,"), "1024-65535");
        assert_eq!(sanitize_port_spec(","), "1024-65535");
        assert_eq!(sanitize_port_spec("-"), "1024-65535");
        assert_eq!(sanitize_port_spec("80,,443"), "1024-65535");
        assert_eq!(sanitize_port_spec("99999999999999999999"), "1024-65535");
    }

    #[test]
    fn test_expand_game_filter_vars() {
        assert_eq!(
            expand_game_filter_vars(
                "--wf-tcp=80,443,%GameFilterTCP% --filter-udp=%GameFilterUDP%",
                "27015-27030",
                "27000-27015"
            ),
            "--wf-tcp=80,443,27015-27030 --filter-udp=27000-27015"
        );
        assert_eq!(
            expand_game_filter_vars("--filter-tcp=443", "27015-27030", "27000-27015"),
            "--filter-tcp=443"
        );
    }

    #[test]
    fn test_expand_game_filter_vars_sanitizes() {
        assert_eq!(
            expand_game_filter_vars("--filter-tcp=%GameFilterTCP%", "abc", ""),
            "--filter-tcp=1024-65535"
        );
    }
}

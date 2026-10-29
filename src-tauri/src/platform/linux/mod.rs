#![allow(unused_imports)]
pub mod autostart;
pub mod app_split;
pub mod conflicts;
pub mod custom_profiles;
pub mod deelevate;
pub mod dns;
pub mod doctor;
pub mod job_object;
pub mod lists;
pub mod nftables;
pub mod privileges;
pub mod process;
pub mod settings;
pub mod smart_hosts;
pub mod updater;
pub mod webview_check;

pub use autostart::{is_autostart_enabled, set_autostart};
pub use conflicts::{resolve_all_conflicts, resolve_all_conflicts_excluding, scan_conflicts, scan_conflicts_excluding, ConflictItem};
pub use custom_profiles::{delete_custom_profile, load_custom_profiles, parse_command_line_args, save_custom_profile};
pub use job_object::{JobObject, JobObjectError};
pub use privileges::{ensure_elevated, is_elevated, relaunch_elevated, PrivilegeError};
pub use process::{
    cleanup_windivert_driver, discover_strategies, kill_stray_winws2_processes, resolve_binaries,
    spawn_winws2, stop_winws2_gracefully, LuaStrategy, ProcessError, SupervisedProcess,
    ZapretBinaries,
};

/// Returns the absolute path of a system binary (L-3 hardening, Linux form).
///
/// System tools are resolved from the fixed system directories (`/usr/bin`,
/// then `/bin`) instead of PATH: a writable directory earlier in PATH could
/// otherwise shadow e.g. `resolvectl` with an attacker-controlled executable
/// running with our privileges (the svc runs as root). If the binary is not
/// present in either directory, `/usr/bin/<binary>` is returned anyway so the
/// spawn fails visibly with NotFound instead of silently falling back to PATH.
pub fn system32(binary: &str) -> std::path::PathBuf {
    let usr = std::path::Path::new("/usr/bin").join(binary);
    if usr.is_file() {
        return usr;
    }
    let bin = std::path::Path::new("/bin").join(binary);
    if bin.is_file() {
        return bin;
    }
    usr
}

#![allow(unused_imports)]
pub mod autostart;
pub mod conflicts;
pub mod custom_profiles;
pub mod dns;
pub mod doctor;
pub mod job_object;
pub mod lists;
pub mod privileges;
pub mod process;
pub mod settings;

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

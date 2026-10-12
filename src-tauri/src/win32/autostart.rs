//! Windows Autostart Manager via Task Scheduler and Registry
//!
//! Provides seamless, UAC-safe autostart on Windows user logon.
//! Uses Windows Task Scheduler (`schtasks.exe`) with `RL HIGHEST` so that
//! the supervisor starts with Administrator rights without prompting UAC every time.

use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

const CREATE_NO_WINDOW: u32 = 0x08000000;
const TASK_NAME: &str = "Zapret2GUI";

/// Determines the canonical production executable path for autostart.
fn resolve_autostart_exe_path() -> Result<PathBuf, String> {
    let current_exe = std::env::current_exe().map_err(|e| e.to_string())?;

    // If running from src-tauri/target/..., attempt to locate root zapret2.exe or zapret2-gui.exe
    if let Some(parent) = current_exe.parent() {
        // Look in parent dirs for production zapret2.exe or zapret2-gui.exe
        let mut curr = parent.to_path_buf();
        for _ in 0..4 {
            let z_exe = curr.join("zapret2.exe");
            if z_exe.is_file() {
                return Ok(z_exe);
            }
            let z_gui = curr.join("zapret2-gui.exe");
            if z_gui.is_file() {
                return Ok(z_gui);
            }
            if let Some(p) = curr.parent() {
                curr = p.to_path_buf();
            } else {
                break;
            }
        }
    }

    Ok(current_exe)
}

/// Checks whether autostart is currently enabled via Task Scheduler.
pub fn is_autostart_enabled() -> bool {
    let output = Command::new("schtasks.exe")
        .args(["/query", "/tn", TASK_NAME])
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    match output {
        Ok(out) => out.status.success(),
        Err(_) => false,
    }
}

/// Enables or disables autostart with Windows.
pub fn set_autostart(enable: bool) -> Result<bool, String> {
    if enable {
        let exe_path = resolve_autostart_exe_path()?;
        let exe_str = exe_path.to_string_lossy().to_string();
        let task_run_arg = format!("\"{}\" --minimized", exe_str);

        let output = Command::new("schtasks.exe")
            .args([
                "/create",
                "/tn",
                TASK_NAME,
                "/tr",
                &task_run_arg,
                "/sc",
                "onlogon",
                "/rl",
                "highest",
                "/f",
                "/np",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("Failed to invoke schtasks.exe: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Failed to configure autostart task: {}", stderr.trim()));
        }

        Ok(true)
    } else {
        let output = Command::new("schtasks.exe")
            .args(["/delete", "/tn", TASK_NAME, "/f"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("Failed to invoke schtasks.exe: {}", e))?;

        // Exit status 0 or task not found is acceptable for disable
        if output.status.success() || !is_autostart_enabled() {
            Ok(false)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("Failed to delete autostart task: {}", stderr.trim()))
        }
    }
}

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
///
/// SECURITY (M-3): this is exactly `current_exe()` — no search upward through
/// parent directories. The old "look up to 4 parents for zapret2.exe" logic
/// could have registered a planted executable in a writable parent dir as a
/// HighestAvailable logon task, i.e. persistent admin-level code execution.
fn resolve_autostart_exe_path() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|e| e.to_string())
}

/// Escapes a string for inclusion in the Task Scheduler XML (M-4).
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Checks whether autostart is currently enabled via Task Scheduler.
pub fn is_autostart_enabled() -> bool {
    let output = Command::new(crate::platform::system32("schtasks.exe"))
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
    // 1. Terminate running task instance if any
    let _ = Command::new(crate::platform::system32("schtasks.exe"))
        .args(["/end", "/tn", TASK_NAME])
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    if enable {
        let exe_path = resolve_autostart_exe_path()?;
        let work_dir = exe_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));

        // Generate robust Task Scheduler XML with InteractiveToken so the app starts in
        // the user's interactive desktop session (with tray icon and window), NOT in Session 0!
        // SECURITY (M-4): exe/workdir are XML-escaped and the command is quoted so
        // install paths containing &, <, > etc. cannot break or inject into the task XML.
        let xml_content = format!(
            r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Author>zapret2-supervisor</Author>
    <Description>zapret2 Windows High-Performance Bypass Supervisor Autostart</Description>
    <URI>\{}</URI>
  </RegistrationInfo>
  <Principals>
    <Principal id="Author">
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <Delay>PT2S</Delay>
    </LogonTrigger>
  </Triggers>
  <Actions Context="Author">
    <Exec>
      <Command>"{}"</Command>
      <Arguments>--autostart --minimized</Arguments>
      <WorkingDirectory>{}</WorkingDirectory>
    </Exec>
  </Actions>
</Task>"#,
            TASK_NAME,
            xml_escape(&exe_path.to_string_lossy()),
            xml_escape(&work_dir.to_string_lossy())
        );

        let temp_xml = std::env::temp_dir().join("zapret2_autostart_task.xml");

        // Write as UTF-16LE with Byte Order Mark (BOM) [0xFF, 0xFE] as required by schtasks
        let mut bytes = vec![0xFF, 0xFE];
        for u in xml_content.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }

        std::fs::write(&temp_xml, &bytes)
            .map_err(|e| format!("Failed to create temporary task XML: {}", e))?;

        let output = Command::new(crate::platform::system32("schtasks.exe"))
            .args([
                "/create",
                "/tn",
                TASK_NAME,
                "/xml",
                &temp_xml.to_string_lossy(),
                "/f",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output();

        let _ = std::fs::remove_file(&temp_xml);

        match output {
            Ok(out) if out.status.success() => Ok(true),
            Ok(out) => {
                // Fallback to interactive CLI command with /IT flag (never /NP)
                let exe_str = exe_path.to_string_lossy().to_string();
                let task_run_arg = format!("\"{}\" --autostart --minimized", exe_str);
                let fallback = Command::new(crate::platform::system32("schtasks.exe"))
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
                        "/it",
                        "/f",
                    ])
                    .creation_flags(CREATE_NO_WINDOW)
                    .output()
                    .map_err(|e| format!("Failed to invoke schtasks.exe fallback: {}", e))?;

                if fallback.status.success() {
                    Ok(true)
                } else {
                    let err = String::from_utf8_lossy(&out.stderr);
                    Err(format!("Failed to register scheduled task: {}", err.trim()))
                }
            }
            Err(e) => Err(format!("Failed to execute schtasks.exe: {}", e)),
        }
    } else {
        let output = Command::new(crate::platform::system32("schtasks.exe"))
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

// The svc binary compiles the whole platform tree but never calls this
// GUI-facing module; without this the svc build warns about every item.
#![allow(dead_code)]

//! Shared WinDivert driver health logic (platform-neutral).
//!
//! The OS-specific parts live in `platform/windows/windivert.rs` (real
//! Service Control Manager queries) and `platform/linux/windivert.rs`
//! (not-applicable stub). This module holds the wire types, the driver-path
//! comparison and the diagnosis decision table — all pure, so they are
//! unit-tested on every platform, including Linux CI.
//!
//! Why this exists: the WinDivert kernel-driver service is registered once
//! and keeps pointing at the folder the app ran from. If the user unpacks a
//! new release into a new folder and deletes the old one, the service points
//! at a deleted file and every engine start dies with
//! "windivert: error opening filter" plus exit code 177. A DISABLED service
//! does the same. Both are invisible to the user — the bypass just silently
//! does nothing. The diagnosis below tells the three causes apart
//! (disabled service, stale path, missing driver file) so the UI can offer
//! the right fix instead of a cryptic log line.

use serde::{Deserialize, Serialize};

/// Wire status of the WinDivert driver registration.
///
/// `state` is one of:
/// - `"ok"` — the service exists, is not disabled, and its binary exists.
/// - `"not_installed"` — no service registered; winws2 creates it on the
///   next engine start.
/// - `"service_disabled"` — the service start type is DISABLED.
/// - `"stale_path"` — the registered binary path no longer exists.
/// - `"driver_file_missing"` — `WinDivert64.sys` is absent from this
///   install (most likely quarantined by an antivirus).
/// - `"check_failed"` — the check itself could not run; see `message`.
/// - `"not_applicable"` — non-Windows platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindivertDriverStatus {
    pub state: String,
    /// Human start-type name: "disabled" | "demand" | "auto" | "system" |
    /// "boot" | "unknown". `None` when the service does not exist.
    pub service_start_type: Option<String>,
    /// Registered binary path (NT `\??\` prefix stripped), if the service exists.
    pub service_binary_path: Option<String>,
    /// Where this install expects `WinDivert64.sys`.
    pub expected_path: Option<String>,
    /// Human-readable Russian one-liner for the state.
    pub message: String,
}

impl WindivertDriverStatus {
    pub fn check_failed(message: impl Into<String>) -> Self {
        WindivertDriverStatus {
            state: "check_failed".into(),
            service_start_type: None,
            service_binary_path: None,
            expected_path: None,
            message: message.into(),
        }
    }
}

/// Wire result of a driver repair attempt.
///
/// `outcome` is one of `"repaired"`, `"reboot_required"`,
/// `"nothing_to_repair"`, `"failed"`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindivertRepairResult {
    pub outcome: String,
    /// Human-readable Russian one-liner for the outcome.
    pub message: String,
}

impl WindivertRepairResult {
    pub fn repaired(message: impl Into<String>) -> Self {
        WindivertRepairResult {
            outcome: "repaired".into(),
            message: message.into(),
        }
    }

    pub fn reboot_required(message: impl Into<String>) -> Self {
        WindivertRepairResult {
            outcome: "reboot_required".into(),
            message: message.into(),
        }
    }

    pub fn nothing_to_repair(message: impl Into<String>) -> Self {
        WindivertRepairResult {
            outcome: "nothing_to_repair".into(),
            message: message.into(),
        }
    }

    pub fn failed(message: impl Into<String>) -> Self {
        WindivertRepairResult {
            outcome: "failed".into(),
            message: message.into(),
        }
    }
}

/// Win32 SERVICE_* start-type constants, kept numeric so this module stays
/// testable without the `windows` crate.
pub const SERVICE_BOOT_START: u32 = 0;
pub const SERVICE_SYSTEM_START: u32 = 1;
pub const SERVICE_AUTO_START: u32 = 2;
pub const SERVICE_DEMAND_START: u32 = 3;
pub const SERVICE_DISABLED: u32 = 4;

/// Human name for a Win32 service start type.
pub fn start_type_name(start_type: u32) -> &'static str {
    match start_type {
        SERVICE_BOOT_START => "boot",
        SERVICE_SYSTEM_START => "system",
        SERVICE_AUTO_START => "auto",
        SERVICE_DEMAND_START => "demand",
        SERVICE_DISABLED => "disabled",
        _ => "unknown",
    }
}

/// Normalize a driver binary path for comparison: strip NT/device prefixes
/// (`\??\`, `\\?\`), strip surrounding quotes, `/` -> `\`, lowercase.
/// Windows paths are case-insensitive, and the service stores the path in
/// whatever case the installer used.
pub fn normalize_driver_path(path: &str) -> String {
    let mut p = path.trim();
    // Quotes first: a quoted NT path starts with the quote, not the prefix.
    p = p.trim_matches('"');
    if let Some(rest) = p.strip_prefix(r"\??\") {
        p = rest;
    } else if let Some(rest) = p.strip_prefix(r"\\?\") {
        p = rest;
    }
    p.replace('/', r"\")
        .trim_end_matches('\\')
        .to_lowercase()
}

/// Case- and prefix-insensitive driver path comparison.
pub fn driver_paths_equal(a: &str, b: &str) -> bool {
    normalize_driver_path(a) == normalize_driver_path(b)
}

/// Driver-mode names. The hidden driver (Monkey) is WinDivert 2.2.2
/// under a different service/file name: the DLL is built from the
/// WinDivert sources plus our rename patch
/// (scripts/windivert-monkey.patch), the .sys is the unmodified stock
/// driver; the device name is unchanged, so this only fools checks
/// that match the service or file name.
///
/// SCM service name for the mode: "Monkey" vs "WinDivert".
pub fn driver_service_name(monkey_mode: bool) -> &'static str {
    if monkey_mode {
        "Monkey"
    } else {
        "WinDivert"
    }
}

/// Kernel-driver file name for the mode: "Monkey64.sys" vs
/// "WinDivert64.sys".
pub fn driver_sys_file_name(monkey_mode: bool) -> &'static str {
    if monkey_mode {
        "Monkey64.sys"
    } else {
        "WinDivert64.sys"
    }
}

/// Facts the diagnosis needs. Pure inputs — no IO happens here.
#[derive(Debug, Clone, Default)]
pub struct DriverFacts {
    pub service_exists: bool,
    pub service_start_type: Option<u32>,
    pub service_binary_path: Option<String>,
    /// Whether the file at the registered binary path exists.
    pub service_path_exists: bool,
    /// Where this install expects `WinDivert64.sys`. `None` when the engine
    /// directory could not be located at all.
    pub expected_path: Option<String>,
    /// Whether `WinDivert64.sys` exists at the expected path.
    pub expected_file_exists: bool,
    /// Driver name for user-facing messages: "WinDivert" or "Monkey".
    /// Defaults to "" (unknown) — callers always set it.
    pub driver_label: &'static str,
}

/// The diagnosis decision table.
///
/// Order matters: a missing driver file kills the engine no matter what the
/// service says, so it is reported first; a DISABLED service kills it no
/// matter where the path points, so it comes before the path checks.
pub fn diagnose_driver(facts: &DriverFacts) -> WindivertDriverStatus {
    // Fallback label: every production caller sets driver_label, but a
    // future forgetful one must not produce messages like
    // "Служба драйвера  отключена" (empty label).
    let label = if facts.driver_label.is_empty() {
        "WinDivert"
    } else {
        facts.driver_label
    };
    let expected_path = facts.expected_path.clone();
    let service_binary_path = facts
        .service_binary_path
        .as_deref()
        .map(|p| normalize_driver_path(p));
    // Display form keeps the original case; only the comparison lowercases.
    // (Quotes before the prefix, same as normalize_driver_path.)
    let display_service_path = facts.service_binary_path.as_deref().map(|p| {
        let p = p.trim().trim_matches('"');
        let p = p
            .strip_prefix(r"\??\")
            .or_else(|| p.strip_prefix(r"\\?\"))
            .unwrap_or(p);
        p.to_string()
    });
    let start_type_name = facts
        .service_start_type
        .map(start_type_name)
        .map(str::to_string);

    let status = |state: &str, message: String| WindivertDriverStatus {
        state: state.into(),
        service_start_type: start_type_name.clone(),
        service_binary_path: display_service_path.clone(),
        expected_path: expected_path.clone(),
        message,
    };

    let Some(expected) = facts.expected_path.as_deref() else {
        return status(
            "check_failed",
            "Не найден winws2.exe в папке приложения — проверьте установку.".into(),
        );
    };
    if !facts.expected_file_exists {
        return status(
            "driver_file_missing",
            format!(
                "Файл драйвера отсутствует в папке приложения ({expected}). \
                 Скорее всего, его унёс в карантин антивирус — восстановите файл и добавьте папку в исключения."
            ),
        );
    }
    if !facts.service_exists {
        return status(
            "not_installed",
            format!(
                "Служба {} не зарегистрирована. Она создастся сама при первом запуске обхода.",
                label
            ),
        );
    }
    if facts.service_start_type == Some(SERVICE_DISABLED) {
        return status(
            "service_disabled",
            format!(
                "Служба драйвера {} отключена, поэтому движок не может запуститься.",
                label
            ),
        );
    }
    if !facts.service_path_exists {
        // An empty registered path (null lpBinaryPathName) prints as "?"
        // rather than "()" — same as a missing one.
        let path = display_service_path
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("?");
        return status(
            "stale_path",
            format!(
                "Служба драйвера {} указывает на папку, которой больше нет ({path}). \
                 Такое бывает после распаковки новой версии в новую папку.",
                label
            ),
        );
    }
    // The driver loads: either the registered path is this install, or a
    // valid driver from another folder. The latter works today but breaks
    // the moment that folder is deleted, so say so in the message.
    let foreign = match (&service_binary_path, &expected_path) {
        (Some(registered), Some(expected)) => !driver_paths_equal(registered, expected),
        _ => false,
    };
    if foreign {
        let path = display_service_path
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("?");
        status(
            "ok",
            format!(
                "Драйвер {} работает, но зарегистрирован из другой папки ({path}). \
                 Если её удалить, обход сломается.",
                label
            ),
        )
    } else {
        status(
            "ok",
            format!("Драйвер {} в порядке.", label),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_facts() -> DriverFacts {
        DriverFacts {
            service_exists: true,
            service_start_type: Some(SERVICE_DEMAND_START),
            service_binary_path: Some(r"\??\C:\zapret2\WinDivert64.sys".into()),
            service_path_exists: true,
            expected_path: Some(r"C:\zapret2\WinDivert64.sys".into()),
            expected_file_exists: true,
            driver_label: "WinDivert",
        }
    }

    #[test]
    fn driver_names_for_both_modes() {
        assert_eq!(driver_service_name(false), "WinDivert");
        assert_eq!(driver_sys_file_name(false), "WinDivert64.sys");
        assert_eq!(driver_service_name(true), "Monkey");
        assert_eq!(driver_sys_file_name(true), "Monkey64.sys");
    }

    #[test]
    fn diagnose_monkey_label_flows_into_messages() {
        // The hidden driver is diagnosed under its own name, not
        // "WinDivert" — otherwise the repair banner would name a
        // service the user never enabled.
        let mut facts = base_facts();
        facts.driver_label = "Monkey";
        facts.service_start_type = Some(SERVICE_DISABLED);
        let status = diagnose_driver(&facts);
        assert_eq!(status.state, "service_disabled");
        assert!(status.message.contains("Monkey"));
        assert!(!status.message.contains("WinDivert"));
    }

    #[test]
    fn normalize_strips_nt_prefix_quotes_slashes_and_case() {
        assert_eq!(
            normalize_driver_path(r#""\??\C:\Foo/WinDivert64.sys""#),
            r"c:\foo\windivert64.sys"
        );
        assert_eq!(
            normalize_driver_path(r"\\?\D:\Bar\WinDivert64.sys"),
            r"d:\bar\windivert64.sys"
        );
        assert_eq!(
            normalize_driver_path(r"C:\Baz\WinDivert64.sys\"),
            r"c:\baz\windivert64.sys"
        );
    }

    #[test]
    fn paths_equal_ignores_prefix_case_and_slashes() {
        assert!(driver_paths_equal(
            r"\??\C:\Zapret2\WinDivert64.sys",
            r"c:/zapret2/windivert64.sys"
        ));
        assert!(!driver_paths_equal(
            r"C:\zapret2\WinDivert64.sys",
            r"C:\other\WinDivert64.sys"
        ));
    }

    #[test]
    fn start_type_names_cover_all_values() {
        assert_eq!(start_type_name(0), "boot");
        assert_eq!(start_type_name(1), "system");
        assert_eq!(start_type_name(2), "auto");
        assert_eq!(start_type_name(3), "demand");
        assert_eq!(start_type_name(4), "disabled");
        assert_eq!(start_type_name(99), "unknown");
    }

    #[test]
    fn diagnose_ok_when_service_matches_expected_path() {
        let status = diagnose_driver(&base_facts());
        assert_eq!(status.state, "ok");
        assert_eq!(status.service_start_type.as_deref(), Some("demand"));
        // Display path keeps its case but loses the NT prefix.
        assert_eq!(
            status.service_binary_path.as_deref(),
            Some(r"C:\zapret2\WinDivert64.sys")
        );
    }

    #[test]
    fn diagnose_ok_but_notes_foreign_folder() {
        let mut facts = base_facts();
        facts.service_binary_path = Some(r"C:\Desktop\123\bin\WinDivert64.sys".into());
        let status = diagnose_driver(&facts);
        assert_eq!(status.state, "ok");
        assert!(status.message.contains("другой папки"));
        assert!(status.message.contains(r"C:\Desktop\123\bin\WinDivert64.sys"));
    }

    #[test]
    fn diagnose_not_installed_when_service_absent() {
        let mut facts = base_facts();
        facts.service_exists = false;
        facts.service_start_type = None;
        facts.service_binary_path = None;
        facts.service_path_exists = false;
        let status = diagnose_driver(&facts);
        assert_eq!(status.state, "not_installed");
    }

    #[test]
    fn diagnose_disabled_beats_stale_path() {
        // A disabled service kills the engine even if the path is fine;
        // the disabled verdict must win over the path check.
        let mut facts = base_facts();
        facts.service_start_type = Some(SERVICE_DISABLED);
        facts.service_path_exists = false;
        let status = diagnose_driver(&facts);
        assert_eq!(status.state, "service_disabled");
    }

    #[test]
    fn diagnose_stale_path_when_registered_file_is_gone() {
        let mut facts = base_facts();
        facts.service_path_exists = false;
        let status = diagnose_driver(&facts);
        assert_eq!(status.state, "stale_path");
        assert!(status.message.contains(r"C:\zapret2\WinDivert64.sys"));
    }

    #[test]
    fn diagnose_missing_driver_file_beats_everything() {
        // Even a healthy-looking service cannot save an install whose
        // own WinDivert64.sys is gone (antivirus quarantine) — and a
        // missing file outranks every service-side diagnosis too: there
        // is no point repairing a registration for a driver that is gone.
        let mut facts = base_facts();
        facts.expected_file_exists = false;
        let status = diagnose_driver(&facts);
        assert_eq!(status.state, "driver_file_missing");
        assert!(status.message.contains("карантин"));

        // Same, with the service itself disabled AND pointing at a dead
        // path: still the missing file, not service_disabled/stale_path.
        facts.service_start_type = Some(SERVICE_DISABLED);
        facts.service_path_exists = false;
        let status = diagnose_driver(&facts);
        assert_eq!(status.state, "driver_file_missing");
    }

    #[test]
    fn diagnose_check_failed_when_engine_dir_unknown() {
        let mut facts = base_facts();
        facts.expected_path = None;
        facts.expected_file_exists = false;
        let status = diagnose_driver(&facts);
        assert_eq!(status.state, "check_failed");
    }
}

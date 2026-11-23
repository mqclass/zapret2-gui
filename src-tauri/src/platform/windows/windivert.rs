// The svc binary compiles the whole platform tree but never calls this
// GUI-facing module; without this the svc build warns about every item.
#![allow(dead_code)]

//! WinDivert driver health check and repair (Windows).
//!
//! The diagnosis decision table lives in `crate::platform::windivert_common`
//! (pure, unit-tested); this module only gathers the facts from the Service
//! Control Manager and performs the repair.
//!
//! Everything goes through the native SCM API. Never parse `sc.exe` output
//! for this: it is localized (Russian on the user's machine), so string
//! matching on it is fragile by construction.
//!
//! Repair strategy: delete the stale registration and let winws2 re-create
//! it at the current location on the next engine start. winws2 owns the
//! registration details (exact `\??\` path form, start type), so deleting is
//! safer than rewriting the service config by hand. Deleting a *running*
//! kernel-driver service only marks it for deletion until reboot, so the
//! repair stops the service first when it is running.

use std::path::PathBuf;
use std::time::Duration;
use windows::core::PCWSTR;
use windows::Win32::Foundation::GetLastError;
use windows::Win32::System::Services::{
    CloseServiceHandle, DeleteService, OpenSCManagerW, OpenServiceW, QueryServiceConfigW,
    QUERY_SERVICE_CONFIGW, SC_MANAGER_CONNECT, SERVICE_QUERY_CONFIG,
};

use super::conflicts::{is_service_running, stop_service_native};
use super::process::resolve_binaries_with_mode;
use crate::platform::windivert_common::{
    diagnose_driver, driver_service_name, driver_sys_file_name, DriverFacts,
    WindivertDriverStatus, WindivertRepairResult,
};

/// Win32 error codes (numeric literals: the `windows` crate exposes them, but
/// plain constants keep this module independent of feature-flag drift).
const ERROR_SERVICE_DOES_NOT_EXIST: u32 = 1060;
const ERROR_SERVICE_MARKED_FOR_DELETE: u32 = 1072;

/// DELETE generic access right (0x00010000): there is no `SERVICE_DELETE`
/// constant in the `windows` crate — delete access on a service object is
/// the generic DELETE right.
const SERVICE_DELETE_ACCESS: u32 = 0x0001_0000;

/// Where this install expects the kernel-driver file (`WinDivert64.sys`
/// in stock mode, `Monkey64.sys` in hidden-driver mode).
///
/// Happy path: the full binary resolution already found everything.
/// Otherwise the .sys (or a sibling) is missing — find the engine directory
/// by `winws2.exe` alone, mirroring the production candidates of
/// `resolve_binaries` for the mode. `None` when the engine is nowhere
/// to be found.
fn expected_driver_path(monkey_mode: bool) -> Option<PathBuf> {
    if let Ok(binaries) = resolve_binaries_with_mode(monkey_mode) {
        return Some(binaries.windivert_sys);
    }
    let sys_file = driver_sys_file_name(monkey_mode);
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))?;
    let dirs: Vec<PathBuf> = if monkey_mode {
        vec![exe_dir.join("bin").join("monkey")]
    } else {
        vec![
            exe_dir.join("bin"),
            exe_dir.join("bin").join("windows-x86_64"),
            exe_dir,
        ]
    };
    for dir in dirs {
        if dir.join("winws2.exe").is_file() {
            return Some(dir.join(sys_file));
        }
    }
    None
}

/// Reads a NUL-terminated UTF-16 string out of the service-config buffer.
/// Bounded by `max_units` — the caller passes the remaining buffer *from
/// the string pointer*, not the whole buffer size: `lpBinaryPathName`
/// points well into the buffer, so a bound measured from the buffer start
/// would overrun it on a malformed (non-terminated) string.
fn read_config_string(ptr: *const u16, max_units: usize) -> String {
    if ptr.is_null() || max_units == 0 {
        return String::new();
    }
    unsafe {
        let mut len = 0usize;
        while len < max_units && *ptr.add(len) != 0 {
            len += 1;
        }
        let slice = std::slice::from_raw_parts(ptr, len);
        String::from_utf16_lossy(slice)
    }
}

/// Queries the driver service config (`service_name` is "WinDivert" or
/// "Monkey" — see `driver_service_name`).
///
/// - `Ok(Some((start_type, binary_path)))` — the service exists.
/// - `Ok(None)` — no such service.
/// - `Err(win32 code)` — the query itself failed.
fn query_service_config(service_name: &str) -> Result<Option<(u32, String)>, u32> {
    let svc_name: Vec<u16> = service_name
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let scm = match OpenSCManagerW(None, None, SC_MANAGER_CONNECT) {
            Ok(handle) => handle,
            Err(_) => return Err(GetLastError().0),
        };
        let service = match OpenServiceW(scm, PCWSTR(svc_name.as_ptr()), SERVICE_QUERY_CONFIG) {
            Ok(handle) => handle,
            Err(_) => {
                // Read the error code before any other Win32 call resets it.
                let code = GetLastError().0;
                let _ = CloseServiceHandle(scm);
                if code == ERROR_SERVICE_DOES_NOT_EXIST {
                    return Ok(None);
                }
                return Err(code);
            }
        };
        // Two-call pattern: the first call yields the required buffer size.
        let mut bytes_needed = 0u32;
        let _ = QueryServiceConfigW(service, None, 0, &mut bytes_needed);
        let result = if bytes_needed == 0 {
            Err(GetLastError().0)
        } else {
            // QUERY_SERVICE_CONFIGW needs 8-byte alignment; a Vec<u8> only
            // guarantees 1, so allocate u64 words (8-aligned by construction).
            let mut buf = vec![0u64; (bytes_needed as usize).div_ceil(8)];
            let config = buf.as_mut_ptr() as *mut QUERY_SERVICE_CONFIGW;
            match QueryServiceConfigW(service, Some(config), bytes_needed, &mut bytes_needed) {
                Ok(()) => {
                    // dwStartType is the SERVICE_START_TYPE newtype; unwrap to u32.
                    let start_type = (*config).dwStartType.0;
                    // Bound the string read by the buffer remainder *from the
                    // string pointer*, not from the buffer start: the pointer
                    // sits at least sizeof(QUERY_SERVICE_CONFIGW) bytes into
                    // the buffer, and a non-terminated string must not read
                    // past the allocation.
                    let buf_start = buf.as_ptr() as usize;
                    let buf_len =
                        (bytes_needed as usize).min(buf.len() * std::mem::size_of::<u64>());
                    let str_ptr = (*config).lpBinaryPathName.0 as usize;
                    let max_units = if str_ptr >= buf_start && str_ptr < buf_start + buf_len {
                        (buf_start + buf_len - str_ptr) / 2
                    } else {
                        0
                    };
                    let path = read_config_string((*config).lpBinaryPathName.0, max_units);
                    Ok(Some((start_type, path)))
                }
                Err(_) => Err(GetLastError().0),
            }
        };
        let _ = CloseServiceHandle(service);
        let _ = CloseServiceHandle(scm);
        result
    }
}

/// Checks whether a WinDivert service registration currently exists.
///
/// - `Some(true)` — the registration exists. An error other than "does not
///   exist" also counts as existing: a service marked for deletion still
///   answers to its name.
/// - `Some(false)` — definitely gone (`ERROR_SERVICE_DOES_NOT_EXIST`).
/// - `None` — could not tell: the SCM itself was unreachable. Callers must
///   not read this as "gone".
fn service_exists(service_name: &str) -> Option<bool> {
    let svc_name: Vec<u16> = service_name
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let scm = match OpenSCManagerW(None, None, SC_MANAGER_CONNECT) {
            Ok(handle) => handle,
            Err(_) => return None,
        };
        let found = match OpenServiceW(scm, PCWSTR(svc_name.as_ptr()), SERVICE_QUERY_CONFIG) {
            Ok(handle) => {
                let _ = CloseServiceHandle(handle);
                true
            }
            Err(_) => GetLastError().0 != ERROR_SERVICE_DOES_NOT_EXIST,
        };
        let _ = CloseServiceHandle(scm);
        Some(found)
    }
}

/// Diagnoses the driver registration for the mode currently in settings.
pub fn check_windivert_driver() -> WindivertDriverStatus {
    let monkey_mode = crate::platform::settings::load_settings().monkey_driver;
    check_windivert_driver_with_mode(monkey_mode)
}

/// Mode-explicit form of [`check_windivert_driver`].
pub fn check_windivert_driver_with_mode(monkey_mode: bool) -> WindivertDriverStatus {
    let label = driver_service_name(monkey_mode);
    let expected_path = expected_driver_path(monkey_mode);
    let expected_file_exists = expected_path
        .as_ref()
        .map(|p| p.is_file())
        .unwrap_or(false);

    let (service_exists, service_start_type, service_binary_path, service_path_exists) =
        match query_service_config(label) {
            Ok(Some((start_type, binary_path))) => {
                // The registered path is an NT path (`\??\C:\...`); strip
                // the prefix before the existence check (quotes first, same
                // order as normalize_driver_path).
                let trimmed = binary_path.trim().trim_matches('"');
                let fs_path = trimmed
                    .strip_prefix(r"\??\")
                    .or_else(|| trimmed.strip_prefix(r"\\?\"))
                    .unwrap_or(trimmed);
                let exists = PathBuf::from(fs_path).is_file();
                (true, Some(start_type), Some(binary_path), exists)
            }
            Ok(None) => (false, None, None, false),
            Err(code) => {
                return WindivertDriverStatus::check_failed(format!(
                    "Не удалось прочитать службу {label} (код ошибки Windows {code})."
                ));
            }
        };

    diagnose_driver(&DriverFacts {
        service_exists,
        service_start_type,
        service_binary_path,
        service_path_exists,
        expected_path: expected_path.map(|p| p.to_string_lossy().to_string()),
        expected_file_exists,
        driver_label: label,
    })
}

/// Repairs a broken driver registration for the mode currently in
/// settings: deletes the stale service so winws2 re-creates it at the
/// current location on the next engine start.
///
/// Safety guards:
/// - A healthy or absent registration is left alone (nothing to repair).
/// - A missing driver file cannot be fixed by touching the service.
/// - The caller must ensure the engine is not running (the Tauri command
///   checks); deleting from under a live engine would blackhole traffic.
///   The repair still stops the service first when it is running, because a
///   running kernel driver marked for deletion lingers until reboot.
pub fn repair_windivert_driver() -> WindivertRepairResult {
    let monkey_mode = crate::platform::settings::load_settings().monkey_driver;
    repair_windivert_driver_with_mode(monkey_mode)
}

/// Mode-explicit form of [`repair_windivert_driver`].
pub fn repair_windivert_driver_with_mode(monkey_mode: bool) -> WindivertRepairResult {
    let label = driver_service_name(monkey_mode);
    let sys_file = driver_sys_file_name(monkey_mode);
    let status = check_windivert_driver_with_mode(monkey_mode);
    match status.state.as_str() {
        "ok" | "not_installed" => {
            return WindivertRepairResult::nothing_to_repair(
                "Драйвер в порядке, чинить нечего.",
            );
        }
        "driver_file_missing" => {
            return WindivertRepairResult::failed(format!(
                "Файл драйвера отсутствует — сначала восстановите {sys_file} \
                 (проверьте карантин антивируса)."
            ));
        }
        "check_failed" => {
            return WindivertRepairResult::failed(format!(
                "Не удалось проверить драйвер: {}",
                status.message
            ));
        }
        _ => {} // "service_disabled" | "stale_path": proceed to delete.
    }

    // Stop the driver first when loaded: deleting a running kernel-driver
    // service only marks it for deletion until reboot.
    if is_service_running(label) {
        stop_service_native(label);
        for _ in 0..20 {
            if !is_service_running(label) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    let svc_name: Vec<u16> = label
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let delete_err: Option<u32> = unsafe {
        let scm = match OpenSCManagerW(None, None, SC_MANAGER_CONNECT) {
            Ok(handle) => handle,
            Err(_) => {
                return WindivertRepairResult::failed(format!(
                    "Не удалось открыть диспетчер служб (код ошибки Windows {}).",
                    GetLastError().0
                ));
            }
        };
        let service = match OpenServiceW(scm, PCWSTR(svc_name.as_ptr()), SERVICE_DELETE_ACCESS) {
            Ok(handle) => handle,
            Err(_) => {
                let code = GetLastError().0;
                let _ = CloseServiceHandle(scm);
                if code == ERROR_SERVICE_DOES_NOT_EXIST {
                    return WindivertRepairResult::nothing_to_repair(
                        "Служба уже удалена — при следующем запуске обхода драйвер настроится заново.",
                    );
                }
                return WindivertRepairResult::failed(format!(
                    "Не удалось открыть службу {label} (код ошибки Windows {code})."
                ));
            }
        };
        let res = DeleteService(service);
        // Capture the error code before CloseServiceHandle (or anything
        // else) has a chance to reset the thread's last-error value.
        let err = if res.is_ok() {
            None
        } else {
            Some(GetLastError().0)
        };
        let _ = CloseServiceHandle(service);
        let _ = CloseServiceHandle(scm);
        err
    };

    match delete_err {
        None => {
            // Verify: the service must be gone, not merely marked for
            // deletion (a loaded driver lingers until reboot).
            match service_exists(label) {
                Some(true) => WindivertRepairResult::reboot_required(
                    "Служба помечена на удаление, но система держит её до перезагрузки. \
                     Перезагрузите компьютер — после этого обход заработает.",
                ),
                Some(false) => WindivertRepairResult::repaired(
                    "Кривая регистрация драйвера удалена. Включите обход — драйвер настроится заново сам.",
                ),
                // DeleteService succeeded but the result could not be
                // verified — a reboot resolves the doubt either way.
                None => WindivertRepairResult::reboot_required(
                    "Служба удалена, но проверить результат не удалось. \
                     Перезагрузите компьютер — после этого обход заработает.",
                ),
            }
        }
        Some(ERROR_SERVICE_MARKED_FOR_DELETE) => WindivertRepairResult::reboot_required(
            "Служба уже помечена на удаление — перезагрузите компьютер, \
             после этого обход заработает.",
        ),
        Some(code) => WindivertRepairResult::failed(format!(
            "Не удалось удалить службу {label} (код ошибки Windows {code})."
        )),
    }
}

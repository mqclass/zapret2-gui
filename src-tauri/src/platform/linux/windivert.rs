// The svc binary compiles the whole platform tree but never calls this
// GUI-facing module; without this the svc build warns about every item.
#![allow(dead_code)]

//! WinDivert driver health check (Linux): not applicable.
//!
//! WinDivert is a Windows kernel driver; the Linux engine uses nfqws2 over
//! netfilter, so there is no driver service to diagnose or repair. The stub
//! keeps the platform surface identical to Windows.

use crate::platform::windivert_common::{WindivertDriverStatus, WindivertRepairResult};

/// Always reports `not_applicable` on Linux.
pub fn check_windivert_driver() -> WindivertDriverStatus {
    WindivertDriverStatus {
        state: "not_applicable".into(),
        service_start_type: None,
        service_binary_path: None,
        expected_path: None,
        message: "Проверка драйвера WinDivert доступна только на Windows.".into(),
    }
}

/// Always fails on Linux — there is nothing to repair.
pub fn repair_windivert_driver() -> WindivertRepairResult {
    WindivertRepairResult::failed("Починка драйвера WinDivert доступна только на Windows.")
}

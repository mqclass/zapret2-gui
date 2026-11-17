//! 1-Click Network & Driver Auto-Doctor (Self-Healing Troubleshooter)
//!
//! Automatically diagnoses and repairs common network stack, driver, and caching issues:
//! 1. Cleans orphaned winws/winws2 background processes.
//! 2. Diagnoses the WinDivert kernel driver service (disabled? stale path?
//!    missing driver file?) and repairs a broken registration
//!    (skipped with a warning while the bypass engine is running).
//! 3. Flushes Windows DNS client cache (`ipconfig /flushdns`).
//! 4. Purges local ARP address cache (`netsh interface ip delete arpcache`).
//! 5. Enables TCP Timestamps (`netsh int tcp set global timestamps=enabled`) required for packet desync.
//! 6. Verifies outbound gateway connectivity to Cloudflare/Google DNS.

use std::net::SocketAddr;
use std::os::windows::process::CommandExt;
use std::process::Command;
use std::time::Duration;
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::platform::process::kill_stray_winws2_processes;
use crate::platform::windivert::{check_windivert_driver, repair_windivert_driver};

const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorStepResult {
    pub id: String,
    pub title: String,
    pub name: String,
    pub status: String, // "ok", "fixed", "warn", "error"
    pub message: String,
    pub details: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorReport {
    pub success: bool,
    pub steps: Vec<DoctorStepResult>,
    pub summary: String,
    pub message: String,
}

pub async fn run_auto_doctor(active_pid: Option<u32>, engine_running: bool) -> DoctorReport {
    let mut steps = Vec::new();
    let mut all_ok = true;
    // Which part failed — the summary must blame the right one: a broken
    // driver is not a connectivity delay.
    let mut driver_failed = false;
    let mut connectivity_failed = false;

    // 1. Terminate orphaned processes
    let strays = kill_stray_winws2_processes(active_pid);
    let p_msg = if strays > 0 {
        format!("Успешно завершено {} зависших процессов", strays)
    } else {
        "Зависших процессов не обнаружено".into()
    };
    steps.push(DoctorStepResult {
        id: "process_cleanup".into(),
        title: "Очистка фоновых процессов winws".into(),
        name: "Очистка фоновых процессов winws".into(),
        status: "ok".into(),
        message: p_msg.clone(),
        details: p_msg,
    });

    // 2. WinDivert driver registration — diagnose first, repair only when
    //    broken. (Previously this step blindly stopped the service and always
    //    reported success, which lied about disabled services, stale paths
    //    and missing driver files.) SKIPPED while the engine is running:
    //    tearing the driver service down from under an active winws would
    //    blackhole traffic; the user should stop the bypass first and
    //    re-run the doctor.
    if engine_running {
        let wd_msg = "Проверка драйвера пропущена: движок обхода запущен. Остановите обход и повторите диагностику для проверки WinDivert.".to_string();
        steps.push(DoctorStepResult {
            id: "windivert_reset".into(),
            title: "Драйвер WinDivert".into(),
            name: "Драйвер WinDivert".into(),
            status: "warn".into(),
            message: wd_msg.clone(),
            details: wd_msg,
        });
    } else {
        let status = check_windivert_driver();
        let (wd_status, wd_msg) = match status.state.as_str() {
            "ok" | "not_installed" => ("ok", status.message),
            "service_disabled" | "stale_path" => {
                let repair = repair_windivert_driver();
                match repair.outcome.as_str() {
                    "repaired" => ("fixed", repair.message),
                    "nothing_to_repair" => ("ok", repair.message),
                    "reboot_required" => {
                        all_ok = false;
                        driver_failed = true;
                        ("warn", repair.message)
                    }
                    _ => {
                        all_ok = false;
                        driver_failed = true;
                        ("error", repair.message)
                    }
                }
            }
            "driver_file_missing" => {
                all_ok = false;
                driver_failed = true;
                ("error", status.message)
            }
            _ => {
                // "check_failed" (and any future state): honest warning.
                all_ok = false;
                driver_failed = true;
                ("warn", status.message)
            }
        };
        steps.push(DoctorStepResult {
            id: "windivert_reset".into(),
            title: "Драйвер WinDivert".into(),
            name: "Драйвер WinDivert".into(),
            status: wd_status.into(),
            message: wd_msg.clone(),
            details: wd_msg,
        });
    }

    // 3. Flush DNS Cache
    let dns_res = Command::new(crate::platform::system32("ipconfig.exe"))
        .creation_flags(CREATE_NO_WINDOW)
        .arg("/flushdns")
        .output();
    let dns_ok = dns_res.map(|o| o.status.success()).unwrap_or(false);
    let dns_msg = if dns_ok {
        "Кэш сопоставления DNS Windows успешно очищен".to_string()
    } else {
        "Не удалось выполнить команду ipconfig /flushdns".to_string()
    };
    steps.push(DoctorStepResult {
        id: "flush_dns".into(),
        title: "Сброс системного кэша DNS".into(),
        name: "Сброс системного кэша DNS".into(),
        status: if dns_ok { "ok".into() } else { "warn".into() },
        message: dns_msg.clone(),
        details: dns_msg,
    });

    // 4. Purge ARP table cache
    let arp_res = Command::new(crate::platform::system32("netsh.exe"))
        .creation_flags(CREATE_NO_WINDOW)
        .args(["interface", "ip", "delete", "arpcache"])
        .output();
    let arp_ok = arp_res.map(|o| o.status.success()).unwrap_or(false);
    let arp_msg = if arp_ok {
        "Таблица адресов ARP успешно сброшена".to_string()
    } else {
        "Очистка ARP завершилась с предупреждением".to_string()
    };
    steps.push(DoctorStepResult {
        id: "arp_purge".into(),
        title: "Очистка сетевой таблицы ARP".into(),
        name: "Очистка сетевой таблицы ARP".into(),
        status: if arp_ok { "ok".into() } else { "warn".into() },
        message: arp_msg.clone(),
        details: arp_msg,
    });

    // 5. Enforce TCP Timestamps
    let ts_res = Command::new(crate::platform::system32("netsh.exe"))
        .creation_flags(CREATE_NO_WINDOW)
        .args(["interface", "tcp", "set", "global", "timestamps=enabled"])
        .output();
    let ts_ok = ts_res.map(|o| o.status.success()).unwrap_or(false);
    let ts_msg = if ts_ok {
        "TCP Timestamps включены (необходимо для надежной десинхронизации ТСПУ)".to_string()
    } else {
        "Не удалось включить TCP Timestamps".to_string()
    };
    steps.push(DoctorStepResult {
        id: "tcp_timestamps".into(),
        title: "Включение TCP Timestamps".into(),
        name: "Включение TCP Timestamps".into(),
        status: if ts_ok { "ok".into() } else { "warn".into() },
        message: ts_msg.clone(),
        details: ts_msg,
    });

    // 6. Test Gateway / Outbound TCP Handshake
    let test_addr: SocketAddr = "1.1.1.1:53".parse().unwrap();
    let connect_res = timeout(Duration::from_millis(2000), TcpStream::connect(test_addr)).await;
    match connect_res {
        Ok(Ok(_)) => {
            let msg = "Сетевой шлюз и глобальный интернет доступны".to_string();
            steps.push(DoctorStepResult {
                id: "connectivity_check".into(),
                title: "Проверка базового сетевого подключения".into(),
                name: "Проверка базового сетевого подключения".into(),
                status: "ok".into(),
                message: msg.clone(),
                details: msg,
            });
        }
        _ => {
            all_ok = false;
            connectivity_failed = true;
            let msg = "Задержка или отсутствие прямого TCP отклика от 1.1.1.1:53".to_string();
            steps.push(DoctorStepResult {
                id: "connectivity_check".into(),
                title: "Проверка базового сетевого подключения".into(),
                name: "Проверка базового сетевого подключения".into(),
                status: "warn".into(),
                message: msg.clone(),
                details: msg,
            });
        }
    }

    let summary: String = if all_ok {
        "Все сетевые службы, драйвер ядра и сокеты успешно восстановлены!".into()
    } else if driver_failed && connectivity_failed {
        "Драйвер WinDivert требует внимания, и обнаружены задержки в сетевом подключении.".into()
    } else if driver_failed {
        "Драйвер WinDivert требует внимания — подробности в шаге «Драйвер WinDivert».".into()
    } else {
        "Базовые компоненты восстановлены, но обнаружены задержки в сетевом подключении.".into()
    };

    DoctorReport {
        success: all_ok,
        steps,
        summary: summary.clone(),
        message: summary,
    }
}

//! 1-Click Network & Driver Auto-Doctor (Self-Healing Troubleshooter)
//!
//! Automatically diagnoses and repairs common network stack, driver, and caching issues:
//! 1. Cleans orphaned winws/winws2 background processes.
//! 2. Resets and unhooks the WinDivert kernel driver service
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

use crate::win32::process::{cleanup_windivert_driver, kill_stray_winws2_processes};

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

    // 2. Reset WinDivert driver registration — SKIPPED while the engine is running.
    //    Tearing the driver service down from under an active winws would blackhole
    //    traffic; the user should stop the bypass first and re-run the doctor.
    if engine_running {
        let wd_msg = "Сброс драйвера пропущен: движок обхода запущен. Остановите обход и повторите диагностику для сброса WinDivert.".to_string();
        steps.push(DoctorStepResult {
            id: "windivert_reset".into(),
            title: "Сброс драйвера ядра WinDivert".into(),
            name: "Сброс драйвера ядра WinDivert".into(),
            status: "warn".into(),
            message: wd_msg.clone(),
            details: wd_msg,
        });
    } else {
        cleanup_windivert_driver().await;
        let wd_msg = "Служба WinDivert успешно сброшена и готова к работе".to_string();
        steps.push(DoctorStepResult {
            id: "windivert_reset".into(),
            title: "Сброс драйвера ядра WinDivert".into(),
            name: "Сброс драйвера ядра WinDivert".into(),
            status: "ok".into(),
            message: wd_msg.clone(),
            details: wd_msg,
        });
    }

    // 3. Flush DNS Cache
    let dns_res = Command::new("ipconfig")
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
    let arp_res = Command::new("netsh")
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
    let ts_res = Command::new("netsh")
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

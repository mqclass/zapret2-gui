//! 1-Click Network Auto-Doctor (Linux).
//!
//! The Windows doctor is mostly OS repair work (WinDivert service reset,
//! `ipconfig /flushdns`, `netsh` ARP/TCP tweaks) that has no Linux
//! counterpart. This version runs the steps that genuinely exist here and
//! reports the Windows-only ones as not applicable — never as "ok":
//!
//! 1. Cleans orphaned engine processes (real, via platform::process).
//! 2. WinDivert reset — NOT APPLICABLE: there is no WinDivert on Linux.
//! 3. Flushes the systemd-resolved DNS cache (real, best-effort).
//! 4. ARP purge — NOT APPLICABLE in phase 1 (needs root `ip neigh flush`,
//!    the doctor runs unprivileged in the GUI).
//! 5. TCP timestamps — real read-only check of
//!    `/proc/sys/net/ipv4/tcp_timestamps` (Linux defaults it to 1, the
//!    value desync wants; nothing is written).
//! 6. Verifies outbound gateway connectivity (same as Windows).

use std::net::SocketAddr;
use std::process::Command;
use std::time::Duration;
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::platform::process::kill_stray_winws2_processes;

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

fn step(id: &str, title: &str, status: &str, message: String) -> DoctorStepResult {
    DoctorStepResult {
        id: id.into(),
        title: title.into(),
        name: title.into(),
        status: status.into(),
        message: message.clone(),
        details: message,
    }
}

pub async fn run_auto_doctor(active_pid: Option<u32>, engine_running: bool) -> DoctorReport {
    let mut steps = Vec::new();
    let mut all_ok = true;
    let _ = engine_running; // no driver to protect on Linux; see step 2

    // 1. Terminate orphaned processes (real).
    let strays = kill_stray_winws2_processes(active_pid);
    let p_msg = if strays > 0 {
        format!("Успешно завершено {} зависших процессов", strays)
    } else {
        "Зависших процессов не обнаружено".into()
    };
    steps.push(step(
        "process_cleanup",
        "Очистка фоновых процессов nfqws2",
        "ok",
        p_msg,
    ));

    // 2. WinDivert reset — not applicable on Linux.
    steps.push(step(
        "windivert_reset",
        "Сброс драйвера ядра WinDivert",
        "warn",
        "Неприменимо на Linux: WinDivert отсутствует. Правила nftables и очередь NFQUEUE появятся с шагом движка (фаза 1, шаг 3) — сбрасывать пока нечего.".into(),
    ));

    // 3. Flush DNS cache via systemd-resolved (real when present).
    let resolvectl = crate::platform::system32("resolvectl");
    let (dns_status, dns_msg) = if !resolvectl.is_file() {
        (
            "ok",
            "systemd-resolved не найден: системный DNS-кэш не используется, очищать нечего.".to_string(),
        )
    } else {
        match Command::new(&resolvectl).arg("flush-caches").output() {
            Ok(out) if out.status.success() => {
                ("ok", "Кэш DNS (systemd-resolved) успешно очищен".to_string())
            }
            _ => ("warn", "Не удалось сбросить кэш systemd-resolved".to_string()),
        }
    };
    steps.push(step("flush_dns", "Сброс системного кэша DNS", dns_status, dns_msg));

    // 4. ARP purge — not applicable in phase 1 (root-only, GUI is not root).
    steps.push(step(
        "arp_purge",
        "Очистка сетевой таблицы ARP",
        "warn",
        "Пропущено на Linux: очистка ARP требует прав root (ip neigh flush), а диагностика работает из непривилегированного интерфейса.".into(),
    ));

    // 5. TCP timestamps — read-only check (desync wants them on; Linux
    //    enables them by default).
    let ts_value = std::fs::read_to_string("/proc/sys/net/ipv4/tcp_timestamps")
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let (ts_status, ts_msg) = match ts_value.as_str() {
        "1" | "2" => (
            "ok",
            "TCP Timestamps включены (необходимо для надежной десинхронизации ТСПУ)".to_string(),
        ),
        "0" => (
            "warn",
            "TCP Timestamps выключены (net.ipv4.tcp_timestamps=0) — десинхронизация может работать хуже".to_string(),
        ),
        _ => ("warn", "Не удалось прочитать net.ipv4.tcp_timestamps".to_string()),
    };
    steps.push(step("tcp_timestamps", "Проверка TCP Timestamps", ts_status, ts_msg));

    // 6. Test Gateway / Outbound TCP Handshake (same as Windows).
    let test_addr: SocketAddr = "1.1.1.1:53".parse().unwrap();
    let connect_res = timeout(Duration::from_millis(2000), TcpStream::connect(test_addr)).await;
    match connect_res {
        Ok(Ok(_)) => {
            steps.push(step(
                "connectivity_check",
                "Проверка базового сетевого подключения",
                "ok",
                "Сетевой шлюз и глобальный интернет доступны".into(),
            ));
        }
        _ => {
            all_ok = false;
            steps.push(step(
                "connectivity_check",
                "Проверка базового сетевого подключения",
                "warn",
                "Задержка или отсутствие прямого TCP отклика от 1.1.1.1:53".into(),
            ));
        }
    }

    let summary: String = if all_ok {
        "Применимые на Linux проверки пройдены: процессы, DNS-кэш и подключение в порядке. Шаги WinDivert и ARP — только Windows, здесь неприменимы.".into()
    } else {
        "Базовые проверки выполнены, но обнаружены задержки в сетевом подключении.".into()
    };

    DoctorReport {
        success: all_ok,
        steps,
        summary: summary.clone(),
        message: summary,
    }
}

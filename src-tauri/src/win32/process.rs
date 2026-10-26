//! Windows Process Supervisor and WinDivert Life-Cycle Management for zapret2
//!
//! # Responsibilities
//! 1. Dynamic binary resolution (`winws2.exe`, `winws.exe`, `WinDivert.dll`, `WinDivert64.sys`, `lists/`).
//! 2. Hybrid strategy discovery: Flowseal presets + zapret2 Lua strategies.
//! 3. Process execution in an isolated process group (`CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW`).
//! 4. Immediate binding to Windows Job Objects (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`).
//! 5. Direct process termination (`kill` + `wait`) with Job Object teardown as safety net.
//! 6. Native fallback driver cleanup via Service Control Manager.

use std::fs;
use std::io;
use std::os::windows::io::RawHandle;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;

use tokio::process::{Child, ChildStderr, ChildStdout, Command};
use tokio::time::timeout;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, QueryFullProcessImageNameW, TerminateProcess,
    PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
};
use windows::core::PWSTR;

use crate::win32::conflicts::stop_service_native;
use crate::win32::job_object::{JobObject, JobObjectError};

/// Creation flags for spawning detached Windows background process groups
const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Errors arising during binary discovery, process lifecycle, or WinDivert manipulation.
#[allow(dead_code)]
#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("Failed to locate zapret2 binaries (winws2.exe, WinDivert.dll, WinDivert64.sys). Searched paths:\n{}", .searched.join("\n"))]
    BinariesNotFound { searched: Vec<String> },

    #[error("Failed to locate Lua strategy directory. Searched paths:\n{}", .searched.join("\n"))]
    StrategiesNotFound { searched: Vec<String> },

    #[error("Failed to spawn process: {0}")]
    SpawnFailed(#[source] io::Error),

    #[error("Spawned process has no raw handle")]
    MissingRawHandle,

    #[error("Spawned process has no process ID")]
    MissingProcessId,

    #[error("Job Object error during process supervision: {0}")]
    JobError(#[from] JobObjectError),

    #[error("Failed to wait for child process: {0}")]
    WaitFailed(#[source] io::Error),

    #[error("ADMIN_REQUIRED: для запуска движка нужны права администратора. Перезапустите приложение с правами администратора.")]
    NotElevated,

    #[error("Некорректная стратегия: {0}")]
    InvalidStrategy(String),

    #[error("Транспорт: {0}")]
    Transport(String),

    #[error("I/O error during process management: {0}")]
    Io(#[from] io::Error),
}

/// Resolved filesystem paths for zapret components.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ZapretBinaries {
    pub bin_dir: PathBuf,
    pub winws2_exe: PathBuf,
    pub winws_exe: Option<PathBuf>,
    pub windivert_dll: PathBuf,
    pub windivert_sys: PathBuf,
    pub lists_dir: Option<PathBuf>,
}

/// Strategy or Preset metadata.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LuaStrategy {
    pub id: String,
    pub name: String,
    pub filename: String,
    pub path: PathBuf,
    pub size_bytes: u64,
    pub description: String,
    pub engine: String, // "winws" or "winws2"
    pub category: String, // "flowseal" | "zapret2" | "custom"
    pub custom_args: Option<Vec<String>>,
}

/// Active supervised process instance containing asynchronous log streams.
#[allow(dead_code)]
pub struct SupervisedProcess {
    pub child: Child,
    pub pid: u32,
    pub job: Arc<JobObject>,
    pub stdout: Option<ChildStdout>,
    pub stderr: Option<ChildStderr>,
}

/// Resolves `winws2.exe`, `winws.exe`, `WinDivert.dll`, and `lists/`.
///
/// SECURITY (M-3): binaries are resolved strictly relative to the directory
/// containing the current executable. We deliberately do NOT consult the
/// current working directory, parent directories, or environment variables:
/// this application runs elevated, and executing a `winws2.exe` planted by a
/// local attacker in a writable CWD (or pointed to via env) would be a
/// privilege-escalation vector.
/// The `ZAPRET2_BIN_DIR` override is honored only when the `--dev` flag is
/// passed explicitly (developer workflows), never in production.
pub fn resolve_binaries() -> Result<ZapretBinaries, ProcessError> {
    let mut candidates = Vec::new();
    let mut searched = Vec::new();

    // Anchor: the directory containing the current executable.
    let exe_dir: Option<PathBuf> = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()));

    // Developer override: only behind an explicit --dev flag. In production
    // the process environment is untrusted, so the env var alone is ignored.
    let dev_mode = std::env::args().any(|a| a == "--dev");
    if dev_mode {
        if let Ok(env_dir) = std::env::var("ZAPRET2_BIN_DIR") {
            candidates.push(PathBuf::from(env_dir));
        }
    }

    // Production candidates: strictly anchored at the executable location.
    if let Some(ref dir) = exe_dir {
        candidates.push(dir.join("bin"));
        candidates.push(dir.join("bin/windows-x86_64"));
        candidates.push(dir.clone());
    }

    // Legacy repo-checkout layout, dev mode only (local development convenience).
    if dev_mode {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        candidates.push(cwd.join("bin"));
        candidates.push(cwd.join("bin/windows-x86_64"));
    }

    for candidate in candidates {
        let display_path = candidate.to_string_lossy().to_string();
        searched.push(display_path);

        let winws2 = candidate.join("winws2.exe");
        let winws1 = candidate.join("winws.exe");
        let windivert_dll = candidate.join("WinDivert.dll");
        let windivert_sys = candidate.join("WinDivert64.sys");

        // All-or-nothing per candidate directory: the engine and the kernel
        // driver components must come from the SAME directory, so a partial
        // planting cannot mix attacker-controlled files into a legit install.
        // winws2.exe is mandatory (primary engine); winws.exe is optional.
        if !(winws2.is_file() && windivert_dll.is_file() && windivert_sys.is_file()) {
            continue;
        }

        let winws1_path = winws1.is_file().then(|| winws1.clone());

        // Resolve lists directory relative to the install location.
        let lists_dir = [candidate.join("lists")]
            .into_iter()
            .chain(exe_dir.iter().map(|d| d.join("lists")))
            .find(|p| p.is_dir());

        return Ok(ZapretBinaries {
            bin_dir: candidate,
            winws2_exe: winws2,
            winws_exe: winws1_path,
            windivert_dll,
            windivert_sys,
            lists_dir,
        });
    }

    Err(ProcessError::BinariesNotFound { searched })
}

/// Discovers Flowseal battle-tested presets, genuine zapret2 Lua strategies, and user custom profiles.
pub fn discover_strategies() -> Result<Vec<LuaStrategy>, ProcessError> {
    let mut list = Vec::new();

    // 1. Flowseal battle-tested presets (primary for Russian ISP / TSPU)
    list.push(LuaStrategy {
        id: "flowseal-alt".into(),
        name: "Flowseal ALT (Fakedsplit) — Рекомендован для РФ".into(),
        filename: "general (ALT).bat (winws)".into(),
        path: PathBuf::from("presets/flowseal-alt"),
        size_bytes: 3877,
        description: "Основной рабочий режим для всех провайдеров РФ (Ростелеком, МТС, Дом.ру, Билайн, ТСПУ)".into(),
        engine: "winws".into(),
        category: "flowseal".into(),
        custom_args: None,
    });

    list.push(LuaStrategy {
        id: "flowseal-alt11".into(),
        name: "Flowseal ALT11 (Усиленный 4K)".into(),
        filename: "general (ALT11).bat (winws)".into(),
        path: PathBuf::from("presets/flowseal-alt11"),
        size_bytes: 4297,
        description: "YouTube 4K 60FPS (seqovl=681, repeats=11) + Discord Голос".into(),
        engine: "winws".into(),
        category: "flowseal".into(),
        custom_args: None,
    });

    list.push(LuaStrategy {
        id: "flowseal-general".into(),
        name: "Flowseal General (Multisplit)".into(),
        filename: "general.bat (winws)".into(),
        path: PathBuf::from("presets/flowseal-general"),
        size_bytes: 3351,
        description: "YouTube + Discord + Сайты (мягкий режим без поддельных пакетов)".into(),
        engine: "winws".into(),
        category: "flowseal".into(),
        custom_args: None,
    });

    list.push(LuaStrategy {
        id: "flowseal-fake-tls-auto".into(),
        name: "Flowseal FAKE TLS AUTO (Игровой)".into(),
        filename: "general (FAKE TLS AUTO).bat (winws)".into(),
        path: PathBuf::from("presets/flowseal-fake-tls-auto"),
        size_bytes: 4017,
        description: "Discord Голос + Минимальный пинг в онлайн-играх".into(),
        engine: "winws".into(),
        category: "flowseal".into(),
        custom_args: None,
    });

    list.push(LuaStrategy {
        id: "flowseal-simple-fake".into(),
        name: "Flowseal Simple Fake".into(),
        filename: "general (SIMPLE FAKE).bat (winws)".into(),
        path: PathBuf::from("presets/flowseal-simple-fake"),
        size_bytes: 3497,
        description: "Простой режим для мобильных раздач и Wi-Fi роутеров".into(),
        engine: "winws".into(),
        category: "flowseal".into(),
        custom_args: None,
    });

    // 2. zapret2 (winws2) Ultimate configuration for Russia
    list.push(LuaStrategy {
        id: "zapret2-ru-ultimate".into(),
        name: "zapret2 РФ Ultimate (YouTube 4K + Discord)".into(),
        filename: "zapret2-ru-ultimate.lua (winws2)".into(),
        path: PathBuf::from("strategies/zapret-antidpi.lua"),
        size_bytes: 4096,
        description: "Флагманский профиль на ядре winws2 v2: Flowseal payload + Lua desync + seqovl=681 + conntrack 4K".into(),
        engine: "winws2".into(),
        category: "zapret2".into(),
        custom_args: None,
    });

    // 3. Discover local Lua scripts for zapret2 engine (filtering out internal helper/test modules)
    const INTERNAL_LUA_SCRIPTS: &[&str] = &[
        "zapret-lib.lua",
        "zapret-obfs.lua",
        "zapret-pcap.lua",
        "zapret-tests.lua",
        "lib.lua",
        "obfs.lua",
        "pcap.lua",
        "tests.lua",
    ];

    let mut search_dirs = Vec::new();
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    search_dirs.push(cwd.join("strategies"));
    search_dirs.push(cwd.join("vendor/zapret2-master/lua"));
    search_dirs.push(cwd.join("zapret2-master/lua"));
    search_dirs.push(cwd.join("lua"));

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            search_dirs.push(exe_dir.join("strategies"));
            search_dirs.push(exe_dir.join("lua"));
        }
    }

    for dir in search_dirs {
        if dir.is_dir() {
            if let Ok(entries) = fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() && path.extension().map_or(false, |ext| ext == "lua") {
                        if let Some(filename_str) = path.file_name().and_then(|f| f.to_str()) {
                            // Skip internal zapret2 helper, test, and capture scripts
                            if INTERNAL_LUA_SCRIPTS.iter().any(|&s| s.eq_ignore_ascii_case(filename_str)) {
                                continue;
                            }

                            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(filename_str);
                            let id = stem.replace("zapret-", "");
                            if list.iter().any(|s| s.id == id || s.filename == filename_str) {
                                continue;
                            }

                            let metadata = entry.metadata().ok();
                            let size_bytes = metadata.map(|m| m.len()).unwrap_or(0);

                            let (name, description) = match id.as_str() {
                                "antidpi" => (
                                    "zapret2 Универсальный Lua".into(),
                                    "Официальный Lua скрипт zapret-antidpi.lua (winws2)".into(),
                                ),
                                "auto" => (
                                    "zapret2 Автоматический Lua".into(),
                                    "Автоматический подбор desync в Lua (winws2)".into(),
                                ),
                                "yt4k" => (
                                    "zapret2 YouTube 4K".into(),
                                    "Агрессивный desync для YouTube 4K: тройные фейки и многоточечная резка по TCP, плюс фейк QUIC по UDP.".into(),
                                ),
                                "discord-light" => (
                                    "zapret2 Discord Лёгкий".into(),
                                    "Лёгкий desync для Discord с минимальными задержками: TCP плюс голос по UDP.".into(),
                                ),
                                "sites-light" => (
                                    "zapret2 Только сайты".into(),
                                    "Минимальное вмешательство для слабых машин и мобильных раздач: одна резка без фейков.".into(),
                                ),
                                "tspu-max" => (
                                    "zapret2 ТСПУ Макс".into(),
                                    "Жёсткий desync для сурового ТСПУ ценой скорости: переупорядочивание сегментов и тройные фейки.".into(),
                                ),
                                _ => (
                                    format!("zapret2 Lua: {}", id),
                                    format!("Скрипт {}", filename_str),
                                ),
                            };

                            list.push(LuaStrategy {
                                id,
                                name,
                                filename: filename_str.to_string(),
                                path: path.clone(),
                                size_bytes,
                                description,
                                engine: "winws2".into(),
                                category: "zapret2".into(),
                                custom_args: None,
                            });
                        }
                    }
                }
            }
        }
    }

    // 3. Load user-defined custom profiles
    let custom_profiles = crate::win32::custom_profiles::load_custom_profiles();
    for custom in custom_profiles {
        if !list.iter().any(|s| s.id == custom.id) {
            list.push(custom);
        }
    }

    Ok(list)
}

/// Spawns the target process (either winws.exe or winws2.exe) in an isolated process group.
pub fn spawn_process(
    exe_path: &Path,
    bin_dir: &Path,
    args: &[String],
    job: Arc<JobObject>,
) -> Result<SupervisedProcess, ProcessError> {
    let mut cmd = Command::new(exe_path);
    cmd.args(args);
    cmd.current_dir(bin_dir);
    cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let mut child = cmd.spawn().map_err(ProcessError::SpawnFailed)?;

    let raw_handle: RawHandle = child
        .raw_handle()
        .ok_or(ProcessError::MissingRawHandle)?;

    let pid = child.id().ok_or(ProcessError::MissingProcessId)?;

    // Immediately bind child process handle to Job Object
    job.assign_raw_handle(raw_handle)?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    Ok(SupervisedProcess {
        child,
        pid,
        job,
        stdout,
        stderr,
    })
}

/// Helper wrapper preserving backward compatibility.
#[allow(dead_code)]
pub fn spawn_winws2(
    binaries: &ZapretBinaries,
    args: &[String],
    job: Arc<JobObject>,
) -> Result<SupervisedProcess, ProcessError> {
    spawn_process(&binaries.winws2_exe, &binaries.bin_dir, args, job)
}

/// Terminates a supervised winws process.
///
/// NOTE: a graceful `CTRL_BREAK_EVENT` shutdown is impossible here by design:
/// the child is spawned with `CREATE_NO_WINDOW`, so it has no console and
/// `AttachConsole` could never succeed for it. Termination is therefore direct:
/// `kill` the child, wait briefly for it to exit, then terminate the Job Object
/// as a safety net so no stray winws instance survives.
pub async fn stop_winws2_gracefully(
    child: &mut Child,
    job: &JobObject,
) -> Result<(), ProcessError> {
    let _ = child.kill().await;
    // Give the process a moment to exit so WinDivert handles are released
    // before the Job Object teardown below.
    let _ = timeout(Duration::from_secs(5), child.wait()).await;
    let _ = job.terminate(1);
    Ok(())
}

/// Purges stuck WinDivert driver services natively and quietly.
pub async fn cleanup_windivert_driver() {
    let service_names = ["WinDivert", "WinDivert14", "WinDivert64"];

    for name in &service_names {
        stop_service_native(name);
        let _ = Command::new(crate::win32::system32("sc.exe"))
            .args(["stop", name])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .await;
    }
}

/// Verifies through an open process handle that the process image file name still
/// matches the expected executable name.
///
/// SECURITY (L-6, TOCTOU guard): the PID may have been reused between the
/// ToolHelp32 snapshot and `OpenProcess` — never terminate based on the stale
/// snapshot name alone. Querying the image path from the handle closes the race.
pub fn process_image_matches(handle: HANDLE, expected_exe_name: &str) -> bool {
    unsafe {
        let mut buf = [0u16; 1024];
        let mut size = buf.len() as u32;
        if QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT(0),
            PWSTR(buf.as_mut_ptr()),
            &mut size,
        )
        .is_err()
        {
            return false;
        }
        let path = String::from_utf16_lossy(&buf[..size as usize]);
        Path::new(&path)
            .file_name()
            .and_then(|n| n.to_str())
            .map_or(false, |n| n.eq_ignore_ascii_case(expected_exe_name))
    }
}

/// Discovers and terminates any orphaned `winws2.exe` or `winws.exe` processes,
/// protecting any actively running supervised process ID.
pub fn kill_stray_winws2_processes(exclude_pid: Option<u32>) -> usize {
    let mut terminated_count = 0;
    let targets = ["winws2.exe", "winws.exe"];

    unsafe {
        if let Ok(handle) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            let mut entry = PROCESSENTRY32W::default();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

            if Process32FirstW(handle, &mut entry).is_ok() {
                loop {
                    let len = entry
                        .szExeFile
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(entry.szExeFile.len());
                    let exe_name = String::from_utf16_lossy(&entry.szExeFile[..len]);

                    if targets.iter().any(|&t| exe_name.eq_ignore_ascii_case(t)) {
                        let pid = entry.th32ProcessID;
                        if Some(pid) != exclude_pid {
                            // Open with query rights too so we can re-verify the
                            // image name from the handle (L-6 TOCTOU guard).
                            if let Ok(proc_handle) = OpenProcess(
                                PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
                                false,
                                pid,
                            ) {
                                // Re-verify: the PID may have been reused since the snapshot.
                                let confirmed = targets
                                    .iter()
                                    .any(|&t| process_image_matches(proc_handle, t));
                                if confirmed && TerminateProcess(proc_handle, 1).is_ok() {
                                    terminated_count += 1;
                                }
                                let _ = CloseHandle(proc_handle);
                            }
                        }
                    }

                    if Process32NextW(handle, &mut entry).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(handle);
        }
    }

    terminated_count
}

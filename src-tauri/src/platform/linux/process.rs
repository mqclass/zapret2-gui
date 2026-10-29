//! Process supervision and engine binary resolution for zapret2 (Linux).
//!
//! # Responsibilities
//! 1. Dynamic binary resolution (`nfqws2`, the native Linux zapret2 engine).
//! 2. Hybrid strategy discovery: Flowseal presets + zapret2 Lua strategies
//!    (the preset/strategy metadata is platform-independent; which engines
//!    can actually launch on Linux is decided by binary resolution and by
//!    the supervisor's engine wiring: Lua strategies run on nfqws2,
//!    Flowseal presets cannot — nfqws2 has no `--dpi-desync`).
//! 3. Process execution in its own session (`setsid` via `pre_exec`) with
//!    `PR_SET_PDEATHSIG`, so a dead supervisor takes the engine with it.
//! 4. Binding to the process-group registry ([`JobObject`]) — the Linux
//!    equivalent of the Windows Job Object binding.
//! 5. Direct process termination (`kill` + `wait`) with group teardown as
//!    the safety net.
//!
//! There is no WinDivert on Linux: packet interception is NFQUEUE +
//! nftables, installed by the supervisor service (root) rather than by a
//! kernel driver shipped next to the binary.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;

use tokio::process::{Child, ChildStderr, ChildStdout, Command};
use tokio::time::timeout;

use crate::platform::job_object::{JobObject, JobObjectError};

/// Errors arising during binary discovery or process lifecycle management.
#[allow(dead_code)]
#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("Failed to locate zapret2 binaries (nfqws2). Searched paths:\n{}", .searched.join("\n"))]
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

    #[error("ADMIN_REQUIRED: для запуска движка нужны права root. Движок работает в службе zapret2-svc (systemd), а не из интерфейса.")]
    NotElevated,

    /// The engine cannot run on this platform as requested (e.g. a
    /// Flowseal strategy, whose `--dpi-desync` flags nfqws2 does not
    /// accept, or the nftables capture could not be installed). The
    /// message is the honest, user-facing reason.
    #[error("{0}")]
    EngineUnavailable(String),

    #[error("Некорректная стратегия: {0}")]
    InvalidStrategy(String),

    #[error("Транспорт: {0}")]
    Transport(String),

    #[error("I/O error during process management: {0}")]
    Io(#[from] io::Error),
}

/// Resolved filesystem paths for zapret components.
///
/// Same shape as the Windows struct because neutral code reads these
/// fields. Linux mapping:
/// - `winws2_exe` — the primary engine: `nfqws2`.
/// - `winws_exe` — the v1 engine slot: `nfqws` when present (winws.exe is
///   the Windows port of nfqws and takes the same desync flag family).
/// - `winws2_split_exe` — always `None` on Linux: the split build is a
///   Windows binary of our fork; per-app split is honestly not enforced
///   here, and the supervisor already treats a missing split binary as a
///   normal state with a logged fallback.
/// - `windivert_dll` / `windivert_sys` — no WinDivert exists on Linux.
///   The fields stay (the struct predates the port) and are left EMPTY;
///   nothing outside this module reads them.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ZapretBinaries {
    pub bin_dir: PathBuf,
    pub winws2_exe: PathBuf,
    pub winws_exe: Option<PathBuf>,
    /// Split-enforcing winws2 build from our fork (mqclass/zapret2).
    /// Windows-only for now — see the struct docs.
    #[serde(default)]
    pub winws2_split_exe: Option<PathBuf>,
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

/// Resolves `nfqws2` (and optionally `nfqws`) plus the `lists/` directory.
///
/// SECURITY (M-3, same posture as Windows): binaries are resolved strictly
/// relative to the directory containing the current executable. We
/// deliberately do NOT consult the current working directory, parent
/// directories, or environment variables in production: the consumer of
/// these paths is a root service, and executing an `nfqws2` planted in a
/// writable CWD would be a privilege-escalation vector.
/// The `ZAPRET2_BIN_DIR` override is honored only behind an explicit
/// `--dev` flag (developer workflows), never in production.
pub fn resolve_binaries() -> Result<ZapretBinaries, ProcessError> {
    let mut candidates = Vec::new();
    let mut searched = Vec::new();

    // Anchor: the directory containing the current executable.
    let exe_dir: Option<PathBuf> = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()));

    // Developer override: only behind an explicit --dev flag.
    let dev_mode = std::env::args().any(|a| a == "--dev");
    if dev_mode {
        if let Ok(env_dir) = std::env::var("ZAPRET2_BIN_DIR") {
            candidates.push(PathBuf::from(env_dir));
        }
    }

    // Production candidates: strictly anchored at the executable location.
    if let Some(ref dir) = exe_dir {
        candidates.push(dir.join("bin"));
        candidates.push(dir.join("bin/linux-x86_64"));
        candidates.push(dir.clone());
    }

    // Legacy repo-checkout layout, dev mode only.
    if dev_mode {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        candidates.push(cwd.join("bin"));
        candidates.push(cwd.join("bin/linux-x86_64"));
    }

    for candidate in candidates {
        let display_path = candidate.to_string_lossy().to_string();
        searched.push(display_path);

        let nfqws2 = candidate.join("nfqws2");
        let nfqws = candidate.join("nfqws");

        // nfqws2 is mandatory (primary engine); nfqws is optional and must
        // come from the SAME directory when present — no mixing binaries
        // from different directories.
        if !nfqws2.is_file() {
            continue;
        }

        let nfqws_path = nfqws.is_file().then(|| nfqws.clone());

        // Resolve lists directory relative to the install location.
        let lists_dir = [candidate.join("lists")]
            .into_iter()
            .chain(exe_dir.iter().map(|d| d.join("lists")))
            .find(|p| p.is_dir());

        return Ok(ZapretBinaries {
            bin_dir: candidate,
            winws2_exe: nfqws2,
            winws_exe: nfqws_path,
            winws2_split_exe: None,
            windivert_dll: PathBuf::new(),
            windivert_sys: PathBuf::new(),
            lists_dir,
        });
    }

    Err(ProcessError::BinariesNotFound { searched })
}

/// Discovers Flowseal battle-tested presets, genuine zapret2 Lua strategies, and user custom profiles.
///
/// The catalog is shared with the Windows build: presets and Lua scripts
/// are data, and the same strategies ship on both platforms. Whether a
/// given entry can launch depends on the resolved engine binaries.
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
    let custom_profiles = crate::platform::custom_profiles::load_custom_profiles();
    for custom in custom_profiles {
        if !list.iter().any(|s| s.id == custom.id) {
            list.push(custom);
        }
    }

    Ok(list)
}

/// Spawns the target engine process in its own session and process group.
///
/// `pre_exec` runs in the child between fork and exec:
/// - `setsid()` — the child leads a new process group (pgid == pid), which
///   is what the [`JobObject`] registry tracks and kills as a unit.
/// - `prctl(PR_SET_PDEATHSIG, SIGKILL)` — if the supervisor dies in ANY
///   way (crash, SIGKILL), the kernel kills the engine leader: the direct
///   analog of the Windows job closing with its owner. The `getppid() == 1`
///   re-check closes the race where the parent died before `prctl` ran.
pub fn spawn_process(
    exe_path: &Path,
    bin_dir: &Path,
    args: &[String],
    job: Arc<JobObject>,
) -> Result<SupervisedProcess, ProcessError> {
    let mut cmd = Command::new(exe_path);
    cmd.args(args);
    cmd.current_dir(bin_dir);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) == -1 {
                return Err(io::Error::last_os_error());
            }
            if libc::getppid() == 1 {
                // Parent already gone before the death signal was armed.
                libc::raise(libc::SIGKILL);
            }
            Ok(())
        });
    }

    let mut child = cmd.spawn().map_err(ProcessError::SpawnFailed)?;

    let pid = child.id().ok_or(ProcessError::MissingProcessId)?;

    // Register the new process group with the supervisor's registry.
    // On failure the just-spawned engine must not be left running
    // unsupervised (a dropped tokio Child is NOT killed), so kill and
    // reap it before propagating the assignment error.
    if let Err(e) = job.assign_pid(pid) {
        kill_and_reap_spawned_child(&mut child, pid);
        return Err(e.into());
    }

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

/// Kills and reaps a just-spawned child after a failed registry
/// assignment (see the call site in [`spawn_process`]).
///
/// The child already ran `setsid`, so it leads its own group: the
/// group SIGKILL takes any helper it managed to fork in the short
/// window, the direct SIGKILL covers the leader itself. Reaping is
/// done with a bounded `waitpid` poll — this runs on a synchronous
/// error path inside an async caller, and tokio's own reaper may win
/// the race (ECHILD), which is treated as success: either way no
/// zombie and no unsupervised engine survives.
fn kill_and_reap_spawned_child(child: &mut Child, pid: u32) {
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
        libc::kill(pid as i32, libc::SIGKILL);
    }
    let _ = child.start_kill();
    unsafe {
        let mut status: libc::c_int = 0;
        for _ in 0..200 {
            let rc = libc::waitpid(pid as i32, &mut status, libc::WNOHANG);
            if rc != 0 {
                // rc == pid: reaped by us. rc == -1 (e.g. ECHILD):
                // tokio's reaper got it first. Both are done.
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
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

/// Terminates a supervised engine process.
///
/// Direct `kill` of the leader first, a brief wait for it to exit (so its
/// NFQUEUE handles are released), then the process-group teardown as the
/// safety net so no stray engine instance survives.
pub async fn stop_winws2_gracefully(
    child: &mut Child,
    job: &JobObject,
) -> Result<(), ProcessError> {
    let _ = child.kill().await;
    // Give the process a moment to exit so queue handles are released
    // before the group teardown below.
    let _ = timeout(Duration::from_secs(5), child.wait()).await;
    let _ = job.terminate(1);
    Ok(())
}

/// Driver cleanup hook (Windows: WinDivert service reset).
///
/// On Linux there is no loadable driver service to reset: interception
/// is NFQUEUE bound by the engine process itself plus nftables rules
/// owned by the supervisor. Those rules have their own lifecycle in
/// the supervisor (installed before spawn, removed on stop, engine
/// death and failed starts — see `platform::nftables`), so this
/// best-effort pre-retry reset hook has no Linux state to clean and
/// stays an explicit no-op.
pub async fn cleanup_windivert_driver() {}

/// Verifies through `/proc/<pid>/exe` that the process image file name
/// still matches the expected executable name.
///
/// SECURITY (L-6, TOCTOU guard): the PID may have been reused between a
/// `/proc` scan and the kill — never terminate based on a stale scan alone.
/// Re-reading the exe symlink right before the kill closes the race the
/// same way the Windows handle query does.
pub fn process_image_matches(pid: u32, expected_exe_name: &str) -> bool {
    let exe = match fs::read_link(format!("/proc/{pid}/exe")) {
        Ok(p) => p,
        Err(_) => return false,
    };
    // A deleted-but-running binary shows up with a " (deleted)" suffix.
    let name = exe
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .trim_end_matches(" (deleted)");
    name == expected_exe_name
}

/// Discovers and terminates any orphaned `nfqws2`/`nfqws` engine processes,
/// protecting the actively supervised process ID.
///
/// Note: as the unprivileged GUI this can only see (and, with EPERM,
/// fail to kill) the root-owned svc engines — the count honestly reports
/// only processes this process actually terminated.
pub fn kill_stray_winws2_processes(exclude_pid: Option<u32>) -> usize {
    let mut terminated_count = 0;
    let targets = ["nfqws2", "nfqws"];
    let own_pid = std::process::id();

    let entries = match fs::read_dir("/proc") {
        Ok(e) => e,
        Err(_) => return 0,
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else { continue };
        let Ok(pid) = name_str.parse::<u32>() else { continue };
        if pid <= 1 || pid == own_pid || Some(pid) == exclude_pid {
            continue;
        }
        // Cheap first pass: exe basename must be an engine binary.
        let exe = match fs::read_link(entry.path().join("exe")) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let exe_name = exe
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .trim_end_matches(" (deleted)");
        if !targets.contains(&exe_name) {
            continue;
        }
        // L-6 TOCTOU guard: re-verify the image from a fresh read before
        // terminating — the PID may have been reused since the scan above.
        if !process_image_matches(pid, exe_name) {
            continue;
        }
        if unsafe { libc::kill(pid as i32, libc::SIGKILL) } == 0 {
            terminated_count += 1;
        }
    }

    terminated_count
}

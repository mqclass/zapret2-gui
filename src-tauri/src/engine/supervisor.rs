//! zapret2 Supervisor Engine & High-Throughput Micro-Batching Log Pipeline
//!
//! # Responsibilities
//! 1. Life-cycle orchestration (`Stopped` -> `Starting` -> `Running` -> `Stopping` -> `Error`).
//! 2. Spawns `winws2.exe` with selected Lua strategies and WFP filter rules.
//! 3. Binds process to the Windows Job Object immediately.
//! 4. Asynchronously reads stdout/stderr line-by-line via `AsyncBufReadExt`.
//! 5. Ring Buffer + Micro-Batching Engine: Buffers lines and flushes batches every 150 ms
//!    over Tauri channel `"log-batch"` using `tauri::Emitter::emit`.
//! 6. Enforces strict memory footprint by capping the ring buffer to 1,000 entries.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::time::interval;

use crate::engine::logger::FileLogger;
use crate::engine::probes::DiagnosticsService;
use crate::win32::job_object::JobObject;
use crate::win32::process::{
    cleanup_windivert_driver, discover_strategies, kill_stray_winws2_processes, resolve_binaries,
    spawn_process, spawn_winws2, stop_winws2_gracefully, ProcessError, SupervisedProcess,
    ZapretBinaries,
};

/// Maximum log capacity in RAM to ensure memory consumption stays well below 20 MB.
const MAX_LOG_CAPACITY: usize = 1_000;

/// Micro-batching flush interval for frontend IPC (150 ms).
const BATCH_FLUSH_INTERVAL_MS: u64 = 150;

/// High-level supervisor operational lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SupervisorState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Error,
}

/// Structured single log event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: u64,
    pub level: String, // "stdout", "stderr", "info", "warn", "error"
    pub message: String,
}

/// Snapshot of the supervisor status for UI rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupervisorStatus {
    pub state: SupervisorState,
    pub pid: Option<u32>,
    pub active_strategy: Option<String>,
    pub binary_path: Option<String>,
    pub uptime_secs: u64,
    pub total_logs_buffered: usize,
}

/// The main Supervisor managing the WinDivert and winws2 lifecycle.
pub struct Supervisor {
    /// Serializes start()/stop() so they can never interleave: a stop() racing
    /// a start() either runs fully before it or fully after it, never in the
    /// middle of a half-initialized engine.
    lifecycle: Arc<tokio::sync::Mutex<()>>,
    state: Arc<RwLock<SupervisorState>>,
    active_pid: Arc<RwLock<Option<u32>>>,
    active_child: Arc<Mutex<Option<tokio::process::Child>>>,
    active_strategy: Arc<RwLock<Option<String>>>,
    binary_path: Arc<RwLock<Option<String>>>,
    start_time: Arc<RwLock<Option<Instant>>>,

    job_object: Arc<JobObject>,
    diagnostics: Arc<DiagnosticsService>,

    ring_buffer: Arc<Mutex<VecDeque<LogEntry>>>,
    log_tx: Sender<LogEntry>,
    file_logger: Arc<FileLogger>,

    app_handle: Arc<RwLock<Option<AppHandle>>>,
}

impl Supervisor {
    /// Initializes a new supervisor instance with Windows Job Object and micro-batching worker.
    pub fn new() -> Result<Self, ProcessError> {
        let job_object = Arc::new(JobObject::create()?);
        let diagnostics = Arc::new(DiagnosticsService::new());
        let ring_buffer = Arc::new(Mutex::new(VecDeque::with_capacity(MAX_LOG_CAPACITY)));
        let file_logger = Arc::new(FileLogger::new());

        let (log_tx, log_rx) = channel::<LogEntry>(2_000);

        let app_handle = Arc::new(RwLock::new(None::<AppHandle>));

        let supervisor = Self {
            lifecycle: Arc::new(tokio::sync::Mutex::new(())),
            state: Arc::new(RwLock::new(SupervisorState::Stopped)),
            active_pid: Arc::new(RwLock::new(None)),
            active_child: Arc::new(Mutex::new(None)),
            active_strategy: Arc::new(RwLock::new(None)),
            binary_path: Arc::new(RwLock::new(None)),
            start_time: Arc::new(RwLock::new(None)),
            job_object,
            diagnostics,
            ring_buffer,
            log_tx,
            file_logger,
            app_handle,
        };

        // Spawn high-throughput micro-batching engine
        supervisor.spawn_micro_batching_worker(log_rx);

        // Periodically flush OS-buffered log data to disk (file writes are buffered,
        // not flushed per line — see FileLogger).
        let file_logger_flush = file_logger.clone();
        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(5));
            loop {
                ticker.tick().await;
                file_logger_flush.flush();
            }
        });

        supervisor.log("info", "zapret2 v1.0.9 initialized. WinDivert and WFP subsystem ready.");
        supervisor.log("info", "Ready to start DPI bypass. Select a strategy to begin.");

        Ok(supervisor)
    }

    /// Sets the Tauri application handle for emitting frontend events.
    pub fn set_app_handle(&self, handle: AppHandle) {
        *self.app_handle.write() = Some(handle.clone());

        // Start background network probes with push notifications to Tauri
        let handle_clone = handle.clone();
        self.diagnostics.clone().start_polling_loop(move |report| {
            let _ = handle_clone.emit("probe-update", report);
        });
    }

    /// Spawns the dedicated Tokio micro-batching worker running every 150 ms.
    fn spawn_micro_batching_worker(&self, mut log_rx: Receiver<LogEntry>) {
        let ring_buffer = self.ring_buffer.clone();
        let app_handle = self.app_handle.clone();

        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_millis(BATCH_FLUSH_INTERVAL_MS));
            let mut pending_batch: Vec<LogEntry> = Vec::new();

            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        if !pending_batch.is_empty() {
                            let batch_to_emit = std::mem::take(&mut pending_batch);

                            // Append to in-memory Ring Buffer
                            {
                                let mut buffer = ring_buffer.lock();
                                for entry in &batch_to_emit {
                                    if buffer.len() >= MAX_LOG_CAPACITY {
                                        buffer.pop_front();
                                    }
                                    buffer.push_back(entry.clone());
                                }
                            }

                            // Emit batch to frontend
                            if let Some(handle) = app_handle.read().as_ref() {
                                let _ = handle.emit("log-batch", &batch_to_emit);
                            }
                        }
                    }
                    entry = log_rx.recv() => {
                        match entry {
                            Some(entry) => {
                                pending_batch.push(entry);
                                // If batch is getting large, flush early to prevent memory spike
                                if pending_batch.len() >= 100 {
                                    let batch_to_emit = std::mem::take(&mut pending_batch);
                                    {
                                        let mut buffer = ring_buffer.lock();
                                        for e in &batch_to_emit {
                                            if buffer.len() >= MAX_LOG_CAPACITY {
                                                buffer.pop_front();
                                            }
                                            buffer.push_back(e.clone());
                                        }
                                    }
                                    if let Some(handle) = app_handle.read().as_ref() {
                                        let _ = handle.emit("log-batch", &batch_to_emit);
                                    }
                                }
                            }
                            None => break,
                        }
                    }
                }
            }
        });
    }

    /// Emits a structured log event into the supervisor pipeline.
    pub fn log(&self, level: &str, message: &str) {
        self.file_logger.write_entry(level, message);

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let entry = LogEntry {
            timestamp: now,
            level: level.to_string(),
            message: message.to_string(),
        };

        let _ = self.log_tx.try_send(entry);
    }

    /// Opens the logs folder in Windows Explorer.
    pub fn open_logs_folder(&self) -> Result<(), String> {
        self.file_logger.open_in_explorer()
    }

    /// Retrieves the absolute file path to the active log file.
    pub fn get_log_file_path(&self) -> String {
        self.file_logger.get_log_file_path().to_string_lossy().to_string()
    }

    /// Retrieves current operational status.
    pub fn get_status(&self) -> SupervisorStatus {
        let state = *self.state.read();
        let pid = *self.active_pid.read();
        let active_strategy = self.active_strategy.read().clone();
        let binary_path = self.binary_path.read().clone();
        let uptime_secs = self
            .start_time
            .read()
            .map(|t| t.elapsed().as_secs())
            .unwrap_or(0);
        let total_logs_buffered = self.ring_buffer.lock().len();

        SupervisorStatus {
            state,
            pid,
            active_strategy,
            binary_path,
            uptime_secs,
            total_logs_buffered,
        }
    }

    /// Retrieves all buffered log entries currently stored in the ring buffer.
    pub fn get_all_logs(&self) -> Vec<LogEntry> {
        self.ring_buffer.lock().iter().cloned().collect()
    }

    /// Retrieves the shared diagnostics service.
    pub fn diagnostics(&self) -> Arc<DiagnosticsService> {
        self.diagnostics.clone()
    }

    /// Clears the log ring buffer.
    pub fn clear_logs(&self) {
        self.ring_buffer.lock().clear();
        if let Some(handle) = self.app_handle.read().as_ref() {
            let _ = handle.emit("logs-cleared", ());
        }
    }

    /// Starts the DPI bypass engine with selected strategy and optional custom arguments.
    pub async fn start(
        self: Arc<Self>,
        strategy_id: Option<String>,
        custom_args: Option<Vec<String>>,
    ) -> Result<(), ProcessError> {
        // SECURITY (M-1): fail-closed privilege gate lives INSIDE start(), not
        // only in the IPC command. The tray "Включить обход" item and the
        // --autostart path call start() directly; without this gate the engine
        // would start half-broken (WinDivert fails without admin rights) and
        // the UI would never show the admin prompt. The error string keeps the
        // ADMIN_REQUIRED prefix so the frontend opens the admin gate modal.
        if !crate::win32::privileges::is_elevated().unwrap_or(false) {
            self.log(
                "error",
                "Запуск движка требует прав администратора — пропуск запуска",
            );
            return Err(ProcessError::NotElevated);
        }

        // Hold the lifecycle lock for the whole start sequence so a concurrent
        // stop() cannot observe a half-initialized engine (see 1.1).
        let _lifecycle_guard = self.lifecycle.lock().await;

        // Support seamless strategy switching or fresh start
        {
            let mut state = self.state.write();
            if *state == SupervisorState::Starting || *state == SupervisorState::Stopping {
                return Ok(());
            }
            if *state == SupervisorState::Running {
                let active = self.active_strategy.read().clone();
                let requested = strategy_id.clone().unwrap_or_else(|| "flowseal-alt".into());
                if active.as_deref() == Some(&requested) && custom_args.is_none() {
                    return Ok(());
                }
                *state = SupervisorState::Starting;
            } else {
                *state = SupervisorState::Starting;
            }
        }

        // If another child process is active, stop it cleanly before spawning the new strategy
        let old_pid_opt = *self.active_pid.read();
        let maybe_child = self.active_child.lock().take();
        if let Some(mut old_child) = maybe_child {
            if let Some(old_pid) = old_pid_opt {
                self.log("info", &format!("Switching config: stopping previous engine (PID {})", old_pid));
                let _ = stop_winws2_gracefully(&mut old_child, &self.job_object).await;
            }
        }

        self.emit_state_change(SupervisorState::Starting);
        self.log("info", "Initiating zapret background supervisor...");

        // 1. Resolve binaries
        let binaries = match resolve_binaries() {
            Ok(b) => b,
            Err(e) => {
                self.log("error", &format!("Binary discovery failure: {}", e));
                *self.state.write() = SupervisorState::Error;
                self.emit_state_change(SupervisorState::Error);
                return Err(e);
            }
        };

        // Determine target strategy & engine
        let strategies = discover_strategies().unwrap_or_default();
        let target_strat = if let Some(ref id) = strategy_id {
            strategies.iter().find(|s| &s.id == id).cloned()
        } else {
            let last_id = crate::win32::settings::load_settings().last_strategy_id;
            last_id
                .and_then(|lid| strategies.iter().find(|s| s.id == lid).cloned())
                .or_else(|| strategies.iter().find(|s| s.id == "flowseal-alt").cloned())
                .or_else(|| strategies.first().cloned())
        };

        let active_id = target_strat
            .as_ref()
            .map(|s| s.id.clone())
            .unwrap_or_else(|| "flowseal-alt".into());

        // Remember last used strategy in settings (serialized vs. concurrent
        // mutations — see win32::settings::update_settings, L-4).
        let _ = crate::win32::settings::update_settings(|s| {
            s.last_strategy_id = Some(active_id.clone());
        });

        let use_winws1 = target_strat.as_ref().map_or(false, |s| s.engine == "winws");
        let target_exe = if use_winws1 {
            if let Some(ref w1) = binaries.winws_exe {
                w1.clone()
            } else {
                binaries.bin_dir.join("winws.exe")
            }
        } else {
            binaries.winws2_exe.clone()
        };

        *self.binary_path.write() = Some(target_exe.to_string_lossy().to_string());
        self.log(
            "info",
            &format!(
                "Using bypass engine: {} [{}]",
                target_exe.file_name().unwrap_or_default().to_string_lossy(),
                active_id
            ),
        );

        // 2. Fast pre-launch setup: enable TCP timestamps asynchronously without blocking
        if use_winws1 {
            tokio::spawn(async {
                let _ = tokio::process::Command::new(crate::win32::system32("netsh.exe"))
                    .args(["interface", "tcp", "set", "global", "timestamps=enabled"])
                    .creation_flags(0x08000000)
                    .output()
                    .await;
            });
        }

        // 3. Assemble command-line arguments (reset state on failure: the engine
        // is not Starting anymore, it failed to start)
        let args = match self.build_arguments(&binaries, Some(&active_id), custom_args) {
            Ok(args) => args,
            Err(e) => {
                self.log("error", &format!("Failed to build engine arguments: {}", e));
                *self.state.write() = SupervisorState::Error;
                self.emit_state_change(SupervisorState::Error);
                return Err(e);
            }
        };
        self.log(
            "info",
            &format!("Bypass parameters: {}", args.join(" ")),
        );

        // 4. Fast spawn process in isolated process group and bind to Job Object
        let proc_attempt = spawn_process(
            &target_exe,
            &binaries.bin_dir,
            &args,
            self.job_object.clone(),
        );

        let SupervisedProcess {
            child,
            pid,
            job: _,
            stdout,
            stderr,
        } = match proc_attempt {
            Ok(proc) => proc,
            Err(first_err) => {
                // If initial spawn failed (e.g. driver contention), perform cleanup and retry once
                self.log(
                    "warn",
                    &format!("Direct launch encountered contention ({}), executing driver cleanup retry...", first_err),
                );
                let _ = kill_stray_winws2_processes(None);
                cleanup_windivert_driver().await;

                match spawn_process(
                    &target_exe,
                    &binaries.bin_dir,
                    &args,
                    self.job_object.clone(),
                ) {
                    Ok(proc) => proc,
                    Err(e) => {
                        self.log("error", &format!("Failed to spawn {}: {}", target_exe.display(), e));
                        *self.state.write() = SupervisorState::Error;
                        self.emit_state_change(SupervisorState::Error);
                        return Err(e);
                    }
                }
            }
        };

        // Store the child BEFORE announcing Running: otherwise a concurrent stop()
        // could observe Running with no child stored yet and leave winws running
        // while the UI reports "stopped".
        *self.active_child.lock() = Some(child);
        *self.active_pid.write() = Some(pid);
        *self.active_strategy.write() = Some(active_id);
        *self.start_time.write() = Some(Instant::now());
        *self.state.write() = SupervisorState::Running;

        self.emit_state_change(SupervisorState::Running);
        self.log(
            "info",
            &format!("Bypass service active (PID: {}) assigned to Windows Job Object", pid),
        );

        // 5. Spawn asynchronous log readers
        let log_tx_stdout = self.log_tx.clone();
        let file_logger_stdout = self.file_logger.clone();
        if let Some(stdout_stream) = stdout {
            tokio::spawn(async move {
                let mut reader = BufReader::new(stdout_stream).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    file_logger_stdout.write_entry("ENGINE", &line);
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let _ = log_tx_stdout.try_send(LogEntry {
                        timestamp: now,
                        level: "stdout".into(),
                        message: line,
                    });
                }
            });
        }

        let log_tx_stderr = self.log_tx.clone();
        let file_logger_stderr = self.file_logger.clone();
        if let Some(stderr_stream) = stderr {
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr_stream).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    file_logger_stderr.write_entry("STDERR", &line);
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let _ = log_tx_stderr.try_send(LogEntry {
                        timestamp: now,
                        level: "stderr".into(),
                        message: line,
                    });
                }
            });
        }

        // 6. Spawn monitor task for process exit (reads the child via active_child)
        let supervisor_clone = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;
                let mut child_guard = supervisor_clone.active_child.lock();
                if let Some(child) = child_guard.as_mut() {
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            supervisor_clone.log(
                                "info",
                                &format!("Bypass process (PID {}) terminated with status: {}", pid, status),
                            );
                            *child_guard = None;
                            drop(child_guard);

                            tokio::spawn(async {
                                cleanup_windivert_driver().await;
                            });

                            {
                                let mut state = supervisor_clone.state.write();
                                if *state == SupervisorState::Running || *state == SupervisorState::Stopping {
                                    *state = SupervisorState::Stopped;
                                }
                                *supervisor_clone.active_pid.write() = None;
                                *supervisor_clone.start_time.write() = None;
                                // The engine is gone — drop stale strategy/binary or the UI
                                // would keep advertising a configuration that no longer runs.
                                *supervisor_clone.active_strategy.write() = None;
                                *supervisor_clone.binary_path.write() = None;
                            }

                            supervisor_clone.emit_state_change(SupervisorState::Stopped);
                            break;
                        }
                        Ok(None) => {}
                        Err(e) => {
                            supervisor_clone.log("error", &format!("Error checking process status: {}", e));
                            *child_guard = None;
                            break;
                        }
                    }
                } else {
                    break;
                }
            }
        });

        Ok(())
    }

    /// Stops the bypass engine and releases WinDivert.
    pub async fn stop(&self) -> Result<(), ProcessError> {
        // Serialized with start(): either the start fully completed before us
        // or it will run after we finish — never interleaved (see 1.1).
        let _lifecycle_guard = self.lifecycle.lock().await;

        let pid = {
            let mut state = self.state.write();
            if *state == SupervisorState::Stopped || *state == SupervisorState::Stopping {
                return Ok(());
            }
            *state = SupervisorState::Stopping;
            *self.active_pid.read()
        };

        self.emit_state_change(SupervisorState::Stopping);
        self.log("info", "Initiating shutdown of bypass engine...");

        let maybe_child = self.active_child.lock().take();
        if let Some(mut child) = maybe_child {
            if let Some(pid_val) = pid {
                self.log("info", &format!("Executing shutdown protocol on PID {}", pid_val));
                let _ = stop_winws2_gracefully(&mut child, &self.job_object).await;
            }
        }

        // Clean WinDivert driver service registrations to guarantee clean kernel state
        cleanup_windivert_driver().await;

        // Persist any buffered log data before reporting the stopped state.
        self.file_logger.flush();

        {
            *self.state.write() = SupervisorState::Stopped;
            *self.active_pid.write() = None;
            *self.start_time.write() = None;
            *self.active_strategy.write() = None;
        }

        self.emit_state_change(SupervisorState::Stopped);
        self.log("info", "zapret supervisor stopped successfully");

        Ok(())
    }

    /// Comprehensive list of domains for Discord media, gateway, CDNs, and GIF search providers (Tenor, Giphy).
    pub const DISCORD_MEDIA_DOMAINS: &str = "discord.media,gateway.discord.gg,discord.com,discordapp.com,discordapp.net,discordcdn.com,discord.gg,dis.gd,discord.co,discord.app,tenor.com,tenor.co,giphy.com";

    /// Builds Flowseal preset command-line arguments.
    /// Validates a user-supplied port range spec (e.g. `1024-65535`, `80,443`).
    /// Falls back to the safe default on any unexpected characters so arbitrary
    /// settings content can never leak into the winws command line.
    fn sanitize_port_spec(raw: &str) -> String {
        let ok = !raw.is_empty()
            && raw.len() <= 64
            && raw.chars().all(|c| c.is_ascii_digit() || c == ',' || c == '-');
        if ok {
            raw.to_string()
        } else {
            "1024-65535".to_string()
        }
    }

    fn build_flowseal_args(preset_id: &str, bin_dir: &Path, lists_dir: &Path) -> Vec<String> {
        let settings = crate::win32::settings::load_settings();
        let is_game = settings.split_tunnel_mode == crate::win32::settings::SplitTunnelMode::GameMode;
        let is_strict = settings.split_tunnel_mode == crate::win32::settings::SplitTunnelMode::Strict;

        let game_tcp = Self::sanitize_port_spec(&settings.game_filter_tcp);
        let game_udp = Self::sanitize_port_spec(&settings.game_filter_udp);

        let wf_tcp = if is_game {
            format!("--wf-tcp=80,443,2053,2083,2087,2096,8443,{}", game_tcp)
        } else {
            "--wf-tcp=80,443,2053,2083,2087,2096,8443".into()
        };

        let wf_udp = if is_game {
            format!("--wf-udp=443,19294-19344,50000-65535,{}", game_udp)
        } else {
            "--wf-udp=443,19294-19344,50000-65535".into()
        };

        let b = |file: &str| {
            let p = bin_dir.join(file);
            if p.is_file() {
                file.to_string()
            } else {
                p.to_string_lossy().replace('\\', "/")
            }
        };
        let l = |file: &str| {
            if bin_dir.join("lists").join(file).is_file() {
                format!("lists/{}", file)
            } else {
                lists_dir.join(file).to_string_lossy().replace('\\', "/")
            }
        };

        let mut args = match preset_id {
            "flowseal-alt11" => vec![
                wf_tcp.clone(),
                wf_udp.clone(),
                "--filter-udp=443".into(),
                format!("--hostlist={}", l("list-general.txt")),
                format!("--hostlist={}", l("list-general-user.txt")),
                format!("--hostlist-exclude={}", l("list-exclude.txt")),
                format!("--hostlist-exclude={}", l("list-exclude-user.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude-user.txt")),
                "--dpi-desync=fake".into(),
                "--dpi-desync-repeats=11".into(),
                format!("--dpi-desync-fake-quic={}", b("quic_initial_www_google_com.bin")),
                "--new".into(),
                "--filter-udp=19294-19344,50000-65535".into(),
                "--dpi-desync=fake".into(),
                "--dpi-desync-any-protocol=1".into(),
                "--dpi-desync-cutoff=n4".into(),
                format!("--dpi-desync-fake-discord={}", b("ACTIVE_DISCORD_UDP.bin")),
                format!("--dpi-desync-fake-stun={}", b("ACTIVE_DISCORD_UDP.bin")),
                format!("--dpi-desync-fake-unknown-udp={}", b("ACTIVE_DISCORD_UDP.bin")),
                "--dpi-desync-repeats=6".into(),
                "--new".into(),
                "--filter-tcp=443,2053,2083,2087,2096,8443".into(),
                format!("--hostlist-domains={}", Self::DISCORD_MEDIA_DOMAINS),
                "--dpi-desync=fake,multisplit".into(),
                "--dpi-desync-split-seqovl=681".into(),
                "--dpi-desync-split-pos=1".into(),
                "--dpi-desync-fooling=ts".into(),
                "--dpi-desync-repeats=8".into(),
                format!("--dpi-desync-split-seqovl-pattern={}", b("tls_clienthello_www_google_com.bin")),
                format!("--dpi-desync-fake-tls={}", b("tls_clienthello_www_google_com.bin")),
                "--new".into(),
                "--filter-tcp=443".into(),
                format!("--hostlist={}", l("list-google.txt")),
                "--ip-id=zero".into(),
                "--dpi-desync=fake,multisplit".into(),
                "--dpi-desync-split-seqovl=681".into(),
                "--dpi-desync-split-pos=1".into(),
                "--dpi-desync-fooling=ts".into(),
                "--dpi-desync-repeats=8".into(),
                format!("--dpi-desync-split-seqovl-pattern={}", b("tls_clienthello_www_google_com.bin")),
                format!("--dpi-desync-fake-tls={}", b("tls_clienthello_www_google_com.bin")),
                "--new".into(),
                "--filter-tcp=80,443".into(),
                format!("--hostlist={}", l("list-general.txt")),
                format!("--hostlist={}", l("list-general-user.txt")),
                format!("--hostlist-exclude={}", l("list-exclude.txt")),
                format!("--hostlist-exclude={}", l("list-exclude-user.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude-user.txt")),
                "--dpi-desync=fake,multisplit".into(),
                "--dpi-desync-split-seqovl=664".into(),
                "--dpi-desync-split-pos=1".into(),
                "--dpi-desync-fooling=ts".into(),
                "--dpi-desync-repeats=8".into(),
                format!("--dpi-desync-split-seqovl-pattern={}", b("tls_clienthello_max_ru.bin")),
                format!("--dpi-desync-fake-tls={}", b("stun2.bin")),
                format!("--dpi-desync-fake-tls={}", b("tls_clienthello_max_ru.bin")),
                format!("--dpi-desync-fake-http={}", b("tls_clienthello_max_ru.bin")),
                "--new".into(),
                "--filter-udp=443".into(),
                format!("--ipset={}", l("ipset-all.txt")),
                format!("--hostlist-exclude={}", l("list-exclude.txt")),
                format!("--hostlist-exclude={}", l("list-exclude-user.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude-user.txt")),
                "--dpi-desync=fake".into(),
                "--dpi-desync-repeats=11".into(),
                format!("--dpi-desync-fake-quic={}", b("quic_initial_www_google_com.bin")),
                "--new".into(),
                "--filter-tcp=80,443,8443".into(),
                format!("--ipset={}", l("ipset-all.txt")),
                format!("--hostlist-exclude={}", l("list-exclude.txt")),
                format!("--hostlist-exclude={}", l("list-exclude-user.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude-user.txt")),
                "--dpi-desync=fake,multisplit".into(),
                "--dpi-desync-split-seqovl=664".into(),
                "--dpi-desync-split-pos=1".into(),
                "--dpi-desync-fooling=ts".into(),
                "--dpi-desync-repeats=8".into(),
                format!("--dpi-desync-split-seqovl-pattern={}", b("tls_clienthello_max_ru.bin")),
                format!("--dpi-desync-fake-tls={}", b("stun2.bin")),
                format!("--dpi-desync-fake-tls={}", b("tls_clienthello_max_ru.bin")),
                format!("--dpi-desync-fake-http={}", b("tls_clienthello_max_ru.bin")),
            ],
            "flowseal-alt" => vec![
                wf_tcp.clone(),
                wf_udp.clone(),
                "--filter-udp=443".into(),
                format!("--hostlist={}", l("list-general.txt")),
                format!("--hostlist={}", l("list-general-user.txt")),
                format!("--hostlist-exclude={}", l("list-exclude.txt")),
                format!("--hostlist-exclude={}", l("list-exclude-user.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude-user.txt")),
                "--dpi-desync=fake".into(),
                "--dpi-desync-repeats=6".into(),
                format!("--dpi-desync-fake-quic={}", b("quic_initial_www_google_com.bin")),
                "--new".into(),
                "--filter-udp=19294-19344,50000-65535".into(),
                "--dpi-desync=fake".into(),
                "--dpi-desync-any-protocol=1".into(),
                "--dpi-desync-cutoff=n4".into(),
                format!("--dpi-desync-fake-discord={}", b("ACTIVE_DISCORD_UDP.bin")),
                format!("--dpi-desync-fake-stun={}", b("ACTIVE_DISCORD_UDP.bin")),
                format!("--dpi-desync-fake-unknown-udp={}", b("ACTIVE_DISCORD_UDP.bin")),
                "--dpi-desync-repeats=6".into(),
                "--new".into(),
                "--filter-tcp=443,2053,2083,2087,2096,8443".into(),
                format!("--hostlist-domains={}", Self::DISCORD_MEDIA_DOMAINS),
                "--dpi-desync=fake,multisplit".into(),
                "--dpi-desync-split-pos=1".into(),
                "--dpi-desync-repeats=6".into(),
                "--dpi-desync-fooling=ts".into(),
                format!("--dpi-desync-fake-tls={}", b("tls_clienthello_www_google_com.bin")),
                "--new".into(),
                "--filter-tcp=443".into(),
                format!("--hostlist={}", l("list-google.txt")),
                "--ip-id=zero".into(),
                "--dpi-desync=fake,fakedsplit".into(),
                "--dpi-desync-repeats=6".into(),
                "--dpi-desync-fooling=ts".into(),
                "--dpi-desync-fakedsplit-pattern=0x00".into(),
                format!("--dpi-desync-fake-tls={}", b("tls_clienthello_www_google_com.bin")),
                "--new".into(),
                "--filter-tcp=80,443".into(),
                format!("--hostlist={}", l("list-general.txt")),
                format!("--hostlist={}", l("list-general-user.txt")),
                format!("--hostlist-exclude={}", l("list-exclude.txt")),
                format!("--hostlist-exclude={}", l("list-exclude-user.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude-user.txt")),
                "--dpi-desync=fake,fakedsplit".into(),
                "--dpi-desync-repeats=6".into(),
                "--dpi-desync-fooling=ts".into(),
                "--dpi-desync-fakedsplit-pattern=0x00".into(),
                format!("--dpi-desync-fake-tls={}", b("stun.bin")),
                format!("--dpi-desync-fake-tls={}", b("tls_clienthello_www_google_com.bin")),
                format!("--dpi-desync-fake-http={}", b("tls_clienthello_max_ru.bin")),
                "--new".into(),
                "--filter-udp=443".into(),
                format!("--ipset={}", l("ipset-all.txt")),
                format!("--hostlist-exclude={}", l("list-exclude.txt")),
                format!("--hostlist-exclude={}", l("list-exclude-user.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude-user.txt")),
                "--dpi-desync=fake".into(),
                "--dpi-desync-repeats=6".into(),
                format!("--dpi-desync-fake-quic={}", b("quic_initial_www_google_com.bin")),
                "--new".into(),
                "--filter-tcp=80,443,8443".into(),
                format!("--ipset={}", l("ipset-all.txt")),
                format!("--hostlist-exclude={}", l("list-exclude.txt")),
                format!("--hostlist-exclude={}", l("list-exclude-user.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude-user.txt")),
                "--dpi-desync=fake,fakedsplit".into(),
                "--dpi-desync-repeats=6".into(),
                "--dpi-desync-fooling=ts".into(),
                "--dpi-desync-fakedsplit-pattern=0x00".into(),
                format!("--dpi-desync-fake-tls={}", b("stun.bin")),
                format!("--dpi-desync-fake-tls={}", b("tls_clienthello_www_google_com.bin")),
                format!("--dpi-desync-fake-http={}", b("tls_clienthello_max_ru.bin")),
            ],
            "flowseal-fake-tls-auto" => vec![
                wf_tcp.clone(),
                wf_udp.clone(),
                "--filter-udp=443".into(),
                format!("--hostlist={}", l("list-general.txt")),
                format!("--hostlist={}", l("list-general-user.txt")),
                format!("--hostlist-exclude={}", l("list-exclude.txt")),
                format!("--hostlist-exclude={}", l("list-exclude-user.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude-user.txt")),
                "--dpi-desync=fake".into(),
                "--dpi-desync-repeats=11".into(),
                format!("--dpi-desync-fake-quic={}", b("quic_initial_www_google_com.bin")),
                "--new".into(),
                "--filter-udp=19294-19344,50000-65535".into(),
                "--dpi-desync=fake".into(),
                "--dpi-desync-any-protocol=1".into(),
                "--dpi-desync-cutoff=n4".into(),
                format!("--dpi-desync-fake-discord={}", b("ACTIVE_DISCORD_UDP.bin")),
                format!("--dpi-desync-fake-stun={}", b("ACTIVE_DISCORD_UDP.bin")),
                format!("--dpi-desync-fake-unknown-udp={}", b("ACTIVE_DISCORD_UDP.bin")),
                "--dpi-desync-repeats=6".into(),
                "--new".into(),
                "--filter-tcp=443,2053,2083,2087,2096,8443".into(),
                format!("--hostlist-domains={}", Self::DISCORD_MEDIA_DOMAINS),
                "--dpi-desync=fake,multidisorder".into(),
                "--dpi-desync-split-pos=1,midsld".into(),
                "--dpi-desync-repeats=11".into(),
                "--dpi-desync-fooling=badseq".into(),
                "--dpi-desync-fake-tls=0x00000000".into(),
                "--dpi-desync-fake-tls=!".into(),
                "--dpi-desync-fake-tls-mod=rnd,dupsid,sni=www.google.com".into(),
                "--new".into(),
                "--filter-tcp=443".into(),
                format!("--hostlist={}", l("list-google.txt")),
                "--ip-id=zero".into(),
                "--dpi-desync=fake,multidisorder".into(),
                "--dpi-desync-split-pos=1,midsld".into(),
                "--dpi-desync-repeats=11".into(),
                "--dpi-desync-fooling=badseq".into(),
                "--dpi-desync-fake-tls=0x00000000".into(),
                "--dpi-desync-fake-tls=!".into(),
                "--dpi-desync-fake-tls-mod=rnd,dupsid,sni=www.google.com".into(),
                "--new".into(),
                "--filter-tcp=80,443".into(),
                format!("--hostlist={}", l("list-general.txt")),
                format!("--hostlist={}", l("list-general-user.txt")),
                format!("--hostlist-exclude={}", l("list-exclude.txt")),
                format!("--hostlist-exclude={}", l("list-exclude-user.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude-user.txt")),
                "--dpi-desync=fake,multidisorder".into(),
                "--dpi-desync-split-pos=1,midsld".into(),
                "--dpi-desync-repeats=11".into(),
                "--dpi-desync-fooling=badseq".into(),
                "--dpi-desync-fake-tls=0x00000000".into(),
                "--dpi-desync-fake-tls=!".into(),
                "--dpi-desync-fake-tls-mod=rnd,dupsid,sni=www.google.com".into(),
                format!("--dpi-desync-fake-http={}", b("tls_clienthello_max_ru.bin")),
                "--new".into(),
                "--filter-udp=443".into(),
                format!("--ipset={}", l("ipset-all.txt")),
                format!("--hostlist-exclude={}", l("list-exclude.txt")),
                format!("--hostlist-exclude={}", l("list-exclude-user.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude-user.txt")),
                "--dpi-desync=fake".into(),
                "--dpi-desync-repeats=11".into(),
                format!("--dpi-desync-fake-quic={}", b("quic_initial_www_google_com.bin")),
                "--new".into(),
                "--filter-tcp=80,443,8443".into(),
                format!("--ipset={}", l("ipset-all.txt")),
                format!("--hostlist-exclude={}", l("list-exclude.txt")),
                format!("--hostlist-exclude={}", l("list-exclude-user.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude-user.txt")),
                "--dpi-desync=fake,multidisorder".into(),
                "--dpi-desync-split-pos=1,midsld".into(),
                "--dpi-desync-repeats=11".into(),
                "--dpi-desync-fooling=badseq".into(),
                "--dpi-desync-fake-tls=0x00000000".into(),
                "--dpi-desync-fake-tls=!".into(),
                "--dpi-desync-fake-tls-mod=rnd,dupsid,sni=www.google.com".into(),
                format!("--dpi-desync-fake-http={}", b("tls_clienthello_max_ru.bin")),
            ],
            "flowseal-simple-fake" => vec![
                wf_tcp.clone(),
                wf_udp.clone(),
                "--filter-udp=443".into(),
                format!("--hostlist={}", l("list-general.txt")),
                format!("--hostlist={}", l("list-general-user.txt")),
                format!("--hostlist-exclude={}", l("list-exclude.txt")),
                format!("--hostlist-exclude={}", l("list-exclude-user.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude-user.txt")),
                "--dpi-desync=fake".into(),
                "--dpi-desync-repeats=6".into(),
                format!("--dpi-desync-fake-quic={}", b("quic_initial_www_google_com.bin")),
                "--new".into(),
                "--filter-udp=19294-19344,50000-65535".into(),
                "--dpi-desync=fake".into(),
                "--dpi-desync-any-protocol=1".into(),
                "--dpi-desync-cutoff=n4".into(),
                format!("--dpi-desync-fake-discord={}", b("ACTIVE_DISCORD_UDP.bin")),
                format!("--dpi-desync-fake-stun={}", b("ACTIVE_DISCORD_UDP.bin")),
                format!("--dpi-desync-fake-unknown-udp={}", b("ACTIVE_DISCORD_UDP.bin")),
                "--dpi-desync-repeats=6".into(),
                "--new".into(),
                "--filter-tcp=443,2053,2083,2087,2096,8443".into(),
                format!("--hostlist-domains={}", Self::DISCORD_MEDIA_DOMAINS),
                "--dpi-desync=fake".into(),
                "--dpi-desync-repeats=6".into(),
                "--dpi-desync-fooling=ts".into(),
                format!("--dpi-desync-fake-tls={}", b("tls_clienthello_www_google_com.bin")),
                "--new".into(),
                "--filter-tcp=443".into(),
                format!("--hostlist={}", l("list-google.txt")),
                "--ip-id=zero".into(),
                "--dpi-desync=hostfakesplit".into(),
                "--dpi-desync-fooling=ts".into(),
                "--dpi-desync-hostfakesplit-mod=host=www.google.com".into(),
                "--new".into(),
                "--filter-tcp=80,443".into(),
                format!("--hostlist={}", l("list-general.txt")),
                format!("--hostlist={}", l("list-general-user.txt")),
                format!("--hostlist-exclude={}", l("list-exclude.txt")),
                format!("--hostlist-exclude={}", l("list-exclude-user.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude-user.txt")),
                "--dpi-desync=fake".into(),
                "--dpi-desync-repeats=6".into(),
                "--dpi-desync-fooling=ts".into(),
                format!("--dpi-desync-fake-tls={}", b("tls_clienthello_www_google_com.bin")),
                format!("--dpi-desync-fake-http={}", b("tls_clienthello_max_ru.bin")),
                "--new".into(),
                "--filter-udp=443".into(),
                format!("--ipset={}", l("ipset-all.txt")),
                format!("--hostlist-exclude={}", l("list-exclude.txt")),
                format!("--hostlist-exclude={}", l("list-exclude-user.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude-user.txt")),
                "--dpi-desync=fake".into(),
                "--dpi-desync-repeats=6".into(),
                format!("--dpi-desync-fake-quic={}", b("quic_initial_www_google_com.bin")),
                "--new".into(),
                "--filter-tcp=80,443,8443".into(),
                format!("--ipset={}", l("ipset-all.txt")),
                format!("--hostlist-exclude={}", l("list-exclude.txt")),
                format!("--hostlist-exclude={}", l("list-exclude-user.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude-user.txt")),
                "--dpi-desync=fake".into(),
                "--dpi-desync-repeats=6".into(),
                "--dpi-desync-fooling=ts".into(),
                format!("--dpi-desync-fake-tls={}", b("tls_clienthello_www_google_com.bin")),
                format!("--dpi-desync-fake-http={}", b("tls_clienthello_max_ru.bin")),
            ],
            _ /* flowseal-general */ => vec![
                wf_tcp.clone(),
                wf_udp.clone(),
                "--filter-udp=443".into(),
                format!("--hostlist={}", l("list-general.txt")),
                format!("--hostlist={}", l("list-general-user.txt")),
                format!("--hostlist-exclude={}", l("list-exclude.txt")),
                format!("--hostlist-exclude={}", l("list-exclude-user.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude-user.txt")),
                "--dpi-desync=fake".into(),
                "--dpi-desync-repeats=6".into(),
                format!("--dpi-desync-fake-quic={}", b("quic_initial_www_google_com.bin")),
                "--new".into(),
                "--filter-udp=19294-19344,50000-65535".into(),
                "--dpi-desync=fake".into(),
                "--dpi-desync-any-protocol=1".into(),
                "--dpi-desync-cutoff=n4".into(),
                format!("--dpi-desync-fake-discord={}", b("ACTIVE_DISCORD_UDP.bin")),
                format!("--dpi-desync-fake-stun={}", b("ACTIVE_DISCORD_UDP.bin")),
                format!("--dpi-desync-fake-unknown-udp={}", b("ACTIVE_DISCORD_UDP.bin")),
                "--dpi-desync-repeats=6".into(),
                "--new".into(),
                "--filter-tcp=443,2053,2083,2087,2096,8443".into(),
                format!("--hostlist-domains={}", Self::DISCORD_MEDIA_DOMAINS),
                "--dpi-desync=multisplit".into(),
                "--dpi-desync-split-seqovl=681".into(),
                "--dpi-desync-split-pos=1".into(),
                format!("--dpi-desync-split-seqovl-pattern={}", b("tls_clienthello_www_google_com.bin")),
                "--new".into(),
                "--filter-tcp=443".into(),
                format!("--hostlist={}", l("list-google.txt")),
                "--ip-id=zero".into(),
                "--dpi-desync=multisplit".into(),
                "--dpi-desync-split-seqovl=681".into(),
                "--dpi-desync-split-pos=1".into(),
                format!("--dpi-desync-split-seqovl-pattern={}", b("tls_clienthello_www_google_com.bin")),
                "--new".into(),
                "--filter-tcp=80,443".into(),
                format!("--hostlist={}", l("list-general.txt")),
                format!("--hostlist={}", l("list-general-user.txt")),
                format!("--hostlist-exclude={}", l("list-exclude.txt")),
                format!("--hostlist-exclude={}", l("list-exclude-user.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude-user.txt")),
                "--dpi-desync=multisplit".into(),
                "--dpi-desync-split-seqovl=568".into(),
                "--dpi-desync-split-pos=1".into(),
                format!("--dpi-desync-split-seqovl-pattern={}", b("tls_clienthello_4pda_to.bin")),
                "--new".into(),
                "--filter-udp=443".into(),
                format!("--ipset={}", l("ipset-all.txt")),
                format!("--hostlist-exclude={}", l("list-exclude.txt")),
                format!("--hostlist-exclude={}", l("list-exclude-user.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude-user.txt")),
                "--dpi-desync=fake".into(),
                "--dpi-desync-repeats=6".into(),
                format!("--dpi-desync-fake-quic={}", b("quic_initial_www_google_com.bin")),
                "--new".into(),
                "--filter-tcp=80,443,8443".into(),
                format!("--ipset={}", l("ipset-all.txt")),
                format!("--hostlist-exclude={}", l("list-exclude.txt")),
                format!("--hostlist-exclude={}", l("list-exclude-user.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude-user.txt")),
                "--dpi-desync=multisplit".into(),
                "--dpi-desync-split-seqovl=568".into(),
                "--dpi-desync-split-pos=1".into(),
            ],
        };

        // 1. If Game Mode is active, attach game port filtering and ACTIVE_GAME_UDP desync
        // SECURITY (L-4): game port specs come from user settings — sanitize them
        // like everywhere else so a malformed value cannot break engine startup.
        let game_tcp = Self::sanitize_port_spec(&settings.game_filter_tcp);
        let game_udp = Self::sanitize_port_spec(&settings.game_filter_udp);
        if is_game {
            args.extend([
                "--new".into(),
                format!("--filter-tcp={}", game_tcp),
                format!("--ipset={}", l("ipset-all.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude-user.txt")),
                "--dpi-desync=fake,fakedsplit".into(),
                "--dpi-desync-repeats=6".into(),
                "--dpi-desync-any-protocol=1".into(),
                "--dpi-desync-cutoff=n4".into(),
                "--dpi-desync-fooling=ts".into(),
                "--dpi-desync-fakedsplit-pattern=0x00".into(),
                format!("--dpi-desync-fake-tls={}", b("stun.bin")),
                format!("--dpi-desync-fake-tls={}", b("tls_clienthello_www_google_com.bin")),
                format!("--dpi-desync-fake-http={}", b("tls_clienthello_max_ru.bin")),
                format!("--dpi-desync-fake-unknown={}", b("stun.bin")),
                format!("--dpi-desync-fake-unknown={}", b("tls_clienthello_www_google_com.bin")),
                "--new".into(),
                format!("--filter-udp={}", game_udp),
                format!("--ipset={}", l("ipset-all.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude.txt")),
                format!("--ipset-exclude={}", l("ipset-exclude-user.txt")),
                "--dpi-desync=fake".into(),
                "--dpi-desync-repeats=12".into(),
                "--dpi-desync-any-protocol=1".into(),
                format!("--dpi-desync-fake-unknown-udp={}", b("ACTIVE_GAME_UDP.bin")),
                "--dpi-desync-cutoff=n3".into(),
            ]);
        }

        // 2. If Strict Split-Tunneling is active, bypass all other websites and ipset-all
        if is_strict {
            let mut filtered = Vec::new();
            let mut current_block = Vec::new();
            for arg in args {
                if arg == "--new" {
                    let has_general = current_block.iter().any(|a: &String| a.contains("list-general.txt") || a.contains("ipset-all.txt"));
                    if !has_general {
                        if !filtered.is_empty() {
                            filtered.push("--new".to_string());
                        }
                        filtered.extend(current_block);
                    }
                    current_block = Vec::new();
                } else if !arg.starts_with("--wf-tcp") && !arg.starts_with("--wf-udp") {
                    current_block.push(arg);
                }
            }
            if !current_block.is_empty() {
                let has_general = current_block.iter().any(|a: &String| a.contains("list-general.txt") || a.contains("ipset-all.txt"));
                if !has_general {
                    if !filtered.is_empty() {
                        filtered.push("--new".to_string());
                    }
                    filtered.extend(current_block);
                }
            }
            let mut final_args = vec![wf_tcp.clone(), wf_udp.clone()];
            final_args.extend(filtered);
            args = final_args;
        }

        // 3. Ensure WinDivert root port filters are updated
        if let Some(first) = args.first_mut() {
            if first.starts_with("--wf-tcp") {
                *first = wf_tcp;
            }
        }
        if args.len() > 1 && args[1].starts_with("--wf-udp") {
            args[1] = wf_udp;
        }

        args
    }

    /// Builds zapret2 Ultimate configuration for Russia on winws2 v2.
    /// Builds zapret2 Ultimate configuration for Russia on winws2 v2.
    fn build_zapret2_ru_ultimate_args(_bin_dir: &Path, _lists_dir: &Path) -> Vec<String> {
        vec![
            "--wf-tcp-out=80,443,2053,2083,2087,2096,8443".into(),
            "--wf-udp-out=443,19294-19344,50000-65535".into(),

            // Lua library initialization (relative to bin_dir)
            "--lua-init=@zapret-lib.lua".into(),
            "--lua-init=@zapret-antidpi.lua".into(),

            // Load pre-crafted binary packet payloads
            "--blob=quic_google:@quic_initial_www_google_com.bin".into(),
            "--blob=discord_udp:@ACTIVE_DISCORD_UDP.bin".into(),
            "--blob=tls_google:@tls_clienthello_www_google_com.bin".into(),
            "--blob=tls_max_ru:@tls_clienthello_max_ru.bin".into(),
            "--blob=stun2:@stun2.bin".into(),

            // 1. YouTube QUIC Handshake Bypass (UDP 443)
            "--new=quic_youtube".into(),
            "--filter-udp=443".into(),
            "--hostlist=lists/list-general.txt".into(),
            "--hostlist=lists/list-general-user.txt".into(),
            "--hostlist-exclude=lists/list-exclude.txt".into(),
            "--payload=quic_initial".into(),
            "--lua-desync=fake:blob=quic_google:repeats=11".into(),

            // 2. Discord Voice UDP & STUN Media (19294-19344, 50000-65535)
            "--new=discord_voice".into(),
            "--filter-udp=19294-19344,50000-65535".into(),
            "--payload=all".into(),
            "--out-range=n1-n4".into(),
            "--lua-desync=fake:blob=discord_udp:repeats=6".into(),

            // 3. Discord Media & Voice Gateway TCP (443, 2053, 2083, 2087, 2096, 8443)
            "--new=discord_media".into(),
            "--filter-tcp=443,2053,2083,2087,2096,8443".into(),
            format!("--hostlist-domains={}", Self::DISCORD_MEDIA_DOMAINS),
            "--payload=tls_client_hello".into(),
            "--out-range=n1-n4".into(),
            "--lua-desync=fake:blob=tls_google:repeats=8:tcp_ts=1000".into(),
            "--lua-desync=multisplit:pos=2:seqovl=681:seqovl_pattern=tls_google:tcp_ts=1000".into(),

            // 4. YouTube 4K Google TCP (443) - seqovl=681 + TLS Fake Google + out-range=n1-n4 for 60FPS
            "--new=youtube_4k".into(),
            "--filter-tcp=443".into(),
            "--hostlist=lists/list-google.txt".into(),
            "--payload=tls_client_hello".into(),
            "--out-range=n1-n4".into(),
            "--lua-desync=fake:blob=tls_google:repeats=8:tcp_ts=1000:ip_id=zero".into(),
            "--lua-desync=multisplit:pos=2:seqovl=681:seqovl_pattern=tls_google:tcp_ts=1000:ip_id=zero".into(),

            // 5. General Blocked Websites HTTPS (80,443) - seqovl=664 + TLS Fake max_ru
            "--new=general_https".into(),
            "--filter-tcp=80,443".into(),
            "--hostlist=lists/list-general.txt".into(),
            "--hostlist=lists/list-general-user.txt".into(),
            "--hostlist-exclude=lists/list-exclude.txt".into(),
            "--payload=tls_client_hello".into(),
            "--out-range=n1-n4".into(),
            "--lua-desync=fake:blob=tls_max_ru:repeats=8:tcp_ts=1000".into(),
            "--lua-desync=multisplit:pos=2:seqovl=664:seqovl_pattern=tls_max_ru:tcp_ts=1000".into(),

            // 6. General HTTP (80)
            "--new=general_http".into(),
            "--filter-tcp=80".into(),
            "--hostlist=lists/list-general.txt".into(),
            "--payload=http_req".into(),
            "--lua-desync=fake:blob=tls_max_ru:tcp_ts=1000".into(),
            "--lua-desync=multisplit:pos=2:seqovl=664:seqovl_pattern=tls_max_ru:tcp_ts=1000".into(),

            // 7. IPSet All Fallback
            "--new=ipset_fallback".into(),
            "--filter-tcp=80,443,8443".into(),
            "--ipset=lists/ipset-all.txt".into(),
            "--hostlist-exclude=lists/list-exclude.txt".into(),
            "--payload=tls_client_hello".into(),
            "--out-range=n1-n4".into(),
            "--lua-desync=fake:blob=tls_max_ru:repeats=8:tcp_ts=1000".into(),
            "--lua-desync=multisplit:pos=2:seqovl=664:seqovl_pattern=tls_max_ru:tcp_ts=1000".into(),
        ]
    }

    /// Builds command arguments based on selected strategy or custom parameters.
    fn build_arguments(
        &self,
        binaries: &ZapretBinaries,
        strategy_id: Option<&str>,
        custom_args: Option<Vec<String>>,
    ) -> Result<Vec<String>, ProcessError> {
        let mut final_args = if let Some(args) = custom_args {
            if !args.is_empty() {
                args
            } else {
                self.resolve_strategy_args(binaries, strategy_id)?
            }
        } else {
            self.resolve_strategy_args(binaries, strategy_id)?
        };

        // Ensure --debug=1 is enabled so winws/winws2 outputs runtime logs
        if !final_args.iter().any(|a| a.starts_with("--debug")) {
            final_args.insert(0, "--debug=1".to_string());
        }

        Ok(final_args)
    }

    /// Resolves preset or custom strategy command-line arguments.
    fn resolve_strategy_args(
        &self,
        binaries: &ZapretBinaries,
        strategy_id: Option<&str>,
    ) -> Result<Vec<String>, ProcessError> {
        let strat_id = strategy_id.unwrap_or("flowseal-alt");

        let lists_dir = binaries
            .lists_dir
            .clone()
            .unwrap_or_else(|| binaries.bin_dir.join("../lists"));

        Self::ensure_user_lists_exist(&lists_dir);

        // 1. If it's a Flowseal preset, build Flowseal arguments using resolved directories
        if strat_id.starts_with("flowseal-") {
            return Ok(Self::build_flowseal_args(strat_id, &binaries.bin_dir, &lists_dir));
        }

        // 2. If it's the flagship zapret2 Ultimate Russia preset, build winws2 arguments
        if strat_id == "zapret2-ru-ultimate" {
            return Ok(Self::build_zapret2_ru_ultimate_args(&binaries.bin_dir, &lists_dir));
        }

        let strategies = discover_strategies().unwrap_or_default();
        let target_strategy = strategies.iter().find(|s| s.id == strat_id).cloned();

        // 3. If it's a custom profile with defined arguments, use them directly
        if let Some(ref strat) = target_strategy {
            if strat.category == "custom" {
                if let Some(ref c_args) = strat.custom_args {
                    if !c_args.is_empty() {
                        return Ok(c_args.clone());
                    }
                }
            }
        }

        // 4. Otherwise, assemble Lua arguments for zapret2 (winws2)
        let mut args = Vec::new();
        args.push("--wf-tcp-out=80,443,2053,2083,2087,2096,8443".into());
        args.push("--wf-udp-out=443,19294-19344,50000-65535".into());
        args.push("--filter-tcp=80,443,2053,2083,2087,2096,8443".into());

        let lib_lua = strategies
            .iter()
            .find(|s| s.filename == "zapret-lib.lua")
            .map(|s| s.path.clone());

        if binaries.bin_dir.join("zapret-lib.lua").is_file() {
            args.push("--lua-init=@zapret-lib.lua".into());
        } else if let Some(ref lib) = lib_lua {
            args.push(format!("--lua-init=@{}", lib.to_string_lossy()));
        }

        if let Some(strat) = target_strategy {
            // SECURITY (M-5): a custom profile without explicit custom_args has
            // no trustworthy Lua script — its path/filename are server-generated
            // placeholders (see save_custom_profile), never a real script.
            // Refuse fail-closed instead of loading an IPC-supplied path into
            // the privileged engine's command line.
            if strat.category == "custom" {
                return Err(ProcessError::InvalidStrategy(
                    "Кастомный профиль не содержит аргументов запуска".to_string(),
                ));
            }
            if binaries.bin_dir.join(&strat.filename).is_file() {
                if strat.filename != "zapret-lib.lua" {
                    args.push(format!("--lua-init=@{}", strat.filename));
                }
            } else if Some(&strat.path) != lib_lua.as_ref() {
                args.push(format!("--lua-init=@{}", strat.path.to_string_lossy()));
            }

            args.push("--payload=tls_client_hello".into());
            args.push("--lua-desync=fake:blob=fake_default_tls:tcp_md5:tls_mod=rnd,rndsni,dupsid".into());
            args.push("--payload=http_req".into());
            args.push("--lua-desync=fake:blob=fake_default_http:tcp_md5".into());
            args.push("--payload=tls_client_hello,http_req".into());
            args.push("--lua-desync=multisplit:pos=2:seqovl=5:seqovl_pattern=0x1603030000".into());
        }

        Ok(args)
    }

    fn ensure_user_lists_exist(lists_dir: &Path) {
        if !lists_dir.exists() {
            let _ = std::fs::create_dir_all(lists_dir);
        }
        let defaults = [
            ("list-general-user.txt", "domain.example.abc\n"),
            ("list-exclude-user.txt", "domain.example.abc\n"),
            ("ipset-exclude-user.txt", "203.0.113.113/32\n"),
        ];
        for (filename, default_content) in defaults {
            let file_path = lists_dir.join(filename);
            if !file_path.exists() {
                let _ = std::fs::write(&file_path, default_content);
            }
        }
    }

    fn emit_state_change(&self, state: SupervisorState) {
        if let Some(handle) = self.app_handle.read().as_ref() {
            let _ = handle.emit("state-changed", state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strict_split_tunnel_must_preserve_wf_filters() {
        let args = vec![
            "--wf-tcp=80,443".to_string(),
            "--wf-udp=443,50000-65535".to_string(),
            "--filter-tcp=80,443".to_string(),
            "--hostlist=lists/list-general.txt".to_string(),
            "--dpi-desync=fake".to_string(),
            "--new".to_string(),
            "--filter-tcp=443".to_string(),
            "--hostlist=lists/list-google.txt".to_string(),
            "--dpi-desync=fake".to_string(),
            "--new".to_string(),
            "--filter-udp=50000-65535".to_string(),
            "--dpi-desync=fake".to_string(),
        ];

        let wf_tcp = "--wf-tcp=80,443".to_string();
        let wf_udp = "--wf-udp=443,19294-19344,50000-65535".to_string();

        let mut filtered = Vec::new();
        let mut current_block = Vec::new();
        for arg in args {
            if arg == "--new" {
                let has_general = current_block.iter().any(|a: &String| a.contains("list-general.txt") || a.contains("ipset-all.txt"));
                if !has_general {
                    if !filtered.is_empty() {
                        filtered.push("--new".to_string());
                    }
                    filtered.extend(current_block);
                }
                current_block = Vec::new();
            } else if !arg.starts_with("--wf-tcp") && !arg.starts_with("--wf-udp") {
                current_block.push(arg);
            }
        }
        if !current_block.is_empty() {
            let has_general = current_block.iter().any(|a: &String| a.contains("list-general.txt") || a.contains("ipset-all.txt"));
            if !has_general {
                if !filtered.is_empty() {
                    filtered.push("--new".to_string());
                }
                filtered.extend(current_block);
            }
        }
        let mut final_args = vec![wf_tcp.clone(), wf_udp.clone()];
        final_args.extend(filtered);

        assert!(final_args.len() >= 2);
        assert_eq!(final_args[0], wf_tcp);
        assert_eq!(final_args[1], wf_udp);
        assert!(!final_args.iter().any(|a| a.contains("list-general.txt")));
        assert!(final_args.iter().any(|a| a.contains("list-google.txt")));
        assert!(final_args.iter().any(|a| a.contains("50000-65535")));
    }
}


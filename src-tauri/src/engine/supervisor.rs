//! zapret2 Supervisor Engine & High-Throughput Micro-Batching Log Pipeline
//!
//! # Responsibilities
//! 1. Life-cycle orchestration (`Stopped` -> `Starting` -> `Running` -> `Stopping` -> `Error`).
//! 2. Spawns `winws2.exe` with selected Lua strategies and WFP filter rules.
//! 3. Binds process to the Windows Job Object immediately.
//! 4. Asynchronously reads stdout/stderr line-by-line via `AsyncBufReadExt`.
//! 5. Ring Buffer + Micro-Batching Engine: Buffers lines and flushes batches every 150 ms
//!    to the installed [`EventSink`](crate::engine::events::EventSink)
//!    (Tauri frontend channel `"log-batch"` in GUI mode).
//! 6. Enforces strict memory footprint by capping the ring buffer to 1,000 entries.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use crate::engine::events::{EventSink, SinkSlot};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc::{channel, unbounded_channel, Receiver, Sender, UnboundedReceiver, UnboundedSender};
use tokio::time::interval;

use crate::engine::logger::FileLogger;
use crate::engine::managed::ManagedSidecar;
use crate::engine::probes::DiagnosticsService;
use crate::engine::singbox::{SingBoxManager, SingBoxStartOptions, SingBoxStatus};
use crate::engine::singbox_config::TunScope;
use crate::platform::job_object::JobObject;
use crate::platform::process::{
    cleanup_windivert_driver, discover_strategies, kill_stray_singbox_processes,
    kill_stray_winws2_processes, resolve_binaries, spawn_process, spawn_winws2, stop_winws2,
    stop_winws2_gracefully, ProcessError, SupervisedProcess, ZapretBinaries,
};

/// Maximum log capacity in RAM to ensure memory consumption stays well below 20 MB.
const MAX_LOG_CAPACITY: usize = 1_000;

/// Whether this platform applies WARP launch arguments to the desync
/// engine (stage 4). Windows only: `--wf-raw-filter` is a WinDivert
/// flag (nfqws2 rejects every `--wf-*` argument on Linux), and the
/// recipe section is part of the same Windows-scoped wiring. The
/// tunnel lifecycle itself is cross-platform - on Linux the tunnel
/// simply starts without desync-side preparation (the Linux port is
/// deferred, and the WARP mode is Windows-first in v1).
#[cfg(windows)]
const WARP_ARGS_SUPPORTED: bool = true;
#[cfg(not(windows))]
const WARP_ARGS_SUPPORTED: bool = false;

/// How long to wait for winws2's `capture is started` line before
/// starting the tunnel (stage 4). NETZapret's CaptureGrace: the
/// desync must be capturing before the first WARP connection, or the
/// handshake goes out undesynced and freezes (their 01.10
/// measurement). Only waited when the tunnel actually has to start -
/// a desync-only launch never pays it.
const WINWS2_CAPTURE_GRACE: Duration = Duration::from_secs(10);

/// Poll interval while waiting for the capture line.
const CAPTURE_READY_POLL: Duration = Duration::from_millis(100);

/// The readiness line both engines print once WinDivert capture is
/// running (winws2: `nfq2/nfqws.c`; winws v1: the same string,
/// verified against the pinned Flowseal 1.10.4 binary).
const CAPTURE_STARTED_MARKER: &str = "capture is started";

/// Auto-hostlist file name inside the user lists directory. One file
/// shared by every strategy: what was learned about the user's
/// provider stays learned across strategy switches.
pub(crate) const AUTO_HOSTLIST_FILE: &str = "list-auto.txt";

/// Ceiling for the auto-hostlist, enforced by the supervisor cleanup
/// (design §4): past this many entries the oldest (head of file —
/// the engine appends at the end) are dropped.
pub(crate) const AUTO_HOSTLIST_CEILING: usize = 2_000;

/// Header written into a freshly created list-auto.txt. Comment lines
/// are ignored by the engine's list loader and by the lists UI; they
/// explain the file to a human who opens the folder.
#[cfg(windows)]
pub(crate) const AUTO_HOSTLIST_HEADER: &str = "# zapret2-gui: сайты, которые программа сама добавила в обход.\r\n# Движок дописывает сюда домены, похожие на заблокированные, чтобы дальше открывать их через обход.\r\n# Строки можно удалять: удалённый отсюда домен программа заносит в исключения и больше не добавляет.\r\n";
#[cfg(not(windows))]
pub(crate) const AUTO_HOSTLIST_HEADER: &str = "# zapret2-gui: сайты, которые программа сама добавила в обход.\n# Движок дописывает сюда домены, похожие на заблокированные, чтобы дальше открывать их через обход.\n# Строки можно удалять: удалённый отсюда домен программа заносит в исключения и больше не добавляет.\n";

/// List paths for the auto-hostlist gating of the Lua branch
/// (branch 4), resolved by the caller exactly like the static lists
/// it mirrors; `auto_path` is the one absolute path (the engine
/// writes to the file, so it must land in the user lists directory,
/// never in the bundled bin/lists copy).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AutoLists {
    pub general: String,
    pub general_user: String,
    pub exclude: String,
    pub exclude_user: String,
    pub auto_path: String,
}

/// Absolute, slash-normalized path of the auto-hostlist inside the
/// user lists directory — the form builders pass to `--hostlist-auto`.
pub(crate) fn auto_hostlist_path(lists_dir: &Path) -> String {
    lists_dir
        .join(AUTO_HOSTLIST_FILE)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Reads a hostlist file into a normalized domain set (lowercase,
/// `^` prefix stripped, comments and blanks skipped). A missing or
/// unreadable file yields an empty set: coverage checks must never
/// fail the cleanup that uses them.
fn read_list_domain_set(path: &Path) -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    if let Ok(content) = std::fs::read_to_string(path) {
        for line in content.lines() {
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') {
                continue;
            }
            set.insert(t.trim_start_matches('^').to_lowercase());
        }
    }
    set
}

/// True when `entry` adds nothing over `set`: the set holds the
/// entry itself or any of its parent domains (the engine matches
/// subdomains automatically, so a static `example.com` already
/// covers a learned `cdn.example.com`).
fn domain_covered_by(entry: &str, set: &std::collections::HashSet<String>) -> bool {
    let mut candidate = entry;
    loop {
        if set.contains(candidate) {
            return true;
        }
        match candidate.split_once('.') {
            Some((_, rest)) if rest.contains('.') => candidate = rest,
            _ => return false,
        }
    }
}

/// Supervisor-side hygiene for list-auto.txt (design §4): dedupe,
/// drop entries already covered by the static include lists or by
/// the exclude lists (graduation — once upstream ships a domain in
/// list-general/list-google the learned copy is dead weight), and
/// enforce the FIFO ceiling. Runs only where the engine is stopped
/// or where a lost fresh append costs at most one re-learn (engine
/// start, after a remote-lists sync); the file is never rewritten
/// while nothing changed, so a no-op cleanup does not even bump
/// the mtime the engine watches for list reloads.
pub(crate) fn cleanup_auto_hostlist(
    lists_dir: &Path,
    bundle_lists_dir: &Path,
) -> Result<(), String> {
    let auto_path = lists_dir.join(AUTO_HOSTLIST_FILE);
    if !auto_path.is_file() {
        return Ok(());
    }
    let content = std::fs::read_to_string(&auto_path)
        .map_err(|e| format!("cannot read {}: {}", auto_path.display(), e))?;

    let mut comments: Vec<String> = Vec::new();
    let mut entries: Vec<String> = Vec::new();
    for line in content.lines() {
        let t = line.trim();
        if t.starts_with('#') {
            comments.push(t.to_string());
        } else if !t.is_empty() {
            entries.push(t.to_string());
        }
    }

    // Coverage: the union of the static include lists and the
    // exclude lists, from both the user lists directory and the
    // bundled bin/lists copies (builders prefer the bundle, so a
    // domain shipped there has graduated too).
    let mut covered = std::collections::HashSet::new();
    for dir in [lists_dir, bundle_lists_dir] {
        for file in [
            "list-general.txt",
            "list-general-user.txt",
            "list-google.txt",
            "list-exclude.txt",
            "list-exclude-user.txt",
        ] {
            covered.extend(read_list_domain_set(&dir.join(file)));
        }
    }

    let mut seen = std::collections::HashSet::new();
    let mut kept: Vec<String> = Vec::new();
    for entry in &entries {
        let key = entry.trim_start_matches('^').to_lowercase();
        if !seen.insert(key.clone()) {
            continue; // exact duplicate (modulo case/^)
        }
        if domain_covered_by(&key, &covered) {
            continue; // graduated into statics, or excluded by the user
        }
        kept.push(entry.clone());
    }
    // FIFO ceiling: the engine appends at the end, so the tail of
    // the file is the newest learning; drop from the head.
    if kept.len() > AUTO_HOSTLIST_CEILING {
        kept.drain(..kept.len() - AUTO_HOSTLIST_CEILING);
    }

    if kept == entries {
        return Ok(()); // nothing to do — leave the file untouched
    }

    let eol = if cfg!(windows) { "\r\n" } else { "\n" };
    let mut out = String::new();
    for comment in &comments {
        out.push_str(comment);
        out.push_str(eol);
    }
    for entry in &kept {
        out.push_str(entry);
        out.push_str(eol);
    }
    crate::platform::settings::atomic_write(&auto_path, out.as_bytes())
        .map_err(|e| format!("cannot rewrite {}: {}", auto_path.display(), e))?;
    Ok(())
}

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
    /// True when the running engine was started with one-off custom
    /// arguments. The fallback active_strategy id (flowseal-alt) says
    /// nothing about such a launch, so this flag — not a null id —
    /// is what the UI must test for a custom run.
    #[serde(default)]
    pub active_custom_args: bool,
    /// Active camouflage transport id, if one was applied at launch.
    pub active_transport: Option<String>,
    pub binary_path: Option<String>,
    pub uptime_secs: u64,
    pub total_logs_buffered: usize,
    /// Last fatal engine error code, e.g. "windivert_conflict"; null when the
    /// last start attempt did not end in a recognized failure.
    #[serde(default)]
    pub last_engine_error: Option<String>,
    /// App split as configured right now in settings ("disabled" |
    /// "allowlist" | "blocklist"). Always live, even while stopped.
    #[serde(default)]
    pub app_split_configured_mode: Option<String>,
    /// App split mode actually passed to the running engine, if any.
    /// None while stopped or when split was not applied.
    #[serde(default)]
    pub app_split_applied_mode: Option<String>,
    /// True only when the running engine really is winws2-split.exe with
    /// --app-split-* arguments. Never true for Flowseal strategies.
    #[serde(default)]
    pub app_split_enforced: bool,
    /// Why split is not enforced, when it is not. Codes: "disabled" |
    /// "flowseal" | "binary_missing" | "not_running". None when enforced.
    #[serde(default)]
    pub app_split_not_enforced_reason: Option<String>,
    /// Split tunnel mode the running engine's arguments were built
    /// with, in settings spelling ("full" | "strict" | "game_mode").
    /// None while stopped, and None when the mode does not apply to
    /// the launched strategy (flagship, custom-args launches): the UI
    /// compares it against the selected mode to decide whether a
    /// restart would change anything.
    #[serde(default)]
    pub split_tunnel_applied_mode: Option<String>,
    /// WARP tunnel snapshot (stage 5): always present, Stopped when
    /// the tunnel never ran. The UI reads the tunnel state from
    /// here (heartbeat snapshot) and from `TunnelState` events.
    #[serde(default)]
    pub tunnel: SingBoxStatus,
}

/// App split outcome of the last launch, kept so get_status() can report
/// what the RUNNING engine actually does (as opposed to what settings
/// currently say, which may have changed since the launch).
#[derive(Debug, Clone)]
struct AppSplitLaunchState {
    applied_mode: Option<String>,
    enforced: bool,
    reason: Option<String>,
}

impl Default for AppSplitLaunchState {
    fn default() -> Self {
        Self {
            applied_mode: None,
            enforced: false,
            reason: Some("not_running".to_string()),
        }
    }
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
    /// Set from custom_args.is_some() on successful spawn in
    /// start_locked; cleared when a new real start attempt begins,
    /// on stop, and when the monitored child dies.
    active_custom_args: Arc<AtomicBool>,
    active_transport: Arc<RwLock<Option<String>>>,
    binary_path: Arc<RwLock<Option<String>>>,
    start_time: Arc<RwLock<Option<Instant>>>,

    job_object: Arc<JobObject>,
    /// WARP tunnel sidecar (stage 3). The manager owns the sing-box
    /// child in the same Job Object as winws2; the supervisor starts
    /// it (stage 4: `start_tunnel`, the reconcile after a desync
    /// start) and stops it on engine stop.
    singbox: Arc<SingBoxManager>,
    diagnostics: Arc<DiagnosticsService>,

    ring_buffer: Arc<Mutex<VecDeque<LogEntry>>>,
    log_tx: Sender<LogEntry>,
    file_logger: Arc<FileLogger>,

    sink: SinkSlot,
    probes_started: Arc<AtomicBool>,

    /// Last fatal engine error code surfaced to the UI, e.g. "windivert_conflict".
    /// Cleared on every start attempt; set by the monitor task when the child
    /// dies with a recognized failure signature.
    last_engine_error: Arc<RwLock<Option<String>>>,
    /// App split outcome of the last launch (see AppSplitLaunchState).
    /// Reset on stop and when the monitored child dies, so a dead engine
    /// never keeps advertising an enforcement that no longer runs.
    app_split_launch: Arc<RwLock<AppSplitLaunchState>>,
    /// Split tunnel mode of the last launch, in settings spelling;
    /// None when the launched strategy does not consume the mode
    /// (flagship, custom args) or nothing was launched. Reset in the
    /// same places as app_split_launch.
    split_tunnel_launch: Arc<RwLock<Option<String>>>,
    /// Set by the stderr reader when the child reports a WinDivert open-filter
    /// failure; reset on every start attempt. Read by the monitor task.
    windivert_open_error: Arc<AtomicBool>,
    /// Set by the stdout/stderr pumps when the engine prints
    /// `capture is started` (stage 4): the tunnel starts only after
    /// this, or after the grace period with the process alive (see
    /// `wait_winws2_ready`). Reset on every start attempt.
    capture_started: Arc<AtomicBool>,
    /// Whether the WARP-args decision for the running desync launch
    /// has been made (stage 4): the exclusion filter + registration
    /// recipe were applied, the filter was blocked by the preset's
    /// own --wf-raw-filter file, or the args are not applicable
    /// under winws v1 (Flowseal). The flag means "a restart would
    /// not change the WARP preparation", not "the filter is in the
    /// arguments" - read by `start_tunnel` to decide whether the
    /// desync needs a restart. Cleared when a new real start
    /// attempt begins and on stop.
    warp_args_applied: Arc<AtomicBool>,
    /// Whether the running desync engine is winws v1 (Flowseal)
    /// rather than winws2 (stage 5, stage 4 review O1). The WARP
    /// args are not applicable under v1 at all (no --wf-raw-filter,
    /// no Lua recipe), so `start_tunnel` must not restart the
    /// desync for them - even when the desync launched with the
    /// switch off and step 3c never made the decision. Set at
    /// spawn, cleared with the other launch state.
    desync_is_winws1: Arc<AtomicBool>,
    /// Whether the one-shot WinDivert recovery restart already ran for the
    /// current user-initiated start. Re-armed by user starts only, after the
    /// early-return checks in start_inner; never by the recovery restart
    /// itself, so recovery cannot loop.
    windivert_recovery_done: Arc<AtomicBool>,
    /// Parameters of the last start attempt, so the recovery restart can
    /// relaunch exactly the same strategy and custom arguments.
    last_start_params: Arc<RwLock<(Option<String>, Option<Vec<String>>)>>,
    /// Monotonic start/stop generation. The recovery worker captures it
    /// before cleanup and aborts if a newer user start/stop happened meanwhile.
    generation: Arc<RwLock<u64>>,
    /// Channel to the one-shot WinDivert recovery worker. The monitor only
    /// sends a restart request here (a sync send, never an await of the
    /// restart itself): awaiting the restart from the monitor would close a
    /// type-level cycle through the spawn Send bound and rustc rejects it.
    recovery_tx: UnboundedSender<(u64, Option<String>, Option<Vec<String>>)>,
    recovery_rx: Arc<Mutex<Option<UnboundedReceiver<(u64, Option<String>, Option<Vec<String>>)>>>>,
    recovery_worker_started: Arc<AtomicBool>,
}

impl Supervisor {
    /// Initializes a new supervisor instance with Windows Job Object and micro-batching worker.
    pub fn new() -> Result<Self, ProcessError> {
        let job_object = Arc::new(JobObject::create()?);
        let diagnostics = Arc::new(DiagnosticsService::new());
        let ring_buffer = Arc::new(Mutex::new(VecDeque::with_capacity(MAX_LOG_CAPACITY)));
        let file_logger = Arc::new(FileLogger::new());

        let (log_tx, log_rx) = channel::<LogEntry>(2_000);
        let (recovery_tx, recovery_rx) = unbounded_channel::<(u64, Option<String>, Option<Vec<String>>)>();

        // The event slot is created before the tunnel manager: the
        // manager gets a clone so its state transitions reach the
        // GUI and the svc pipe clients (stage 5).
        let sink = SinkSlot::new();
        // WARP tunnel sidecar (stage 3): shares the Job Object, the
        // log channel, and the file logger with the supervisor.
        let singbox = Arc::new(SingBoxManager::new(
            job_object.clone(),
            log_tx.clone(),
            file_logger.clone(),
            sink.clone(),
        ));

        let supervisor = Self {
            lifecycle: Arc::new(tokio::sync::Mutex::new(())),
            state: Arc::new(RwLock::new(SupervisorState::Stopped)),
            active_pid: Arc::new(RwLock::new(None)),
            active_child: Arc::new(Mutex::new(None)),
            active_strategy: Arc::new(RwLock::new(None)),
            active_custom_args: Arc::new(AtomicBool::new(false)),
            active_transport: Arc::new(RwLock::new(None)),
            binary_path: Arc::new(RwLock::new(None)),
            start_time: Arc::new(RwLock::new(None)),
            job_object,
            singbox,
            diagnostics,
            ring_buffer,
            log_tx,
            file_logger: file_logger.clone(),
            sink,
            probes_started: Arc::new(AtomicBool::new(false)),
            last_engine_error: Arc::new(RwLock::new(None)),
            app_split_launch: Arc::new(RwLock::new(AppSplitLaunchState::default())),
            split_tunnel_launch: Arc::new(RwLock::new(None)),
            windivert_open_error: Arc::new(AtomicBool::new(false)),
            windivert_recovery_done: Arc::new(AtomicBool::new(false)),
            capture_started: Arc::new(AtomicBool::new(false)),
            warp_args_applied: Arc::new(AtomicBool::new(false)),
            desync_is_winws1: Arc::new(AtomicBool::new(false)),
            last_start_params: Arc::new(RwLock::new((None, None))),
            generation: Arc::new(RwLock::new(0)),
            recovery_tx,
            recovery_rx: Arc::new(Mutex::new(Some(recovery_rx))),
            recovery_worker_started: Arc::new(AtomicBool::new(false)),
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

    /// Installs the UI event sink. The background network probe loop is started
    /// exactly once, on the first call; it always emits through the *current*
    /// sink so the destination can be swapped at runtime.
    pub fn set_event_sink(&self, sink: Arc<dyn EventSink>) {
        self.sink.set(sink);

        if !self.probes_started.swap(true, Ordering::SeqCst) {
            let slot = self.sink.clone();
            self.diagnostics.clone().start_polling_loop(move |report| {
                slot.emit_probe_update(&report);
            });
        }
    }

    /// The WARP tunnel sidecar manager (stage 3). Stage 4 calls
    /// `start()` / `stop()` on it from the supervisor's lifecycle
    /// (`start_tunnel` / `stop_tunnel` and the reconcile after a
    /// desync start); the GUI binary's `main.rs` registers it in
    /// `AppState::sidecars` so `install_update` refuses while the
    /// tunnel is active. Stage 5 exposes its status over IPC.
    ///
    /// The accessor itself is only called from the GUI binary - the
    /// svc binary never registers sidecars (the updater runs in the
    /// GUI), so without this the svc build warns.
    #[allow(dead_code)]
    pub fn singbox(&self) -> &Arc<SingBoxManager> {
        &self.singbox
    }

    /// Spawns the dedicated Tokio micro-batching worker running every 150 ms.
    fn spawn_micro_batching_worker(&self, mut log_rx: Receiver<LogEntry>) {
        let ring_buffer = self.ring_buffer.clone();
        let sink = self.sink.clone();

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

                            // Emit batch to the installed sink
                            sink.emit_log_batch(&batch_to_emit);
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
                                    sink.emit_log_batch(&batch_to_emit);
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

    /// Opens the logs folder in Windows Explorer. Returns the opened path
    /// so the UI can surface it on failure instead of failing silently.
    pub fn open_logs_folder(&self) -> Result<std::path::PathBuf, String> {
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
        let active_custom_args = self.active_custom_args.load(Ordering::SeqCst);
        let active_transport = self.active_transport.read().clone();
        let binary_path = self.binary_path.read().clone();
        let uptime_secs = self
            .start_time
            .read()
            .map(|t| t.elapsed().as_secs())
            .unwrap_or(0);
        let total_logs_buffered = self.ring_buffer.lock().len();
        let last_engine_error = self.last_engine_error.read().clone();

        // Configured mode is always live from settings; the applied side
        // is meaningful only while an engine runs. A stopped engine
        // enforces nothing, whatever the last launch did.
        let app_split_configured_mode = Some(
            format!(
                "{:?}",
                crate::platform::settings::load_settings().app_split_mode
            )
            .to_lowercase(),
        );
        let (app_split_applied_mode, app_split_enforced, app_split_not_enforced_reason) =
            if state == SupervisorState::Running {
                let launch = self.app_split_launch.read().clone();
                (
                    launch.applied_mode,
                    launch.enforced,
                    launch.reason,
                )
            } else if app_split_configured_mode.as_deref() == Some("disabled") {
                (None, false, Some("disabled".to_string()))
            } else {
                (None, false, Some("not_running".to_string()))
            };

        // Same gating as the app split applied side: the tunnel mode a
        // launch was built with is meaningful only while that engine
        // runs; a stopped engine applies no mode at all.
        let split_tunnel_applied_mode = if state == SupervisorState::Running {
            self.split_tunnel_launch.read().clone()
        } else {
            None
        };

        SupervisorStatus {
            state,
            pid,
            active_strategy,
            active_custom_args,
            active_transport,
            binary_path,
            uptime_secs,
            total_logs_buffered,
            last_engine_error,
            app_split_configured_mode,
            app_split_applied_mode,
            app_split_enforced,
            app_split_not_enforced_reason,
            split_tunnel_applied_mode,
            tunnel: self.singbox.status(),
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
        self.sink.emit_logs_cleared();
    }

    /// User-initiated start (UI, tray, autostart). Bumps the generation and
    /// runs the shared start sequence; a real (non-no-op) start re-arms the
    /// one-shot recovery, a no-op start does not.
    pub async fn start(
        self: Arc<Self>,
        strategy_id: Option<String>,
        custom_args: Option<Vec<String>>,
    ) -> Result<(), ProcessError> {
        *self.generation.write() += 1;
        // The recovery worker is spawned once, on the first user start. It
        // owns the restart path so the monitor never awaits a restart itself.
        if !self
            .recovery_worker_started
            .swap(true, Ordering::SeqCst)
        {
            if let Some(rx) = self.recovery_rx.lock().take() {
                let worker = self.clone();
                tokio::spawn(async move {
                    let mut rx = rx;
                    while let Some((generation, strategy_id, custom_args)) =
                        rx.recv().await
                    {
                        // Hold the lifecycle lock for the whole cleanup and
                        // restart: a user start/stop can only bump the
                        // generation (before blocking on this lock) or run
                        // after we release it, so it can never spawn an engine
                        // that our stray-kill would then destroy. Without the
                        // lock, the checks below race user starts: the kill
                        // runs after the first check but before the second,
                        // and would kill a freshly spawned engine.
                        let guard = worker.lifecycle.lock().await;
                        if *worker.generation.read() != generation {
                            continue;
                        }
                        if *worker.state.read() != SupervisorState::Stopped {
                            continue;
                        }
                        worker.log(
                            "info",
                            "WinDivert conflict detected: cleaning up stray engines and retrying once...",
                        );
                        // No supervised child can exist here: state is Stopped
                        // under the lock, and every spawn stores its child
                        // under this same lock, so exclude=None is safe.
                        let _ = kill_stray_winws2_processes(None);
                        cleanup_windivert_driver().await;
                        if *worker.generation.read() != generation {
                            continue;
                        }
                        if *worker.state.read() != SupervisorState::Stopped {
                            continue;
                        }
                        if let Err(e) = worker
                            .clone()
                            .start_locked(strategy_id, custom_args, false, false, &guard)
                            .await
                        {
                            worker.log(
                                "error",
                                &format!("WinDivert recovery restart failed: {}", e),
                            );
                            // The restart itself failed (spawn, binaries,
                            // transport): no child exists to die and set the
                            // error, so set it here or the UI would silently
                            // fall back to stopped with no explanation. The
                            // retry ran in the WinDivert-conflict context, so
                            // this code is the honest label for it.
                            *worker.last_engine_error.write() =
                                Some("windivert_conflict".to_string());
                        }
                    }
                });
            }
        }
        self.start_inner(strategy_id, custom_args, true).await
    }

    /// Shared start sequence used by user starts and by the WinDivert
    /// recovery restart. `reset_recovery` re-arms the one-shot recovery
    /// allowance; it is true only for user starts and only after the
    /// early-return checks, so a no-op start (already running the same
    /// strategy) does not re-arm it and the recovery restart itself never
    /// re-arms it, which would loop forever.
    ///
    /// The lifecycle lock is held for the whole start sequence so a
    /// concurrent stop() cannot observe a half-initialized engine (see 1.1).
    /// It is acquired here, and the body lives in start_locked so the
    /// recovery worker can hold the same lock across its cleanup and restart
    /// without deadlocking on a re-acquire.
    async fn start_inner(
        self: Arc<Self>,
        strategy_id: Option<String>,
        custom_args: Option<Vec<String>>,
        reset_recovery: bool,
    ) -> Result<(), ProcessError> {
        // Clone the Arc for the lock: the guard borrows from it, and `self`
        // must still move into start_locked, so the guard cannot borrow
        // from `self` directly.
        let this = self.clone();
        let guard = this.lifecycle.lock().await;
        self.start_locked(strategy_id, custom_args, reset_recovery, false, &guard)
            .await
    }

    /// Start body, run with the lifecycle lock already held (see start_inner).
    /// Must never acquire the lifecycle lock itself.
    /// `force_restart` bypasses the "already running the same strategy"
    /// no-op: the WARP restart in `start_tunnel` relaunches the exact
    /// same strategy with the tunnel args added, and without the force
    /// flag start_locked would return without restarting anything.
    async fn start_locked(
        self: Arc<Self>,
        strategy_id: Option<String>,
        custom_args: Option<Vec<String>>,
        reset_recovery: bool,
        force_restart: bool,
        _guard: &tokio::sync::MutexGuard<'_, ()>,
    ) -> Result<(), ProcessError> {
        // SECURITY (M-1): fail-closed privilege gate lives INSIDE the start
        // path, not only in the IPC command. The tray "Включить обход" item and
        // the --autostart path call start() directly; without this gate the engine
        // would start half-broken (WinDivert fails without admin rights) and
        // the UI would never show the admin prompt. The error string keeps the
        // ADMIN_REQUIRED prefix so the frontend opens the admin gate modal.
        if !crate::platform::privileges::is_elevated().unwrap_or(false) {
            self.log(
                "error",
                "Запуск движка требует прав администратора — пропуск запуска",
            );
            return Err(ProcessError::NotElevated);
        }

        // Fail loudly on an explicitly requested but unknown strategy id.
        // Previously an unknown id silently fell through to flowseal-alt
        // in the resolution below, so a typo or a stale selection started
        // a different strategy than the user asked for, and even overwrote
        // last_strategy_id with it. This check runs before any state
        // mutation, so a rejected start leaves a running engine untouched.
        // The None path (tray/autostart/first run) keeps its compatibility
        // fallback below, with logging.
        if let Some(ref id) = strategy_id {
            let discovered = match crate::platform::process::discover_strategies() {
                Ok(list) => list,
                Err(e) => return Err(e),
            };
            if !discovered.iter().any(|s| &s.id == id) {
                let msg = format!("Неизвестная стратегия: {}", id);
                self.log(
                    "error",
                    &format!("{} — запуск отменён, другая стратегия не подменялась", msg),
                );
                return Err(ProcessError::InvalidStrategy(msg));
            }
        }

        // Fresh attempt: forget any previous engine error and any stale
        // WinDivert signature, and remember the params for a possible
        // recovery restart.
        self.windivert_open_error
            .store(false, Ordering::SeqCst);
        *self.last_engine_error.write() = None;
        *self.last_start_params.write() = (strategy_id.clone(), custom_args.clone());

        // Support seamless strategy switching or fresh start
        {
            let mut state = self.state.write();
            if *state == SupervisorState::Starting || *state == SupervisorState::Stopping {
                return Ok(());
            }
            if *state == SupervisorState::Running {
                let active = self.active_strategy.read().clone();
                let requested = strategy_id.clone().unwrap_or_else(|| "flowseal-alt".into());
                if active.as_deref() == Some(&requested) && custom_args.is_none() && !force_restart
                {
                    // No-op start, but the generation was already
                    // bumped (in start()), which cancels the pending
                    // reconcile task from the original start - spawn
                    // a fresh one so the tunnel still comes up when
                    // wanted (stage 4, review m4).
                    self.spawn_tunnel_reconcile_if_needed();
                    return Ok(());
                }
                *state = SupervisorState::Starting;
            } else {
                *state = SupervisorState::Starting;
            }
        }

        // A real (non-no-op) attempt supersedes any previous launch:
        // forget the old custom-args flag now, so a failed attempt
        // cannot keep advertising a custom run. It is set again on
        // successful spawn below, from custom_args.is_some().
        self.active_custom_args.store(false, Ordering::SeqCst);
        // Stage 4 (review m2): the same "real attempt only" rule for
        // the capture/WARP flags - they live here, below the no-op
        // return, so a no-op start cannot clear them. Clearing
        // warp_args_applied would make start_tunnel restart the
        // desync for nothing, and clearing capture_started would
        // make the next readiness wait burn the full 10 s grace
        // even though the engine already reported capture. A new
        // attempt has not captured yet and has not applied the WARP
        // args yet; both are set again below (the flag by the pumps,
        // the args flag at step 3c).
        self.capture_started.store(false, Ordering::SeqCst);
        self.warp_args_applied.store(false, Ordering::SeqCst);
        // Stage 5 (O1): the engine type belongs to the launch, and
        // this attempt has not spawned yet - a failed start must
        // not leave a stale "winws1" claim behind.
        self.desync_is_winws1.store(false, Ordering::SeqCst);
        let is_custom_args = custom_args.is_some();

        // A real start attempt (not a no-op early return above) re-arms the
        // one-shot WinDivert recovery allowance, and only for user starts.
        if reset_recovery {
            self.windivert_recovery_done
                .store(false, Ordering::SeqCst);
        }

        // If another child process is active, stop it cleanly before spawning the new strategy
        let old_pid_opt = *self.active_pid.read();
        let maybe_child = self.active_child.lock().take();
        // Linux: remember whether the switch above actually took (and
        // stops below) a previous engine. Its monitor task exits
        // silently once the child is taken, so if this start then
        // fails before the nftables apply step further down, nobody
        // else removes the old launch's rules — the early error paths
        // below do it when this is set.
        #[cfg(target_os = "linux")]
        let stopped_previous_engine = maybe_child.is_some();
        if let Some(mut old_child) = maybe_child {
            if let Some(old_pid) = old_pid_opt {
                self.log("info", &format!("Switching config: stopping previous engine (PID {})", old_pid));
                // No Job Object teardown here (stage 4): the shared job
                // also holds the sing-box sidecar, and a strategy
                // switch must not kill the tunnel (the
                // independent-switches matrix). kill+wait is a
                // complete stop for winws2 (it spawns no children);
                // the spawn-retry path below still sweeps strays.
                let _ = stop_winws2(&mut old_child).await;
            }
        }

        self.emit_state_change(SupervisorState::Starting);
        self.log("info", "Initiating zapret background supervisor...");

        // 1. Resolve binaries
        let binaries = match resolve_binaries() {
            Ok(b) => b,
            Err(e) => {
                self.log("error", &format!("Binary discovery failure: {}", e));
                #[cfg(target_os = "linux")]
                if stopped_previous_engine {
                    let _ = crate::platform::nftables::remove_rules();
                }
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
            // Compatibility: no explicit id (tray/autostart/first run). A
            // stale stored last_strategy_id (deleted custom profile,
            // retired preset) still falls back to the default, but loudly:
            // this is the only substitution left, and it is logged both
            // to stderr and to the engine log.
            let last_id = crate::platform::settings::load_settings().last_strategy_id;
            let from_last = last_id
                .as_ref()
                .and_then(|lid| strategies.iter().find(|s| s.id == *lid).cloned());
            if let (Some(lid), None) = (&last_id, &from_last) {
                eprintln!(
                    "start: stored last_strategy_id '{}' is no longer known, falling back to default",
                    lid
                );
                self.log(
                    "info",
                    &format!(
                        "Сохранённая стратегия '{}' больше недоступна, запуск по умолчанию",
                        lid
                    ),
                );
            }
            from_last
                .or_else(|| strategies.iter().find(|s| s.id == "flowseal-alt").cloned())
                .or_else(|| strategies.first().cloned())
        };

        // Hardening: an explicitly requested id must never silently fall
        // through to the default here (e.g. if the strategy vanished
        // between the pre-check above and this second discovery). The
        // pre-check rejects unknown ids; this closes the remaining race
        // window before last_strategy_id is overwritten below.
        if strategy_id.is_some() && target_strat.is_none() {
            #[cfg(target_os = "linux")]
            if stopped_previous_engine {
                let _ = crate::platform::nftables::remove_rules();
            }
            return Err(ProcessError::InvalidStrategy(format!(
                "Неизвестная стратегия: {}",
                strategy_id.as_deref().unwrap_or_default()
            )));
        }

        let active_id = target_strat
            .as_ref()
            .map(|s| s.id.clone())
            .unwrap_or_else(|| "flowseal-alt".into());

        // Remember last used strategy in settings (serialized vs. concurrent
        // mutations — see platform::settings::update_settings, L-4).
        let _ = crate::platform::settings::update_settings(|s| {
            s.last_strategy_id = Some(active_id.clone());
        });

        let use_winws1 = target_strat.as_ref().map_or(false, |s| s.engine == "winws");
        let mut target_exe = if use_winws1 {
            if let Some(ref w1) = binaries.winws_exe {
                w1.clone()
            } else {
                binaries.bin_dir.join("winws.exe")
            }
        } else {
            binaries.winws2_exe.clone()
        };

        // Linux: Flowseal strategies are winws argument lists built
        // entirely on --dpi-desync-*, and the pinned Linux engine
        // (nfqws2 v1.0.7) accepts none of them — its desync is
        // Lua-only, verified against the binary's own --help and
        // --dry-run ("unrecognized option: dpi-desync"). There is no
        // honest way to run them here, so refuse with the reason
        // instead of spawning an engine that dies on its first
        // argument and looks like a random crash.
        #[cfg(target_os = "linux")]
        if use_winws1 {
            let msg = format!(
                "Стратегия '{}' — пресет Flowseal для winws (--dpi-desync), а движок Linux (nfqws2) эти флаги не поддерживает: desync у него только через Lua. Выберите Lua-стратегию zapret2.",
                active_id
            );
            self.log("error", &msg);
            if stopped_previous_engine {
                let _ = crate::platform::nftables::remove_rules();
            }
            *self.state.write() = SupervisorState::Error;
            self.emit_state_change(SupervisorState::Error);
            return Err(ProcessError::EngineUnavailable(msg));
        }

        // App split binary selection. The split build is a winws2 fork, so
        // it can only replace winws2.exe: Flowseal strategies run on
        // winws.exe, whose flag set (--dpi-desync-*) winws2.exe does not
        // even accept, and a missing split binary is a normal pre-fork
        // state, not a start failure. Every branch is logged, so the
        // engine log alone explains which binary runs and why.
        let split_settings = crate::platform::settings::load_settings();
        let split_mode_str = format!("{:?}", split_settings.app_split_mode).to_lowercase();
        let mut split_launch = AppSplitLaunchState {
            applied_mode: None,
            enforced: false,
            reason: Some("disabled".to_string()),
        };
        if split_settings.app_split_mode == crate::platform::settings::AppSplitMode::Disabled {
            self.log(
                "info",
                "App split disabled: running the standard engine binary",
            );
        } else if use_winws1 {
            // Flowseal keeps winws.exe; split is honestly not applied.
            split_launch.reason = Some("flowseal".to_string());
            self.log(
                "info",
                &format!(
                    "App split '{}' configured but strategy '{}' is Flowseal (winws): split does not apply to winws.exe, keeping the standard binary",
                    split_mode_str, active_id
                ),
            );
        } else if let Some(ref split_exe) = binaries.winws2_split_exe {
            target_exe = split_exe.clone();
            split_launch = AppSplitLaunchState {
                applied_mode: Some(split_mode_str.clone()),
                enforced: true,
                reason: None,
            };
            self.log(
                "info",
                &format!(
                    "App split '{}' enforced: using winws2-split.exe for strategy '{}' ({} app paths)",
                    split_mode_str,
                    active_id,
                    split_settings.app_split_list.len().min(100)
                ),
            );
        } else {
            // Fork binary absent: fall back to the plain engine rather
            // than failing the start the user asked for. On Windows the
            // fork build may simply not be installed yet; on Linux it
            // can never exist (winws2-split is a Windows build), so the
            // log says exactly that instead of "not installed yet".
            // The status reason code stays "binary_missing" on both:
            // it is part of the fixed UI vocabulary.
            split_launch.reason = Some("binary_missing".to_string());
            #[cfg(windows)]
            self.log(
                "info",
                &format!(
                    "App split '{}' configured but winws2-split.exe is not installed yet: falling back to winws2.exe without app split",
                    split_mode_str
                ),
            );
            #[cfg(target_os = "linux")]
            self.log(
                "info",
                &format!(
                    "App split '{}' configured but per-app split is not available on Linux (winws2-split is a Windows build of the fork): running nfqws2 without app split",
                    split_mode_str
                ),
            );
        }
        *self.app_split_launch.write() = split_launch;

        // Split tunnel mode of this launch. The mode is consumed by
        // exactly two argument builders: Flowseal (branch 1) and the
        // discovered-Lua presets (branch 4). The flagship's argument
        // list is fixed and ignores the mode, and a custom profile (or
        // a one-off custom_args launch) runs caller-supplied arguments:
        // for those the honest applied value is None, matching the UI
        // scope note. Written here, before build_arguments, from the
        // same fresh settings the builders will read.
        let tunnel_applied = if is_custom_args
            || active_id == "zapret2-ru-ultimate"
            || crate::engine::imported_strategies::is_imported(&active_id)
            || target_strat
                .as_ref()
                .map_or(false, |s| s.category == "custom")
        {
            None
        } else {
            Some(
                Self::split_tunnel_mode_str(
                    crate::platform::settings::load_settings().split_tunnel_mode,
                )
                .to_string(),
            )
        };
        *self.split_tunnel_launch.write() = tunnel_applied;

        *self.binary_path.write() = Some(target_exe.to_string_lossy().to_string());
        self.log(
            "info",
            &format!(
                "Using bypass engine: {} [{}]",
                target_exe.file_name().unwrap_or_default().to_string_lossy(),
                active_id
            ),
        );

        // 2. Fast pre-launch setup: enable TCP timestamps asynchronously without blocking.
        //    Windows-only: netsh is the Windows TCP-stack knob, and winws.exe
        //    (use_winws1) is itself a Windows engine — Linux defaults TCP
        //    timestamps on and nfqws2 needs no equivalent tweak.
        #[cfg(windows)]
        if use_winws1 {
            tokio::spawn(async {
                let _ = tokio::process::Command::new(crate::platform::system32("netsh.exe"))
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
                #[cfg(target_os = "linux")]
                if stopped_previous_engine {
                    let _ = crate::platform::nftables::remove_rules();
                }
                *self.state.write() = SupervisorState::Error;
                self.emit_state_change(SupervisorState::Error);
                return Err(e);
            }
        };

        // 3b. Apply the active camouflage transport (if any) on top of the
        // strategy arguments. Transports never change which strategy was
        // selected — they only transform the final argument list, and they
        // fail closed with an explicit reason instead of silently doing nothing.
        let (args, applied_transport) = match crate::engine::transport::apply_active_transport(args) {
            Ok(result) => result,
            Err(e) => {
                self.log("error", &format!("Transport refused engine start: {}", e));
                #[cfg(target_os = "linux")]
                if stopped_previous_engine {
                    let _ = crate::platform::nftables::remove_rules();
                }
                *self.state.write() = SupervisorState::Error;
                self.emit_state_change(SupervisorState::Error);
                return Err(e.into());
            }
        };
        if let Some(ref transport_id) = applied_transport {
            self.log(
                "info",
                &format!("Camouflage transport '{}' applied to launch arguments", transport_id),
            );
        }
        *self.active_transport.write() = applied_transport;

        // App split arguments for winws2-split.exe only (interface fixed
        // with the fork side). Appended after the transport transform and
        // before spawn. The parameters log below shows the launch line
        // with app-split paths masked (see below), never the real paths.
        // The supervisor's own PID is excluded so the tgproxy
        // bridge traffic (which runs inside this process) is not split.
        // The 100-entry cap mirrors set_app_split_config; the take() here
        // is a second line of defence against direct settings edits.
        let mut args = args;
        let split_launch_snapshot = self.app_split_launch.read().clone();
        if split_launch_snapshot.enforced {
            // Paths and the PID exclusion ride only with --app-split-mode:
            // enforced == true with applied_mode == None must never send
            // bare paths to the engine (unreachable today, but the two
            // constructors could drift apart).
            if let Some(ref applied) = split_launch_snapshot.applied_mode {
                args.push(format!("--app-split-mode={}", applied));
                for path in split_settings.app_split_list.iter().take(100) {
                    args.push(format!("--app-split-path={}", path));
                }
                args.push(format!(
                    "--app-split-exclude-pid={}",
                    std::process::id()
                ));
            }
        }

        // 3c. WARP launch arguments (stage 4, Windows only): when the
        // tunnel will run (switch on + binary present) or already runs,
        // the desync launch takes the tunnel out of the WinDivert
        // capture (--wf-raw-filter) and adds the registration recipe
        // for api.cloudflareclient.com. The decision is made here, at
        // arg-build time, because the filter can only be applied before
        // the spawn - a tunnel started later against an unfiltered
        // desync needs a desync restart (see start_tunnel). The
        // `needs_tunnel` binding is reused by the reconcile (step 7).
        let tunnel_wanted = crate::platform::settings::load_settings().warp_enabled
            && binaries.sing_box_exe.is_some();
        let tunnel_already_running = self.singbox.is_active();
        let needs_tunnel = tunnel_wanted || tunnel_already_running;
        if needs_tunnel {
            if WARP_ARGS_SUPPORTED {
                use crate::engine::tunnel_capture::WarpArgsOutcome;
                match crate::engine::tunnel_capture::apply_warp_args(&mut args, !use_winws1) {
                    WarpArgsOutcome::Applied => {
                        self.warp_args_applied.store(true, Ordering::SeqCst);
                        self.log(
                            "info",
                            "WARP launch args applied: tunnel excluded from WinDivert capture, api.cloudflareclient.com recipe added",
                        );
                    }
                    WarpArgsOutcome::FilterBlockedByFile => {
                        self.warp_args_applied.store(true, Ordering::SeqCst);
                        self.log(
                            "warn",
                            "WARP recipe applied, but the preset's own --wf-raw-filter comes from a file and cannot be extended: the tunnel stays in the WinDivert capture and runs slower",
                        );
                    }
                    WarpArgsOutcome::NotApplicableWinws1 => {
                        // The decision is final for this launch too:
                        // a restart would reach the same outcome, so
                        // the flag is set (review m1) - otherwise
                        // start_tunnel would restart the desync for
                        // nothing on the default (Flowseal) strategy.
                        self.warp_args_applied.store(true, Ordering::SeqCst);
                        self.log(
                            "info",
                            "Flowseal strategy (winws v1) cannot exclude the tunnel from WinDivert (--wf-raw-part is OR-combined): the tunnel runs through the capture and is slower; registration still gets desynced (the preset covers all TCP 443)",
                        );
                    }
                }
            } else {
                self.log(
                    "info",
                    "WARP launch args are Windows-only: the tunnel starts without desync-side preparation on this platform",
                );
            }
        }

        // Linux: translate the Windows root capture into NFQUEUE +
        // nftables before spawning (and before the parameter log below,
        // so the log shows the command line nfqws2 actually receives).
        // nfqws2 rejects every --wf-* argument, so the list is
        // rewritten — root filters out, --qnum in — and the exact port
        // sets those filters carried become queue rules in our own
        // nftables table. Installing the rules is part of the start:
        // if nft refuses them, the engine must not run capturing only
        // half the intended traffic, so the start fails with the
        // reason. Between here and the spawn there is no other early
        // return, so installed rules can only outlive this start via
        // the spawn-failure path, which removes them (see below).
        #[cfg(target_os = "linux")]
        let args = {
            let rewritten = crate::platform::nftables::rewrite_args_for_nfqws2(args);
            if rewritten.port_sets.is_empty() {
                self.log(
                    "info",
                    "Linux: в аргументах нет корневых --wf-фильтров, правила nftables не устанавливаются (перехватывать нечего)",
                );
            }
            if let Err(e) = crate::platform::nftables::apply_rules(
                &rewritten.port_sets,
                rewritten.queue_num,
            ) {
                let msg = format!(
                    "Не удалось установить правила nftables для движка: {}",
                    e
                );
                self.log("error", &msg);
                *self.state.write() = SupervisorState::Error;
                self.emit_state_change(SupervisorState::Error);
                return Err(ProcessError::EngineUnavailable(msg));
            }
            if !rewritten.port_sets.is_empty() {
                self.log(
                    "info",
                    &format!(
                        "Linux: перехват через NFQUEUE {}, правила nftables установлены (таблица inet zapret2)",
                        rewritten.queue_num
                    ),
                );
            }
            rewritten.args
        };

        // Log the launch line with app-split paths masked: full paths
        // (including the user profile directory) must not land in engine
        // logs, matching the C-side no-paths policy. Only the mode and
        // the path count are printed; the arguments passed to the engine
        // above are unchanged.
        let split_path_count = args
            .iter()
            .filter(|a| a.starts_with("--app-split-path="))
            .count();
        let masked_args: Vec<&str> = args
            .iter()
            .filter(|a| !a.starts_with("--app-split-path="))
            .map(|s| s.as_str())
            .collect();
        self.log(
            "info",
            &format!(
                "Bypass parameters: {}{}",
                masked_args.join(" "),
                if split_path_count > 0 {
                    format!(" [--app-split-path: {} path(s) hidden]", split_path_count)
                } else {
                    String::new()
                }
            ),
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
                        // Linux: the engine never came up, so the
                        // nftables queue rules installed for it above
                        // must not outlive the failed start.
                        #[cfg(target_os = "linux")]
                        {
                            let _ = crate::platform::nftables::remove_rules();
                        }
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
        // Stage 5 (O1): record the engine type of this launch so
        // start_tunnel can skip the pointless restart under v1.
        self.desync_is_winws1.store(use_winws1, Ordering::SeqCst);
        self.active_custom_args
            .store(is_custom_args, Ordering::SeqCst);
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
        let capture_started_stdout = self.capture_started.clone();
        if let Some(stdout_stream) = stdout {
            tokio::spawn(async move {
                let mut reader = BufReader::new(stdout_stream).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    // Stage 4: the engine prints this once WinDivert
                    // capture is running; the tunnel starts only after
                    // it (or the grace period, see wait_winws2_ready).
                    if line.contains(CAPTURE_STARTED_MARKER) {
                        capture_started_stdout.store(true, Ordering::SeqCst);
                    }
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
        let windivert_open_error = self.windivert_open_error.clone();
        let capture_started_stderr = self.capture_started.clone();
        if let Some(stderr_stream) = stderr {
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr_stream).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    // WinDivert open-filter failure: the driver is held by
                    // another bypass tool or left in a stale state. The
                    // monitor task pairs this with exit code 10.
                    let lowered = line.to_lowercase();
                    if lowered.contains("windivert")
                        && lowered.contains("error opening filter")
                    {
                        windivert_open_error.store(true, Ordering::SeqCst);
                    }
                    // Stage 4: same capture marker as on stdout (the
                    // line's stream is not guaranteed across engines).
                    if line.contains(CAPTURE_STARTED_MARKER) {
                        capture_started_stderr.store(true, Ordering::SeqCst);
                    }
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
            // The guard must not live across any await below: it is not Send,
            // so exit handling runs after the loop, outside the guard's scope.
            let mut exited_status = None;
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
                            exited_status = Some(status);
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

            if let Some(status) = exited_status {
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
                    supervisor_clone
                        .active_custom_args
                        .store(false, Ordering::SeqCst);
                    *supervisor_clone.active_transport.write() = None;
                    *supervisor_clone.binary_path.write() = None;
                    *supervisor_clone.app_split_launch.write() =
                        AppSplitLaunchState::default();
                    *supervisor_clone.split_tunnel_launch.write() = None;
                    // Stage 5 (O1): no engine, no engine type.
                    supervisor_clone
                        .desync_is_winws1
                        .store(false, Ordering::SeqCst);
                }

                // Capture the generation and params BEFORE the grace poll: a
                // user stop/start during the poll must cancel recovery, and
                // reading them after the poll would see the new generation
                // and mistake it for the one this death belongs to.
                let generation_before = *supervisor_clone.generation.read();
                let (strategy_id, custom_args) =
                    supervisor_clone.last_start_params.read().clone();

                // WinDivert open-filter failure (exit code 10 plus the stderr
                // signature) means the driver is held by another bypass tool
                // or left stale. The stderr reader drains the pipes
                // asynchronously, so give it a short grace period before deciding.
                let mut windivert_conflict = false;
                if status.code() == Some(10) {
                    for _ in 0..10 {
                        if supervisor_clone
                            .windivert_open_error
                            .load(Ordering::SeqCst)
                        {
                            windivert_conflict = true;
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
                let superseded =
                    *supervisor_clone.generation.read() != generation_before;
                let mut recovery_requested = false;
                if windivert_conflict && !superseded {
                    if !supervisor_clone
                        .windivert_recovery_done
                        .swap(true, Ordering::SeqCst)
                    {
                        supervisor_clone.log(
                            "info",
                            "WinDivert conflict detected: scheduling one recovery attempt...",
                        );
                        let _ = supervisor_clone.recovery_tx.send((
                            generation_before,
                            strategy_id,
                            custom_args,
                        ));
                        recovery_requested = true;
                    } else {
                        // Second failure with the same conflict: recovery was
                        // already spent, so the banner text ("already tried
                        // once") is true from the moment it appears.
                        *supervisor_clone.last_engine_error.write() =
                            Some("windivert_conflict".to_string());
                        supervisor_clone.log(
                            "error",
                            "WinDivert filter open failed (exit code 10): the driver is held by another bypass tool or left in a stale state.",
                        );
                    }
                }

                // Linux: the engine died on its own, so the nftables
                // queue rules installed for it are removed here. When
                // the death was superseded, a newer start already owns
                // the table (its apply deleted and recreated it) and
                // must not lose it; a scheduled recovery restart
                // manages the table itself. The generation is rechecked
                // immediately before the removal as well: a start that
                // began after the `superseded` snapshot above (during
                // the grace poll or the recovery dispatch) may already
                // have installed its own fresh table, and removing the
                // rules now would strip capture from that engine.
                #[cfg(target_os = "linux")]
                if !superseded && !recovery_requested {
                    if *supervisor_clone.generation.read() == generation_before {
                        let _ = crate::platform::nftables::remove_rules();
                    }
                }

                // The detached driver cleanup runs on every death except when
                // a recovery restart is scheduled (the worker cleans up itself
                // right before relaunching, and a detached `sc stop` racing
                // the fresh engine's driver open must not run here) and except
                // when this death was superseded (a new engine may already be
                // opening the driver).
                if !recovery_requested && !superseded {
                    tokio::spawn(async {
                        cleanup_windivert_driver().await;
                    });
                }

                // A superseded death must not emit Stopped over a fresh
                // Starting/Running: the new start emits its own events.
                if !superseded {
                    supervisor_clone.emit_state_change(SupervisorState::Stopped);
                }
            }
        });

        // 7. WARP tunnel reconcile (stage 4): the desync is up; if the
        // tunnel switch is on (or the tunnel already runs), bring the
        // tunnel up with the matching scope - but detached. The tunnel
        // start (readiness + warmup) can take tens of seconds on a
        // dead exit, and start_locked holds the lifecycle lock: doing
        // it here would make the "Включить обход" call wait for the
        // tunnel and block stop() behind it. The task re-checks the
        // generation and the desync state before touching the tunnel,
        // so a stop() in between cancels it; the final check and the
        // start run under the lifecycle lock, so a stop cannot slip
        // between the check and the start (see reconcile_tunnel_task).
        // A tunnel failure here never fails the desync start: the
        // tunnel is a fallback, and the desync is already running.
        if needs_tunnel {
            let task_self = self.clone();
            let task_generation = *self.generation.read();
            tokio::spawn(async move {
                task_self.reconcile_tunnel_task(task_generation).await;
            });
        }

        Ok(())
    }

    /// Stops the bypass engine and releases WinDivert.
    pub async fn stop(&self) -> Result<(), ProcessError> {
        // A user stop cancels any pending WinDivert recovery restart and
        // dismisses its banner.
        *self.generation.write() += 1;
        *self.last_engine_error.write() = None;
        // Serialized with start(): either the start fully completed before us
        // or it will run after we finish — never interleaved (see 1.1).
        let _lifecycle_guard = self.lifecycle.lock().await;
        // Re-clear under the lock: a recovery worker finishing its restart
        // attempt while we waited on the lock could have set the error after
        // our pre-lock clear; an explicit stop must always dismiss the banner.
        *self.last_engine_error.write() = None;

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

        // Stage 3/4: shutdown is the reverse of startup - the tunnel
        // stops first, then winws2. A no-op while the tunnel never
        // started. The `warp_enabled` switch is deliberately NOT
        // touched here: it is the user's persistent preference ("the
        // tunnel comes up with the bypass"), and only `stop_tunnel`
        // turns it off. A later start with the switch still on brings
        // the tunnel back.
        let _ = self.singbox.stop().await;

        let maybe_child = self.active_child.lock().take();
        if let Some(mut child) = maybe_child {
            if let Some(pid_val) = pid {
                self.log("info", &format!("Executing shutdown protocol on PID {}", pid_val));
                let _ = stop_winws2_gracefully(&mut child, &self.job_object).await;
            }
        }

        // Clean WinDivert driver service registrations to guarantee clean kernel state
        cleanup_windivert_driver().await;

        // Linux: remove our nftables table so no queue rules outlive
        // the stopped engine. The rules carry the bypass verdict, so a
        // stale table would not break traffic — but it is our state,
        // and leaving it behind would make the next start's stale
        // cleanup guess whether it is ours.
        #[cfg(target_os = "linux")]
        {
            let _ = crate::platform::nftables::remove_rules();
        }

        // Persist any buffered log data before reporting the stopped state.
        self.file_logger.flush();

        {
            *self.state.write() = SupervisorState::Stopped;
            *self.active_pid.write() = None;
            *self.start_time.write() = None;
            *self.active_strategy.write() = None;
            self.active_custom_args.store(false, Ordering::SeqCst);
            *self.active_transport.write() = None;
            *self.app_split_launch.write() = AppSplitLaunchState::default();
            *self.split_tunnel_launch.write() = None;
            // Stage 4: no engine, no capture, no warp args.
            self.capture_started.store(false, Ordering::SeqCst);
            self.warp_args_applied.store(false, Ordering::SeqCst);
            // Stage 5 (O1): no engine, no engine type.
            self.desync_is_winws1.store(false, Ordering::SeqCst);
        }

        self.emit_state_change(SupervisorState::Stopped);
        self.log("info", "zapret supervisor stopped successfully");

        Ok(())
    }

    // ------------------------------------------------------------------
    // WARP tunnel: the second switch (stage 4)
    // ------------------------------------------------------------------

    /// Whether enabling the tunnel needs a desync restart: the
    /// running desync launched without the WARP args AND the args
    /// apply to its engine. Under winws v1 (Flowseal) they do not
    /// (no --wf-raw-filter, no Lua recipe), so restarting would
    /// change nothing - the tunnel just starts ProxyOnly and goes
    /// through WinDivert (the honest stage 4 limitation). Pure, so
    /// the truth table is unit-tested without a live engine.
    fn tunnel_needs_desync_restart(warp_args_applied: bool, desync_is_winws1: bool) -> bool {
        !warp_args_applied && !desync_is_winws1
    }

    /// Start the WARP tunnel: the tunnel side of the two switches
    /// (design section 3). Sets the durable `warp_enabled` switch,
    /// then:
    /// - desync stopped: start the tunnel alone (Everything scope).
    /// - desync running with the WARP args applied: wait for capture
    ///   readiness, then start the tunnel (ProxyOnly scope).
    /// - desync running WITHOUT the WARP args (Windows only: it was
    ///   started before the tunnel was switched on): restart the
    ///   desync with the same parameters - start_locked applies the
    ///   args and spawns the reconcile task, which starts the tunnel.
    ///   This call returns the tunnel status as it is at that moment
    ///   (Stopped): the tunnel comes up in the background via the
    ///   reconcile task. Waiting for it here would mean holding the
    ///   lifecycle lock through the tunnel start - the task exists
    ///   precisely to avoid that.
    /// - tunnel already up: idempotent (like the manager's start),
    ///   except a wrong scope, which is fixed by a stop+start.
    ///
    /// Fails closed when the sing-box binary is absent (WARP
    /// unavailable) or the tunnel start itself fails. A failed tunnel
    /// start leaves the switch ON - the user's intent stands, and the
    /// tunnel can be retried (stage 6); only an explicit stop_tunnel
    /// turns it off.
    ///
    /// Serialized with start()/stop() via the lifecycle lock (design
    /// section 7). The tunnel start (readiness + warmup) runs under
    /// the lock: a concurrent stop() waits for it (seconds in the
    /// common case, up to ~30 s for a hung engine) and then stops
    /// everything - the alternative (a check-then-start race) could
    /// leave a tunnel running after the user stopped the bypass.
    ///
    /// Wired to the IPC tunnel toggle (stage 5): the GUI calls it
    /// through `TunnelStart`, the svc dispatches it here.
    ///
    /// A failed start is recorded on the manager (Error state +
    /// last_error, emitted to the UI sink) before the error is
    /// returned: the svc `TunnelStart` handler is fire-and-forget,
    /// so the remote GUI learns about the failure only through the
    /// tunnel state (stage 5 review m1). `singbox.start()` records
    /// its own failures the same way; this wrapper covers the early
    /// returns that fail before the manager is ever started.
    pub async fn start_tunnel(self: Arc<Self>) -> Result<SingBoxStatus, ProcessError> {
        match self.clone().start_tunnel_attempt().await {
            Ok(status) => Ok(status),
            Err(error) => {
                // A start cancelled by a stop is not a failure: the
                // stop owns the state (Starting while it runs,
                // Stopped once it lands), and recording it would
                // resurrect a deliberately stopped tunnel as Error
                // (stage 7 review m1). The manager already logged
                // the cancellation itself. Every other error is
                // recorded - including the early failures, which
                // arrive with the state still Stopped (the manager
                // was never started), so the filter must stay on
                // the error type, not on the state.
                if !matches!(error, ProcessError::TunnelCancelled) {
                    self.singbox.fail_start(error.to_string());
                }
                Err(error)
            }
        }
    }

    /// The start itself: switch, idempotency, desync restart when
    /// needed, capture readiness, then the manager start. Split out
    /// of `start_tunnel` so the wrapper above can record every
    /// failure on the manager in one place.
    async fn start_tunnel_attempt(self: Arc<Self>) -> Result<SingBoxStatus, ProcessError> {
        // The switch is set first: the desync restart below (and the
        // reconcile it spawns) reads it to decide the tunnel must run.
        let _ = crate::platform::settings::update_settings(|s| {
            s.warp_enabled = true;
        });

        let _guard = self.lifecycle.lock().await;

        let binaries = resolve_binaries()?;
        let desync_running = *self.state.read() == SupervisorState::Running;

        // Tunnel already up (or coming up): idempotent, except a
        // wrong scope - the desync may have come up after the tunnel
        // without a reconcile (e.g. a crash between the two).
        if self.singbox.is_active() {
            let want_scope = if desync_running {
                TunScope::ProxyOnly
            } else {
                TunScope::Everything
            };
            if self.singbox.tun_scope() == Some(want_scope) {
                return Ok(self.singbox.status());
            }
            self.log(
                "info",
                "tunnel is running with the wrong scope: restarting it",
            );
            let _ = self.singbox.stop().await;
            let options = Self::tunnel_start_options(&binaries, want_scope, desync_running)
                .ok_or_else(|| {
                    ProcessError::Tunnel("sing-box не найден в установке".to_string())
                })?;
            return Ok(self.start_tunnel_inner(options).await?);
        }

        // From here on a tunnel start (or a desync restart for one)
        // is needed, so the binary must exist. Checked once, here:
        // restarting the desync for a tunnel that cannot run would
        // leave the user with no desync at all if the restart failed.
        if binaries.sing_box_exe.is_none() {
            return Err(ProcessError::Tunnel(
                "sing-box не найден в установке: WARP-режим недоступен. Обновите приложение - движок поставляется вместе с ним.".to_string(),
            ));
        }

        if !desync_running {
            // Tunnel alone: Everything scope, no capture to wait for.
            let options = Self::tunnel_start_options(&binaries, TunScope::Everything, false)
                .ok_or_else(|| {
                    ProcessError::Tunnel("sing-box не найден в установке".to_string())
                })?;
            return Ok(self.start_tunnel_inner(options).await?);
        }

        // Desync running. Windows: if it launched without the WARP
        // args, restart it with the same parameters - start_locked
        // applies the args and spawns the reconcile task, which
        // starts the tunnel. The restart is forced: start_locked
        // would otherwise no-op on "same strategy, no custom args".
        // No restart under winws v1 (stage 5, stage 4 review O1):
        // the args are not applicable there at all, so a restart
        // would change nothing - this also covers the launch that
        // started with the switch off, where step 3c never made
        // the decision and warp_args_applied is still false.
        if WARP_ARGS_SUPPORTED
            && Self::tunnel_needs_desync_restart(
                self.warp_args_applied.load(Ordering::SeqCst),
                self.desync_is_winws1.load(Ordering::SeqCst),
            )
        {
            self.log(
                "info",
                "desync is running without the WARP launch args: restarting it so the tunnel stays out of the WinDivert capture",
            );
            let (strategy_id, custom_args) = self.last_start_params.read().clone();
            self.clone()
                .start_locked(strategy_id, custom_args, false, true, &_guard)
                .await?;
            return Ok(self.singbox.status());
        }

        // Desync running with the args (or on a platform without warp
        // args): wait for capture readiness, then start ProxyOnly.
        if !self.wait_winws2_ready().await {
            return Err(ProcessError::Tunnel(
                "движок десинка остановился, пока туннель ждал готовности захвата".to_string(),
            ));
        }
        let options = Self::tunnel_start_options(&binaries, TunScope::ProxyOnly, true)
            .ok_or_else(|| ProcessError::Tunnel("sing-box не найден в установке".to_string()))?;
        Ok(self.start_tunnel_inner(options).await?)
    }

    /// Stop the WARP tunnel: the tunnel side of the two switches.
    /// Turns the durable `warp_enabled` switch off and stops the
    /// engine. The desync is untouched (the filter stays in its
    /// arguments until the next desync restart - harmless: it
    /// excludes an endpoint nothing connects to anymore). A no-op
    /// while the tunnel is stopped (the switch is still turned off -
    /// the caller's intent is explicit).
    ///
    /// Wired to the IPC tunnel toggle (stage 5): the GUI calls it
    /// through `TunnelStop`, the svc dispatches it here.
    ///
    /// The manager cancellation goes BEFORE the lifecycle lock
    /// (stage 7, O2): a `start_tunnel` in progress holds the lock
    /// through the whole tunnel start (up to ~70 s), and the
    /// generation bump makes its manager start abort within ~250 ms
    /// instead of making this stop wait it out.
    pub async fn stop_tunnel(&self) -> Result<(), ProcessError> {
        let _ = crate::platform::settings::update_settings(|s| {
            s.warp_enabled = false;
        });
        self.singbox.request_cancel();
        let _guard = self.lifecycle.lock().await;
        self.singbox.stop().await?;
        Ok(())
    }

    /// Spawn a detached tunnel reconcile when the tunnel is wanted
    /// but not already up with the right scope (stage 4, review m4).
    /// Used on the no-op start path: `start()` bumps the generation
    /// before it knows the start is a no-op, which cancels the
    /// reconcile task the original start spawned - without a fresh
    /// task the tunnel would silently never come up. The task itself
    /// re-checks everything under the lifecycle lock, so this
    /// pre-check only avoids pointless spawns.
    fn spawn_tunnel_reconcile_if_needed(self: &Arc<Self>) {
        let tunnel_wanted = crate::platform::settings::load_settings().warp_enabled
            && resolve_binaries()
                .map(|b| b.sing_box_exe.is_some())
                .unwrap_or(false);
        let tunnel_needed = tunnel_wanted || self.singbox.is_active();
        let tunnel_ok =
            self.singbox.is_active() && self.singbox.tun_scope() == Some(TunScope::ProxyOnly);
        if tunnel_needed && !tunnel_ok {
            let task_self = self.clone();
            let task_generation = *self.generation.read();
            tokio::spawn(async move {
                task_self.reconcile_tunnel_task(task_generation).await;
            });
        }
    }

    /// The detached tunnel reconcile after a desync start (stage 4).
    /// Spawned by step 7 of start_locked when the tunnel is wanted
    /// or already running. Steps:
    /// 1. Wait for winws2 capture readiness (or the grace period).
    ///    No lock held here - the wait is up to 10 s.
    /// 2. Take the lifecycle lock and re-check the generation and
    ///    the desync state: a stop() in between cancels the task.
    ///    The lock is held across the check AND the tunnel start, so
    ///    a stop cannot slip between them (see start_tunnel for why
    ///    that race matters).
    /// 3. Re-read the switch and the tunnel state, then start the
    ///    tunnel (ProxyOnly) or re-scope it (Everything -> ProxyOnly).
    /// A tunnel failure is logged and swallowed: the desync is
    /// already running, and the tunnel is a fallback. Note the
    /// status window: between the desync start and the tunnel start
    /// (up to the capture grace) the tunnel status reads Stopped -
    /// the manager only reports Starting once start() runs.
    async fn reconcile_tunnel_task(self: Arc<Self>, generation: u64) {
        if !self.wait_winws2_ready().await {
            self.log(
                "info",
                "desync engine died while the tunnel waited for capture readiness - tunnel not started",
            );
            return;
        }

        let _guard = self.lifecycle.lock().await;
        if *self.generation.read() != generation {
            return;
        }
        if *self.state.read() != SupervisorState::Running {
            return;
        }
        // The switch may have been flipped while we waited.
        if !crate::platform::settings::load_settings().warp_enabled {
            return;
        }
        let binaries = match resolve_binaries() {
            Ok(b) => b,
            Err(e) => {
                self.log(
                    "warn",
                    &format!("tunnel reconcile: cannot resolve binaries: {e}"),
                );
                return;
            }
        };
        let Some(options) = Self::tunnel_start_options(&binaries, TunScope::ProxyOnly, true) else {
            self.log(
                "warn",
                "tunnel switch is on but sing-box is not installed: tunnel not started",
            );
            return;
        };
        if self.singbox.is_active() {
            if self.singbox.tun_scope() == Some(TunScope::ProxyOnly) {
                return;
            }
            self.log(
                "info",
                "tunnel is running with the wrong scope for an active desync: restarting it with ProxyOnly",
            );
            let _ = self.singbox.stop().await;
        }
        if let Err(e) = self.start_tunnel_inner(options).await {
            self.log(
                "warn",
                &format!("tunnel failed to start after the desync: {e} (desync keeps running)"),
            );
        }
    }

    /// Wait for winws2 to report capture readiness: the
    /// `capture is started` line, or the grace period elapsing with
    /// the process alive (NETZapret's CaptureGrace - the line is a
    /// log line, not a contract, and an engine that stays alive for
    /// 10 s is capturing). Returns false only when the child died
    /// during the wait, in which case the caller must not start the
    /// tunnel (the desync it was supposed to follow is gone).
    async fn wait_winws2_ready(&self) -> bool {
        // Linux: the nftables queue rules are installed before the
        // spawn, so capture is active by the time the engine is
        // Running - there is no readiness line to wait for (nfqws2
        // never prints one: the line is WinDivert-specific).
        if !cfg!(windows) {
            return true;
        }
        let deadline = Instant::now() + WINWS2_CAPTURE_GRACE;
        loop {
            if self.capture_started.load(Ordering::SeqCst) {
                return true;
            }
            if !self.desync_child_alive() {
                return false;
            }
            if Instant::now() >= deadline {
                self.log(
                    "info",
                    "winws2 did not print 'capture is started' within 10 s, but the process is alive - starting the tunnel",
                );
                return true;
            }
            tokio::time::sleep(CAPTURE_READY_POLL).await;
        }
    }

    /// Is the supervised desync child still alive? `false` when the
    /// slot is empty (taken by stop(), the monitor, or a switch) or
    /// the child has exited. A try_wait error counts as dead, like
    /// the monitor treats it.
    fn desync_child_alive(&self) -> bool {
        let mut guard = self.active_child.lock();
        match guard.as_mut() {
            None => false,
            Some(child) => matches!(child.try_wait(), Ok(None)),
        }
    }

    /// Build the manager's start options from resolved binaries
    /// (stage 4). Returns None when the sing-box binary is absent
    /// (WARP unavailable - the desync is unaffected). `tun_enabled`
    /// is always true here: the false seam exists only for the
    /// sandbox live tests.
    fn tunnel_start_options(
        binaries: &ZapretBinaries,
        scope: TunScope,
        desync_on: bool,
    ) -> Option<SingBoxStartOptions> {
        let binary = binaries.sing_box_exe.clone()?;
        let settings = crate::platform::settings::load_settings();
        Some(SingBoxStartOptions {
            binary,
            work_dir: binaries.bin_dir.clone(),
            home_dir: crate::engine::singbox::singbox_home_dir(),
            tun_scope: scope,
            desync_on,
            lists_dir: binaries.lists_dir.clone(),
            dns_provider: settings.dns_provider,
            tun_enabled: true,
            fixed_clash_port: None,
            upstream_override: None,
        })
    }

    /// Start the tunnel through the manager: sweep orphaned sing-box
    /// processes first (a stray from a crashed run holds the TUN
    /// adapter and the Clash port), then start. The caller holds the
    /// lifecycle lock. The currently managed tunnel PID (if any) is
    /// excluded from the sweep - a future caller that does not hold
    /// the lock must not kill a live engine it does not own.
    async fn start_tunnel_inner(
        &self,
        options: SingBoxStartOptions,
    ) -> Result<SingBoxStatus, crate::engine::singbox::SingBoxError> {
        let exclude = self.singbox.status().pid;
        let killed = kill_stray_singbox_processes(exclude);
        if killed > 0 {
            self.log(
                "info",
                &format!(
                    "removed {killed} orphaned sing-box process(es) before starting the tunnel"
                ),
            );
        }
        self.singbox.start(options).await
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
        let settings = crate::platform::settings::load_settings();
        let auto = if settings.auto_hostlist_enabled {
            Some(auto_hostlist_path(lists_dir))
        } else {
            None
        };
        Self::build_flowseal_args_inner(
            preset_id,
            bin_dir,
            lists_dir,
            settings.split_tunnel_mode,
            &settings.game_filter_tcp,
            &settings.game_filter_udp,
            auto.as_deref(),
        )
    }

    fn build_flowseal_args_inner(
        preset_id: &str,
        bin_dir: &Path,
        lists_dir: &Path,
        mode: crate::platform::settings::SplitTunnelMode,
        game_filter_tcp: &str,
        game_filter_udp: &str,
        auto: Option<&str>,
    ) -> Vec<String> {
        let is_game = mode == crate::platform::settings::SplitTunnelMode::GameMode;
        let is_strict = mode == crate::platform::settings::SplitTunnelMode::Strict;

        let game_tcp = Self::sanitize_port_spec(game_filter_tcp);
        let game_udp = Self::sanitize_port_spec(game_filter_udp);

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
            _ /* flowseal-general: the only id reaching here; unknown flowseal-* ids are rejected in resolve_strategy_args */ => vec![
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

        // Auto-hostlist (design §3): into every TCP section gated
        // on list-general, after its exclude lines. Runs before the
        // game extension (its sections are ipset-based — out of
        // scope) and before the STRICT filter below, which drops
        // general blocks whole, auto lines included: STRICT stays
        // free of auto by construction, as the design requires.
        if let Some(auto_path) = auto {
            args = Self::inject_auto_hostlist(args, auto_path);
        }

        // 1. If Game Mode is active, attach game port filtering and ACTIVE_GAME_UDP desync
        // SECURITY (L-4): game port specs come from user settings — sanitize them
        // like everywhere else so a malformed value cannot break engine startup.
        let game_tcp = Self::sanitize_port_spec(game_filter_tcp);
        let game_udp = Self::sanitize_port_spec(game_filter_udp);
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

    /// Builds winws2 arguments for an imported kinlay0 strategy.
    /// Pure: the caller resolves the filesystem-shaped inputs (the
    /// auto-hostlist path and the list resolver, same shape as
    /// branch 4's l()) so tests can pin the output.
    ///
    /// Layout mirrors the flagship: our standard root capture
    /// filters, the three stock lua-init libraries the collection
    /// loads (minus its blobs.lua — blobs are registered our way,
    /// only the ones this strategy references), then the strategy's
    /// profiles in the .bat's own order as --new=<slug> sections.
    ///
    /// Honest scope limits, same as the flagship: the split-tunnel
    /// Lua modes do not reshape these lists (start_locked reports
    /// the mode as not applied for imported ids), and the .bat
    /// gamefilter sections are not carried over — the GUI's own
    /// game sections belong to the Flowseal/Lua-preset builders.
    ///
    /// Auto-hostlist (design §3): with the setting on, the
    /// domain_tls section — the TCP section gated on list-general —
    /// gains --hostlist-auto after its exclude lines, like the
    /// flagship's general sections. The QUIC section stays out:
    /// the UDP failure detector is out of phase-1 scope there too.
    fn build_imported_args(
        imported: &crate::engine::imported_strategies::ImportedStrategy,
        auto: Option<&str>,
        list_path: &dyn Fn(&str) -> String,
    ) -> Vec<String> {
        let mut args: Vec<String> = vec![
            "--wf-tcp-out=80,443,2053,2083,2087,2096,8443".into(),
            "--wf-udp-out=443,19294-19344,50000-65535".into(),
            "--lua-init=@zapret-lib.lua".into(),
            "--lua-init=@zapret-antidpi.lua".into(),
            "--lua-init=@zapret-auto.lua".into(),
        ];
        for name in imported.blobs {
            if let Some(file) = crate::engine::imported_strategies::blob_file(name) {
                args.push(format!("--blob={}:@{}", name, file));
            }
        }
        for (slug, profile_args) in imported.profiles {
            args.push(format!("--new={}", slug));
            for arg in *profile_args {
                if let Some(pos) = arg.find("{L}") {
                    let file = &arg[pos + 3..];
                    args.push(format!("{}{}", &arg[..pos], list_path(file)));
                } else {
                    args.push(arg.to_string());
                }
            }
        }
        if let Some(auto_path) = auto {
            let anchor = format!(
                "--hostlist-exclude={}",
                list_path("list-exclude-user.txt")
            );
            Self::insert_after_in_section(
                &mut args,
                "domain_tls",
                &anchor,
                &[format!("--hostlist-auto={}", auto_path)],
            );
        }
        args
    }

    /// Builds zapret2 Ultimate configuration for Russia on winws2 v2.
    /// Thin wrapper: reads the auto-hostlist setting and resolves
    /// the absolute auto path from the user lists directory (the
    /// parameter this builder historically ignored); the argument
    /// list itself is built by the pure inner function so tests can
    /// pin both states.
    fn build_zapret2_ru_ultimate_args(_bin_dir: &Path, lists_dir: &Path) -> Vec<String> {
        let settings = crate::platform::settings::load_settings();
        let auto = if settings.auto_hostlist_enabled {
            Some(auto_hostlist_path(lists_dir))
        } else {
            None
        };
        Self::build_ultimate_args_with_auto(auto.as_deref())
    }

    fn build_ultimate_args_with_auto(auto: Option<&str>) -> Vec<String> {
        let mut args = vec![
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
        ];

        if let Some(auto_path) = auto {
            // Auto-hostlist (design §3): only the two general
            // sections learn — youtube_4k/discord_media are gated on
            // their own narrow sets and must not apply their
            // specialized handling to random learned domains, and
            // ipset_fallback is an IP filter, not a domain one. The
            // auto path is the only absolute path in this builder
            // (everything else is "lists/..." relative to the engine
            // cwd): the engine WRITES to this file, so it must land
            // in the user lists directory. At the same time — and
            // only with the setting on — the general sections gain
            // the exclude pairs they historically lacked, because
            // the invariant "exclude always wins" must hold in every
            // section that carries auto: general_https gets the user
            // exclude next to its static one; general_http gets the
            // whole pair (its include side is NOT extended —
            // list-general-user stays out of this section, as
            // before). With the setting off the argument list above
            // is byte for byte the historical one.
            Self::insert_after_in_section(
                &mut args,
                "general_https",
                "--hostlist-exclude=lists/list-exclude.txt",
                &[
                    "--hostlist-exclude=lists/list-exclude-user.txt".to_string(),
                    format!("--hostlist-auto={}", auto_path),
                ],
            );
            Self::insert_after_in_section(
                &mut args,
                "general_http",
                "--hostlist=lists/list-general.txt",
                &[
                    "--hostlist-exclude=lists/list-exclude.txt".to_string(),
                    "--hostlist-exclude=lists/list-exclude-user.txt".to_string(),
                    format!("--hostlist-auto={}", auto_path),
                ],
            );
        }

        args
    }

    /// Inserts `new_lines` right after `anchor` inside the named
    /// `--new=<section>` span of an argument list. A missing section
    /// or anchor leaves the list untouched (the builders' tests pin
    /// the shapes this is used with).
    fn insert_after_in_section(
        args: &mut Vec<String>,
        section: &str,
        anchor: &str,
        new_lines: &[String],
    ) {
        let marker = format!("--new={}", section);
        let Some(start) = args.iter().position(|a| a == &marker) else {
            return;
        };
        let end = args
            .iter()
            .skip(start + 1)
            .position(|a| a.starts_with("--new="))
            .map(|p| start + 1 + p)
            .unwrap_or(args.len());
        if let Some(rel) = args[start..end].iter().position(|a| a == anchor) {
            let at = start + rel + 1;
            for (i, line) in new_lines.iter().enumerate() {
                args.insert(at + i, line.clone());
            }
        }
    }

    /// Inserts `--hostlist-auto=<auto_path>` into every section of
    /// a Flowseal argument list that is (a) TCP (`--filter-tcp`)
    /// and (b) gated on list-general.txt, right after the section's
    /// last `--hostlist-exclude` line (design §3). UDP sections
    /// carrying list-general (the QUIC sections at the head of each
    /// preset) are deliberately skipped: the UDP failure detector
    /// is out of phase-1 scope, and google-only / ipset-only /
    /// domains-only sections are not general sections.
    fn inject_auto_hostlist(args: Vec<String>, auto_path: &str) -> Vec<String> {
        let auto_arg = format!("--hostlist-auto={}", auto_path);
        let mut out: Vec<String> = Vec::with_capacity(args.len() + 4);
        let mut block: Vec<String> = Vec::new();
        for arg in args {
            if arg == "--new" {
                Self::inject_auto_into_block(&mut block, &auto_arg);
                out.append(&mut block);
                out.push(arg);
            } else {
                block.push(arg);
            }
        }
        Self::inject_auto_into_block(&mut block, &auto_arg);
        out.append(&mut block);
        out
    }

    fn inject_auto_into_block(block: &mut Vec<String>, auto_arg: &str) {
        let is_tcp = block.iter().any(|a| a.starts_with("--filter-tcp"));
        let has_general = block.iter().any(|a| {
            a.strip_prefix("--hostlist=")
                .map_or(false, |v| v.rsplit('/').next() == Some("list-general.txt"))
        });
        if !is_tcp || !has_general {
            return;
        }
        let insert_at = block
            .iter()
            .rposition(|a| a.starts_with("--hostlist-exclude="))
            .map(|i| i + 1)
            .or_else(|| {
                block
                    .iter()
                    .rposition(|a| a.starts_with("--hostlist="))
                    .map(|i| i + 1)
            });
        if let Some(at) = insert_at {
            block.insert(at, auto_arg.to_string());
        }
    }

    /// Builds command arguments based on selected strategy or custom parameters.
    fn build_arguments(
        &self,
        binaries: &ZapretBinaries,
        strategy_id: Option<&str>,
        custom_args: Option<Vec<String>>,
    ) -> Result<Vec<String>, ProcessError> {
        // Auto-hostlist lifecycle (design §3/§4): every start path —
        // preset or one-off custom args — funnels through here, so
        // the ensure and the cleanup live here and not only in
        // resolve_strategy_args. The auto file must exist before any
        // spawn that could pass --hostlist-auto, and this point of
        // start_locked is guaranteed to run with the previous engine
        // already stopped, which is what makes the cleanup race-free.
        let lists_dir = binaries
            .lists_dir
            .clone()
            .unwrap_or_else(|| binaries.bin_dir.join("../lists"));
        Self::ensure_user_lists_exist(&lists_dir);
        if let Err(e) = cleanup_auto_hostlist(&lists_dir, &binaries.bin_dir.join("lists")) {
            self.log(
                "warn",
                &format!("Auto-hostlist cleanup failed, continuing start: {}", e),
            );
        }

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

        // 1. If it's a Flowseal preset, build Flowseal arguments using resolved directories.
        // Known presets are exactly the five discovered in
        // platform::process::discover_strategies; any other flowseal-* string
        // (e.g. a typo) used to silently get the weakest general preset
        // via the catch-all in build_flowseal_args.
        if strat_id.starts_with("flowseal-") {
            const KNOWN_FLOWSEAL: [&str; 5] = [
                "flowseal-alt",
                "flowseal-alt11",
                "flowseal-general",
                "flowseal-fake-tls-auto",
                "flowseal-simple-fake",
            ];
            if !KNOWN_FLOWSEAL.contains(&strat_id) {
                return Err(ProcessError::InvalidStrategy(format!(
                    "Неизвестная стратегия Flowseal: {}",
                    strat_id
                )));
            }
            return Ok(Self::build_flowseal_args(strat_id, &binaries.bin_dir, &lists_dir));
        }

        // 2. If it's the flagship zapret2 Ultimate Russia preset, build winws2 arguments
        if strat_id == "zapret2-ru-ultimate" {
            return Ok(Self::build_zapret2_ru_ultimate_args(&binaries.bin_dir, &lists_dir));
        }

        // 2b. Imported community strategies (kinlay0 collection,
        // pinned): bundled argument vectors translated from the
        // collection's .bat files (engine::imported_strategies). The
        // list resolution mirrors branch 4's l() exactly: the bundled
        // copy under bin/lists wins over the user lists directory.
        if let Some(imported) = crate::engine::imported_strategies::find(strat_id) {
            let settings = crate::platform::settings::load_settings();
            let auto = if settings.auto_hostlist_enabled {
                Some(auto_hostlist_path(&lists_dir))
            } else {
                None
            };
            let l = |file: &str| {
                if binaries.bin_dir.join("lists").join(file).is_file() {
                    format!("lists/{}", file)
                } else {
                    lists_dir.join(file).to_string_lossy().replace('\\', "/")
                }
            };
            return Ok(Self::build_imported_args(imported, auto.as_deref(), &l));
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

        // 4. Otherwise, assemble Lua arguments for zapret2 (winws2).
        // The list itself is built by the pure helper
        // build_lua_preset_args; this caller only resolves the
        // filesystem-shaped inputs (lua-init lines, list paths) and
        // reads the current settings, mirroring how
        // build_flowseal_args resolves its l() paths.
        let lib_lua = strategies
            .iter()
            .find(|s| s.filename == "zapret-lib.lua")
            .map(|s| s.path.clone());

        let strat = match target_strategy {
            Some(s) => s,
            None => {
                // No discovered strategy matches: fail loudly instead of
                // starting the engine with generic args for an id nobody
                // resolved (start() already rejects unknown ids, so this is
                // defense in depth against future callers).
                return Err(ProcessError::InvalidStrategy(format!(
                    "Неизвестная стратегия: {}",
                    strat_id
                )));
            }
        };

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

        // Lua library init line: the bin copy wins over the discovered
        // path; absent entirely when neither exists (as before).
        let lib_init: Option<String> =
            if binaries.bin_dir.join("zapret-lib.lua").is_file() {
                Some("--lua-init=@zapret-lib.lua".into())
            } else if let Some(ref lib) = lib_lua {
                Some(format!("--lua-init=@{}", lib.to_string_lossy()))
            } else {
                None
            };

        // Preset script init line: skipped when the preset IS the
        // library (already loaded above, or not loaded at all).
        let preset_init: Option<String> =
            if binaries.bin_dir.join(&strat.filename).is_file() {
                if strat.filename != "zapret-lib.lua" {
                    Some(format!("--lua-init=@{}", strat.filename))
                } else {
                    None
                }
            } else if Some(&strat.path) != lib_lua.as_ref() {
                Some(format!("--lua-init=@{}", strat.path.to_string_lossy()))
            } else {
                None
            };

        // List resolution mirrors build_flowseal_args' l(): the bundled
        // copy under bin/lists wins over the user lists directory.
        let l = |file: &str| {
            if binaries.bin_dir.join("lists").join(file).is_file() {
                format!("lists/{}", file)
            } else {
                lists_dir.join(file).to_string_lossy().replace('\\', "/")
            }
        };

        let settings = crate::platform::settings::load_settings();
        let game_tcp = Self::sanitize_port_spec(&settings.game_filter_tcp);
        let game_udp = Self::sanitize_port_spec(&settings.game_filter_udp);

        // Auto-hostlist paths (design §3): the static lists resolve
        // through the same l() as everywhere else; the auto path is
        // absolute in the user lists directory — the engine writes
        // to it, so the bundled bin/lists copy must never be used.
        let auto_lists = if settings.auto_hostlist_enabled {
            Some(AutoLists {
                general: l("list-general.txt"),
                general_user: l("list-general-user.txt"),
                exclude: l("list-exclude.txt"),
                exclude_user: l("list-exclude-user.txt"),
                auto_path: auto_hostlist_path(&lists_dir),
            })
        } else {
            None
        };

        Ok(Self::build_lua_preset_args(
            settings.split_tunnel_mode,
            &game_tcp,
            &game_udp,
            lib_init.as_deref(),
            preset_init.as_deref(),
            &l("list-google.txt"),
            &l("ipset-all.txt"),
            auto_lists.as_ref(),
            strat.id == "syndata",
        ))
    }

    /// Settings spelling of the split tunnel mode ("full" | "strict" |
    /// "game_mode"), identical to the serde names the frontend sends
    /// to set_split_tunnel_mode, so status values compare directly
    /// against the selected mode.
    fn split_tunnel_mode_str(
        mode: crate::platform::settings::SplitTunnelMode,
    ) -> &'static str {
        use crate::platform::settings::SplitTunnelMode;
        match mode {
            SplitTunnelMode::Full => "full",
            SplitTunnelMode::Strict => "strict",
            SplitTunnelMode::GameMode => "game_mode",
        }
    }

    /// Pure argument builder for the discovered-Lua branch (branch 4
    /// of resolve_strategy_args). Everything filesystem-shaped is
    /// resolved by the caller and passed in: the two --lua-init lines
    /// (None when a script is not loaded), the resolved list paths
    /// used by STRICT and GAME, and the already-sanitized game port
    /// specs. FULL reproduces the historical branch-4 list byte for
    /// byte; STRICT and GAME deviate from it exactly as documented on
    /// each block below.
    ///
    /// `auto` (design §3): when Some, the FULL/GAME main TCP section
    /// gains hostlist gating plus `--hostlist-auto` and stops being
    /// "desync everything captured"; when None the section stays
    /// ungated and the list is byte for byte the historical one.
    /// STRICT never takes the gate (its sections are narrow by
    /// construction), so the parameter is ignored there.
    ///
    /// `syndata`: only the syndata preset (id "syndata") passes true.
    /// Its technique lives in a desync instance of its own (it must
    /// see SYN packets, so it cannot ride the payload-bound fake /
    /// multisplit instances like every other preset variation): the
    /// builder adds one `--lua-desync=syndata:...` line per gated TCP
    /// section, before the section's payload lines, so the instance
    /// inherits the section filters and the auto gate but no payload
    /// restriction. With false the output is unchanged for all
    /// existing presets. The GAME extra sections never get it: game
    /// behaviour stays identical on every preset (fake_plain).
    fn build_lua_preset_args(
        mode: crate::platform::settings::SplitTunnelMode,
        game_tcp: &str,
        game_udp: &str,
        lib_init: Option<&str>,
        preset_init: Option<&str>,
        google_list: &str,
        ipset_all: &str,
        auto: Option<&AutoLists>,
        syndata: bool,
    ) -> Vec<String> {
        use crate::platform::settings::SplitTunnelMode;
        let is_strict = mode == SplitTunnelMode::Strict;
        let is_game = mode == SplitTunnelMode::GameMode;

        let mut args: Vec<String> = Vec::new();

        // Root WinDivert capture filters. STRICT never narrows them
        // (capture without desync is harmless; parity with Flowseal);
        // GAME widens them by the game port specs.
        if is_game {
            args.push(format!(
                "--wf-tcp-out=80,443,2053,2083,2087,2096,8443,{}",
                game_tcp
            ));
            args.push(format!(
                "--wf-udp-out=443,19294-19344,50000-65535,{}",
                game_udp
            ));
        } else {
            args.push("--wf-tcp-out=80,443,2053,2083,2087,2096,8443".into());
            args.push("--wf-udp-out=443,19294-19344,50000-65535".into());
        }

        // UDP fake payloads, mirrored from the flagship's sections. Without
        // them the UDP sections below would have no blob to send; the
        // files live in bin/ next to the engine, same as the flagship.
        args.push("--blob=discord_udp:@ACTIVE_DISCORD_UDP.bin".into());
        args.push("--blob=quic_google:@quic_initial_www_google_com.bin".into());
        if is_game {
            args.push("--blob=game_udp:@ACTIVE_GAME_UDP.bin".into());
        }

        // The preset's TLS desync pair: identical parameters in the
        // FULL section and in both STRICT gated sections. The mode
        // changes WHERE desync applies, never HOW it desyncs.
        let fake_tls =
            "--lua-desync=fake:blob=fake_default_tls:tcp_md5:tls_mod=rnd,rndsni,dupsid";
        let multisplit =
            "--lua-desync=multisplit:pos=2:seqovl=5:seqovl_pattern=0x1603030000";
        // The syndata preset's extra instance: same fake blob and
        // mutation as fake_tls, but delivered inside the SYN packet.
        // Placed before the section's payload lines (see below), so
        // no --payload restriction applies to it.
        let syndata_spec =
            "--lua-desync=syndata:blob=fake_default_tls:tcp_md5:tls_mod=rnd,rndsni,dupsid";

        if is_strict {
            // STRICT (design variant A): the ungated TCP section is
            // replaced by two gated sections on port 443 only, so
            // desync touches YouTube/Discord traffic and nothing
            // else. No port 80, no http_req pair, no Cloudflare
            // ports. The lua-init lines move in front of the sections:
            // their FULL position was inside the unnamed first
            // section, which STRICT does not have; the flagship
            // places --lua-init before all sections, so this follows
            // its proven layout.
            if let Some(init) = lib_init {
                args.push(init.into());
            }
            if let Some(init) = preset_init {
                args.push(init.into());
            }
            args.push("--new=discord_tcp".into());
            args.push("--filter-tcp=443".into());
            args.push(format!(
                "--hostlist-domains={}",
                Self::DISCORD_MEDIA_DOMAINS
            ));
            if syndata {
                args.push(syndata_spec.into());
            }
            args.push("--payload=tls_client_hello".into());
            args.push(fake_tls.into());
            args.push("--payload=tls_client_hello".into());
            args.push(multisplit.into());
            args.push("--new=google_tcp".into());
            args.push("--filter-tcp=443".into());
            args.push(format!("--hostlist={}", google_list));
            if syndata {
                args.push(syndata_spec.into());
            }
            args.push("--payload=tls_client_hello".into());
            args.push(fake_tls.into());
            args.push("--payload=tls_client_hello".into());
            args.push(multisplit.into());
        } else {
            // FULL (and the GAME base): the historical ungated TCP
            // section, byte for byte when auto is None.
            args.push("--filter-tcp=80,443,2053,2083,2087,2096,8443".into());
            if let Some(auto) = auto {
                // Auto-hostlist gating (design §3), right after the
                // filter, before the payload lines: listed hosts get
                // desync immediately, unlisted ones go through the
                // engine's failure detector and are learned into the
                // auto file. list-google is part of the gate on
                // purpose, not an option: youtube/googlevideo live
                // only in list-google, and without it a YouTube
                // preset would route its own traffic through the
                // failure detector (and the auto-tune probes with
                // it) instead of the immediate include.
                args.push(format!("--hostlist={}", auto.general));
                args.push(format!("--hostlist={}", auto.general_user));
                args.push(format!("--hostlist={}", google_list));
                args.push(format!("--hostlist-exclude={}", auto.exclude));
                args.push(format!("--hostlist-exclude={}", auto.exclude_user));
                args.push(format!("--hostlist-auto={}", auto.auto_path));
            }
            if let Some(init) = lib_init {
                args.push(init.into());
            }
            if let Some(init) = preset_init {
                args.push(init.into());
            }
            if syndata {
                args.push(syndata_spec.into());
            }
            args.push("--payload=tls_client_hello".into());
            args.push(fake_tls.into());
            args.push("--payload=http_req".into());
            args.push("--lua-desync=fake:blob=fake_default_http:tcp_md5".into());
            args.push("--payload=tls_client_hello,http_req".into());
            args.push(multisplit.into());
        }

        // UDP sections. Previously branch 4 captured voice/QUIC UDP via
        // --wf-udp-out but defined no --filter-udp section, so Discord
        // voice and YouTube QUIC passed through untouched: voice did
        // not work on any discovered preset. These two sections mirror
        // the flagship's discord_voice and quic_youtube sections (same
        // blobs, same repeats); the QUIC section is intentionally
        // ungated like branch 4's TCP section. Caveat: presets whose
        // fake() loops (yt4k, tspu-max) triple the UDP fakes too
        // (voice: up to 72 in a burst at flow start, out-range
        // n1-n4 x 18, ~86 KB; QUIC: 33 per Initial, vs the
        // flagship's 6/11), and sites-light's pass-through fake()
        // leaves UDP unprotected by design.
        args.push("--new=discord_voice".into());
        args.push("--filter-udp=19294-19344,50000-65535".into());
        args.push("--payload=all".into());
        args.push("--out-range=n1-n4".into());
        args.push("--lua-desync=fake:blob=discord_udp:repeats=6".into());
        if !is_strict {
            // STRICT drops the quic section entirely (parity with
            // Flowseal STRICT): YouTube over QUIC gets no desync and
            // the browser is expected to fall back to TCP. The voice
            // section above stays: voice is Discord.
            args.push("--new=quic".into());
            args.push("--filter-udp=443".into());
            args.push("--payload=quic_initial".into());
            args.push("--lua-desync=fake:blob=quic_google:repeats=11".into());
        }

        if is_game {
            // GAME sections go after the base sections (same order
            // as Flowseal). The game range overlaps base ports
            // (2053, 2083, 2087, 2096, 8443); which section the
            // engine picks on overlap is an upstream assumption
            // (first fully matching section), not a verified fact —
            // see the design draft's open checks. Both sections call
            // the neutral fake_plain() from zapret-lib.lua, never the
            // preset's fake(): game behaviour stays identical on
            // every preset (no yt4k triple send, no sites-light
            // pass-through).
            args.push("--new=game_tcp".into());
            args.push(format!("--filter-tcp={}", game_tcp));
            args.push(format!("--ipset={}", ipset_all));
            args.push("--payload=tls_client_hello,http_req".into());
            args.push("--lua-desync=fake_plain:blob=fake_default_tls".into());
            args.push("--new=game_udp".into());
            args.push(format!("--filter-udp={}", game_udp));
            args.push(format!("--ipset={}", ipset_all));
            args.push("--payload=all".into());
            args.push("--out-range=n1-n3".into());
            args.push("--lua-desync=fake_plain:blob=game_udp:repeats=12".into());
        }

        args
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
        // Auto-hostlist (design §3, critical invariant): the file
        // must exist before any engine start that passes
        // --hostlist-auto. Plain hostlists are fatal to the engine
        // when missing ("failed to register hostlist" on the pinned
        // binary), and the Windows builds' behavior for a missing
        // auto file is not something to discover in production, so
        // the app creates the file itself — empty apart from the
        // explanatory header — and never overwrites an existing one.
        let auto_path = lists_dir.join(AUTO_HOSTLIST_FILE);
        if !auto_path.exists() {
            let _ = std::fs::write(&auto_path, AUTO_HOSTLIST_HEADER);
        }
    }

    fn emit_state_change(&self, state: SupervisorState) {
        self.sink.emit_state_changed(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tunnel_restart_decision_truth_table() {
        // Stage 5 (O1): the restart is needed exactly when the
        // running desync lacks the WARP args AND the args apply to
        // its engine. Under winws v1 they never apply, so no
        // restart - including the launch that started with the
        // switch off, where step 3c never made the decision.
        assert!(Supervisor::tunnel_needs_desync_restart(false, false));
        assert!(!Supervisor::tunnel_needs_desync_restart(true, false));
        assert!(!Supervisor::tunnel_needs_desync_restart(false, true));
        assert!(!Supervisor::tunnel_needs_desync_restart(true, true));
    }

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

    /// Shared inputs for the branch-4 builder tests: the bin copies of
    /// both scripts loaded relatively, default game port specs, and
    /// bundled list paths (the l() relative form).
    fn lua_test_args(
        mode: crate::platform::settings::SplitTunnelMode,
    ) -> Vec<String> {
        Supervisor::build_lua_preset_args(
            mode,
            "1024-65535",
            "1024-65535",
            Some("--lua-init=@zapret-lib.lua"),
            Some("--lua-init=@zapret-yt4k.lua"),
            "lists/list-google.txt",
            "lists/ipset-all.txt",
            None,
            false,
        )
    }

    #[test]
    fn test_lua_full_mode_matches_historical_branch4_args() {
        // FULL is the anchor: byte for byte the list branch 4 built
        // before the tunnel mode existed.
        let expected: Vec<String> = [
            "--wf-tcp-out=80,443,2053,2083,2087,2096,8443",
            "--wf-udp-out=443,19294-19344,50000-65535",
            "--blob=discord_udp:@ACTIVE_DISCORD_UDP.bin",
            "--blob=quic_google:@quic_initial_www_google_com.bin",
            "--filter-tcp=80,443,2053,2083,2087,2096,8443",
            "--lua-init=@zapret-lib.lua",
            "--lua-init=@zapret-yt4k.lua",
            "--payload=tls_client_hello",
            "--lua-desync=fake:blob=fake_default_tls:tcp_md5:tls_mod=rnd,rndsni,dupsid",
            "--payload=http_req",
            "--lua-desync=fake:blob=fake_default_http:tcp_md5",
            "--payload=tls_client_hello,http_req",
            "--lua-desync=multisplit:pos=2:seqovl=5:seqovl_pattern=0x1603030000",
            "--new=discord_voice",
            "--filter-udp=19294-19344,50000-65535",
            "--payload=all",
            "--out-range=n1-n4",
            "--lua-desync=fake:blob=discord_udp:repeats=6",
            "--new=quic",
            "--filter-udp=443",
            "--payload=quic_initial",
            "--lua-desync=fake:blob=quic_google:repeats=11",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        let args = lua_test_args(crate::platform::settings::SplitTunnelMode::Full);
        assert_eq!(args, expected);
    }

    #[test]
    fn test_lua_syndata_preset_adds_syn_instance_before_payloads() {
        use crate::platform::settings::SplitTunnelMode;
        const SYNDATA_SPEC: &str =
            "--lua-desync=syndata:blob=fake_default_tls:tcp_md5:tls_mod=rnd,rndsni,dupsid";
        let build = |mode: SplitTunnelMode| {
            Supervisor::build_lua_preset_args(
                mode,
                "1024-65535",
                "1024-65535",
                Some("--lua-init=@zapret-lib.lua"),
                Some("--lua-init=@zapret-syndata.lua"),
                "lists/list-google.txt",
                "lists/ipset-all.txt",
                None,
                true,
            )
        };
        let count = |args: &[String]| args.iter().filter(|a| a.as_str() == SYNDATA_SPEC).count();

        // FULL: exactly one instance, after the preset init and
        // before the first payload-bound instance (no --payload
        // restriction may precede it, or it would never see a SYN).
        let full = build(SplitTunnelMode::Full);
        assert_eq!(count(&full), 1);
        let syn_idx = full.iter().position(|a| a == SYNDATA_SPEC).unwrap();
        let init_idx = full
            .iter()
            .position(|a| a == "--lua-init=@zapret-syndata.lua")
            .unwrap();
        let payload_idx = full
            .iter()
            .position(|a| a.starts_with("--payload="))
            .unwrap();
        assert!(init_idx < syn_idx && syn_idx < payload_idx);
        // Without the flag the instance is absent (existing presets
        // are byte-identical, as pinned by the FULL test above).
        assert_eq!(count(&lua_test_args(SplitTunnelMode::Full)), 0);

        // STRICT: one instance per gated TCP section.
        let strict = build(SplitTunnelMode::Strict);
        assert_eq!(count(&strict), 2);

        // GAME: still exactly one (the main section); the game
        // sections keep fake_plain and get no syndata.
        let game = build(SplitTunnelMode::GameMode);
        assert_eq!(count(&game), 1);
        assert!(game
            .iter()
            .any(|a| a == "--lua-desync=fake_plain:blob=fake_default_tls"));
    }

    #[test]
    fn test_lua_syndata_preset_stays_behind_auto_gate() {
        use crate::platform::settings::SplitTunnelMode;
        const SYNDATA_SPEC: &str =
            "--lua-desync=syndata:blob=fake_default_tls:tcp_md5:tls_mod=rnd,rndsni,dupsid";
        // With auto-hostlist on (FULL and GAME share the gated main
        // section), the instance must stay behind the WHOLE gate —
        // all six lines — and still ahead of the first payload line,
        // so no hostlist restriction applies to it either.
        let auto = lua_test_auto();
        let gate: Vec<String> = vec![
            format!("--hostlist={}", auto.general),
            format!("--hostlist={}", auto.general_user),
            "--hostlist=lists/list-google.txt".to_string(),
            format!("--hostlist-exclude={}", auto.exclude),
            format!("--hostlist-exclude={}", auto.exclude_user),
            format!("--hostlist-auto={}", auto.auto_path),
        ];
        for mode in [SplitTunnelMode::Full, SplitTunnelMode::GameMode] {
            let args = Supervisor::build_lua_preset_args(
                mode,
                "1024-65535",
                "1024-65535",
                Some("--lua-init=@zapret-lib.lua"),
                Some("--lua-init=@zapret-syndata.lua"),
                "lists/list-google.txt",
                "lists/ipset-all.txt",
                Some(&auto),
                true,
            );
            assert_eq!(
                args.iter().filter(|a| a.as_str() == SYNDATA_SPEC).count(),
                1,
                "{:?}: syndata count with auto",
                mode
            );
            let syn_idx = args.iter().position(|a| a == SYNDATA_SPEC).unwrap();
            let last_gate_idx = gate
                .iter()
                .map(|g| {
                    args.iter()
                        .position(|a| a == g)
                        .unwrap_or_else(|| panic!("{:?}: gate line {} missing", mode, g))
                })
                .max()
                .unwrap();
            let payload_idx = args
                .iter()
                .position(|a| a.starts_with("--payload="))
                .unwrap();
            assert!(
                last_gate_idx < syn_idx && syn_idx < payload_idx,
                "{:?}: syndata must sit after the whole auto gate and before the first payload line",
                mode
            );
        }
    }

    #[test]
    fn test_lua_strict_mode_gates_tcp_and_drops_quic() {
        let args = lua_test_args(crate::platform::settings::SplitTunnelMode::Strict);

        // Root capture filters are NOT narrowed (Flowseal parity).
        assert_eq!(args[0], "--wf-tcp-out=80,443,2053,2083,2087,2096,8443");
        assert_eq!(args[1], "--wf-udp-out=443,19294-19344,50000-65535");

        // No ungated TCP section survives: the only --filter-tcp
        // values are the two gated 443 sections.
        assert!(!args
            .iter()
            .any(|a| a == "--filter-tcp=80,443,2053,2083,2087,2096,8443"));
        let tcp_filters: Vec<&String> = args
            .iter()
            .filter(|a| a.starts_with("--filter-tcp="))
            .collect();
        assert_eq!(tcp_filters.len(), 2);
        assert!(tcp_filters.iter().all(|a| *a == "--filter-tcp=443"));

        // No http_req pair anywhere (neither the standalone payload
        // nor the combined one from FULL).
        assert!(!args.iter().any(|a| a.contains("http_req")));

        // Both gated sections exist with their gates.
        assert!(args.iter().any(|a| a == "--new=discord_tcp"));
        assert!(args.iter().any(|a| a
            == &format!(
                "--hostlist-domains={}",
                Supervisor::DISCORD_MEDIA_DOMAINS
            )));
        assert!(args.iter().any(|a| a == "--new=google_tcp"));
        assert!(args
            .iter()
            .any(|a| a == "--hostlist=lists/list-google.txt"));

        // The lua-init lines precede the first gated section.
        let lib_idx = args
            .iter()
            .position(|a| a == "--lua-init=@zapret-lib.lua")
            .expect("lib init present");
        let first_section_idx = args
            .iter()
            .position(|a| a == "--new=discord_tcp")
            .expect("discord_tcp present");
        assert!(lib_idx < first_section_idx);

        // Voice stays; the quic section is gone entirely.
        assert!(args.iter().any(|a| a == "--new=discord_voice"));
        assert!(!args.iter().any(|a| a == "--new=quic"));
        assert!(!args.iter().any(|a| a == "--payload=quic_initial"));
    }

    /// Linux wiring (phase 1, step 3): the port sets translated to
    /// nftables rules must be EXACTLY the wf capture sets the builders
    /// emit for every tunnel mode — no wider (foreign traffic would be
    /// queued), no narrower (the mode would silently under-capture).
    /// Compared token for token against the wf specs as written.
    #[cfg(target_os = "linux")]
    #[test]
    fn test_linux_nft_sets_equal_builder_wf_sets() {
        use crate::platform::nftables;
        use crate::platform::settings::SplitTunnelMode;

        let cases = [
            (SplitTunnelMode::Full, "80,443,2053,2083,2087,2096,8443", "443,19294-19344,50000-65535"),
            (SplitTunnelMode::Strict, "80,443,2053,2083,2087,2096,8443", "443,19294-19344,50000-65535"),
            (
                SplitTunnelMode::GameMode,
                "80,443,2053,2083,2087,2096,8443,1024-65535",
                "443,19294-19344,50000-65535,1024-65535",
            ),
        ];
        for (mode, wf_tcp, wf_udp) in cases {
            let args = lua_test_args(mode);
            let original_sets = nftables::extract_wf_port_sets(&args);
            assert_eq!(original_sets.tcp.join(","), wf_tcp, "wf source drifted for {:?}", mode);
            assert_eq!(original_sets.udp.join(","), wf_udp, "wf source drifted for {:?}", mode);

            let rewritten = nftables::rewrite_args_for_nfqws2(args);
            assert_eq!(rewritten.port_sets, original_sets, "translation changed the sets for {:?}", mode);
            assert!(!rewritten.args.iter().any(|a| a.starts_with("--wf-")));
            assert_eq!(
                rewritten.args.iter().filter(|a| a.starts_with("--qnum=")).count(),
                1
            );
            // The install commands queue exactly those ports.
            let cmds = nftables::build_install_commands(&rewritten.port_sets, rewritten.queue_num);
            let joined: Vec<String> = cmds.iter().flatten().cloned().collect();
            for token in original_sets.tcp.iter().chain(original_sets.udp.iter()) {
                assert!(
                    joined.iter().any(|a| a.contains(token.as_str())),
                    "port token {} missing from nft commands for {:?}",
                    token,
                    mode
                );
            }
        }

        // The flagship builder takes the same path (its wf pair is at
        // the head of the list).
        let ultimate = Supervisor::build_zapret2_ru_ultimate_args(
            std::path::Path::new("bin"),
            std::path::Path::new("lists"),
        );
        let rewritten = nftables::rewrite_args_for_nfqws2(ultimate);
        assert_eq!(rewritten.port_sets.tcp.join(","), "80,443,2053,2083,2087,2096,8443");
        assert_eq!(rewritten.port_sets.udp.join(","), "443,19294-19344,50000-65535");
        assert!(!rewritten.args.iter().any(|a| a.starts_with("--wf-")));
    }

    #[test]
    fn test_lua_game_mode_extends_full_with_game_sections() {
        let args =
            lua_test_args(crate::platform::settings::SplitTunnelMode::GameMode);

        // Capture filters widened by the game specs.
        assert_eq!(
            args[0],
            "--wf-tcp-out=80,443,2053,2083,2087,2096,8443,1024-65535"
        );
        assert_eq!(
            args[1],
            "--wf-udp-out=443,19294-19344,50000-65535,1024-65535"
        );

        // Game blob defined; base sections untouched.
        assert!(args
            .iter()
            .any(|a| a == "--blob=game_udp:@ACTIVE_GAME_UDP.bin"));
        assert!(args
            .iter()
            .any(|a| a == "--filter-tcp=80,443,2053,2083,2087,2096,8443"));
        assert!(args.iter().any(|a| a == "--new=discord_voice"));
        assert!(args.iter().any(|a| a == "--new=quic"));

        // Game sections present, with the neutral fake_plain and the
        // ipset gate, after the base sections.
        assert!(args.iter().any(|a| a == "--new=game_tcp"));
        assert!(args.iter().any(|a| a == "--new=game_udp"));
        assert!(args
            .iter()
            .any(|a| a == "--lua-desync=fake_plain:blob=fake_default_tls"));
        assert!(args
            .iter()
            .any(|a| a == "--lua-desync=fake_plain:blob=game_udp:repeats=12"));
        assert!(args
            .iter()
            .any(|a| a == "--ipset=lists/ipset-all.txt"));
        let quic_idx = args
            .iter()
            .position(|a| a == "--new=quic")
            .expect("quic present");
        let game_idx = args
            .iter()
            .position(|a| a == "--new=game_tcp")
            .expect("game_tcp present");
        assert!(game_idx > quic_idx);
    }

    // ---- Auto-hostlist (design: AUTO-HOSTLIST-DESIGN-DRAFT) ----

    fn lua_test_auto() -> AutoLists {
        AutoLists {
            general: "lists/list-general.txt".into(),
            general_user: "lists/list-general-user.txt".into(),
            exclude: "lists/list-exclude.txt".into(),
            exclude_user: "lists/list-exclude-user.txt".into(),
            auto_path: "/user/lists/list-auto.txt".into(),
        }
    }

    fn lua_args_with_auto(
        mode: crate::platform::settings::SplitTunnelMode,
    ) -> Vec<String> {
        let auto = lua_test_auto();
        Supervisor::build_lua_preset_args(
            mode,
            "1024-65535",
            "1024-65535",
            Some("--lua-init=@zapret-lib.lua"),
            Some("--lua-init=@zapret-yt4k.lua"),
            "lists/list-google.txt",
            "lists/ipset-all.txt",
            Some(&auto),
            false,
        )
    }

    #[test]
    fn test_lua_full_auto_gates_main_section_in_design_order() {
        use crate::platform::settings::SplitTunnelMode;
        let with_auto = lua_args_with_auto(SplitTunnelMode::Full);
        let without = lua_test_args(SplitTunnelMode::Full);

        // The gate sits right after the main TCP filter, before the
        // lua-init and payload lines, in the design's exact order.
        let filter_idx = with_auto
            .iter()
            .position(|a| a == "--filter-tcp=80,443,2053,2083,2087,2096,8443")
            .expect("main filter present");
        let gate: Vec<&str> = [
            "--hostlist=lists/list-general.txt",
            "--hostlist=lists/list-general-user.txt",
            "--hostlist=lists/list-google.txt",
            "--hostlist-exclude=lists/list-exclude.txt",
            "--hostlist-exclude=lists/list-exclude-user.txt",
            "--hostlist-auto=/user/lists/list-auto.txt",
        ]
        .to_vec();
        for (i, want) in gate.iter().enumerate() {
            assert_eq!(&with_auto[filter_idx + 1 + i], want, "gate line {} misplaced", i);
        }

        // Removing the six gate lines reproduces the historical
        // FULL list byte for byte: auto changes nothing else.
        let stripped: Vec<String> = with_auto
            .iter()
            .filter(|a| !gate.contains(&a.as_str()))
            .cloned()
            .collect();
        assert_eq!(stripped, without);
    }

    #[test]
    fn test_lua_game_auto_gates_only_the_main_section() {
        use crate::platform::settings::SplitTunnelMode;
        let args = lua_args_with_auto(SplitTunnelMode::GameMode);
        assert_eq!(
            args.iter().filter(|a| a.starts_with("--hostlist-auto=")).count(),
            1
        );
        // The game sections stay ipset-gated, no hostlists at all.
        let game_idx = args
            .iter()
            .position(|a| a == "--new=game_tcp")
            .expect("game_tcp present");
        assert!(!args[game_idx..].iter().any(|a| a.starts_with("--hostlist")));
    }

    #[test]
    fn test_lua_strict_ignores_auto() {
        use crate::platform::settings::SplitTunnelMode;
        // STRICT takes no gate by construction: Some and None are
        // byte-identical.
        assert_eq!(
            lua_args_with_auto(SplitTunnelMode::Strict),
            lua_test_args(SplitTunnelMode::Strict)
        );
    }

    /// Splits an argument list into sections on a separator token
    /// ("--new" for Flowseal, "--new=<name>" handling is the
    /// caller's — flagship sections are checked by span instead).
    fn split_on_new(args: &[String]) -> Vec<Vec<String>> {
        let mut blocks: Vec<Vec<String>> = vec![Vec::new()];
        for a in args {
            if a == "--new" {
                blocks.push(Vec::new());
            } else {
                blocks.last_mut().unwrap().push(a.clone());
            }
        }
        blocks
    }

    fn flowseal_inner(
        preset: &str,
        mode: crate::platform::settings::SplitTunnelMode,
        auto: Option<&str>,
    ) -> Vec<String> {
        Supervisor::build_flowseal_args_inner(
            preset,
            std::path::Path::new("bin-test"),
            std::path::Path::new("lists-test"),
            mode,
            "1024-65535",
            "1024-65535",
            auto,
        )
    }

    #[test]
    fn test_flowseal_simple_fake_without_auto_matches_historical_args() {
        use crate::platform::settings::SplitTunnelMode;
        // OFF pin: byte for byte the list this preset built before
        // the auto-hostlist feature (bin-test/lists-test stand in
        // for bin_dir/lists_dir; neither exists, so b()/l() take
        // their path forms deterministically).
        let b = |f: &str| format!("bin-test/{}", f);
        let l = |f: &str| format!("lists-test/{}", f);
        let expected: Vec<String> = vec![
            "--wf-tcp=80,443,2053,2083,2087,2096,8443".into(),
            "--wf-udp=443,19294-19344,50000-65535".into(),
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
            format!("--hostlist-domains={}", Supervisor::DISCORD_MEDIA_DOMAINS),
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
        ];
        let args = flowseal_inner("flowseal-simple-fake", SplitTunnelMode::Full, None);
        assert_eq!(args, expected);
    }

    #[test]
    fn test_flowseal_auto_lands_only_in_tcp_general_sections() {
        use crate::platform::settings::SplitTunnelMode;
        const AUTO: &str = "/user/lists/list-auto.txt";
        for preset in [
            "flowseal-alt",
            "flowseal-alt11",
            "flowseal-general",
            "flowseal-fake-tls-auto",
            "flowseal-simple-fake",
        ] {
            for mode in [SplitTunnelMode::Full, SplitTunnelMode::GameMode] {
                let args = flowseal_inner(preset, mode, Some(AUTO));
                let mut auto_total = 0;
                for block in split_on_new(&args) {
                    let is_tcp = block.iter().any(|a| a.starts_with("--filter-tcp"));
                    let has_general = block.iter().any(|a| {
                        a.strip_prefix("--hostlist=")
                            .map_or(false, |v| v.rsplit('/').next() == Some("list-general.txt"))
                    });
                    let auto_at: Vec<usize> = block
                        .iter()
                        .enumerate()
                        .filter(|(_, a)| a.starts_with("--hostlist-auto="))
                        .map(|(i, _)| i)
                        .collect();
                    if is_tcp && has_general {
                        assert_eq!(
                            auto_at.len(),
                            1,
                            "{} {:?}: TCP general section must carry exactly one auto line",
                            preset,
                            mode
                        );
                        // ... right after the section's last exclude line.
                        let at = auto_at[0];
                        assert!(
                            block[at - 1].starts_with("--hostlist-exclude="),
                            "{} {:?}: auto line must follow the exclude lines",
                            preset,
                            mode
                        );
                        assert_eq!(block[at], format!("--hostlist-auto={}", AUTO));
                        auto_total += 1;
                    } else {
                        assert!(
                            auto_at.is_empty(),
                            "{} {:?}: non-target section gained an auto line",
                            preset,
                            mode
                        );
                    }
                }
                assert!(auto_total >= 1, "{} {:?}: no section learned", preset, mode);
            }

            // STRICT drops the general blocks whole — auto with them.
            let strict_args = flowseal_inner(preset, SplitTunnelMode::Strict, Some(AUTO));
            assert!(
                !strict_args.iter().any(|a| a.starts_with("--hostlist-auto=")),
                "{}: STRICT must not carry auto",
                preset
            );
        }
    }

    #[test]
    fn test_ultimate_without_auto_matches_historical_args() {
        // OFF pin: byte for byte the flagship list before the
        // auto-hostlist feature.
        let expected: Vec<String> = vec![
            "--wf-tcp-out=80,443,2053,2083,2087,2096,8443".into(),
            "--wf-udp-out=443,19294-19344,50000-65535".into(),
            "--lua-init=@zapret-lib.lua".into(),
            "--lua-init=@zapret-antidpi.lua".into(),
            "--blob=quic_google:@quic_initial_www_google_com.bin".into(),
            "--blob=discord_udp:@ACTIVE_DISCORD_UDP.bin".into(),
            "--blob=tls_google:@tls_clienthello_www_google_com.bin".into(),
            "--blob=tls_max_ru:@tls_clienthello_max_ru.bin".into(),
            "--blob=stun2:@stun2.bin".into(),
            "--new=quic_youtube".into(),
            "--filter-udp=443".into(),
            "--hostlist=lists/list-general.txt".into(),
            "--hostlist=lists/list-general-user.txt".into(),
            "--hostlist-exclude=lists/list-exclude.txt".into(),
            "--payload=quic_initial".into(),
            "--lua-desync=fake:blob=quic_google:repeats=11".into(),
            "--new=discord_voice".into(),
            "--filter-udp=19294-19344,50000-65535".into(),
            "--payload=all".into(),
            "--out-range=n1-n4".into(),
            "--lua-desync=fake:blob=discord_udp:repeats=6".into(),
            "--new=discord_media".into(),
            "--filter-tcp=443,2053,2083,2087,2096,8443".into(),
            format!("--hostlist-domains={}", Supervisor::DISCORD_MEDIA_DOMAINS),
            "--payload=tls_client_hello".into(),
            "--out-range=n1-n4".into(),
            "--lua-desync=fake:blob=tls_google:repeats=8:tcp_ts=1000".into(),
            "--lua-desync=multisplit:pos=2:seqovl=681:seqovl_pattern=tls_google:tcp_ts=1000".into(),
            "--new=youtube_4k".into(),
            "--filter-tcp=443".into(),
            "--hostlist=lists/list-google.txt".into(),
            "--payload=tls_client_hello".into(),
            "--out-range=n1-n4".into(),
            "--lua-desync=fake:blob=tls_google:repeats=8:tcp_ts=1000:ip_id=zero".into(),
            "--lua-desync=multisplit:pos=2:seqovl=681:seqovl_pattern=tls_google:tcp_ts=1000:ip_id=zero".into(),
            "--new=general_https".into(),
            "--filter-tcp=80,443".into(),
            "--hostlist=lists/list-general.txt".into(),
            "--hostlist=lists/list-general-user.txt".into(),
            "--hostlist-exclude=lists/list-exclude.txt".into(),
            "--payload=tls_client_hello".into(),
            "--out-range=n1-n4".into(),
            "--lua-desync=fake:blob=tls_max_ru:repeats=8:tcp_ts=1000".into(),
            "--lua-desync=multisplit:pos=2:seqovl=664:seqovl_pattern=tls_max_ru:tcp_ts=1000".into(),
            "--new=general_http".into(),
            "--filter-tcp=80".into(),
            "--hostlist=lists/list-general.txt".into(),
            "--payload=http_req".into(),
            "--lua-desync=fake:blob=tls_max_ru:tcp_ts=1000".into(),
            "--lua-desync=multisplit:pos=2:seqovl=664:seqovl_pattern=tls_max_ru:tcp_ts=1000".into(),
            "--new=ipset_fallback".into(),
            "--filter-tcp=80,443,8443".into(),
            "--ipset=lists/ipset-all.txt".into(),
            "--hostlist-exclude=lists/list-exclude.txt".into(),
            "--payload=tls_client_hello".into(),
            "--out-range=n1-n4".into(),
            "--lua-desync=fake:blob=tls_max_ru:repeats=8:tcp_ts=1000".into(),
            "--lua-desync=multisplit:pos=2:seqovl=664:seqovl_pattern=tls_max_ru:tcp_ts=1000".into(),
        ];
        assert_eq!(Supervisor::build_ultimate_args_with_auto(None), expected);
    }

    #[test]
    fn test_ultimate_with_auto_extends_only_general_sections() {
        const AUTO: &str = "/user/lists/list-auto.txt";
        let base = Supervisor::build_ultimate_args_with_auto(None);
        let with_auto = Supervisor::build_ultimate_args_with_auto(Some(AUTO));

        // Reconstruct the expected list from the pinned base: the
        // ONLY differences are the two insertion points.
        let mut expected = base.clone();
        let https_anchor = expected
            .iter()
            .position(|a| a == "--new=general_https")
            .unwrap()
            + 5; // marker, filter, hostlist, hostlist-user, exclude → insert after exclude
        expected.insert(https_anchor, format!("--hostlist-auto={}", AUTO));
        expected.insert(
            https_anchor,
            "--hostlist-exclude=lists/list-exclude-user.txt".into(),
        );
        let http_hostlist = expected
            .iter()
            .position(|a| a == "--new=general_http")
            .unwrap()
            + 2; // marker, filter → the --hostlist line; insert after it
        expected.insert(http_hostlist + 1, format!("--hostlist-auto={}", AUTO));
        expected.insert(
            http_hostlist + 1,
            "--hostlist-exclude=lists/list-exclude-user.txt".into(),
        );
        expected.insert(
            http_hostlist + 1,
            "--hostlist-exclude=lists/list-exclude.txt".into(),
        );
        assert_eq!(with_auto, expected);
        assert_eq!(
            with_auto.iter().filter(|a| a.starts_with("--hostlist-auto=")).count(),
            2
        );
        // general_http did NOT gain list-general-user (design §3:
        // its include side is unchanged).
        let http_idx = with_auto.iter().position(|a| a == "--new=general_http").unwrap();
        let ipset_idx = with_auto.iter().position(|a| a == "--new=ipset_fallback").unwrap();
        assert!(!with_auto[http_idx..ipset_idx]
            .iter()
            .any(|a| a.contains("list-general-user")));
    }

    fn temp_lists_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "zapret2-auto-hostlist-test-{}-{}",
            tag,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_ensure_creates_auto_file_with_header_and_never_overwrites() {
        let dir = temp_lists_dir("ensure");
        Supervisor::ensure_user_lists_exist(&dir);
        let auto_path = dir.join(AUTO_HOSTLIST_FILE);
        assert!(auto_path.is_file(), "ensure must create list-auto.txt");
        let content = std::fs::read_to_string(&auto_path).unwrap();
        assert!(content.starts_with('#'), "fresh auto file carries the header");

        std::fs::write(&auto_path, "learned.example\n").unwrap();
        Supervisor::ensure_user_lists_exist(&dir);
        assert_eq!(
            std::fs::read_to_string(&auto_path).unwrap(),
            "learned.example\n",
            "ensure must not overwrite an existing auto file"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_cleanup_auto_hostlist_dedups_and_graduates() {
        let dir = temp_lists_dir("cleanup");
        let bundle = temp_lists_dir("cleanup-bundle");
        std::fs::write(dir.join("list-general.txt"), "example.com\n").unwrap();
        std::fs::write(dir.join("list-google.txt"), "googlevideo.com\n").unwrap();
        std::fs::write(dir.join("list-exclude-user.txt"), "banned.org\n").unwrap();
        let header = "# zapret2-gui: header line\n# second header line\n";
        let body = [
            "example.com",        // graduated: exact static include
            "cdn.example.com",    // graduated: parent in list-general
            "sub.googlevideo.com", // graduated: parent in list-google
            "banned.org",         // covered by exclude-user
            "sub.banned.org",     // covered by exclude parent
            "fresh-site.net",     // kept
            "FRESH-site.net",     // duplicate modulo case
            "fresh-site.net",     // exact duplicate
            "other.example.org",  // kept
        ];
        std::fs::write(
            dir.join(AUTO_HOSTLIST_FILE),
            format!("{}{}\n", header, body.join("\n")),
        )
        .unwrap();

        cleanup_auto_hostlist(&dir, &bundle).expect("cleanup must succeed");
        let result = std::fs::read_to_string(dir.join(AUTO_HOSTLIST_FILE)).unwrap();
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines[0], "# zapret2-gui: header line");
        assert_eq!(lines[1], "# second header line");
        assert_eq!(lines[2..], ["fresh-site.net", "other.example.org"]);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&bundle);
    }

    #[test]
    fn test_cleanup_auto_hostlist_fifo_ceiling() {
        let dir = temp_lists_dir("ceiling");
        let bundle = temp_lists_dir("ceiling-bundle");
        let total = AUTO_HOSTLIST_CEILING + 5;
        let mut content = String::new();
        for i in 0..total {
            content.push_str(&format!("e{}.example\n", i));
        }
        std::fs::write(dir.join(AUTO_HOSTLIST_FILE), &content).unwrap();

        cleanup_auto_hostlist(&dir, &bundle).expect("cleanup must succeed");
        let result = std::fs::read_to_string(dir.join(AUTO_HOSTLIST_FILE)).unwrap();
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines.len(), AUTO_HOSTLIST_CEILING);
        assert_eq!(lines[0], "e5.example", "oldest five dropped from the head");
        assert_eq!(lines[lines.len() - 1], format!("e{}.example", total - 1));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&bundle);
    }

    #[test]
    fn test_cleanup_auto_hostlist_no_change_leaves_file_untouched() {
        let dir = temp_lists_dir("nochange");
        let bundle = temp_lists_dir("nochange-bundle");
        let original = "# header\nfresh-site.net\nother.example.org\n";
        std::fs::write(dir.join(AUTO_HOSTLIST_FILE), original).unwrap();
        cleanup_auto_hostlist(&dir, &bundle).expect("cleanup must succeed");
        assert_eq!(
            std::fs::read_to_string(dir.join(AUTO_HOSTLIST_FILE)).unwrap(),
            original,
            "a no-op cleanup must not rewrite the file"
        );
        // Missing file is a no-op, not an error.
        let empty = temp_lists_dir("missing");
        cleanup_auto_hostlist(&empty, &bundle).expect("missing auto file is fine");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&bundle);
        let _ = std::fs::remove_dir_all(&empty);
    }

    /// Builds imported-strategy args the way branch 2b of
    /// resolve_strategy_args does, with bundled list resolution.
    fn imported_test_args(
        id: &str,
        auto: Option<&str>,
    ) -> Vec<String> {
        let imported = crate::engine::imported_strategies::find(id)
            .unwrap_or_else(|| panic!("unknown imported strategy {}", id));
        Supervisor::build_imported_args(imported, auto, &|f| format!("lists/{}", f))
    }

    /// The `--new=<slug>` section span of a built argument list.
    fn section_of<'a>(args: &'a [String], slug: &str) -> &'a [String] {
        let marker = format!("--new={}", slug);
        let start = args
            .iter()
            .position(|a| a == &marker)
            .unwrap_or_else(|| panic!("section {} missing", slug));
        let end = args
            .iter()
            .skip(start + 1)
            .position(|a| a.starts_with("--new="))
            .map(|p| start + 1 + p)
            .unwrap_or(args.len());
        &args[start..end]
    }

    #[test]
    fn test_imported_args_shape_for_all_strategies() {
        let all = crate::engine::imported_strategies::all();
        assert_eq!(all.len(), 7, "first kinlay0 group is exactly 7 strategies");
        for strat in all {
            let args = imported_test_args(strat.id, None);
            // Our root capture first, stock lua inits, no collection
            // wrapping anywhere in the output.
            assert_eq!(args[0], "--wf-tcp-out=80,443,2053,2083,2087,2096,8443", "{}", strat.id);
            assert_eq!(args[1], "--wf-udp-out=443,19294-19344,50000-65535", "{}", strat.id);
            assert_eq!(
                args.iter().filter(|a| a.starts_with("--lua-init=")).count(),
                3,
                "{}",
                strat.id
            );
            for arg in &args {
                for banned in [
                    "blobs.lua",
                    "gamefilter",
                    "GameFilter",
                    "%",
                    "--ctrack",
                    "--wf-tcp-in",
                    "--wf-udp-in",
                    "{L}",
                ] {
                    assert!(
                        !arg.contains(banned),
                        "{}: arg {:?} contains banned {:?}",
                        strat.id,
                        arg,
                        banned
                    );
                }
            }
            // The .bat game sections are dropped: 9 profiles become 7.
            let sections: Vec<&String> = args
                .iter()
                .filter(|a| a.starts_with("--new="))
                .collect();
            assert_eq!(sections.len(), 7, "{}", strat.id);
            // Blob completeness: every blob a desync chain references
            // is registered our way before the profiles start.
            let first_section = args
                .iter()
                .position(|a| a.starts_with("--new="))
                .unwrap();
            for arg in &args {
                if let Some(pos) = arg.find("--lua-desync=") {
                    for part in arg[pos..].split(':') {
                        if let Some(name) = part.strip_prefix("blob=") {
                            let reg = format!("--blob={}:@", name);
                            assert!(
                                args[..first_section]
                                    .iter()
                                    .any(|a| a.starts_with(&reg)),
                                "{}: blob {} used but not registered",
                                strat.id,
                                name
                            );
                        }
                    }
                }
            }
            // The auto-hostlist target section exists in every one.
            let _ = section_of(&args, "domain_tls");
        }
    }

    #[test]
    fn test_imported_double_fake_really_has_two_fakes() {
        // The family's defining trait, pinned so a translator or pin
        // change cannot silently flatten it: in domain_tls the DOUBLE
        // FAKE chains carry two fake instances on the TLS payload
        // (plus one on HTTP), SINGLE FAKE carries one (plus one).
        let count_fakes = |id: &str| {
            let args = imported_test_args(id, None);
            section_of(&args, "domain_tls")
                .iter()
                .filter(|a| a.starts_with("--lua-desync=fake:"))
                .count()
        };
        assert_eq!(count_fakes("kinlay0-double-fake"), 3);
        assert_eq!(count_fakes("kinlay0-double-fake-alt"), 3);
        assert_eq!(count_fakes("kinlay0-double-fake-alt2"), 3);
        assert_eq!(count_fakes("kinlay0-single-fake"), 2);
        // hostfakesplit's domain_tls leads with hostfakesplit, not fake.
        let args = imported_test_args("kinlay0-hostfakesplit-ts", None);
        let section = section_of(&args, "domain_tls");
        assert!(section
            .iter()
            .any(|a| a.starts_with("--lua-desync=hostfakesplit:")));
    }

    #[test]
    fn test_imported_auto_hostlist_injected_only_into_domain_tls() {
        let with = imported_test_args(
            "kinlay0-double-fake",
            Some("/tmp/lists/list-auto.txt"),
        );
        let auto_args: Vec<&String> = with
            .iter()
            .filter(|a| a.starts_with("--hostlist-auto="))
            .collect();
        assert_eq!(auto_args.len(), 1);
        assert_eq!(auto_args[0], "--hostlist-auto=/tmp/lists/list-auto.txt");
        // It sits inside domain_tls, right after the user exclude.
        let section = section_of(&with, "domain_tls");
        let at = section
            .iter()
            .position(|a| a.starts_with("--hostlist-auto="))
            .unwrap();
        assert_eq!(
            section[at - 1],
            "--hostlist-exclude=lists/list-exclude-user.txt"
        );
        // Setting off: no auto line anywhere.
        let without = imported_test_args("kinlay0-double-fake", None);
        assert!(!without.iter().any(|a| a.starts_with("--hostlist-auto=")));
    }

    /// Manual validation hook: dumps the built argument lists (one
    /// argument per line) to /tmp/imported-dryrun/ so they can be fed
    /// to the real nfqws2 v1.0.7 with --dry-run (strip --wf-* first,
    /// as the Linux nftables rewrite does). Not a CI test.
    #[test]
    #[ignore]
    fn dump_imported_args_for_dryrun() {
        let dir = std::path::Path::new("/tmp/imported-dryrun");
        std::fs::create_dir_all(dir).unwrap();
        for strat in crate::engine::imported_strategies::all() {
            let args = imported_test_args(strat.id, None);
            std::fs::write(dir.join(format!("{}.args", strat.id)), args.join("\n"))
                .unwrap();
        }
    }
}


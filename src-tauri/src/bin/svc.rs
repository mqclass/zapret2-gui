//! zapret2-svc: headless supervisor service.
//!
//! Runs the bypass engine with no UI and no WebView2 dependency. This process
//! owns the Windows Job Object (with `KILL_ON_JOB_CLOSE`): if the service
//! dies, `winws2.exe` dies with it — the orphan safety property. GUI crashes
//! do NOT kill it; GUI clients attach over the named pipe instead.
//!
//! Single-instance is enforced via a global named mutex; a second copy exits.

#[path = "../engine/mod.rs"]
mod engine;
#[path = "../platform/mod.rs"]
mod platform;
#[path = "../svc_client.rs"]
mod svc_client;
#[path = "../svc_ipc.rs"]
mod svc_ipc;
#[path = "../svc_server.rs"]
mod svc_server;

use std::sync::Arc;

use clap::Parser;
use tokio::sync::broadcast;
#[cfg(windows)]
use windows::core::w;
#[cfg(windows)]
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
#[cfg(windows)]
use windows::Win32::System::Threading::CreateMutexW;

use crate::engine::{EventSink, Supervisor, TgProxyManager};
use crate::svc_server::{run_pipe_server, PipeSink};

#[derive(Parser, Debug)]
#[command(name = "zapret2-svc", about = "Headless zapret2 supervisor service")]
struct Args {
    /// Print version and exit.
    #[arg(long)]
    version_info: bool,
}

/// Returns false when another svc instance already holds the mutex.
#[cfg(windows)]
fn claim_single_instance() -> bool {
    unsafe {
        // NOTE: w! requires a string literal, so the mutex name is inlined
        // here instead of using SVC_SINGLE_INSTANCE_MUTEX.
        match CreateMutexW(None, false, w!("Global\\zapret2-svc-single-instance")) {
            Ok(_) => GetLastError() != ERROR_ALREADY_EXISTS,
            Err(_) => false,
        }
    }
}

/// Holds the single-instance lock fd for the process lifetime (Linux).
#[cfg(target_os = "linux")]
static INSTANCE_LOCK_FD: std::sync::OnceLock<i32> = std::sync::OnceLock::new();

/// Returns false when another svc instance already holds the lock.
///
/// Linux form of the named-mutex claim: an exclusive non-blocking
/// `flock` on a lock file. The fd is kept open for the whole process
/// lifetime; the kernel releases the lock when the process dies in any
/// way, so a crashed svc never leaves a stale "already running" claim
/// (unlike a PID file). The svc runs as root under systemd, so the lock
/// lives in /run; /tmp is the fallback for unprivileged dev runs.
#[cfg(target_os = "linux")]
fn claim_single_instance() -> bool {
    let lock_path = if std::path::Path::new("/run").is_dir() {
        "/run/zapret2-svc.lock"
    } else {
        "/tmp/zapret2-svc.lock"
    };
    let c_path = match std::ffi::CString::new(lock_path) {
        Ok(p) => p,
        Err(_) => return false,
    };
    unsafe {
        let fd = libc::open(c_path.as_ptr(), libc::O_CREAT | libc::O_RDWR, 0o644);
        if fd < 0 {
            return false;
        }
        if libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) != 0 {
            libc::close(fd);
            return false;
        }
        let _ = INSTANCE_LOCK_FD.set(fd);
    }
    true
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    if args.version_info {
        println!("zapret2-svc {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    // The engine needs admin rights for WinDivert/WFP. If we are not elevated,
    // this relaunches zapret2-svc.exe via UAC and terminates this copy.
    // Done BEFORE the single-instance claim: a non-elevated process cannot
    // create a mutex in the Global\ namespace, which would misleadingly
    // report "already running".
    if let Err(e) = platform::privileges::ensure_elevated() {
        eprintln!("zapret2-svc requires administrator rights: {e}");
        std::process::exit(12);
    }

    if !claim_single_instance() {
        eprintln!("zapret2-svc is already running.");
        std::process::exit(11);
    }

    let supervisor = Arc::new(
        Supervisor::new().expect("failed to initialize supervisor (Job Object)"),
    );

    // Broadcast engine events to pipe clients.
    let (event_tx, _rx) = broadcast::channel(256);
    let sink: Arc<dyn EventSink> = Arc::new(PipeSink::new(event_tx.clone()));
    supervisor.set_event_sink(sink.clone());

    // The Telegram MTProto→WebSocket bridge lives here, not in the GUI
    // process: a GUI crash must not take the bridge down, just like it
    // must not take the bypass engine down.
    let tgproxy = Arc::new(TgProxyManager::new(supervisor.clone()));
    tgproxy.set_event_sink(sink);

    // Restore the bridge when it was left enabled (best-effort).
    let tgproxy_auto = tgproxy.clone();
    tokio::spawn(async move {
        let enabled = platform::settings::load_settings().tgproxy_enabled;
        if enabled {
            if let Err(e) = tgproxy_auto.start().await {
                eprintln!("svc: failed to auto-start Telegram bridge: {e}");
            }
        }
    });

    #[cfg(windows)]
    println!("zapret2-svc started, listening on the named pipe.");
    #[cfg(target_os = "linux")]
    println!("zapret2-svc started (IPC transport arrives in phase 1 step 2).");

    // Serve GUI clients until Ctrl+C.
    let server_task = tokio::spawn(run_pipe_server(
        supervisor.clone(),
        tgproxy.clone(),
        event_tx,
    ));

    tokio::signal::ctrl_c().await.ok();
    println!("zapret2-svc shutting down...");
    server_task.abort();
    let _ = tgproxy.stop().await;
    let _ = supervisor.stop().await;
}

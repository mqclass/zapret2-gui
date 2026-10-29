//! zapret2-svc: headless supervisor service.
//!
//! Runs the bypass engine with no UI and no WebView2 dependency. This process
//! owns the Windows Job Object (with `KILL_ON_JOB_CLOSE`): if the service
//! dies, `winws2.exe` dies with it — the orphan safety property. GUI crashes
//! do NOT kill it.
//!
//! Single-instance is enforced via a global named mutex on Windows and
//! an flock-ed lock file on Linux; a second copy exits. GUI clients
//! attach over the local IPC transport (named pipe on Windows, Unix
//! socket on Linux) instead of owning the engine themselves.

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
/// (unlike a PID file).
///
/// The lock's directory is chosen by *writability*, probed by actually
/// opening the file — not by the directory merely existing: /run exists
/// on every Linux box but is root-owned, so an unprivileged dev run
/// cannot create the lock there and must fall through to
/// $XDG_RUNTIME_DIR or /tmp (an existence check picked /run and then
/// failed the whole claim). Order: /run (the svc runs as root under
/// systemd, so this is the production path) → $XDG_RUNTIME_DIR → /tmp.
/// A directory that opens but whose lock is already held means a live
/// instance in a place we can see: that is a real "already running",
/// not a fall-through.
#[cfg(target_os = "linux")]
fn claim_single_instance() -> bool {
    use std::os::unix::ffi::OsStrExt;

    let mut candidates: Vec<std::path::PathBuf> = vec![std::path::PathBuf::from("/run")];
    if let Some(xdg) = std::env::var_os("XDG_RUNTIME_DIR") {
        if !xdg.is_empty() {
            candidates.push(std::path::PathBuf::from(xdg));
        }
    }
    candidates.push(std::path::PathBuf::from("/tmp"));

    for dir in candidates {
        let lock_path = dir.join("zapret2-svc.lock");
        let c_path = match std::ffi::CString::new(lock_path.as_os_str().as_bytes()) {
            Ok(p) => p,
            Err(_) => continue,
        };
        unsafe {
            let fd = libc::open(c_path.as_ptr(), libc::O_CREAT | libc::O_RDWR, 0o644);
            if fd < 0 {
                // Not writable for us (permissions, read-only fs, ...):
                // probe the next candidate instead of failing the claim.
                continue;
            }
            if libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) != 0 {
                libc::close(fd);
                return false;
            }
            let _ = INSTANCE_LOCK_FD.set(fd);
        }
        return true;
    }
    false
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
    // Linux: do NOT claim "listening" here — the server task below binds
    // the socket asynchronously and may still fail (e.g. another live
    // listener). svc_server prints the authoritative "listening on unix
    // socket" line only after the bind succeeds.
    #[cfg(target_os = "linux")]
    println!(
        "zapret2-svc started, unix socket endpoint {}.",
        crate::svc_ipc::svc_socket_path().display()
    );

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

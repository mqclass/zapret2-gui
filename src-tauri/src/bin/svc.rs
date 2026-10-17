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
#[path = "../win32/mod.rs"]
mod win32;
#[path = "../svc_ipc.rs"]
mod svc_ipc;
#[path = "../svc_server.rs"]
mod svc_server;

use std::sync::Arc;

use clap::Parser;
use tokio::sync::broadcast;
use windows::core::w;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::CreateMutexW;

use crate::engine::{EventSink, Supervisor};
use crate::svc_server::{run_pipe_server, PipeSink};

#[derive(Parser, Debug)]
#[command(name = "zapret2-svc", about = "Headless zapret2 supervisor service")]
struct Args {
    /// Print version and exit.
    #[arg(long)]
    version_info: bool,
}

/// Returns false when another svc instance already holds the mutex.
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

#[tokio::main]
async fn main() {
    let args = Args::parse();
    if args.version_info {
        println!("zapret2-svc {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    if !claim_single_instance() {
        eprintln!("zapret2-svc is already running.");
        std::process::exit(11);
    }

    // The engine needs admin rights for WinDivert/WFP.
    if !win32::privileges::is_elevated().unwrap_or(false) {
        eprintln!("zapret2-svc requires administrator rights; requesting elevation...");
        if let Err(e) = win32::privileges::ensure_elevated() {
            eprintln!("elevation failed: {e}");
            std::process::exit(12);
        }
        return; // the elevated copy continues from here
    }

    let supervisor = Arc::new(
        Supervisor::new().expect("failed to initialize supervisor (Job Object)"),
    );

    // Broadcast engine events to pipe clients.
    let (event_tx, _rx) = broadcast::channel(256);
    let sink: Arc<dyn EventSink> = Arc::new(PipeSink::new(event_tx.clone()));
    supervisor.set_event_sink(sink);

    println!("zapret2-svc started, listening on the named pipe.");

    // Serve GUI clients until Ctrl+C.
    let server_task = tokio::spawn(run_pipe_server(supervisor.clone(), event_tx));

    tokio::signal::ctrl_c().await.ok();
    println!("zapret2-svc shutting down...");
    server_task.abort();
    let _ = supervisor.stop().await;
}

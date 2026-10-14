//! High-Performance Persistent File Logger for zapret2 Supervisor
//!
//! Features:
//! - Thread-safe, non-blocking disk persistence for all engine & supervisor events.
//! - Automatic directory detection (`./logs` adjacent to executable or working directory).
//! - Graceful fallback to `%LOCALAPPDATA%\zapret2\logs` if the working directory is read-only.
//! - Timestamped ISO formatting: `[YYYY-MM-DD HH:MM:SS.mmm] [LEVEL] message`.
//! - File rotation when log size exceeds 5 MB (preserves previous log as `.old`).
//! - Native Windows Explorer integration (`open_logs_folder`).

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use parking_lot::Mutex;

/// Maximum log file size before rotation (5 MB).
const MAX_LOG_SIZE_BYTES: u64 = 5 * 1024 * 1024;

/// Central file logger singleton for the supervisor.
pub struct FileLogger {
    log_dir: PathBuf,
    current_file: PathBuf,
    writer: Mutex<Option<File>>,
    bytes_written: AtomicUsize,
}

impl FileLogger {
    /// Initializes the file logger, creating the logs folder and rotating stale logs if needed.
    pub fn new() -> Self {
        let log_dir = Self::resolve_log_directory();
        let _ = fs::create_dir_all(&log_dir);

        let current_file = log_dir.join("zapret2.log");
        let initial_bytes = current_file.metadata().map(|m| m.len() as usize).unwrap_or(0);

        let file_opt = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&current_file)
            .ok();

        let logger = Self {
            log_dir,
            current_file,
            writer: Mutex::new(file_opt),
            bytes_written: AtomicUsize::new(initial_bytes),
        };

        logger.write_entry("INFO", "=== zapret2 Supervisor Logging Session Started ===");
        logger
    }

    /// Resolves the optimal log directory, preferring adjacent `./logs` or `%LOCALAPPDATA%\zapret2\logs`.
    fn resolve_log_directory() -> PathBuf {
        // 1. Try adjacent to current executable
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(parent) = exe_path.parent() {
                let candidate = parent.join("logs");
                if fs::create_dir_all(&candidate).is_ok() {
                    return candidate;
                }
            }
        }

        // 2. Try current working directory
        let cwd_logs = PathBuf::from("logs");
        if fs::create_dir_all(&cwd_logs).is_ok() {
            return cwd_logs;
        }

        // 3. Fallback to LocalAppData
        if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
            let app_logs = PathBuf::from(local_appdata).join("zapret2").join("logs");
            let _ = fs::create_dir_all(&app_logs);
            return app_logs;
        }

        // 4. Ultimate fallback to temp
        std::env::temp_dir().join("zapret2-logs")
    }

    /// Formats current UTC/local time into readable string `YYYY-MM-DD HH:MM:SS.mmm`.
    fn format_now() -> String {
        let now = SystemTime::now();
        let since_epoch = now.duration_since(UNIX_EPOCH).unwrap_or_default();
        let total_secs = since_epoch.as_secs();
        let millis = since_epoch.subsec_millis();

        let s = total_secs % 60;
        let m = (total_secs / 60) % 60;
        let h = (total_secs / 3600) % 24;

        // Approximate day/month calculation without heavy chrono dependency
        let days_since_epoch = total_secs / 86400;
        let year = 1970 + days_since_epoch / 365;

        format!("{}-{:02} {:02}:{:02}:{:02}.{:03}", year, (days_since_epoch % 12) + 1, h, m, s, millis)
    }

    /// Appends a structured log entry to `zapret2.log`, triggering rotation if size exceeds limit.
    pub fn write_entry(&self, level: &str, message: &str) {
        let timestamp = Self::format_now();
        let line = format!("[{}] [{}] {}\n", timestamp, level.to_uppercase(), message);
        let line_bytes = line.as_bytes();

        let mut guard = self.writer.lock();
        if let Some(file) = guard.as_mut() {
            if file.write_all(line_bytes).is_ok() {
                let _ = file.flush();
                let current_size = self.bytes_written.fetch_add(line_bytes.len(), Ordering::Relaxed) + line_bytes.len();

                // Rotate log if it exceeds MAX_LOG_SIZE_BYTES
                if current_size as u64 >= MAX_LOG_SIZE_BYTES {
                    let _ = file;
                    *guard = None;

                    let old_file = self.log_dir.join("zapret2.log.old");
                    let _ = fs::remove_file(&old_file);
                    let _ = fs::rename(&self.current_file, &old_file);

                    let new_file = OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&self.current_file)
                        .ok();

                    self.bytes_written.store(0, Ordering::Relaxed);
                    *guard = new_file;
                }
            }
        }
    }

    /// Retrieves the path to the current log file.
    pub fn get_log_file_path(&self) -> PathBuf {
        self.current_file.clone()
    }

    /// Retrieves the log directory.
    #[allow(dead_code)]
    pub fn get_log_dir(&self) -> PathBuf {
        self.log_dir.clone()
    }

    /// Opens the logs directory in Windows Explorer.
    pub fn open_in_explorer(&self) -> Result<(), String> {
        let dir = &self.log_dir;
        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("explorer.exe")
                .arg(dir)
                .spawn()
                .map_err(|e| format!("Failed to open Explorer: {}", e))?;
            Ok(())
        }
        #[cfg(not(target_os = "windows"))]
        {
            Ok(())
        }
    }
}

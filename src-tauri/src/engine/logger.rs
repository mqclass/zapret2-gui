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
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use parking_lot::Mutex;

/// Creation flag to spawn helper processes without a console window.
const CREATE_NO_WINDOW: u32 = 0x08000000;

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

    /// Resolves the optimal log directory.
    ///
    /// SECURITY (L-5): `%LOCALAPPDATA%` is preferred — it is per-user with a
    /// restrictive default ACL. Whichever directory is chosen, its ACL is
    /// locked down to the current user + SYSTEM + Administrators, because logs
    /// contain adapter names, DNS configuration and full engine arguments.
    fn resolve_log_directory() -> PathBuf {
        // 1. Per-user LocalAppData (restrictive ACL by default)
        if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
            let app_logs = PathBuf::from(local_appdata).join("zapret2").join("logs");
            if fs::create_dir_all(&app_logs).is_ok() {
                Self::harden_log_dir_acl(&app_logs);
                return app_logs;
            }
        }

        // 2. Adjacent to current executable (portable installs)
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(parent) = exe_path.parent() {
                let candidate = parent.join("logs");
                if fs::create_dir_all(&candidate).is_ok() {
                    Self::harden_log_dir_acl(&candidate);
                    return candidate;
                }
            }
        }

        // 3. Last resort: temp dir (potentially shared) — locked down explicitly.
        let tmp = std::env::temp_dir().join("zapret2-logs");
        if fs::create_dir_all(&tmp).is_ok() {
            Self::harden_log_dir_acl(&tmp);
        }
        tmp
    }

    /// Restricts a log directory ACL to the current user, SYSTEM and
    /// Administrators (inheritance removed). Best-effort: failures are ignored
    /// and the directory remains usable.
    fn harden_log_dir_acl(dir: &Path) {
        let username = std::env::var("USERNAME").unwrap_or_else(|_| "Administrators".to_string());
        let dir_str = dir.to_string_lossy().to_string();
        let _ = std::process::Command::new(crate::win32::system32("icacls.exe"))
            .args([
                dir_str.as_str(),
                "/inheritance:r",
                "/grant:r",
                "*S-1-5-18:(OI)(CI)F", // SYSTEM
                "*S-1-5-32-544:(OI)(CI)F", // Administrators
                &format!("{}:(OI)(CI)F", username),
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
    }

    /// Formats current time into readable string `YYYY-MM-DD HH:MM:SS.mmm` (UTC).
    ///
    /// Uses Howard Hinnant's civil-from-days algorithm — no heavy date dependencies.
    fn format_now() -> String {
        let now = SystemTime::now();
        let since_epoch = now.duration_since(UNIX_EPOCH).unwrap_or_default();
        let total_secs = since_epoch.as_secs();
        let millis = since_epoch.subsec_millis();

        let s = total_secs % 60;
        let m = (total_secs / 60) % 60;
        let h = (total_secs / 3600) % 24;

        let (year, month, day) = Self::civil_from_days(total_secs / 86400);

        format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}", year, month, day, h, m, s, millis)
    }

    /// Converts days since the Unix epoch to a (year, month, day) calendar date.
    fn civil_from_days(z: u64) -> (i64, u32, u32) {
        let z = z as i64 + 719468;
        let era = z.div_euclid(146097);
        let doe = z.rem_euclid(146097); // day of era [0, 146096]
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // year of era [0, 399]
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of year [0, 365]
        let mp = (5 * doy + 2) / 153; // month of [Mar=0 .. Feb=11]
        let d = doy - (153 * mp + 2) / 5 + 1; // day of month [1, 31]
        let m = if mp < 10 { mp + 3 } else { mp - 9 }; // calendar month [1, 12]
        (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
    }

    /// Appends a structured log entry to `zapret2.log`, triggering rotation if size exceeds limit.
    ///
    /// Writes are buffered by the OS: there is intentionally NO per-line `flush()` here
    /// (a syscall per log line while holding a blocking mutex). Call [`Self::flush`]
    /// to force buffered data to disk — it runs automatically on rotation and on a
    /// periodic timer owned by the supervisor.
    pub fn write_entry(&self, level: &str, message: &str) {
        let timestamp = Self::format_now();
        let line = format!("[{}] [{}] {}\n", timestamp, level.to_uppercase(), message);
        let line_bytes = line.as_bytes();

        let mut guard = self.writer.lock();
        if let Some(file) = guard.as_mut() {
            if file.write_all(line_bytes).is_ok() {
                let current_size = self.bytes_written.fetch_add(line_bytes.len(), Ordering::Relaxed) + line_bytes.len();

                // Rotate log if it exceeds MAX_LOG_SIZE_BYTES
                if current_size as u64 >= MAX_LOG_SIZE_BYTES {
                    // Flush before rotating so no buffered data is lost in the rename.
                    let _ = file.flush();
                    Self::rotate_locked(&mut guard, &self.log_dir, &self.current_file);
                    self.bytes_written.store(0, Ordering::Relaxed);
                }
            }
        }
    }

    /// Performs the log rotation; caller must hold the writer lock.
    fn rotate_locked(
        guard: &mut parking_lot::MutexGuard<Option<File>>,
        log_dir: &Path,
        current_file: &Path,
    ) {
        **guard = None;

        let old_file = log_dir.join("zapret2.log.old");
        let _ = fs::remove_file(&old_file);
        let _ = fs::rename(current_file, &old_file);

        let new_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(current_file)
            .ok();

        **guard = new_file;
    }

    /// Forces any OS-buffered log data to disk.
    pub fn flush(&self) {
        if let Some(file) = self.writer.lock().as_mut() {
            let _ = file.flush();
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
            std::process::Command::new(crate::win32::system32("explorer.exe"))
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

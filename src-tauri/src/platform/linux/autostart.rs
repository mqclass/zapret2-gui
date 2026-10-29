//! Autostart on Linux.
//!
//! Not wired up in phase 1: on Windows autostart is a Task Scheduler job;
//! the Linux form is a systemd user unit (GUI) next to the system unit for
//! zapret2-svc, and the units arrive together with the svc transport step
//! (phase 1, step 2). Until then this module answers honestly instead of
//! faking success: nothing is ever registered, "enabled" is always false,
//! and enabling returns an error the UI can show.

/// Checks whether autostart is currently enabled. It never is in phase 1:
/// no user unit is installed by this build.
pub fn is_autostart_enabled() -> bool {
    false
}

/// Enables or disables autostart.
///
/// Disabling is trivially satisfied (nothing can be registered), enabling
/// is refused with the reason — a silent Ok(true) here would tell the user
/// the app will start on login when it will not.
pub fn set_autostart(enable: bool) -> Result<bool, String> {
    if enable {
        Err(
            "Автозапуск на Linux пока не поддерживается: он появится вместе \
             с systemd-юнитами zapret2 (фаза 1, шаг 2)."
                .to_string(),
        )
    } else {
        Ok(false)
    }
}

//! Custom Profile Storage and Command Argument Parser (Linux)
//!
//! Same storage format and parser contract as the Windows build. Two
//! platform adaptations:
//! - The storage file lives in the shared config directory (see
//!   `platform::settings::config_dir`): exe-adjacent `configs/` for
//!   portable installs, XDG config home otherwise — the same directory
//!   `app_settings.json` resolves to, instead of unconditionally creating
//!   `configs/` next to the executable (a system-wide install location is
//!   not writable by the unprivileged GUI).
//! - The parser expands `%BIN%` / `%LISTS%` with `/` separators and also
//!   strips a leading `nfqws2` executable name, since pasted command lines
//!   on Linux come from zapret shell scripts, not .bat files.

use std::fs;
use std::path::{Path, PathBuf};
use crate::platform::process::LuaStrategy;
use crate::platform::settings::atomic_write;

/// Returns the path to the custom profiles JSON storage.
pub fn get_custom_profiles_path() -> PathBuf {
    let configs_dir = crate::platform::settings::config_dir();
    if !configs_dir.exists() {
        let _ = fs::create_dir_all(&configs_dir);
    }
    configs_dir.join("custom_profiles.json")
}

/// Loads all saved custom profiles.
pub fn load_custom_profiles() -> Vec<LuaStrategy> {
    let path = get_custom_profiles_path();
    if !path.exists() {
        return Vec::new();
    }

    match fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Saves or updates a custom profile and writes to disk.
pub fn save_custom_profile(mut strategy: LuaStrategy) -> Result<(), String> {
    // SECURITY (M-5): IPC is a trust boundary — never accept path/filename from
    // the caller for custom profiles. The UI sanitizes them, but a malicious
    // local IPC caller could otherwise smuggle an arbitrary Lua path into the
    // privileged engine's command line (see resolve_strategy_args §4).
    // Generate both server-side, deterministically from the profile id.
    let safe_id: String = strategy
        .id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(64)
        .collect();
    if safe_id.is_empty() {
        return Err("Некорректный идентификатор профиля".to_string());
    }
    strategy.id = safe_id.clone();
    strategy.filename = format!("{}.conf", safe_id);
    strategy.path = PathBuf::from(format!("configs/{}.json", safe_id));
    // The engine must be a known value; anything else falls back to winws2
    // instead of trusting an IPC-supplied string.
    if strategy.engine != "winws" && strategy.engine != "winws2" {
        strategy.engine = "winws2".to_string();
    }
    strategy.category = "custom".to_string();

    let mut profiles = load_custom_profiles();

    if let Some(pos) = profiles.iter().position(|p| p.id == strategy.id) {
        profiles[pos] = strategy;
    } else {
        profiles.push(strategy);
    }

    let path = get_custom_profiles_path();
    let json = serde_json::to_string_pretty(&profiles).map_err(|e| e.to_string())?;
    atomic_write(&path, json.as_bytes()).map_err(|e| e.to_string())?;
    Ok(())
}

/// Deletes a custom profile by its ID.
pub fn delete_custom_profile(id: &str) -> Result<(), String> {
    let mut profiles = load_custom_profiles();
    profiles.retain(|p| p.id != id);

    let path = get_custom_profiles_path();
    let json = serde_json::to_string_pretty(&profiles).map_err(|e| e.to_string())?;
    atomic_write(&path, json.as_bytes()).map_err(|e| e.to_string())?;
    Ok(())
}

/// Parses a raw command string (copied from a zapret .bat or shell script)
/// into tokenized arguments. Expands `%BIN%` and `%LISTS%`, removes leading
/// executable names and batch escape characters.
pub fn parse_command_line_args(raw: &str, bin_dir: &Path, lists_dir: &Path) -> Vec<String> {
    let bin_str = bin_dir.to_string_lossy().to_string();
    let lists_str = lists_dir.to_string_lossy().to_string();

    let expanded = raw
        .replace("%BIN%", &format!("{}/", bin_str.trim_end_matches('/')))
        .replace("%LISTS%", &format!("{}/", lists_str.trim_end_matches('/')))
        .replace('^', "");

    // Remove leading `start ...` or executable names if present
    let mut clean = expanded.trim();
    if clean.starts_with("start") {
        if let Some(idx) = clean.find("winws").or_else(|| clean.find("nfqws")) {
            clean = &clean[idx..];
        }
    }
    if clean.starts_with("winws.exe")
        || clean.starts_with("winws2.exe")
        || clean.starts_with("\"winws.exe\"")
        || clean.starts_with("\"winws2.exe\"")
        || clean.starts_with("nfqws2")
        || clean.starts_with("nfqws")
        || clean.starts_with("\"nfqws2\"")
    {
        if let Some(idx) = clean.find(' ') {
            clean = clean[idx..].trim();
        }
    }

    // Split tokens respecting quotes
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for c in clean.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
            }
            ' ' | '\t' | '\r' | '\n' if !in_quotes => {
                if !current.is_empty() {
                    tokens.push(current.trim_matches('"').to_string());
                    current.clear();
                }
            }
            _ => {
                current.push(c);
            }
        }
    }

    if !current.is_empty() {
        tokens.push(current.trim_matches('"').to_string());
    }

    tokens
}

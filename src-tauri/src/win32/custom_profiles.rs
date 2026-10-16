//! Custom Profile Storage and Command Argument Parser
//!
//! Allows users to define, save, edit, and delete their own custom DPI bypass profiles.
//! Supports pasting raw parameters or entire lines from .bat scripts with automatic tokenization
//! and %BIN% / %LISTS% variable expansion.

use std::fs;
use std::path::{Path, PathBuf};
use crate::win32::process::LuaStrategy;
use crate::win32::settings::atomic_write;

/// Returns the path to the custom profiles JSON storage.
pub fn get_custom_profiles_path() -> PathBuf {
    let base_dir = if let Ok(exe) = std::env::current_exe() {
        exe.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."))
    } else {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    };
    let configs_dir = base_dir.join("configs");
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
    let mut profiles = load_custom_profiles();
    strategy.category = "custom".to_string();

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

/// Parses a raw command string (which may be copied from a .bat file) into tokenized arguments.
/// Expands `%BIN%` and `%LISTS%`, removes leading executable names and batch escape characters.
pub fn parse_command_line_args(raw: &str, bin_dir: &Path, lists_dir: &Path) -> Vec<String> {
    let bin_str = bin_dir.to_string_lossy().to_string();
    let lists_str = lists_dir.to_string_lossy().to_string();

    let expanded = raw
        .replace("%BIN%", &format!("{}\\", bin_str.trim_end_matches('\\')))
        .replace("%LISTS%", &format!("{}\\", lists_str.trim_end_matches('\\')))
        .replace('^', "");

    // Remove leading `start ...` or executable names if present
    let mut clean = expanded.trim();
    if clean.starts_with("start") {
        if let Some(idx) = clean.find("winws") {
            clean = &clean[idx..];
        }
    }
    if clean.starts_with("winws.exe")
        || clean.starts_with("winws2.exe")
        || clean.starts_with("\"winws.exe\"")
        || clean.starts_with("\"winws2.exe\"")
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

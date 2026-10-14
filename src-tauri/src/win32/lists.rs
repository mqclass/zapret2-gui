//! Domain and Hostlist Management for zapret2
//!
//! Provides inspection, manipulation, domain addition/removal,
//! and Windows DNS cache flush for zapret hostlists.

use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::win32::process::resolve_binaries;

const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Resolves the lists directory.
pub fn get_lists_dir() -> Result<PathBuf, String> {
    if let Ok(binaries) = resolve_binaries() {
        if let Some(lists) = binaries.lists_dir {
            if lists.is_dir() {
                return Ok(lists);
            }
        }
        let fallback = binaries.bin_dir.join("../lists");
        if fallback.is_dir() {
            return Ok(fallback);
        }
    }

    // Try current executable's directory
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let p1 = parent.join("lists");
            if p1.is_dir() {
                return Ok(p1);
            }
            if let Some(p) = parent.parent() {
                let p2 = p.join("lists");
                if p2.is_dir() {
                    return Ok(p2);
                }
            }
        }
    }

    let default_lists = PathBuf::from("lists");
    if default_lists.is_dir() {
        return Ok(default_lists);
    }

    Err("Не удалось найти папку lists".to_string())
}

/// Lists all .txt domain/hostlist files in the lists directory.
pub fn list_domain_files() -> Result<Vec<String>, String> {
    let dir = get_lists_dir()?;
    let mut files = Vec::new();

    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    if ext == "txt" {
                        if let Some(name) = path.file_name() {
                            files.push(name.to_string_lossy().to_string());
                        }
                    }
                }
            }
        }
    }

    files.sort();
    Ok(files)
}

/// Reads all domain entries from a specific file.
pub fn read_domains(filename: &str) -> Result<Vec<String>, String> {
    let dir = get_lists_dir()?;
    let safe_name = Path::new(filename)
        .file_name()
        .ok_or_else(|| "Неверное имя файла".to_string())?;
    let file_path = dir.join(safe_name);

    if !file_path.exists() {
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(&file_path).map_err(|e| e.to_string())?;
    let mut domains = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        domains.push(trimmed.to_string());
    }

    Ok(domains)
}

/// Sanitizes a domain input string (strips protocol, port, path, whitespace).
fn sanitize_domain(raw: &str) -> Option<String> {
    let mut s = raw.trim().to_lowercase();
    if s.is_empty() {
        return None;
    }
    if let Some(stripped) = s.strip_prefix("https://") {
        s = stripped.to_string();
    } else if let Some(stripped) = s.strip_prefix("http://") {
        s = stripped.to_string();
    }

    // Strip path if provided
    if let Some(idx) = s.find('/') {
        s = s[..idx].to_string();
    }
    // Strip port if provided
    if let Some(idx) = s.find(':') {
        s = s[..idx].to_string();
    }

    let trimmed = s.trim().to_string();
    if trimmed.is_empty() || !trimmed.contains('.') {
        return None;
    }
    Some(trimmed)
}

/// Appends a new domain to a specific list.
pub fn add_domain_to_list(filename: &str, raw_domain: &str) -> Result<Vec<String>, String> {
    let domain = sanitize_domain(raw_domain)
        .ok_or_else(|| "Некорректный домен (например, google.com или rutracker.org)".to_string())?;

    let dir = get_lists_dir()?;
    let safe_name = Path::new(filename)
        .file_name()
        .ok_or_else(|| "Неверное имя файла".to_string())?;
    let file_path = dir.join(safe_name);

    let mut domains = read_domains(filename)?;
    if domains.iter().any(|d| d.eq_ignore_ascii_case(&domain)) {
        return Err(format!("Домен '{}' уже есть в списке", domain));
    }

    domains.push(domain);
    domains.sort();

    // Write back sorted unique list
    let content = domains.join("\r\n") + "\r\n";
    fs::write(&file_path, content).map_err(|e| format!("Ошибка записи файла: {}", e))?;

    Ok(domains)
}

/// Removes a domain from a specific list.
pub fn remove_domain_from_list(filename: &str, raw_domain: &str) -> Result<Vec<String>, String> {
    let domain = raw_domain.trim().to_lowercase();
    let dir = get_lists_dir()?;
    let safe_name = Path::new(filename)
        .file_name()
        .ok_or_else(|| "Неверное имя файла".to_string())?;
    let file_path = dir.join(safe_name);

    let mut domains = read_domains(filename)?;
    domains.retain(|d| !d.eq_ignore_ascii_case(&domain));

    let content = domains.join("\r\n") + "\r\n";
    fs::write(&file_path, content).map_err(|e| format!("Ошибка записи файла: {}", e))?;

    Ok(domains)
}

/// Flushes Windows DNS client cache via ipconfig /flushdns.
pub fn flush_dns_cache() -> Result<String, String> {
    let output = Command::new("ipconfig.exe")
        .arg("/flushdns")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("Не удалось выполнить ipconfig: {}", e))?;

    if output.status.success() {
        Ok("Кэш сопоставителя DNS успешно очищен.".to_string())
    } else {
        let err = String::from_utf8_lossy(&output.stderr);
        Err(format!("Ошибка сброса DNS: {}", err.trim()))
    }
}

/// Opens the lists directory in Windows Explorer.
pub fn open_lists_folder() -> Result<(), String> {
    let dir = get_lists_dir()?;
    Command::new("explorer.exe")
        .arg(&dir)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("Не удалось открыть проводник: {}", e))?;
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ListSyncItem {
    pub filename: String,
    pub lines_count: usize,
    pub status: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ListSyncReport {
    pub success: bool,
    pub files_updated: usize,
    pub total_domains: usize,
    pub details: Vec<String>,
    pub files: Vec<ListSyncItem>,
    pub message: String,
}

/// Downloads fresh domain lists and ipsets from official Flowseal repository.
pub async fn update_remote_lists() -> Result<ListSyncReport, String> {
    let dir = get_lists_dir()?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP Client error: {}", e))?;

    let targets = [
        ("list-general.txt", "https://raw.githubusercontent.com/Flowseal/zapret-discord-youtube/main/lists/list-general.txt"),
        ("list-google.txt", "https://raw.githubusercontent.com/Flowseal/zapret-discord-youtube/main/lists/list-google.txt"),
        ("list-exclude.txt", "https://raw.githubusercontent.com/Flowseal/zapret-discord-youtube/main/lists/list-exclude.txt"),
        ("ipset-all.txt", "https://raw.githubusercontent.com/Flowseal/zapret-discord-youtube/main/lists/ipset-all.txt"),
    ];

    let mut files_updated = 0;
    let mut total_domains = 0;
    let mut details = Vec::new();
    let mut files = Vec::new();

    for (filename, url) in targets {
        match client.get(url).send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(text) = resp.text().await {
                    if text.trim().len() > 20 {
                        let target_path = dir.join(filename);
                        let bak_path = dir.join(format!("{}.bak", filename));

                        // Backup existing file
                        if target_path.exists() {
                            let _ = fs::copy(&target_path, &bak_path);
                        }

                        let lines_count = text.lines().count();
                        if fs::write(&target_path, &text).is_ok() {
                            files_updated += 1;
                            total_domains += lines_count;
                            details.push(format!("{}: обновлено {} записей", filename, lines_count));
                            files.push(ListSyncItem {
                                filename: filename.to_string(),
                                lines_count,
                                status: "updated".to_string(),
                            });
                        }
                    }
                }
            }
            Ok(resp) => {
                details.push(format!("{}: сервер вернул HTTP {}", filename, resp.status()));
            }
            Err(e) => {
                details.push(format!("{}: ошибка скачивания ({})", filename, e));
            }
        }
    }

    // Flush Windows DNS resolver after lists update
    let _ = flush_dns_cache();

    let message = format!("Обновлено файлов: {}, суммарно доменов: {}", files_updated, total_domains);

    Ok(ListSyncReport {
        success: files_updated > 0,
        files_updated,
        total_domains,
        details,
        files,
        message,
    })
}


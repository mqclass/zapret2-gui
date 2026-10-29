//! Domain and Hostlist Management for zapret2
//!
//! Provides inspection, manipulation, domain addition/removal,
//! and DNS cache flush for zapret hostlists (Linux).
//!
//! Line endings: unlike the Windows build, list files here are written
//! with LF — nfqws2 parses lines natively, and a stray CR would leak into
//! domain entries.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::platform::process::resolve_binaries;
use crate::platform::settings::atomic_write;

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

/// Sanitizes a domain input string (strips protocol, port, path, whitespace; preserves CIDRs and subdomain prefixes).
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

    // Check if input represents an IP CIDR block (e.g. 192.168.1.0/24 or 2001:db8::/32)
    let is_cidr = if let Some((ip_part, mask_part)) = s.split_once('/') {
        ip_part.parse::<std::net::IpAddr>().is_ok()
            && mask_part.parse::<u8>().map_or(false, |m| m <= 128)
    } else {
        false
    };

    if !is_cidr {
        // Strip URL path if provided
        if let Some(idx) = s.find('/') {
            s = s[..idx].to_string();
        }
        // Strip port if provided (ignore IPv6 colons)
        if !s.contains("::") && s.matches(':').count() == 1 {
            if let Some(idx) = s.find(':') {
                s = s[..idx].to_string();
            }
        }
    }

    let trimmed = s.trim().to_string();
    if trimmed.is_empty() || (!trimmed.contains('.') && !trimmed.contains(':')) {
        return None;
    }
    Some(trimmed)
}

/// Appends a new domain to a specific list while preserving comments, headers, and structure.
pub fn add_domain_to_list(filename: &str, raw_domain: &str) -> Result<Vec<String>, String> {
    let domain = sanitize_domain(raw_domain)
        .ok_or_else(|| "Некорректный домен (например, google.com или rutracker.org)".to_string())?;

    let dir = get_lists_dir()?;
    let safe_name = Path::new(filename)
        .file_name()
        .ok_or_else(|| "Неверное имя файла".to_string())?;
    let file_path = dir.join(safe_name);

    let existing_domains = read_domains(filename)?;
    if existing_domains.iter().any(|d| d.eq_ignore_ascii_case(&domain)) {
        return Err(format!("Домен '{}' уже есть в списке", domain));
    }

    let mut file_content = if file_path.exists() {
        fs::read_to_string(&file_path).map_err(|e| e.to_string())?
    } else {
        String::new()
    };

    if !file_content.is_empty() && !file_content.ends_with('\n') {
        file_content.push_str("\n");
    }
    file_content.push_str(&domain);
    file_content.push_str("\n");

    atomic_write(&file_path, file_content.as_bytes()).map_err(|e| format!("Ошибка записи файла: {}", e))?;

    read_domains(filename)
}

/// Removes a domain from a specific list while preserving comments and structure.
pub fn remove_domain_from_list(filename: &str, raw_domain: &str) -> Result<Vec<String>, String> {
    let domain = raw_domain.trim().to_lowercase();
    let dir = get_lists_dir()?;
    let safe_name = Path::new(filename)
        .file_name()
        .ok_or_else(|| "Неверное имя файла".to_string())?;
    let file_path = dir.join(safe_name);

    if !file_path.exists() {
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(&file_path).map_err(|e| e.to_string())?;
    let target_stripped = domain.strip_prefix('^').unwrap_or(&domain);
    let mut new_lines = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            new_lines.push(line.to_string());
        } else {
            let line_lower = trimmed.to_lowercase();
            let line_stripped = line_lower.strip_prefix('^').unwrap_or(&line_lower);
            if line_lower != domain && line_stripped != target_stripped {
                new_lines.push(line.to_string());
            }
        }
    }

    let updated_content = new_lines.join("\n") + "\n";
    atomic_write(&file_path, updated_content.as_bytes()).map_err(|e| format!("Ошибка записи файла: {}", e))?;

    read_domains(filename)
}


/// Flushes the system DNS cache via systemd-resolved (`resolvectl`).
///
/// glibc itself does not cache DNS; where systemd-resolved is absent
/// there is no system cache to flush, which is reported as such rather
/// than as an error.
pub fn flush_dns_cache() -> Result<String, String> {
    let resolvectl = crate::platform::system32("resolvectl");
    if !resolvectl.is_file() {
        return Ok("systemd-resolved не найден: системный DNS-кэш не используется, очищать нечего.".to_string());
    }
    let output = Command::new(&resolvectl)
        .arg("flush-caches")
        .output()
        .map_err(|e| format!("Не удалось выполнить resolvectl: {}", e))?;

    if output.status.success() {
        Ok("Кэш DNS (systemd-resolved) успешно очищен.".to_string())
    } else {
        let err = String::from_utf8_lossy(&output.stderr);
        Err(format!("Ошибка сброса DNS: {}", err.trim()))
    }
}

/// Opens the lists directory in the desktop file manager (xdg-open).
pub fn open_lists_folder() -> Result<(), String> {
    let dir = get_lists_dir()?;
    Command::new("xdg-open")
        .arg(&dir)
        .spawn()
        .map_err(|e| format!("Не удалось открыть папку списков: {}", e))?;
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

                        // SECURITY (H-2): record previous data-record count for
                        // anomaly detection below (comments/blank lines excluded).
                        let prev_count = fs::read_to_string(&target_path)
                            .map(|c| {
                                c.lines()
                                    .filter(|l| {
                                        let t = l.trim();
                                        !t.is_empty() && !t.starts_with('#')
                                    })
                                    .count()
                            })
                            .unwrap_or(0);

                        let merged_text = if target_path.exists() {
                            let existing_content = fs::read_to_string(&target_path).unwrap_or_default();
                            let mut seen: HashSet<String> = HashSet::new();
                            let mut merged_lines = Vec::new();

                            // 1. Add upstream lines
                            for line in text.lines() {
                                let trimmed = line.trim();
                                if trimmed.is_empty() {
                                    continue;
                                }
                                if trimmed.starts_with('#') {
                                    merged_lines.push(trimmed.to_string());
                                } else {
                                    let key = trimmed.trim_start_matches('^').to_lowercase();
                                    if seen.insert(key) {
                                        merged_lines.push(trimmed.to_string());
                                    }
                                }
                            }

                            // 2. Preserve existing user-added entries
                            for line in existing_content.lines() {
                                let trimmed = line.trim();
                                if trimmed.is_empty() || trimmed.starts_with('#') {
                                    continue;
                                }
                                let key = trimmed.trim_start_matches('^').to_lowercase();
                                if seen.insert(key) {
                                    merged_lines.push(trimmed.to_string());
                                }
                            }

                            if filename == "list-google.txt" {
                                if seen.insert("dl.google.com".to_string()) {
                                    merged_lines.push("dl.google.com".to_string());
                                }
                            } else if filename == "list-general.txt" {
                                let discord_gif_essentials = [
                                    "tenor.com", "media.tenor.com", "c.tenor.com", "t.tenor.com", "api.tenor.com",
                                    "tenor.co", "gifdeliverynetwork.com", "giphy.com", "media.giphy.com",
                                    "api.giphy.com", "i.giphy.com",
                                ];
                                for domain in discord_gif_essentials {
                                    if seen.insert(domain.to_string()) {
                                        merged_lines.push(domain.to_string());
                                    }
                                }
                            }

                            let mut res = merged_lines.join("\n");
                            res.push_str("\n");
                            res
                        } else {
                            text.clone()
                        };

                        let lines_count = merged_text.lines().count();
                        let data_count = merged_text
                            .lines()
                            .filter(|l| {
                                let t = l.trim();
                                !t.is_empty() && !t.starts_with('#')
                            })
                            .count();

                        // SECURITY (H-2): anomaly rollback. If the merged list collapsed
                        // to less than half of its previous size, treat the upstream
                        // payload as corrupted or hostile, restore the backup and refuse
                        // the update instead of silently breaking the user's bypass.
                        if prev_count > 0 && data_count * 2 < prev_count {
                            if bak_path.exists() {
                                let _ = fs::copy(&bak_path, &target_path);
                            }
                            details.push(format!(
                                "{}: АНОМАЛИЯ — новый список ({} записей) меньше половины предыдущего ({}). Откат к бэкапу, обновление отклонено. Источник: {}",
                                filename, data_count, prev_count, url
                            ));
                            files.push(ListSyncItem {
                                filename: filename.to_string(),
                                lines_count: prev_count,
                                status: "rolled_back".to_string(),
                            });
                            continue;
                        }

                        if atomic_write(&target_path, merged_text.as_bytes()).is_ok() {
                            // Also mirror to bin/lists if it exists
                            if let Ok(exe_path) = std::env::current_exe() {
                                if let Some(parent) = exe_path.parent() {
                                    let bin_list = parent.join("bin/lists").join(filename);
                                    if bin_list.parent().map_or(false, |p| p.is_dir()) {
                                        let _ = atomic_write(&bin_list, merged_text.as_bytes());
                                    }
                                }
                            }

                            files_updated += 1;
                            total_domains += lines_count;
                            // SECURITY (H-2): log the source and merged record counts
                            // so every applied update is auditable by the user.
                            details.push(format!(
                                "{}: актуализировано {} записей (было {}, источник {})",
                                filename, lines_count, prev_count, url
                            ));
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

    // Flush the DNS resolver cache after lists update
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_domain_handles_cidr_and_subdomains() {
        // Subnets for ipset-all.txt / ipset-exclude.txt must not have mask stripped as a URL path
        let cidr = "192.168.1.0/24";
        let sanitized = sanitize_domain(cidr).expect("CIDR should be valid in ipset list");
        assert_eq!(sanitized, "192.168.1.0/24", "Subnet mask must not be stripped");
    }

    // The Windows build also carries FS tests that write into the resolved
    // lists dir; they are omitted here because they require a staged
    // install layout (real engine binaries) that a port skeleton does not
    // have. They return together with the engine step's test fixtures.
}

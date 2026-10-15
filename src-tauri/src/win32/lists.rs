//! Domain and Hostlist Management for zapret2
//!
//! Provides inspection, manipulation, domain addition/removal,
//! and Windows DNS cache flush for zapret hostlists.

use std::collections::HashSet;
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
        file_content.push_str("\r\n");
    }
    file_content.push_str(&domain);
    file_content.push_str("\r\n");

    fs::write(&file_path, file_content).map_err(|e| format!("Ошибка записи файла: {}", e))?;

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

    let updated_content = new_lines.join("\r\n") + "\r\n";
    fs::write(&file_path, updated_content).map_err(|e| format!("Ошибка записи файла: {}", e))?;

    read_domains(filename)
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

                            let mut res = merged_lines.join("\r\n");
                            res.push_str("\r\n");
                            res
                        } else {
                            text.clone()
                        };

                        let lines_count = merged_text.lines().count();
                        if fs::write(&target_path, &merged_text).is_ok() {
                            // Also mirror to bin/lists if it exists
                            if let Ok(exe_path) = std::env::current_exe() {
                                if let Some(parent) = exe_path.parent() {
                                    let bin_list = parent.join("bin/lists").join(filename);
                                    if bin_list.parent().map_or(false, |p| p.is_dir()) {
                                        let _ = fs::write(bin_list, &merged_text);
                                    }
                                }
                            }

                            files_updated += 1;
                            total_domains += lines_count;
                            details.push(format!("{}: актуализировано {} записей (с сохранением пользовательских)", filename, lines_count));
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

    #[test]
    fn test_add_and_remove_domain_preserves_comments_and_subdomains() {
        let lists_dir = get_lists_dir().expect("lists dir must resolve");
        let test_filename = "test_custom_comments_preservation.txt";
        let test_path = lists_dir.join(test_filename);

        let initial_content = "# Header description\r\n# Important TSPU rule\r\n^dns.google\r\nalpha.com\r\n";
        let _ = fs::write(&test_path, initial_content);

        // 1. Add new domain
        let updated = add_domain_to_list(test_filename, "beta.com").expect("Add domain should succeed");
        assert!(updated.contains(&"beta.com".to_string()));

        let disk_content = fs::read_to_string(&test_path).unwrap();
        assert!(disk_content.contains("# Header description"), "Header comment must remain");
        assert!(disk_content.contains("# Important TSPU rule"), "Rule comment must remain");
        assert!(disk_content.contains("^dns.google"), "Existing subdomains must remain");
        assert!(disk_content.contains("beta.com"), "New domain must be appended");

        // 2. Remove domain with ^ matching
        let remaining = remove_domain_from_list(test_filename, "dns.google").expect("Remove should succeed");
        assert!(!remaining.contains(&"^dns.google".to_string()));

        let disk_content_after_remove = fs::read_to_string(&test_path).unwrap();
        assert!(disk_content_after_remove.contains("# Header description"), "Header must remain after remove");
        assert!(disk_content_after_remove.contains("beta.com"), "Beta domain must remain");

        // Clean up
        let _ = fs::remove_file(&test_path);
    }

    #[test]
    fn test_google_and_general_list_separation() {
        let lists_dir = get_lists_dir().expect("lists dir must resolve");
        let google_path = lists_dir.join("list-google.txt");
        let general_path = lists_dir.join("list-general.txt");

        if google_path.exists() {
            let google_content = fs::read_to_string(&google_path).unwrap();
            assert!(!google_content.contains("tenor.com"), "list-google.txt must not contain tenor.com");
            assert!(!google_content.contains("giphy.com"), "list-google.txt must not contain giphy.com");
            assert!(!google_content.contains("discordapp.net"), "list-google.txt must not contain discordapp.net");
            assert!(google_content.contains("googlevideo.com"), "list-google.txt must contain googlevideo.com");
            assert!(google_content.contains("youtube.com"), "list-google.txt must contain youtube.com");
        }

        if general_path.exists() {
            let general_content = fs::read_to_string(&general_path).unwrap();
            assert!(general_content.contains("tenor.com"), "list-general.txt must contain tenor.com");
            assert!(general_content.contains("giphy.com"), "list-general.txt must contain giphy.com");
            assert!(general_content.contains("discord.com"), "list-general.txt must contain discord.com");
        }
    }
}




use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;

use crate::models::{OrganizerAction, OrganizerRule, RunResult};

fn rules_path() -> std::path::PathBuf {
    let base = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    base.join("cleansweep").join("rules.json")
}

fn ensure_dir(path: &Path) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
}

#[tauri::command]
pub fn get_rules() -> Result<Vec<OrganizerRule>, String> {
    let path = rules_path();
    if !path.exists() {
        return Ok(vec![]);
    }
    let data = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    serde_json::from_str(&data).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_rules(rules: Vec<OrganizerRule>) -> Result<(), String> {
    let path = rules_path();
    ensure_dir(&path);
    let data = serde_json::to_string_pretty(&rules).map_err(|e| e.to_string())?;
    fs::write(&path, data).map_err(|e| e.to_string())
}

fn file_age_days(path: &Path) -> i64 {
    path.metadata()
        .ok()
        .and_then(|m| m.modified().ok())
        .map(|t| {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let modified = t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
            ((now.saturating_sub(modified)) / 86400) as i64
        })
        .unwrap_or(0)
}

fn matches_rule(path: &Path, rule: &OrganizerRule) -> bool {
    let meta = match path.metadata() {
        Ok(m) => m,
        Err(_) => return false,
    };

    let size_bytes = meta.len();
    let size_mb = size_bytes as f64 / (1024.0 * 1024.0);
    let age_days = file_age_days(path);
    let conds = &rule.conditions;

    // Extension filter
    if !conds.extensions.is_empty() {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if !conds.extensions.iter().any(|e| e.to_lowercase() == ext) {
            return false;
        }
    }

    // Size range
    if let Some(min) = conds.min_size_mb {
        if size_mb < min {
            return false;
        }
    }
    if let Some(max) = conds.max_size_mb {
        if size_mb > max {
            return false;
        }
    }

    // Age
    if let Some(older) = conds.older_than_days {
        if age_days < older {
            return false;
        }
    }

    // Name regex
    if let Some(ref pattern) = conds.name_pattern_regex {
        if !pattern.is_empty() {
            let re = match Regex::new(pattern) {
                Ok(r) => r,
                Err(_) => return false,
            };
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            if !re.is_match(name) {
                return false;
            }
        }
    }

    true
}

fn apply_rule(path: &Path, rule: &OrganizerRule, errors: &mut Vec<String>) -> bool {
    let dest_dir = Path::new(&rule.destination);
    if let Err(e) = fs::create_dir_all(dest_dir) {
        errors.push(format!("Cannot create dest dir {}: {}", rule.destination, e));
        return false;
    }

    let file_name = match path.file_name() {
        Some(n) => n,
        None => return false,
    };
    let dest_path = dest_dir.join(file_name);

    match rule.action {
        OrganizerAction::Move => {
            if let Err(e) = fs::rename(path, &dest_path) {
                // Try copy + delete across volumes
                if let Err(e2) = fs::copy(path, &dest_path) {
                    errors.push(format!("Move failed for {}: {} / {}", path.display(), e, e2));
                    return false;
                }
                let _ = fs::remove_file(path);
            }
        }
        OrganizerAction::Copy => {
            if let Err(e) = fs::copy(path, &dest_path) {
                errors.push(format!("Copy failed for {}: {}", path.display(), e));
                return false;
            }
        }
        OrganizerAction::Archive => {
            let zip_name = format!(
                "{}.zip",
                file_name.to_string_lossy()
            );
            let zip_path = dest_dir.join(&zip_name);
            let zip_file = match fs::File::create(&zip_path) {
                Ok(f) => f,
                Err(e) => {
                    errors.push(format!("Cannot create zip {}: {}", zip_path.display(), e));
                    return false;
                }
            };
            let mut zip = zip::ZipWriter::new(zip_file);
            let options = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            let data = match fs::read(path) {
                Ok(d) => d,
                Err(e) => {
                    errors.push(format!("Read failed for {}: {}", path.display(), e));
                    return false;
                }
            };
            if let Err(e) = zip.start_file(file_name.to_string_lossy().as_ref(), options) {
                errors.push(format!("Zip start_file failed: {}", e));
                return false;
            }
            use std::io::Write;
            if let Err(e) = zip.write_all(&data) {
                errors.push(format!("Zip write failed: {}", e));
                return false;
            }
            if let Err(e) = zip.finish() {
                errors.push(format!("Zip finish failed: {}", e));
                return false;
            }
            let _ = fs::remove_file(path);
        }
    }

    true
}

#[tauri::command]
pub async fn run_rules(rule_ids: Vec<String>) -> Result<RunResult, String> {
    let all_rules = get_rules()?;
    let rules: Vec<&OrganizerRule> = if rule_ids.is_empty() {
        all_rules.iter().filter(|r| r.enabled).collect()
    } else {
        all_rules
            .iter()
            .filter(|r| rule_ids.contains(&r.id))
            .collect()
    };

    let mut processed: u64 = 0;
    let mut errors: Vec<String> = Vec::new();

    for rule in rules {
        let src = Path::new(&rule.source_folder);
        if !src.exists() {
            errors.push(format!("Source folder not found: {}", rule.source_folder));
            continue;
        }

        for entry in walkdir::WalkDir::new(src)
            .max_depth(1)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if matches_rule(path, rule) {
                if apply_rule(path, rule, &mut errors) {
                    processed += 1;
                }
            }
        }
    }

    Ok(RunResult { processed, errors })
}

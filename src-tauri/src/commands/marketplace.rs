/// Organizer Rule Marketplace — import and export rules as portable JSON.
///
/// Rules can be:
///   - Exported to a JSON file the user can share (e.g. post on GitHub Gist)
///   - Imported from a local JSON file
///   - Imported from a URL (GitHub Gist raw, pastebin, CDN)
///
/// The JSON format is the same as `rules.json` — a serialised `Vec<OrganizerRule>`.
/// Each imported rule gets a fresh UUID to avoid ID collisions with existing rules.

use crate::models::OrganizerRule;

// ─── helpers ──────────────────────────────────────────────────────────────────

/// Generate a simple random UUID v4 without pulling in the `uuid` crate.
/// Uses `std::time` + process ID as entropy source (fine for non-cryptographic IDs).
fn new_id() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::time::{SystemTime, UNIX_EPOCH};

    let mut h = DefaultHasher::new();
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos()
        .hash(&mut h);
    std::process::id().hash(&mut h);

    // Mix in a counter for uniqueness within the same millisecond
    static COUNTER: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);
    COUNTER
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        .hash(&mut h);

    format!("{:016x}", h.finish())
}

/// Re-assign fresh IDs to imported rules to avoid conflicts with existing rules.
fn freshen_ids(mut rules: Vec<OrganizerRule>) -> Vec<OrganizerRule> {
    for rule in &mut rules {
        rule.id = new_id();
    }
    rules
}

// ─── Tauri commands ───────────────────────────────────────────────────────────

/// Export `rule_ids` (or all rules if empty) to `dest_path` as a JSON file.
/// Returns the number of rules written.
#[tauri::command]
pub fn export_rules(rule_ids: Vec<String>, dest_path: String) -> Result<u32, String> {
    let all = crate::commands::organizer::get_rules()?;
    let to_export: Vec<&OrganizerRule> = if rule_ids.is_empty() {
        all.iter().collect()
    } else {
        all.iter().filter(|r| rule_ids.contains(&r.id)).collect()
    };

    if to_export.is_empty() {
        return Err("No rules to export".to_string());
    }

    let json = serde_json::to_string_pretty(&to_export).map_err(|e| e.to_string())?;
    std::fs::write(&dest_path, &json).map_err(|e| e.to_string())?;

    Ok(to_export.len() as u32)
}

/// Import rules from a local JSON file. Merges into existing rules (no deduplication
/// beyond ID freshening — caller should review before saving).
/// Returns the imported rules so the frontend can preview them.
#[tauri::command]
pub fn import_rules_from_file(file_path: String) -> Result<Vec<OrganizerRule>, String> {
    let data = std::fs::read_to_string(&file_path).map_err(|e| e.to_string())?;
    let imported: Vec<OrganizerRule> =
        serde_json::from_str(&data).map_err(|e| format!("Invalid rules JSON: {}", e))?;
    Ok(freshen_ids(imported))
}

/// Import rules from a URL (raw JSON). Performs a synchronous HTTP GET.
/// Supports http:// and https://.
#[tauri::command]
pub fn import_rules_from_url(url: String) -> Result<Vec<OrganizerRule>, String> {
    // Validate URL scheme
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("URL must start with http:// or https://".to_string());
    }

    // Use std::process to shell out to PowerShell (avoids pulling in a full HTTP crate)
    let data = fetch_url(&url)?;
    let imported: Vec<OrganizerRule> =
        serde_json::from_str(&data).map_err(|e| format!("Invalid rules JSON from URL: {}", e))?;
    Ok(freshen_ids(imported))
}

/// Save merged rules (existing + imported). Caller passes the full desired list.
/// Returns the count saved.
#[tauri::command]
pub fn save_merged_rules(rules: Vec<OrganizerRule>) -> Result<u32, String> {
    let count = rules.len() as u32;
    crate::commands::organizer::save_rules(rules)?;
    Ok(count)
}

// ─── HTTP fetch helper ────────────────────────────────────────────────────────

fn fetch_url(url: &str) -> Result<String, String> {
    #[cfg(target_os = "windows")]
    {
        // PowerShell Invoke-WebRequest is available on all supported Windows versions
        let script = format!(
            "(Invoke-WebRequest -Uri '{}' -UseBasicParsing).Content",
            url.replace('\'', "\\'")
        );
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .map_err(|e| format!("PowerShell exec failed: {}", e))?;

        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(format!("HTTP fetch failed: {}", err));
        }
        return Ok(String::from_utf8_lossy(&out.stdout).trim().to_string());
    }

    #[cfg(not(target_os = "windows"))]
    {
        // Fallback: use curl if available
        let out = std::process::Command::new("curl")
            .args(["-fsSL", url])
            .output()
            .map_err(|e| format!("curl exec failed: {}", e))?;

        if !out.status.success() {
            return Err("HTTP fetch failed".to_string());
        }
        return Ok(String::from_utf8_lossy(&out.stdout).trim().to_string());
    }
}

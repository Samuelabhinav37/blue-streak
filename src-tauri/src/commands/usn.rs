/// Windows NTFS USN Change Journal integration.
/// Lets Blue Streak ask: "what files changed since my last scan?"
/// instead of re-walking the entire directory tree.
///
/// On non-Windows platforms this module exposes the same public surface
/// but every function returns an empty/default result so callers need
/// no #[cfg] guards themselves.

use serde::{Deserialize, Serialize};
use crate::models::SharedScanIndex;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsnState {
    pub drive: String,
    pub last_usn: i64,
    pub journal_id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsnChange {
    pub path: String,
    pub reason: String,
}

// ─── Windows implementation ───────────────────────────────────────────────────

#[cfg(target_os = "windows")]
mod win {
    use super::UsnChange;
    use std::mem;

    use std::os::windows::io::AsRawHandle;

    /// Query the NTFS USN journal metadata for `drive` (e.g. "C").
    /// Returns `(next_usn, journal_id)` or an error string.
    pub fn query_journal(drive: &str) -> Result<(i64, u64), String> {
        use std::fs::OpenOptions;
        use std::os::windows::fs::OpenOptionsExt;

        // FILE_FLAG_BACKUP_SEMANTICS = 0x02000000
        // GENERIC_READ = 0x80000000
        let path = format!("\\\\.\\{}:", drive);
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(0x02000000)
            .open(&path)
            .map_err(|e| format!("Cannot open drive {}: {}", drive, e))?;

        // USN_JOURNAL_DATA_V0 layout (64 bytes)
        #[repr(C)]
        struct UsnJournalData {
            usn_journal_id: u64,
            first_usn: i64,
            next_usn: i64,
            lowest_valid_usn: i64,
            max_usn: i64,
            maximum_size: u64,
            allocation_delta: u64,
        }

        let handle = file.as_raw_handle();
        let mut data: UsnJournalData = unsafe { mem::zeroed() };
        let mut bytes_returned: u32 = 0;

        // FSCTL_QUERY_USN_JOURNAL = 0x000900F4
        let ok = unsafe {
            winapi::um::ioapiset::DeviceIoControl(
                handle as *mut _,
                0x000900F4,
                std::ptr::null_mut(),
                0,
                &mut data as *mut _ as *mut _,
                mem::size_of::<UsnJournalData>() as u32,
                &mut bytes_returned,
                std::ptr::null_mut(),
            )
        };

        if ok == 0 {
            return Err(format!(
                "FSCTL_QUERY_USN_JOURNAL failed: {}",
                std::io::Error::last_os_error()
            ));
        }

        Ok((data.next_usn, data.usn_journal_id))
    }

    /// Read all USN records since `start_usn` on `drive`.
    /// Returns changed file paths and the new USN cursor.
    pub fn read_changes_since(
        drive: &str,
        start_usn: i64,
    ) -> Result<(Vec<UsnChange>, i64), String> {
        use std::fs::OpenOptions;
        use std::os::windows::fs::OpenOptionsExt;

        let path = format!("\\\\.\\{}:", drive);
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(0x02000000)
            .open(&path)
            .map_err(|e| format!("Cannot open drive {}: {}", drive, e))?;

        let handle = file.as_raw_handle();

        // READ_USN_JOURNAL_DATA_V0 — 40 bytes
        #[repr(C)]
        struct ReadUsnJournalData {
            start_usn: i64,
            reason_mask: u32,
            return_only_on_close: u32,
            time_out: u64,
            bytes_to_wait_for: u64,
            usn_journal_id: u64,
        }

        // USN_REASON_FILE_CREATE | DATA_OVERWRITE | DATA_EXTEND | RENAME_NEW_NAME
        // | FILE_DELETE | HARD_LINK_CHANGE | STREAM_CHANGE
        let params = ReadUsnJournalData {
            start_usn,
            reason_mask: 0x0000_0001  // DATA_OVERWRITE
                | 0x0000_0002         // DATA_EXTEND
                | 0x0000_0100         // FILE_CREATE
                | 0x0000_0200         // FILE_DELETE
                | 0x0000_1000         // RENAME_NEW_NAME
                | 0x0002_0000,        // HARD_LINK_CHANGE
            return_only_on_close: 0,
            time_out: 0,
            bytes_to_wait_for: 0,
            usn_journal_id: 0,  // 0 = any journal
        };

        let mut buf = vec![0u8; 65536]; // 64 KB read buffer
        let mut changes: Vec<UsnChange> = Vec::new();
        let mut current_usn = start_usn;

        loop {
            let mut bytes_returned: u32 = 0;

            // FSCTL_READ_USN_JOURNAL = 0x000900BB
            let ok = unsafe {
                winapi::um::ioapiset::DeviceIoControl(
                    handle as *mut _,
                    0x000900BB,
                    &params as *const _ as *mut _,
                    mem::size_of::<ReadUsnJournalData>() as u32,
                    buf.as_mut_ptr() as *mut _,
                    buf.len() as u32,
                    &mut bytes_returned,
                    std::ptr::null_mut(),
                )
            };

            if ok == 0 || bytes_returned < 8 {
                break;
            }

            // First 8 bytes = next USN cursor (i64)
            let next_usn = i64::from_le_bytes(buf[..8].try_into().unwrap());
            if next_usn == current_usn {
                break; // No more records
            }
            current_usn = next_usn;

            // Parse USN_RECORD_V2 structs starting at offset 8
            let mut offset = 8usize;
            while offset + 60 <= bytes_returned as usize {
                let record_len =
                    u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap()) as usize;
                if record_len < 60 || offset + record_len > bytes_returned as usize {
                    break;
                }

                let reason =
                    u32::from_le_bytes(buf[offset + 40..offset + 44].try_into().unwrap());
                let file_name_offset =
                    u16::from_le_bytes(buf[offset + 58..offset + 60].try_into().unwrap()) as usize;
                let file_name_len =
                    u16::from_le_bytes(buf[offset + 56..offset + 58].try_into().unwrap()) as usize;

                if file_name_offset + file_name_len <= record_len {
                    let name_bytes = &buf[offset + file_name_offset
                        ..offset + file_name_offset + file_name_len];
                    // File name is UTF-16 LE
                    let name_u16: Vec<u16> = name_bytes
                        .chunks_exact(2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]]))
                        .collect();
                    let name = String::from_utf16_lossy(&name_u16);
                    changes.push(UsnChange {
                        path: format!("{}:\\{}", drive, name),
                        reason: usn_reason_str(reason),
                    });
                }

                offset += record_len;
            }
        }

        Ok((changes, current_usn))
    }

    fn usn_reason_str(reason: u32) -> String {
        let mut parts = Vec::new();
        if reason & 0x0000_0001 != 0 { parts.push("modified"); }
        if reason & 0x0000_0100 != 0 { parts.push("created"); }
        if reason & 0x0000_0200 != 0 { parts.push("deleted"); }
        if reason & 0x0000_1000 != 0 { parts.push("renamed"); }
        if parts.is_empty() { parts.push("changed"); }
        parts.join("+")
    }
}

// ─── Tauri commands ───────────────────────────────────────────────────────────

use tauri::State;
use crate::commands::cache::SharedCache;

/// Phase 5 (Milestone D): Apply a batch of USN changes to the shared `ScanIndex`.
///
/// For each change:
/// - `deleted` / `renamed` → remove the old entry from the index.
/// - `created` / `modified` → stat the file, build a new `IndexEntry`, insert.
///
/// After merging, rebuilds the secondary indices (`by_size`) and recalculates
/// `dir_sizes` so all views reflect the updated state.
///
/// Called by the frontend after `usn_get_changes` returns new changes, before
/// showing the DiskMap or running a duplicate scan.
#[tauri::command]
pub fn apply_usn_delta(
    changes: Vec<UsnChange>,
    scan_index: State<'_, SharedScanIndex>,
) -> Result<usize, String> {
    use std::time::{SystemTime, UNIX_EPOCH};
    use crate::models::IndexEntry;

    if changes.is_empty() {
        return Ok(0);
    }

    let mut index = scan_index.write().map_err(|e| e.to_string())?;

    let mut applied = 0usize;

    for change in &changes {
        let is_delete = change.reason.contains("deleted");
        let is_rename = change.reason.contains("renamed");

        if is_delete || is_rename {
            // Remove stale entry
            let path_lc = change.path.to_ascii_lowercase();
            index.entries.retain(|e| e.full_path.to_ascii_lowercase() != path_lc);
            applied += 1;
        }

        if !is_delete {
            // For creates / modifies / renames: stat the new path and insert/update
            let p = std::path::Path::new(&change.path);
            if let Ok(meta) = std::fs::metadata(p) {
                let now_secs = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;
                let modified_secs = meta.modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(now_secs);
                let age_days = (now_secs.saturating_sub(modified_secs) / 86_400) as i64;

                let is_dir = meta.is_dir();
                let size = if is_dir { 0 } else { meta.len() };

                let short = change.path
                    .rsplit(['\\', '/'])
                    .next()
                    .unwrap_or(&change.path);
                let (name_offset, name_len) = index.names.push(short);

                // Remove any existing entry with the same path first (idempotent)
                let path_lc = change.path.to_ascii_lowercase();
                index.entries.retain(|e| e.full_path.to_ascii_lowercase() != path_lc);

                index.entries.push(IndexEntry {
                    file_ref:    0,
                    parent_ref:  0,
                    size,
                    age_days,
                    name_offset,
                    name_len,
                    flags: if is_dir { 1 } else { 0 },
                    full_path: change.path.clone(),
                });
                applied += 1;
            }
        }
    }

    // Rebuild secondary indices after delta
    index.build_indices();
    index.build_dir_sizes();

    Ok(applied)
}

/// Get the current USN cursor for `drive` and store it as the baseline.
/// Call this right after a full scan to mark the starting point.
#[tauri::command]
pub fn usn_snapshot(
    drive: String,
    cache: State<'_, SharedCache>,
) -> Result<UsnState, String> {
    #[cfg(target_os = "windows")]
    {
        let (next_usn, journal_id) = win::query_journal(&drive)?;
        let cache = cache.lock().map_err(|e| e.to_string())?;
        cache
            .save_usn_state(&drive, next_usn, journal_id)
            .map_err(|e| e.to_string())?;
        return Ok(UsnState { drive, last_usn: next_usn, journal_id });
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = cache;
        Ok(UsnState { drive, last_usn: 0, journal_id: 0 })
    }
}

/// Return files that changed on `drive` since the last snapshot.
/// Frontend uses this to know which files need re-scanning.
#[tauri::command]
pub fn usn_get_changes(
    drive: String,
    cache: State<'_, SharedCache>,
) -> Result<Vec<UsnChange>, String> {
    #[cfg(target_os = "windows")]
    {
        let (saved_usn, _journal_id) = {
            let cache = cache.lock().map_err(|e| e.to_string())?;
            cache.load_usn_state(&drive).unwrap_or((0, 0))
        };

        if saved_usn == 0 {
            return Ok(vec![]);
        }

        let (changes, new_usn) = win::read_changes_since(&drive, saved_usn)?;

        // Advance the cursor
        {
            let cache = cache.lock().map_err(|e| e.to_string())?;
            let (_, journal_id) = cache.load_usn_state(&drive).unwrap_or((0, 0));
            let _ = cache.save_usn_state(&drive, new_usn, journal_id);
        }

        return Ok(changes);
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (drive, cache);
        Ok(vec![])
    }
}

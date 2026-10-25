/// Phase 5 (Milestone C3): Snapshot persistence for ScanIndex.
///
/// Serializes the in-memory `ScanIndex` to disk with `bincode` after every
/// scan so that the next app launch can restore it in <200 ms instead of
/// running a full scan (2–60 s depending on filesystem and privileges).
///
/// On startup:
///   1. Load snapshot from `%LOCALAPPDATA%\cleansweep\scan.bin` (if it exists).
///   2. Call `usn_get_changes` to detect what changed since the snapshot was taken.
///   3. Apply the USN delta to the restored index.
///   4. The index is now current — all views are ready without any filesystem walk.
///
/// Storage path: `%LOCALAPPDATA%\cleansweep\scan.bin`
/// File format: `bincode`-encoded `ScanIndex` struct (compact binary, ~140 MB
/// for 1M files vs ~1 GB in-memory because strings are arena-packed).

use std::io::{BufReader, BufWriter};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::models::{ScanIndex, SharedScanIndex};

// ─── helpers ─────────────────────────────────────────────────────────────────

fn snapshot_path() -> Result<PathBuf, String> {
    let base = dirs::data_local_dir()
        .ok_or_else(|| "Cannot locate %LOCALAPPDATA%".to_string())?;
    let dir = base.join("cleansweep");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("scan.bin"))
}

// ─── public API ──────────────────────────────────────────────────────────────

/// Save the current `SharedScanIndex` to `%LOCALAPPDATA%\cleansweep\scan.bin`.
///
/// Called automatically after every successful scan completes.
/// Returns the number of bytes written on success.
#[tauri::command]
pub fn save_snapshot(scan_index: State<SharedScanIndex>) -> Result<u64, String> {
    let index = scan_index.read().map_err(|e| e.to_string())?;

    if index.entries.is_empty() {
        return Err("No scan data to save".to_string());
    }

    let path = snapshot_path()?;
    let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
    let mut writer = BufWriter::new(file);

    bincode::serialize_into(&mut writer, &*index)
        .map_err(|e| format!("Snapshot serialize error: {}", e))?;

    // Flush and measure written bytes
    let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    Ok(meta.len())
}

/// Load the snapshot from disk into `SharedScanIndex`.
///
/// On success, returns a summary `{ entry_count, root, scanned_at }`.
/// Returns `Err` if no snapshot file exists or if it cannot be decoded.
/// Callers should follow up with a USN delta check to pick up changes since
/// the snapshot was taken.
#[tauri::command]
pub fn load_snapshot(scan_index: State<SharedScanIndex>) -> Result<SnapshotInfo, String> {
    let path = snapshot_path()?;
    if !path.exists() {
        return Err("No snapshot file found".to_string());
    }

    let file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
    let reader = BufReader::new(file);

    let loaded: ScanIndex = bincode::deserialize_from(reader)
        .map_err(|e| format!("Snapshot deserialize error: {}", e))?;

    let info = SnapshotInfo {
        entry_count: loaded.entries.len(),
        root: loaded.root.clone(),
        scanned_at: loaded.scanned_at,
    };

    let mut index = scan_index.write().map_err(|e| e.to_string())?;
    *index = loaded;

    Ok(info)
}

/// Delete the snapshot file (e.g. when the user clears the cache).
#[tauri::command]
pub fn delete_snapshot() -> Result<(), String> {
    let path = snapshot_path()?;
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotInfo {
    pub entry_count: usize,
    pub root: String,
    pub scanned_at: u64,
}

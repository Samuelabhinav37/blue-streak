/// NTFS MFT direct scanning — WizTree-style, Windows only.
///
/// # Architecture
///
/// WizTree reads the $MFT file directly from the NTFS volume.  The $MFT is a
/// regular file (stored at a well-known location) containing one 1024-byte
/// record per file/directory on the volume.  Reading it sequentially gives
/// names, parent refs, sizes and timestamps for every entry in one disk pass —
/// no per-file or per-directory syscalls, no ACL checks.
///
/// ## How we access the $MFT
///
/// We open the volume via `\\.\C:` with `GENERIC_READ | FILE_FLAG_BACKUP_SEMANTICS`
/// (requires admin), then call `NtCreateFile` on the special path
/// `\$MFT` inside the volume to obtain a handle to the MFT data stream.
/// This handle is wrapped in a `BufReader` and passed to `MftParser::from_read_seek`.
///
/// The `mft` crate (omerbenamram/mft) parses every 1024-byte record, decodes
/// the `$FILE_NAME` attribute (name + parent ref + timestamps) and the `$DATA`
/// attribute (logical file size).  It also handles multi-extent runs, fixups,
/// and compressed/sparse records automatically.
///
/// ## Performance
///
/// Reading the $MFT is one sequential I/O stream — near disk-bandwidth speed.
/// A typical C:\ drive with 1 M files has a ~1–2 GB $MFT that reads in 2–5 s.
/// Path reconstruction (BFS, O(n)) adds another ~100 ms.
/// Total: **~3–6 s** for 1 M files vs 10+ min with FindFirstFile.
///
/// ## Fallback
///
/// If admin access is denied, or if the $MFT open fails for any reason, the
/// caller falls back to `jwalk` automatically — no error is shown to the user.

use serde::{Deserialize, Serialize};

/// A single entry produced by the MFT walk (name, size, age, is_dir).
#[derive(Debug, Clone)]
pub struct FlatEntry {
    pub path:       String,
    pub size_bytes: u64,
    pub age_days:   i64,
    pub is_dir:     bool,
}

// ─── Tauri-facing types ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MftEntry {
    pub path:       String,
    pub size_bytes: u64,
    pub file_ref:   u64,
    pub is_dir:     bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MftScanResult {
    pub entries:          Vec<MftEntry>,
    pub total_files:      u64,
    pub total_dirs:       u64,
    pub total_size_bytes: u64,
    pub duration_ms:      u64,
}

// ─── Windows implementation ───────────────────────────────────────────────────

#[cfg(target_os = "windows")]
pub mod win {
    use std::collections::HashMap;
    use std::io::{Cursor, Read};
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    use super::FlatEntry;

    // ── Admin check ──────────────────────────────────────────────────────────

    pub fn is_admin() -> bool {
        use std::fs::OpenOptions;
        use std::os::windows::fs::OpenOptionsExt;
        OpenOptions::new()
            .read(true)
            .custom_flags(0x02000000) // FILE_FLAG_BACKUP_SEMANTICS
            .open("\\\\.\\C:")
            .is_ok()
    }

    /// Read the entire $MFT into RAM as a `Vec<u8>`.
    ///
    /// Phase 5 (Bug 1 fix): eliminates the 3× I/O cost of the old
    /// `read_mft_to_temp` approach (READ from disk → WRITE temp → READ temp).
    /// We now do a single sequential read directly into heap memory and wrap it
    /// with `Cursor` so `MftParser::from_read_seek` can seek into it.
    fn read_mft_to_memory(drive: &str) -> Result<Vec<u8>, String> {
        use std::fs::OpenOptions;
        use std::os::windows::fs::OpenOptionsExt;

        let mft_path = format!("\\\\?\\{}:\\$MFT", drive.trim_end_matches([':', '\\']));
        let mut src = OpenOptions::new()
            .read(true)
            .custom_flags(0x02000000) // FILE_FLAG_BACKUP_SEMANTICS
            .open(&mft_path)
            .map_err(|e| format!("Cannot open $MFT on {}: {}", drive, e))?;

        let mut buf = Vec::with_capacity(256 * 1024 * 1024); // 256 MiB initial
        let mut chunk = vec![0u8; 4 * 1024 * 1024]; // 4 MiB read chunks
        loop {
            match src.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                Err(e) => return Err(format!("MFT read error: {}", e)),
            }
        }
        Ok(buf)
    }

    // ── Timestamp → age_days ─────────────────────────────────────────────────

    fn timestamp_to_age_days(ts: &mft::Timestamp) -> i64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let then = ts.as_second();
        let diff_secs = now.saturating_sub(then);
        (diff_secs / 86_400) as i64
    }

    // ── size from $DATA attribute ─────────────────────────────────────────────

    /// Extract logical file size from a parsed $DATA attribute.
    fn size_from_data_attr(a: &mft::attribute::MftAttribute) -> u64 {
        use mft::attribute::{MftAttributeContent, header::ResidentialHeader};
        match &a.data {
            MftAttributeContent::DataRun(_) => {
                if let ResidentialHeader::NonResident(ref nr) = a.header.residential_header {
                    nr.file_size
                } else {
                    0
                }
            }
            MftAttributeContent::AttrX80(ref d) => d.data().len() as u64,
            MftAttributeContent::Raw(ref r)     => r.data.len() as u64,
            _ => 0,
        }
    }

    // ── Public walk_drive ─────────────────────────────────────────────────────

    /// Walk `drive` via the raw MFT and return every file/dir with size + age.
    ///
    /// Phase 5 improvements over Phase 3 implementation:
    ///
    /// 1. **In-memory MFT** (Bug 1 fix): reads $MFT into `Vec<u8>` + `Cursor`
    ///    instead of copying to a temp file on disk.  Eliminates 2× extra I/O.
    ///
    /// 2. **HashMap path reconstruction** (Bug 2 fix): during Pass 1 we build
    ///    `parent_map: HashMap<u64, u64>` (file_ref → parent_ref) and
    ///    `name_map: HashMap<u64, String>` (file_ref → short_name).
    ///    Pass 2 reconstructs full paths via O(1) HashMap lookups instead of
    ///    using the 128-entry LRU cache which had a >99% miss rate.
    ///
    /// Returns `Err` if not running as Administrator.
    pub fn walk_drive(
        drive: &str,
        progress_cb: &(impl Fn(u64) + Sync),
    ) -> Result<Vec<FlatEntry>, String> {
        let _t = Instant::now();
        let drive_letter = drive.trim_end_matches([':', '\\']);

        // ── Pass 1: read $MFT into RAM, collect raw data + build path maps ────
        let mft_bytes = read_mft_to_memory(drive)?;
        let cursor = Cursor::new(mft_bytes);
        let mut parser = mft::MftParser::from_read_seek(cursor, None)
            .map_err(|e| format!("MftParser error: {}", e))?;

        #[allow(dead_code)]
        struct RawData {
            file_ref:   u64,
            parent_ref: u64,
            name:       String,
            size_bytes: u64,
            age_days:   i64,
            is_dir:     bool,
        }

        let total = parser.get_entry_count();
        let mut raw: Vec<RawData> = Vec::with_capacity(total as usize);
        // file_ref → parent_ref (for path reconstruction)
        let mut parent_map: HashMap<u64, u64> = HashMap::with_capacity(total as usize);
        // file_ref → short name (for path reconstruction)
        let mut name_map: HashMap<u64, String> = HashMap::with_capacity(total as usize);
        let mut count = 0u64;

        for entry_result in parser.iter_entries() {
            count += 1;
            if count % 50_000 == 0 {
                progress_cb(count);
            }

            let entry = match entry_result {
                Ok(e) => e,
                Err(_) => continue,
            };

            if !entry.header.is_valid() {
                continue;
            }

            let file_ref = entry.header.record_number;

            let is_dir = entry.header.flags.contains(
                mft::entry::EntryFlags::INDEX_PRESENT,
            );

            // Extract name + parent ref from $FILE_NAME attribute
            let mut entry_name = String::new();
            let mut parent_ref = 0u64;
            for attr in entry.iter_attributes().filter_map(|a| a.ok()) {
                if attr.header.type_code == mft::attribute::MftAttributeType::FileName {
                    if let mft::attribute::MftAttributeContent::AttrX30(ref fn_attr) = attr.data {
                        entry_name = fn_attr.name.clone();
                        parent_ref = fn_attr.parent.entry;
                        break;
                    }
                }
            }

            if entry_name.is_empty() { continue; }

            parent_map.insert(file_ref, parent_ref);
            name_map.insert(file_ref, entry_name.clone());

            let size_bytes: u64 = if is_dir {
                0
            } else {
                entry
                    .iter_attributes()
                    .filter_map(|a| a.ok())
                    .find(|a| a.header.type_code == mft::attribute::MftAttributeType::DATA)
                    .map(|a| size_from_data_attr(&a))
                    .unwrap_or(0)
            };

            let age_days: i64 = entry
                .iter_attributes()
                .filter_map(|a| a.ok())
                .find(|a| a.header.type_code == mft::attribute::MftAttributeType::StandardInformation)
                .and_then(|a| match a.data {
                    mft::attribute::MftAttributeContent::AttrX10(ref si) => {
                        Some(timestamp_to_age_days(&si.modified))
                    }
                    _ => None,
                })
                .unwrap_or(0);

            raw.push(RawData { file_ref, parent_ref, name: entry_name, size_bytes, age_days, is_dir });
        }
        progress_cb(count);

        // ── Pass 2: reconstruct full paths via HashMap lookups (O(1) per hop) ─
        // NTFS root directory always has file_ref = 5.
        const NTFS_ROOT: u64 = 5;
        let mut result: Vec<FlatEntry> = Vec::with_capacity(raw.len());

        for rd in &raw {
            // Walk the parent chain until we reach the NTFS root (ref 5) or a
            // cycle.  Build path components in reverse then join them.
            let mut components: Vec<&str> = vec![rd.name.as_str()];
            let mut cur = rd.parent_ref;
            let mut depth = 0u8;
            loop {
                if cur == NTFS_ROOT || cur == 0 { break; }
                if let Some(name) = name_map.get(&cur) {
                    components.push(name.as_str());
                }
                match parent_map.get(&cur) {
                    Some(&p) if p != cur => cur = p,
                    _ => break,
                }
                depth += 1;
                if depth > 64 { break; } // guard against cycles
            }
            components.reverse();

            let rel_path = components.join("\\");
            let full_path = format!("{}:\\{}", drive_letter, rel_path);

            if full_path.contains("[Unknown]") { continue; }

            result.push(FlatEntry {
                path:       full_path,
                size_bytes: rd.size_bytes,
                age_days:   rd.age_days,
                is_dir:     rd.is_dir,
            });
        }

        Ok(result)
    }
}

// ─── Tauri command ─────────────────────────────────────────────────────────────

#[tauri::command]
pub fn scan_mft(drive: String) -> Result<MftScanResult, String> {
    #[cfg(target_os = "windows")]
    {
        use std::time::Instant;
        let t = Instant::now();
        let entries = win::walk_drive(&drive, &|_| {})?;
        let duration_ms = t.elapsed().as_millis() as u64;

        let mut total_files = 0u64;
        let mut total_dirs  = 0u64;
        let mut total_size  = 0u64;
        let mft_entries: Vec<MftEntry> = entries
            .iter()
            .map(|e| {
                if e.is_dir { total_dirs  += 1; }
                else        { total_files += 1; total_size += e.size_bytes; }
                MftEntry {
                    path:       e.path.clone(),
                    size_bytes: e.size_bytes,
                    file_ref:   0,
                    is_dir:     e.is_dir,
                }
            })
            .collect();

        return Ok(MftScanResult {
            entries: mft_entries,
            total_files,
            total_dirs,
            total_size_bytes: total_size,
            duration_ms,
        });
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = drive;
        Err("MFT scanning is only supported on Windows".to_string())
    }
}

// ─── Non-Windows stubs ────────────────────────────────────────────────────────

#[cfg(not(target_os = "windows"))]
pub mod win {
    use super::FlatEntry;

    pub fn walk_drive(
        _drive: &str,
        _progress_cb: &(impl Fn(u64) + Sync),
    ) -> Result<Vec<FlatEntry>, String> {
        Err("MFT scanning is only supported on Windows".to_string())
    }

    pub fn is_admin() -> bool { false }
}

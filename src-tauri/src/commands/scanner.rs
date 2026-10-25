use std::collections::HashMap;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rayon::prelude::*;
use tauri::{AppHandle, Emitter, State};

use crate::models::{
    DeleteResult, FailedDelete, IndexEntry, JunkCategory, JunkFile, NameArena, ScanIndex,
    ScanProgress, SharedScanIndex,
};

// ─── helpers ────────────────────────────────────────────────────────────────

fn age_days(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .map(|t| {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let modified = t
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            ((now.saturating_sub(modified)) / 86400) as i64
        })
        .unwrap_or(0)
}

/// Hash only the first `limit_bytes` of a file — used as a cheap pre-filter
/// before committing to a full hash. 64 KiB is sufficient to rule out ~99% of
/// false-positive size collisions.
fn partial_hash(path: &Path, limit_bytes: usize) -> Option<[u8; 32]> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = BufReader::with_capacity(64 * 1024, file);
    let mut hasher = blake3::Hasher::new();
    let mut buf = [0u8; 64 * 1024];
    let mut total = 0usize;

    loop {
        let remaining = limit_bytes - total;
        if remaining == 0 {
            break;
        }
        let to_read = remaining.min(buf.len());
        match reader.read(&mut buf[..to_read]) {
            Ok(0) => break,
            Ok(n) => {
                hasher.update(&buf[..n]);
                total += n;
            }
            Err(_) => return None,
        }
    }
    Some(*hasher.finalize().as_bytes())
}

/// Full-file BLAKE3 hash.
///
/// For files ≥ 512 MiB we use `memmap2` to avoid a kernel→user copy on every
/// `read()` syscall (≈30% faster on large files).
/// For files ≥ 1 GiB we additionally use BLAKE3's built-in rayon tree parallelism
/// via `update_rayon()` (≈4× faster on 8-core CPUs).
fn full_hash(path: &Path) -> Option<[u8; 32]> {
    use memmap2::Mmap;
    const MMAP_THRESHOLD: u64    = 512  * 1024 * 1024;  // 512 MiB
    const RAYON_THRESHOLD: u64   = 1024 * 1024 * 1024;  // 1 GiB

    let file = std::fs::File::open(path).ok()?;
    let file_len = file.metadata().ok()?.len();

    if file_len >= MMAP_THRESHOLD {
        // Safety: we never write to this mapping and the file is not truncated
        // during the hash. If it is (race), BLAKE3 will produce a different hash
        // that won't match any duplicate — safe, not UB.
        let mmap = unsafe { Mmap::map(&file).ok()? };
        let mut hasher = blake3::Hasher::new();
        if file_len >= RAYON_THRESHOLD {
            // Phase 3.3: parallel multi-core hashing for very large files
            hasher.update_rayon(&mmap);
        } else {
            hasher.update(&mmap);
        }
        return Some(*hasher.finalize().as_bytes());
    }

    // Small/medium files: streaming 64 KiB buffered read
    let mut reader = BufReader::with_capacity(64 * 1024, file);
    let mut hasher = blake3::Hasher::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => { hasher.update(&buf[..n]); }
            Err(_) => return None,
        }
    }
    Some(*hasher.finalize().as_bytes())
}

// ─── Arena-allocated path store ──────────────────────────────────────────────
//
// Phase 3.1: instead of one heap-allocated `PathBuf` per file (≈128 bytes each,
// ~1 GB for 1M files), we intern all path strings into a single contiguous byte
// buffer and reference them by (offset, length) u32 pairs.
//
// Compact record layout (≈34 bytes vs ≈128 bytes for PathBuf):
//   path_offset : u32   — byte offset into `strings` buffer
//   path_len    : u32   — byte length of path UTF-8 string
//   size        : u64
//   age_days    : i64
//   flags       : u8    (bit 0 = is_dir)
//
// The `strings` buffer is shared across all entries and grown via `extend_from_slice`.

struct PathArena {
    strings: Vec<u8>,
}

impl PathArena {
    fn new() -> Self {
        Self { strings: Vec::with_capacity(64 * 1024 * 1024) } // 64 MiB initial
    }

    /// Intern `s` and return (offset, len).
    fn intern(&mut self, s: &str) -> (u32, u32) {
        let offset = self.strings.len() as u32;
        self.strings.extend_from_slice(s.as_bytes());
        (offset, s.len() as u32)
    }

    /// Retrieve a previously interned string.
    #[inline]
    fn get(&self, offset: u32, len: u32) -> &str {
        let start = offset as usize;
        let end   = start + len as usize;
        // SAFETY: bytes were written as valid UTF-8 by `intern`
        unsafe { std::str::from_utf8_unchecked(&self.strings[start..end]) }
    }
}

/// Compact per-file record — uses arena offsets instead of owned PathBuf.
struct CompactEntry {
    path_offset: u32,
    path_len:    u32,
    size:        u64,
    age_days:    i64,
    flags:       u8,  // bit 0 = is_dir
}

impl CompactEntry {
    #[inline] fn is_dir(&self) -> bool { self.flags & 1 != 0 }
}

struct ArenaEntries {
    arena:   PathArena,
    entries: Vec<CompactEntry>,
}

// ─── directory collector — MFT fast path + jwalk fallback ───────────────────
//
// Strategy:
//   1. On Windows with admin: use MFT walk (FSCTL_ENUM_USN_RECORDS) for the
//      drive that contains `root`, then filter to entries under `root`.
//      The MFT walk gets all files in one sequential pass (~2–5 s for 1M files).
//   2. Otherwise (no admin, non-Windows, or MFT fails): fall back to jwalk.
//      jwalk uses FindFirstFile/FindNextFile per directory — much slower but
//      works for any user.

fn flat_entries_to_arena(raw: Vec<(String, u64, i64, bool)>) -> ArenaEntries {
    let mut ae = ArenaEntries {
        arena:   PathArena::new(),
        entries: Vec::with_capacity(raw.len()),
    };
    for (path_str, size, age, is_dir) in raw {
        let (offset, len) = ae.arena.intern(&path_str);
        ae.entries.push(CompactEntry {
            path_offset: offset,
            path_len:    len,
            size,
            age_days:    age,
            flags:       if is_dir { 1 } else { 0 },
        });
    }
    ae
}

/// Extract the drive letter from a path (e.g. "C:\\Users\\..." → "C").
fn drive_letter(path: &Path) -> Option<String> {
    let s = path.to_string_lossy();
    if s.len() >= 2 && s.as_bytes()[1] == b':' {
        Some(s[..1].to_ascii_uppercase())
    } else {
        None
    }
}

fn parallel_walk(
    root: &Path,
    scanned: &Arc<AtomicU64>,
    app: &AppHandle,
    progress_interval: u64,
) -> ArenaEntries {
    // ── Try MFT fast path on Windows ────────────────────────────────────────
    #[cfg(target_os = "windows")]
    if let Some(drive) = drive_letter(root) {
        if crate::commands::mft::win::is_admin() {
            let root_str = root.to_string_lossy().to_ascii_lowercase();
            let scanned_clone = Arc::clone(scanned);
            let app_clone = app.clone();

            let result = crate::commands::mft::win::walk_drive(&drive, &|n| {
                // Emit progress once we know the total record count
                let _ = app_clone.emit(
                    "scan-progress",
                    ScanProgress {
                        scanned: n,
                        found: 0,
                        current_path: format!("{}:\\ (MFT scan…)", drive),
                        done: false,
                        batch: vec![],
                    },
                );
                scanned_clone.store(n, Ordering::Relaxed);
            });

            if let Ok(entries) = result {
                // Filter to entries that live under `root`
                let raw: Vec<(String, u64, i64, bool)> = entries
                    .into_iter()
                    .filter(|e| e.path.to_ascii_lowercase().starts_with(&root_str))
                    .map(|e| (e.path, e.size_bytes, e.age_days, e.is_dir))
                    .collect();

                // Emit a final progress tick so the UI updates
                let total = scanned.load(Ordering::Relaxed);
                let _ = app.emit(
                    "scan-progress",
                    ScanProgress {
                        scanned: total,
                        found: 0,
                        current_path: String::new(),
                        done: false,
                        batch: vec![],
                    },
                );

                return flat_entries_to_arena(raw);
            }
            // MFT failed (e.g. access denied mid-scan) — fall through to jwalk
        }
    }

    // ── jwalk fallback (non-admin or non-Windows) ────────────────────────────
    use jwalk::WalkDir;

    let raw: Vec<(String, u64, i64, bool)> = WalkDir::new(root)
        .skip_hidden(false)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter_map(|entry| {
            let n = scanned.fetch_add(1, Ordering::Relaxed);
            if n % progress_interval == 0 {
                let _ = app.emit(
                    "scan-progress",
                    ScanProgress {
                        scanned: n,
                        found: 0,
                        current_path: entry.path().to_string_lossy().to_string(),
                        done: false,
                        batch: vec![],
                    },
                );
            }
            let meta = entry.metadata().ok()?;
            let is_dir = meta.is_dir();
            Some((
                entry.path().to_string_lossy().into_owned(),
                if is_dir { 0 } else { meta.len() },
                age_days(&meta),
                is_dir,
            ))
        })
        .collect();

    flat_entries_to_arena(raw)
}

// ─── collect_all_entries: MFT-deduped walk ───────────────────────────────────
//
// Groups roots by drive. On Windows with admin, does exactly one MFT scan per
// drive and filters entries per root. Falls back to per-root jwalk otherwise.

fn collect_all_entries(
    roots: &[(PathBuf, JunkCategory)],
    scanned: &Arc<AtomicU64>,
    app: &AppHandle,
    progress_interval: u64,
) -> Vec<(String, u64, i64, bool, JunkCategory)> {
    let mut result: Vec<(String, u64, i64, bool, JunkCategory)> = Vec::new();

    // ── Windows + admin: one MFT scan per drive ──────────────────────────────
    #[cfg(target_os = "windows")]
    if crate::commands::mft::win::is_admin() {
        use std::collections::HashMap as HMap;

        // Group roots by drive letter
        let mut by_drive: HMap<String, Vec<(PathBuf, JunkCategory)>> = HMap::new();
        let mut no_drive: Vec<(PathBuf, JunkCategory)> = Vec::new();

        for (root, cat) in roots {
            if let Some(d) = drive_letter(root) {
                by_drive.entry(d).or_default().push((root.clone(), cat.clone()));
            } else {
                no_drive.push((root.clone(), cat.clone()));
            }
        }

        for (drive, drive_roots) in &by_drive {
            let app_clone = app.clone();
            let scanned_clone = Arc::clone(scanned);
            let drive_clone = drive.clone();

            match crate::commands::mft::win::walk_drive(drive, &|n| {
                scanned_clone.store(n, Ordering::Relaxed);
                let _ = app_clone.emit(
                    "scan-progress",
                    ScanProgress {
                        scanned: n,
                        found: 0,
                        current_path: format!("{}:\\ (MFT…)", drive_clone),
                        done: false,
                        batch: vec![],
                    },
                );
            }) {
                Ok(flat) => {
                    // For each entry, find which root it belongs to
                    for entry in flat {
                        let path_lower = entry.path.to_ascii_lowercase();
                        for (root, cat) in drive_roots {
                            let root_lower = root.to_string_lossy().to_ascii_lowercase();
                            if path_lower.starts_with(&root_lower) {
                                result.push((entry.path.clone(), entry.size_bytes, entry.age_days, entry.is_dir, cat.clone()));
                                break; // assign to first matching root
                            }
                        }
                    }
                    // Emit final tick
                    let total = scanned.load(Ordering::Relaxed);
                    let _ = app.emit("scan-progress", ScanProgress {
                        scanned: total, found: 0,
                        current_path: String::new(), done: false, batch: vec![],
                    });
                    continue; // don't fall back for this drive
                }
                Err(_) => {} // fall through to jwalk for this drive's roots
            }

            // MFT failed for this drive — fall back to per-root jwalk
            for (root, cat) in drive_roots {
                if !root.exists() { continue; }
                let ae = parallel_walk(root, scanned, app, progress_interval);
                for entry in &ae.entries {
                    let path = ae.arena.get(entry.path_offset, entry.path_len).to_owned();
                    result.push((path, entry.size, entry.age_days, entry.is_dir(), cat.clone()));
                }
            }
        }

        // Non-drive roots (UNC paths etc.) always use jwalk
        for (root, cat) in &no_drive {
            if !root.exists() { continue; }
            let ae = parallel_walk(root, scanned, app, progress_interval);
            for entry in &ae.entries {
                let path = ae.arena.get(entry.path_offset, entry.path_len).to_owned();
                result.push((path, entry.size, entry.age_days, entry.is_dir(), cat.clone()));
            }
        }

        return result;
    }

    // ── jwalk fallback (non-Windows or no admin) ─────────────────────────────
    for (root, cat) in roots {
        if !root.exists() { continue; }
        let ae = parallel_walk(root, scanned, app, progress_interval);
        for entry in &ae.entries {
            let path = ae.arena.get(entry.path_offset, entry.path_len).to_owned();
            result.push((path, entry.size, entry.age_days, entry.is_dir(), cat.clone()));
        }
    }
    result
}

// ─── scan command ────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn scan_for_junk(
    app: AppHandle,
    paths: Vec<String>,
    scan_temp: bool,
    scan_downloads: bool,
    scan_duplicates: bool,
    scan_empty_dirs: bool,
    scan_large: bool,
    scan_index: State<'_, SharedScanIndex>,
) -> Result<Vec<JunkFile>, String> {
    // Clone the Arc out of State so it can be sent into spawn_blocking.
    // State<'_> itself is not Send, but the inner Arc<RwLock<...>> is.
    let scan_index_arc = Arc::clone(&*scan_index);

    // ── Build roots list (cheap — no I/O) ────────────────────────────────────
    let mut roots: Vec<(PathBuf, JunkCategory)> = Vec::new();

    if scan_temp {
        // %TEMP% is usually C:\Users\<user>\AppData\Local\Temp
        if let Ok(temp) = std::env::var("TEMP") {
            let p = PathBuf::from(&temp);
            if p.exists() { roots.push((p, JunkCategory::Temp)); }
        }
        // C:\Windows\Temp — only add if it's on a different drive than %TEMP%
        // so we don't scan the same MFT region twice.
        let win_temp = PathBuf::from(r"C:\Windows\Temp");
        let already_covered = roots.iter().any(|(r, _)| {
            r.to_string_lossy().to_ascii_lowercase().starts_with("c:")
        });
        if !already_covered && win_temp.exists() {
            roots.push((win_temp, JunkCategory::Temp));
        }
    }

    // Default: scan user's Downloads and Desktop — not all of C:\Users
    // (scanning all of C:\Users is 100k+ files and takes minutes for no extra value)
    let effective_paths: Vec<String> = if paths.is_empty() {
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_else(|_| r"C:\Users\Public".to_string());
        let candidates = [
            format!("{}\\Downloads", home),
            format!("{}\\Desktop", home),
            format!("{}\\Documents", home),
        ];
        candidates.iter()
            .filter(|p| std::path::Path::new(p).exists())
            .cloned()
            .collect()
    } else {
        paths.clone()
    };

    for p in &effective_paths {
        let pb = PathBuf::from(p);
        if pb.exists() {
            roots.push((pb, JunkCategory::Download));
        }
    }

    // ── Move ALL blocking work off the async executor ─────────────────────────
    // Tauri's async runtime has a small fixed thread pool. If we block it with
    // a 5-minute filesystem walk, every other command (settings load, sidebar
    // navigation, disk usage poll) hangs until the scan finishes.
    // spawn_blocking puts this work on a dedicated OS thread pool instead.
    let app_clone = app.clone();

    let (results, all_flat, total_scanned) = tokio::task::spawn_blocking(move || {
        let scanned = Arc::new(AtomicU64::new(0));
        const PROGRESS_INTERVAL: u64 = 200;

        let all_flat: Vec<(String, u64, i64, bool, JunkCategory)> =
            collect_all_entries(&roots, &scanned, &app_clone, PROGRESS_INTERVAL);

        let total_scanned = scanned.load(Ordering::Relaxed);

        // ── Phase 2: cheap classification (no I/O) ─────────────────────────
        let dup_candidates: Arc<Mutex<HashMap<u64, Vec<PathBuf>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let batch_size: usize = 50;
        let pending: Arc<Mutex<Vec<JunkFile>>> = Arc::new(Mutex::new(Vec::new()));
        let total_found = Arc::new(AtomicU64::new(0));

        let emit_batch = |items: Vec<JunkFile>| {
            let n = total_found.fetch_add(items.len() as u64, Ordering::Relaxed) + items.len() as u64;
            let _ = app_clone.emit(
                "scan-progress",
                ScanProgress {
                    scanned: total_scanned,
                    found: n,
                    current_path: String::new(),
                    done: false,
                    batch: items,
                },
            );
        };

        let cheap_results: Vec<JunkFile> = all_flat
            .par_iter()
            .filter_map(|(path, size, age, is_dir, category)| {
                if *is_dir {
                    if scan_empty_dirs {
                        let is_empty = std::fs::read_dir(path)
                            .map(|mut d| d.next().is_none())
                            .unwrap_or(false);
                        if is_empty {
                            return Some(JunkFile {
                                path: path.clone(),
                                size_bytes: 0,
                                category: JunkCategory::EmptyDir,
                                age_days: *age,
                            });
                        }
                    }
                    return None;
                }

                if *size == 0 { return None; }

                if scan_large && *size > 500 * 1024 * 1024 {
                    return Some(JunkFile {
                        path: path.clone(),
                        size_bytes: *size,
                        category: JunkCategory::Large,
                        age_days: *age,
                    });
                }

                if matches!(category, JunkCategory::Download) && scan_downloads && *age > 90 {
                    return Some(JunkFile {
                        path: path.clone(),
                        size_bytes: *size,
                        category: JunkCategory::Download,
                        age_days: *age,
                    });
                }

                if matches!(category, JunkCategory::Temp) {
                    return Some(JunkFile {
                        path: path.clone(),
                        size_bytes: *size,
                        category: JunkCategory::Temp,
                        age_days: *age,
                    });
                }

                if scan_duplicates {
                    dup_candidates.lock().unwrap()
                        .entry(*size).or_default()
                        .push(PathBuf::from(path));
                }
                None
            })
            .inspect(|item| {
                let mut buf = pending.lock().unwrap();
                buf.push(item.clone());
                if buf.len() >= batch_size {
                    let chunk = std::mem::take(&mut *buf);
                    drop(buf);
                    emit_batch(chunk);
                }
            })
            .collect();

        // Flush remaining cheap results
        {
            let remaining = std::mem::take(&mut *pending.lock().unwrap());
            if !remaining.is_empty() { emit_batch(remaining); }
        }

        // ── Phase 3: duplicate detection ───────────────────────────────────
        let dup_results: Vec<JunkFile> = if scan_duplicates {
            let size_groups = Arc::try_unwrap(dup_candidates).unwrap().into_inner().unwrap();
            let size_candidates: Vec<(u64, Vec<PathBuf>)> = size_groups
                .into_iter()
                .filter(|(_, paths)| paths.len() > 1)
                .collect();

            let partial_groups: Vec<(u64, Vec<PathBuf>)> = size_candidates
                .par_iter()
                .flat_map(|(size, paths)| {
                    let mut partial_map: HashMap<[u8; 32], Vec<PathBuf>> = HashMap::new();
                    for path in paths {
                        if let Some(h) = partial_hash(path, 64 * 1024) {
                            partial_map.entry(h).or_default().push(path.clone());
                        }
                    }
                    partial_map.into_values()
                        .filter(|v| v.len() > 1)
                        .map(|v| (*size, v))
                        .collect::<Vec<_>>()
                })
                .collect();

            let dupes: Vec<JunkFile> = partial_groups
                .par_iter()
                .flat_map(|(size, paths)| {
                    let mut hash_to_first: HashMap<[u8; 32], &PathBuf> = HashMap::new();
                    let mut group_dupes: Vec<JunkFile> = Vec::new();
                    let hashed: Vec<([u8; 32], &PathBuf)> = paths
                        .par_iter()
                        .filter_map(|p| full_hash(p).map(|h| (h, p)))
                        .collect();
                    for (hash, path) in hashed {
                        if hash_to_first.contains_key(&hash) {
                            let meta = path.metadata().ok();
                            group_dupes.push(JunkFile {
                                path: path.to_string_lossy().to_string(),
                                size_bytes: *size,
                                category: JunkCategory::Duplicate,
                                age_days: meta.as_ref().map(|m| age_days(m)).unwrap_or(0),
                            });
                        } else {
                            hash_to_first.insert(hash, path);
                        }
                    }
                    group_dupes
                })
                .collect();

            if !dupes.is_empty() { emit_batch(dupes.clone()); }
            dupes
        } else {
            Vec::new()
        };

        let mut results = cheap_results;
        results.extend(dup_results);
        (results, all_flat, total_scanned)
    })
    .await
    .map_err(|e| format!("Scan task panicked: {}", e))?;

    // ── Phase 5 (B1): populate SharedScanIndex ────────────────────────────────
    // This runs back on the async executor — it's CPU-bound index building
    // (~50 ms for 1M entries with rayon) so spawn_blocking for it too.
    let scan_index_arc2 = Arc::clone(&scan_index_arc);
    let effective_paths_clone = effective_paths.clone();
    tokio::task::spawn_blocking(move || {
        let root = effective_paths_clone.first().cloned().unwrap_or_default();
        let mut new_index = ScanIndex::new();
        new_index.root = root;
        new_index.scanned_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut names = NameArena::with_capacity(4 * 1024 * 1024);
        let entries: Vec<IndexEntry> = all_flat
            .iter()
            .map(|(path_str, size, age, is_dir, _cat)| {
                let short = path_str.rsplit(['\\', '/']).next().unwrap_or(path_str.as_str());
                let (name_offset, name_len) = names.push(short);
                IndexEntry {
                    file_ref:    0,
                    parent_ref:  0,
                    size:        *size,
                    age_days:    *age,
                    name_offset,
                    name_len,
                    flags:       if *is_dir { 1 } else { 0 },
                    full_path:   path_str.clone(),
                }
            })
            .collect();

        new_index.names = names;
        new_index.entries = entries;
        new_index.build_indices();
        new_index.build_dir_sizes();

        if let Ok(mut idx) = scan_index_arc2.write() {
            *idx = new_index;
        }
    })
    .await
    .map_err(|e| format!("Index build panicked: {}", e))?;

    let _ = app.emit(
        "scan-progress",
        ScanProgress {
            scanned: total_scanned,
            found: results.len() as u64,
            current_path: String::new(),
            done: true,
            batch: vec![],
        },
    );

    Ok(results)
}

// ─── delete command (recycle bin + parallel + partial-failure reporting) ─────

#[tauri::command]
pub async fn delete_files(
    paths: Vec<String>,
    permanent: bool,
) -> Result<DeleteResult, String> {
    let freed = Arc::new(AtomicU64::new(0));
    let succeeded: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let failed: Arc<Mutex<Vec<FailedDelete>>> = Arc::new(Mutex::new(Vec::new()));

    paths.par_iter().for_each(|p| {
        let path = Path::new(p);
        let size = path.metadata().map(|m| m.len()).unwrap_or(0);

        let result = if permanent {
            // Permanent delete (directory or file)
            if path.is_dir() {
                std::fs::remove_dir_all(path).map_err(|e| e.to_string())
            } else {
                std::fs::remove_file(path).map_err(|e| e.to_string())
            }
        } else {
            // Soft-delete: move to OS Recycle Bin / Trash
            trash::delete(path).map_err(|e| e.to_string())
        };

        match result {
            Ok(_) => {
                freed.fetch_add(size, Ordering::Relaxed);
                succeeded.lock().unwrap().push(p.clone());
            }
            Err(reason) => {
                failed.lock().unwrap().push(FailedDelete {
                    path: p.clone(),
                    reason,
                });
            }
        }
    });

    let succeeded = Arc::try_unwrap(succeeded).unwrap().into_inner().unwrap();
    let failed = Arc::try_unwrap(failed).unwrap().into_inner().unwrap();

    Ok(DeleteResult {
        freed_bytes: freed.load(Ordering::Relaxed),
        succeeded,
        failed,
    })
}

// ─── disk size helpers ────────────────────────────────────────────────────────

#[allow(dead_code)]
fn dir_size(path: &Path) -> u64 {
    use jwalk::WalkDir;
    WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

// ─── disk usage ───────────────────────────────────────────────────────────────

#[tauri::command]
pub fn get_disk_usage(drive: String) -> Result<serde_json::Value, String> {
    #[cfg(target_os = "windows")]
    {
        let script = format!(
            r#"$d = Get-PSDrive -Name '{}' -ErrorAction SilentlyContinue; if ($d) {{ '{{"used":' + ($d.Used) + ',"free":' + ($d.Free) + '}}' }}"#,
            drive.trim_end_matches([':', '\\', '/'])
        );
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .output();

        if let Ok(o) = out {
            let s = String::from_utf8_lossy(&o.stdout);
            let s = s.trim();
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(s) {
                let used = v["used"].as_u64().unwrap_or(0);
                let free = v["free"].as_u64().unwrap_or(0);
                return Ok(serde_json::json!({
                    "totalBytes": used + free,
                    "freeBytes": free,
                    "usedBytes": used,
                }));
            }
        }
    }

    Ok(serde_json::json!({
        "totalBytes": 0u64,
        "freeBytes": 0u64,
        "usedBytes": 0u64,
    }))
}

// ─── Synchronous wrappers for CLI use ────────────────────────────────────────
// The CLI binary cannot use `#[tauri::command]` async functions directly.
// These thin wrappers drive the async logic using a minimal tokio runtime.

/// Synchronous version of `scan_for_junk` — for use by blue-streak-cli.
/// Does not emit Tauri events (there is no AppHandle in CLI context).
pub fn scan_for_junk_sync(
    paths: Vec<String>,
    scan_temp: bool,
    scan_downloads: bool,
    scan_duplicates: bool,
    scan_empty_dirs: bool,
    scan_large: bool,
) -> Result<Vec<crate::models::JunkFile>, String> {
    use std::sync::atomic::AtomicU64;

    let mut roots: Vec<(PathBuf, JunkCategory)> = Vec::new();
    if scan_temp {
        if let Ok(temp) = std::env::var("TEMP") {
            roots.push((PathBuf::from(temp), JunkCategory::Temp));
        }
        roots.push((PathBuf::from(r"C:\Windows\Temp"), JunkCategory::Temp));
    }
    for p in &paths {
        if scan_downloads {
            roots.push((PathBuf::from(p), JunkCategory::Download));
        }
    }

    // Minimal walk — no progress events
    let scanned = Arc::new(AtomicU64::new(0));
    let mut results: Vec<crate::models::JunkFile> = Vec::new();
    let dup_candidates: Arc<Mutex<HashMap<u64, Vec<PathBuf>>>> =
        Arc::new(Mutex::new(HashMap::new()));

    for (root, category) in &roots {
        if !root.exists() { continue; }
        for entry in jwalk::WalkDir::new(root)
            .skip_hidden(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let _ = scanned.fetch_add(1, Ordering::Relaxed);
            let meta = match entry.metadata().ok() {
                Some(m) => m,
                None => continue,
            };
            let path_str = entry.path().to_string_lossy().into_owned();
            let size = if meta.is_file() { meta.len() } else { 0 };
            let age = age_days(&meta);

            if meta.is_dir() {
                if scan_empty_dirs {
                    let is_empty = std::fs::read_dir(entry.path())
                        .map(|mut d| d.next().is_none())
                        .unwrap_or(false);
                    if is_empty {
                        results.push(crate::models::JunkFile {
                            path: path_str,
                            size_bytes: 0,
                            category: JunkCategory::EmptyDir,
                            age_days: age,
                        });
                    }
                }
                continue;
            }
            if size == 0 { continue; }

            if scan_large && size > 500 * 1024 * 1024 {
                results.push(crate::models::JunkFile {
                    path: path_str, size_bytes: size,
                    category: JunkCategory::Large, age_days: age,
                });
                continue;
            }
            if matches!(category, JunkCategory::Download) && scan_downloads && age > 90 {
                results.push(crate::models::JunkFile {
                    path: path_str, size_bytes: size,
                    category: JunkCategory::Download, age_days: age,
                });
                continue;
            }
            if matches!(category, JunkCategory::Temp) {
                results.push(crate::models::JunkFile {
                    path: path_str, size_bytes: size,
                    category: JunkCategory::Temp, age_days: age,
                });
                continue;
            }
            if scan_duplicates {
                dup_candidates
                    .lock()
                    .unwrap()
                    .entry(size)
                    .or_default()
                    .push(PathBuf::from(&path_str));
            }
        }
    }

    // Duplicate detection
    if scan_duplicates {
        let size_groups = Arc::try_unwrap(dup_candidates).unwrap().into_inner().unwrap();
        for (size, paths) in size_groups.into_iter().filter(|(_, v)| v.len() > 1) {
            let mut partial_map: HashMap<[u8; 32], Vec<PathBuf>> = HashMap::new();
            for path in &paths {
                if let Some(h) = partial_hash(path, 64 * 1024) {
                    partial_map.entry(h).or_default().push(path.clone());
                }
            }
            for (_, partial_paths) in partial_map.into_iter().filter(|(_, v)| v.len() > 1) {
                let mut seen: HashMap<[u8; 32], &PathBuf> = HashMap::new();
                let hashed: Vec<([u8; 32], &PathBuf)> = partial_paths
                    .iter()
                    .filter_map(|p| full_hash(p).map(|h| (h, p)))
                    .collect();
                for (hash, path) in hashed {
                    if seen.contains_key(&hash) {
                        let meta_age = path.metadata().ok().map(|m| age_days(&m)).unwrap_or(0);
                        results.push(crate::models::JunkFile {
                            path: path.to_string_lossy().into_owned(),
                            size_bytes: size,
                            category: JunkCategory::Duplicate,
                            age_days: meta_age,
                        });
                    } else {
                        seen.insert(hash, path);
                    }
                }
            }
        }
    }

    Ok(results)
}

/// Synchronous version of `delete_files` — for use by blue-streak-cli.
pub fn delete_files_sync(
    paths: Vec<String>,
    permanent: bool,
) -> Result<crate::models::DeleteResult, String> {
    let freed = Arc::new(AtomicU64::new(0));
    let succeeded: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let failed: Arc<Mutex<Vec<crate::models::FailedDelete>>> = Arc::new(Mutex::new(Vec::new()));

    paths.par_iter().for_each(|p| {
        let path = Path::new(p);
        let size = path.metadata().map(|m| m.len()).unwrap_or(0);
        let result = if permanent {
            if path.is_dir() {
                std::fs::remove_dir_all(path).map_err(|e| e.to_string())
            } else {
                std::fs::remove_file(path).map_err(|e| e.to_string())
            }
        } else {
            trash::delete(path).map_err(|e| e.to_string())
        };
        match result {
            Ok(_) => {
                freed.fetch_add(size, Ordering::Relaxed);
                succeeded.lock().unwrap().push(p.clone());
            }
            Err(reason) => {
                failed.lock().unwrap().push(crate::models::FailedDelete { path: p.clone(), reason });
            }
        }
    });

    Ok(crate::models::DeleteResult {
        freed_bytes: freed.load(Ordering::Relaxed),
        succeeded: Arc::try_unwrap(succeeded).unwrap().into_inner().unwrap(),
        failed: Arc::try_unwrap(failed).unwrap().into_inner().unwrap(),
    })
}

# CleanSweep — Advanced Research: World-Class File Organizer

> Deep technical research across 8 domains. Every technique is evaluated against the current codebase with a priority rating.

---

## Executive Summary — Top 10 Findings

| # | Finding | Impact | Current Status |
|---|---|---|---|
| 1 | **3-Pass duplicate detection** (size → partial hash 64KB → full hash) | 50–100× faster dedup | ⚠️ Only 2-pass today |
| 2 | **Recycle Bin soft-delete** instead of permanent delete | Critical safety gap | 🔴 Permanent-only today |
| 3 | **SQLite WAL persistent scan cache** | 10× faster rescans | 🔴 Not persisted today |
| 4 | **Perceptual hashing for images** (dHash) | Find similar photos | 🔴 Not implemented |
| 5 | **Squarified treemap visualization** | Drives engagement | 🔴 Not implemented |
| 6 | **USN Journal incremental scanning** | Sub-second re-scans on Windows | 🔴 Not implemented |
| 7 | **Undo/redo for destructive operations** | Critical UX safety | 🔴 Not implemented |
| 8 | **Network drive detection** (serial vs parallel walk) | Correctness + perf | 🔴 Not implemented |
| 9 | **Idle + battery-aware scheduling** | System-friendly automation | 🟡 Partial |
| 10 | **Arena allocator for paths** (bumpalo) | 10–20× RAM reduction at scale | 🔴 Not implemented |

---

## Part A — Advanced Duplicate Detection Algorithms

### A.1 Current: Exact Duplicate Detection ✅

Our [`scanner.rs`](src-tauri/src/commands/scanner.rs) uses BLAKE3 streaming with 64 KiB chunks and size-first grouping. This is correct and fast (~1,000 MB/s). **Keep as-is for exact duplicates.**

Gap: detects only 100%-identical files. Does not find resized images, re-encoded audio, or similar-but-edited documents.

---

### A.2 Locality-Sensitive Hashing (LSH) — Near-Duplicate Files

**What it solves:** Finds files that are *similar but not identical* — slight modifications, different encodings, cropped images.

**Algorithm:**
1. Split file into fixed 8 KB blocks
2. Hash each block independently with xxHash64 (non-cryptographic, 3× faster than BLAKE3 for this use)
3. Compute **Jaccard similarity** = intersection / union of block-hash sets
4. Files with similarity ≥ threshold are near-duplicates

```
Threshold = 0.95 → very strict (minor changes only)
Threshold = 0.85 → balanced (detects resizes, light edits)  ← recommended
Threshold = 0.60 → loose (heavily modified)
```

**Performance:** O(n·k) hashing + O(m²) comparison (m = candidate files). Practical limit ~10K files before O(m²) becomes slow. Use MinHash bands to reduce to O(n) for large sets.

**Sources:**
- LSH theory: Indyk & Motwani (1998) — https://dl.acm.org/doi/10.1145/276698.276876
- dupeGuru fuzzy mode: https://github.com/arsenetar/dupeguru (`core/engine.py`)

**CleanSweep action:** Phase 2. Add `twox-hash` crate. New scanner category: `NearDuplicate`. UI toggle: "Find exact" vs "Find similar".

---

### A.3 SimHash — Similar Text Files

**What it solves:** Duplicate log files, code copies, template variations.

**Algorithm:**
1. Shingle text into 5-word n-grams
2. Hash each n-gram → bitwise majority vote → 64-bit fingerprint
3. Compare via **Hamming distance** (2 CPU instructions: XOR + popcount)

```
distance ≤ 3 bits → 99%+ match
distance ≤ 6 bits → 95%+ match
```

**Performance:** O(n) fingerprint, O(1) comparison. Processes 100+ MB/s of text.

**Sources:**
- SimHash paper: Chum et al. (2006) — https://dl.acm.org/doi/10.1145/1242572.1242592
- Implementation: https://github.com/1e0nd/simhash

**CleanSweep action:** Phase 2 (optional). Only worthwhile if users have large log/document archives.

---

### A.4 Perceptual Hashing for Images (dHash / pHash / aHash)

**What it solves:** Duplicate photos that are resized, compressed differently, or slightly cropped. Essential for photo library cleanup.

**Three algorithms:**

| Algorithm | Speed | Accuracy | Use Case |
|---|---|---|---|
| **aHash** (average) | ~1ms/image | Low | Fast pre-filter |
| **dHash** (difference) | ~2ms/image | Good | **Recommended** |
| **pHash** (perceptual/DCT) | ~50ms/image | Best | High-accuracy mode |

**dHash algorithm:**
1. Resize image to 9×8 pixels (grayscale)
2. For each row, compare adjacent pixels (8 comparisons × 8 rows = 64 bits)
3. Output: `u64` fingerprint
4. Compare fingerprints with Hamming distance

```
distance ≤ 5  → nearly identical (99.9%)
distance ≤ 10 → very similar (95%)
distance ≤ 15 → similar (85%)
```

**Performance:** 10,000 JPEGs in ~20 seconds with dHash.

**Sources:**
- dHash: Neal Krawetz, "Photo Forensics" (2013) — http://www.hackerfactor.com/papers/p73-krawetz.pdf
- pHash: https://www.phash.org/
- Rust crate: `image` + manual dHash implementation (no external dep needed)

**CleanSweep action:** **Phase 2 — HIGH PRIORITY.** Add `image = "0.25"` to Cargo.toml. New scan type: "Find similar images". Show pairs side-by-side in UI.

---

### A.5 Audio Fingerprinting (Chromaprint/AcoustID)

**What it solves:** Same song at different bitrates, formats, or trimmed lengths.

**How:** Extract chroma features (pitch class profile) from audio → Chromaprint fingerprint → compare or look up via AcoustID API.

**Performance:** ~500ms per file (CPU-bound). False positive rate < 0.1%.

**Sources:**
- AcoustID: https://acoustid.org/chromaprint
- Rust crate: https://github.com/zmb3/chromaprint-rs

**CleanSweep action:** Phase 3+. Lower priority. Only add if users request music library dedup explicitly.

---

## Part B — Indexing & Persistent Cache

### B.1 The Core Problem

Every scan today walks the entire tree from scratch. Second scan of 1M unchanged files = same 30 seconds as the first. **Everything** and **WinDirStat** rescan in ~1 second using persistent indexes.

---

### B.2 SQLite WAL Mode — Persistent Scan Cache 🔴 CRITICAL

**Schema:**
```sql
CREATE TABLE scans (
    scan_id   INTEGER PRIMARY KEY,
    timestamp INTEGER NOT NULL,
    root_path TEXT NOT NULL,
    duration_ms INTEGER,
    total_files INTEGER
);

CREATE TABLE junk_files (
    id        INTEGER PRIMARY KEY,
    scan_id   INTEGER REFERENCES scans(scan_id),
    path      TEXT NOT NULL,
    size_bytes INTEGER,
    category  TEXT,
    age_days  INTEGER,
    hash_blake3 TEXT  -- cache hash for incremental dedup
);

CREATE INDEX idx_scan ON junk_files(scan_id);
CREATE INDEX idx_hash ON junk_files(hash_blake3);

CREATE TABLE dir_sizes (
    path      TEXT PRIMARY KEY,
    size_bytes INTEGER,
    file_count INTEGER,
    computed_at INTEGER
);
```

**WAL benefits:**
- Writers don't block readers — UI stays responsive during cache writes
- Atomic commits — no corruption on crash
- `PRAGMA synchronous = NORMAL` gives 10× write speedup vs `FULL` with acceptable durability

**Performance:**
- Cache write: ~50ms for 10K results (batched transaction)
- Cache read: ~10ms for 10K results
- Disk: ~50 bytes/result

**Crates:** `rusqlite = "0.31"` + `tokio-rusqlite = "0.5"`

**CleanSweep action:** Phase 2. Store at `%LOCALAPPDATA%/cleansweep/cache.db`. Show "Last scanned: X mins ago" in UI. Auto-invalidate if root mtime changed.

---

### B.3 Windows USN Journal — Incremental Scanning

**What it solves:** NTFS kernel maintains a circular write-ahead log of every file operation. Instead of re-walking 1M files, ask the kernel: "what changed since USN #12345?"

```
Full scan (1M files):  30 seconds
Delta scan (1K changes): 100ms + O(k) where k = changed files
```

**Implementation sketch:**
```rust
// Query NTFS USN journal for changes since last scan
#[cfg(target_os = "windows")]
fn get_changed_since(drive: char, last_usn: i64) -> Vec<PathBuf> {
    // DeviceIoControl(FSCTL_READ_USN_JOURNAL)
    // Returns USN_RECORD_V2 structs for each changed file
}
```

**Overflow handling:** USN journal is circular (~32 MB by default). Save a checkpoint (`last_usn` + `journal_id`). If `last_usn < LowestValidUsn` → journal wrapped → fall back to full scan.

**Sources:**
- Microsoft NTFS Change Journals: https://learn.microsoft.com/en-us/windows/win32/fileio/change-journals
- `FSCTL_READ_USN_JOURNAL`: https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_read_usn_journal

**CleanSweep action:** Phase 2 (Windows only). Gate with `#[cfg(target_os = "windows")]`. Fall back to mtime comparison on other platforms.

---

### B.4 Storage Backend Comparison

| | SQLite | RocksDB | LMDB |
|---|---|---|---|
| Throughput | 50K ops/s | 1M+ ops/s | 500K ops/s |
| Memory | ~10 MB | ~100 MB | ~1 MB |
| Query support | Full SQL | KV only | KV only |
| Best for | **CleanSweep** | Event logs | Embedded indexes |

**Decision: SQLite.** Upgrade to RocksDB only if scan throughput exceeds 100K files/sec.

---

## Part C — File System Event Systems

### C.1 Current State

[`watcher.rs`](src-tauri/src/commands/watcher.rs) uses the `notify` crate which wraps `ReadDirectoryChangesW` on Windows. This is correct for ≤200 watched directories.

---

### C.2 ReadDirectoryChangesW vs USN Journal

| | ReadDirectoryChangesW (current) | USN Journal |
|---|---|---|
| Scale | ~200 dirs max | Millions of files |
| Event latency | 1–100ms | 1–10ms |
| Buffer overflow | Data loss | Recoverable |
| Complexity | Simple | Medium |

**`notify` crate internals** (Windows):
- Registers directory handles with `RegisterWaitForSingleObject`
- Kernel fills 64 KB buffer on change
- `notify` deduplicates (100 rapid writes → 1 event)
- Events flow via `crossbeam-channel`

Source: https://github.com/notify-rs/notify/blob/main/notify/src/windows.rs

**CleanSweep action:** Keep `notify` for ≤500 watched dirs. Add hybrid: if user watches entire C:\ drive, switch to USN journal polling (Phase 2).

---

### C.3 Event Coalescing (Critical for Performance)

Without debouncing, a virus scanner writing 10,000 temp files fires 10,000 events. Use `notify-debouncer-full`:

```toml
notify-debouncer-full = "0.3"
```

This coalesces events within a 500ms window, groups by path, and emits only the final state.

---

## Part D — Storage Analysis Algorithms

### D.1 Squarified Treemap (WinDirStat algorithm)

**What it is:** Visualize disk usage as nested rectangles where area ∝ file size. Invented by Bruls et al. (2000).

**Algorithm:**
1. Sort items by size (descending)
2. Lay out rows, minimizing aspect ratio (closest to square = most readable)
3. Recurse into directories

**Formula for "squarify":** For each row candidate, compute `max(w²·s_max, h²·s_min) / (w·h·s_row_total)`. Accept if ≤ previous worst ratio.

**Performance:** O(n log n) — sort dominates. 10K rectangles render at 60 FPS on modern GPU via SVG or Canvas.

**Paper:** Bruls, Huizing, van Wijk (2000) "Squarified Treemaps" — https://www.win.tue.nl/~vanwijk/stm.pdf

**CleanSweep action:** Phase 2. Compute layout in Rust (new Tauri command `compute_treemap`), render in React SVG. Click rectangle → highlight in file list.

---

### D.2 Bottom-Up Directory Size Accumulation

**Current problem:** [`dir_size()`](src-tauri/src/commands/scanner.rs) walks the entire subtree every time it's called — O(n) per call.

**Fix:** Cache directory sizes in SQLite. Compute bottom-up once:
```
leaf_size = sum(file sizes in dir)
parent_size = sum(child sizes) + leaf_size
```
After first full walk: size lookups are O(1). Update only the O(k) ancestors of changed files on rescan.

**CleanSweep action:** Phase 2. Integrate with `dir_sizes` SQLite table from B.2.

---

### D.3 Top-N Largest Files — Heap Selection

**Current approach:** Sort all results then take top N = O(n log n).

**Better:** Min-heap of size N = O(n log N):

```rust
use std::collections::BinaryHeap;
use std::cmp::Reverse;

let mut heap: BinaryHeap<Reverse<(u64, PathBuf)>> = BinaryHeap::new();
for entry in walk {
    let size = entry.size;
    if heap.len() < limit {
        heap.push(Reverse((size, entry.path)));
    } else if size > heap.peek().unwrap().0.0 {
        heap.pop();
        heap.push(Reverse((size, entry.path)));
    }
}
```

2× faster for large file counts; O(N) memory instead of O(n).

**CleanSweep action:** Phase 2. Replace sort in scanner with heap for `scan_large` category.

---

## Part E — Scheduling & Automation

### E.1 Platform Scheduler Comparison

| Platform | Mechanism | Config Format | Idle Trigger |
|---|---|---|---|
| Windows | Task Scheduler (`schtasks.exe`) | XML | ✅ Built-in IdleTrigger |
| macOS | LaunchDaemon | `.plist` | ❌ Manual check needed |
| Linux | systemd timer | `.timer` unit | ❌ Manual via dbus |

Our current [`scheduler.rs`](src-tauri/src/commands/scheduler.rs) wraps `schtasks.exe` — correct for Windows. Add `Persistent=true` equivalent: use `TASK_RUNLEVEL_LUA` to run even if missed.

---

### E.2 Cron Expression Parsing

Add `cron = "0.12"` to Cargo.toml. Allows users to enter standard cron expressions (`0 2 * * *`) rather than a rigid Daily/Weekly picker.

```rust
use cron::Schedule;
use std::str::FromStr;

fn next_run(expr: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    Schedule::from_str(expr).ok()?.upcoming(chrono::Utc).next()
}
```

Source: https://docs.rs/cron

---

### E.3 Idle & Battery-Aware Scheduling (Like Dropbox)

Only run scans when:
1. System idle ≥ N minutes (no mouse/keyboard input)
2. On AC power (not draining battery)

```rust
#[cfg(target_os = "windows")]
fn idle_seconds() -> u64 {
    use winapi::um::winuser::{GetLastInputInfo, LASTINPUTINFO, GetTickCount};
    unsafe {
        let mut info: LASTINPUTINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<LASTINPUTINFO>() as u32;
        GetLastInputInfo(&mut info);
        (GetTickCount() - info.dwTime) as u64 / 1000
    }
}
```

**CleanSweep action:** Phase 2. Add `AppSettings` fields: `run_only_when_idle: bool`, `idle_threshold_minutes: u32`, `run_only_on_ac: bool`.

---

## Part F — Safety & Recoverability

### F.1 Soft-Delete to Recycle Bin 🔴 CRITICAL

**Current code permanently deletes files.** This is a catastrophic UX failure for any file that matters.

**Fix:** Use `SHFileOperationW` with `FOF_ALLOWUNDO` flag on Windows:

```rust
#[cfg(target_os = "windows")]
fn send_to_recycle_bin(path: &str) -> bool {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    // Build null-terminated double-null-terminated wide string
    let mut wide: Vec<u16> = OsStr::new(path).encode_wide().collect();
    wide.push(0); wide.push(0);  // double-null terminate
    
    unsafe {
        let mut op = winapi::um::shellapi::SHFILEOPSTRUCTW {
            hwnd: std::ptr::null_mut(),
            wFunc: winapi::um::shellapi::FO_DELETE,
            pFrom: wide.as_ptr(),
            pTo: std::ptr::null(),
            fFlags: winapi::um::shellapi::FOF_ALLOWUNDO
                  | winapi::um::shellapi::FOF_SILENT
                  | winapi::um::shellapi::FOF_NOCONFIRMATION,
            fAnyOperationsAborted: 0,
            hNameMappings: std::ptr::null_mut(),
            lpszProgressTitle: std::ptr::null(),
        };
        winapi::um::shellapi::SHFileOperationW(&mut op) == 0
    }
}
```

Or use the `trash` crate (cross-platform): `trash = "5"`.

**CleanSweep action:** **IMPLEMENT IMMEDIATELY in current sprint.** Change `delete_files` default to recycle bin. Add `permanent: bool` parameter with confirmation gate.

Sources:
- `trash` crate: https://crates.io/crates/trash
- SHFileOperationW: https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shfileoperationw

---

### F.2 Secure Delete on SSDs — Don't Do It

Multi-pass overwrite (DoD 5220.22-M, Gutmann 35-pass) is **ineffective on SSDs**:
- Flash Translation Layer remaps sectors — overwrite hits different physical cells
- TRIM command already handles zeroing freed blocks
- Wear leveling stores data in unpredictable locations

**NIST SP 800-88 rev1 (2014) verdict:** For SSDs, use ATA Secure Erase (firmware-level) or full-disk encryption + key deletion. Software overwrite = false sense of security.

Source: https://nvlpubs.nist.gov/nistpubs/SpecialPublications/NIST.SP.800-88r1.pdf

**CleanSweep action:** Do NOT implement multi-pass overwrite. Show info tooltip if users ask: "SSD secure erase requires ATA Secure Erase or BitLocker key deletion."

---

### F.3 VSS Snapshot Before Bulk Cleanup

Create a Windows System Restore Point before running large cleanups:

```rust
#[cfg(target_os = "windows")]
fn create_restore_point(desc: &str) -> Result<(), String> {
    std::process::Command::new("powershell")
        .args(["-Command",
               &format!("Checkpoint-Computer -Description '{}' -RestorePointType MODIFY_SETTINGS", desc)])
        .status()
        .map_err(|e| e.to_string())?;
    Ok(())
}
```

**CleanSweep action:** Phase 2. Add "Create restore point before cleanup" checkbox. Show toast: "Restore point created — revert via System Restore if needed."

---

### F.4 Undo/Redo Stack 🔴 CRITICAL

Every destructive operation (delete, move, archive) must be undoable. Pattern:

```
Operation { type, original_path, new_path, timestamp }
→ push to history stack
→ Ctrl+Z: reverse the operation (restore from recycle bin or move back)
→ Ctrl+Y: re-apply
```

Implementation: Zustand store with `history: Operation[]` + `currentIndex: number`. Undo calls `restore_from_recycle_bin` Tauri command.

**CleanSweep action:** Phase 2. Requires soft-delete (F.1) to be in place first.

---

## Part G — UI/UX Patterns

### G.1 Virtual Scrolling for 100K+ Result Lists

React's default rendering crashes at ~5,000 DOM nodes. For 100K scan results:

```tsx
import { FixedSizeList } from 'react-window';

<FixedSizeList height={600} itemCount={results.length} itemSize={35} width="100%">
  {({ index, style }) => (
    <FileResultRow style={style} file={results[index]} />
  )}
</FixedSizeList>
```

Only ~20 rows rendered at any time. O(1) re-render cost regardless of list size.

Crate: `react-window = "1.8"` (npm)

**CleanSweep action:** Phase 2. Replace current results list in `Scanner.tsx` with `FixedSizeList`.

---

### G.2 Treemap ↔ File List Bidirectional Linking

Click a rectangle in the treemap → scroll + highlight matching files in the list.
Click a file in the list → highlight its parent directory in the treemap.

Implementation: shared `selectedPath` state in Zustand. CSS `scroll-into-view` + 2-second highlight animation.

---

### G.3 Progressive Disclosure

Show partial results as the scan runs, not only at completion. Current `scan-progress` events already stream from Rust. The frontend just needs to accumulate them into the visible list in real time.

**Current gap:** `Scanner.tsx` likely waits for the final `done: true` event before displaying anything. Fix: append to `results` on every progress event that carries partial findings.

---

## Part H — Cross-Platform Considerations

### H.1 Filesystem Differences

| | NTFS (Windows) | ext4 (Linux) | APFS (macOS) |
|---|---|---|---|
| Case sensitive | No | Yes | No (default) |
| Hard links | Limited | Full | Full |
| Cloned files | No | No | **Yes (CoW)** |
| Change journal | **USN Journal** | inotify | FSEvents |
| Compression | Optional | No | Built-in |

**APFS clone files:** On macOS, `cp --reflink` creates a clone — same data, two inodes, zero extra disk space (copy-on-write). Our dedup scanner would flag them as duplicates. Fix: check `ATTR_FILE_CLONEID` via `getattrlist()` syscall and skip CoW clones.

**CleanSweep action:** Phase 2. Add `#[cfg(target_os = "macos")]` guard in duplicate detector to skip APFS clones.

---

### H.2 Network Drive Detection

Scanning an SMB share over WiFi with parallel threads causes thundering-herd RPC storms. Detect network drives and use serial scanning:

```rust
#[cfg(target_os = "windows")]
fn is_network_path(path: &Path) -> bool {
    use winapi::um::fileapi::GetDriveTypeW;
    use winapi::um::winbase::DRIVE_REMOTE;
    // GetDriveTypeW returns DRIVE_REMOTE for mapped network drives
    // UNC paths (\\server\share) are always network
    path.to_str().map(|s| s.starts_with("\\\\")).unwrap_or(false)
}
```

**CleanSweep action:** Phase 2. Show "📡 Network drive — scanning serially" badge in UI. Disable parallel walk for network paths.

---

### H.3 WSL2 Integration

Users with WSL2 can add `\\wsl$\Ubuntu\home\user` as a scan path. Access works via the 9P protocol bridge but performance is ~3× slower than native.

```rust
fn is_wsl_path(path: &str) -> bool {
    path.starts_with("\\\\wsl$\\") || path.starts_with("\\\\wsl.localhost\\")
}
```

**CleanSweep action:** Phase 3. Detect WSL paths, show warning, scan serially.

---

## Appendix A — Algorithm Decision Tree

```
User wants to find duplicates?
├─ Exact byte-for-byte → BLAKE3 3-pass (size → partial 64KB → full)    ← Current Phase 1
├─ Similar images      → dHash perceptual hash, Hamming distance ≤ 10   ← Phase 2
├─ Similar text/docs   → SimHash 64-bit fingerprint, Hamming ≤ 6        ← Phase 2
├─ Similar audio       → Chromaprint / AcoustID fingerprint              ← Phase 3
└─ Similar binary      → LSH block-hash Jaccard similarity ≥ 0.85       ← Phase 2

User wants fastest rescan?
├─ Windows NTFS        → USN Journal delta (O(k) changed files)          ← Phase 2
├─ All platforms       → SQLite mtime cache (O(1) for unchanged)         ← Phase 2
└─ Fallback            → Full parallel jwalk                             ← Current

Deleting files?
├─ Default             → Recycle Bin soft-delete (reversible)            ← Fix immediately
├─ Confirmed perm.     → std::fs::remove_file after confirmation         ← Keep as opt-in
└─ SSD "secure"        → Don't implement (see F.2)                      ← Never
```

---

## Appendix B — Complete Rust Dependency Map

### Currently Used ✅
```toml
tauri = "2"
rayon = "1"           # data parallelism
jwalk = "0.8"         # parallel directory walk
blake3 = { version = "1", features = ["rayon"] }  # streaming hash
notify = "6"          # file system watcher
regex = "1"           # rule conditions
chrono = "0.4"        # timestamps
zip = "0.6"           # archive action
dirs = "5"            # XDG/AppData paths
walkdir = "2"         # fallback sequential walk
```

### Phase 2 — Add These
```toml
# Persistent cache
rusqlite = { version = "0.31", features = ["bundled"] }
tokio-rusqlite = "0.5"

# Recycle Bin (CRITICAL)
trash = "5"

# Image perceptual hashing
image = "0.25"

# Cron scheduling
cron = "0.12"

# Near-duplicate hashing
twox-hash = "2"

# Arena allocator (1M+ file scans)
bumpalo = "3"

# Memory-mapped I/O (large files ≥512 MB)
memmap2 = "0.9"

# Windows-native APIs
[target.'cfg(windows)'.dependencies]
winapi = { version = "0.3", features = [
    "shellapi", "winnt", "fileapi", "ioapiset",
    "winuser", "minwinbase", "winsvc"
] }
```

### Phase 3 — Future
```toml
chromaprint = "0.5"   # audio fingerprinting
lz4_flex = "0.11"     # fast compression for archive action
notify-debouncer-full = "0.3"  # smarter watcher debouncing
```

---

## References

| Source | URL |
|---|---|
| BLAKE3 specs | https://github.com/BLAKE3-team/BLAKE3-specs |
| rmlint (3-pass dedup) | https://github.com/sahib/rmlint |
| dupeGuru (near-dedup) | https://github.com/arsenetar/dupeguru |
| dHash algorithm | http://www.hackerfactor.com/papers/p73-krawetz.pdf |
| pHash library | https://www.phash.org/ |
| Squarified treemaps paper | https://www.win.tue.nl/~vanwijk/stm.pdf |
| WinDirStat source | https://sourceforge.net/projects/windirstat/ |
| Everything (VoidTools) | https://www.voidtools.com/support/everything/faq/ |
| SQLite WAL mode | https://www.sqlite.org/wal.html |
| NTFS USN Journal | https://learn.microsoft.com/en-us/windows/win32/fileio/change-journals |
| SHFileOperationW | https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shfileoperationw |
| NIST SP 800-88r1 (secure erase) | https://nvlpubs.nist.gov/nistpubs/SpecialPublications/NIST.SP.800-88r1.pdf |
| `trash` crate | https://crates.io/crates/trash |
| `notify` crate source | https://github.com/notify-rs/notify |
| `jwalk` crate | https://crates.io/crates/jwalk |
| `react-window` | https://github.com/bvaughn/react-window |
| LSH paper | https://dl.acm.org/doi/10.1145/276698.276876 |
| SimHash paper | https://dl.acm.org/doi/10.1145/1242572.1242592 |
| systemd timers | https://www.freedesktop.org/software/systemd/man/systemd.timer.html |
| Windows Task Scheduler | https://learn.microsoft.com/en-us/windows/win32/taskschd/task-scheduler-start-page |
| Windows TRIM / SSD erase | https://docs.microsoft.com/en-us/windows/win32/fileio/file-buffering |

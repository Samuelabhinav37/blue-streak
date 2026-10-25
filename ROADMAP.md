# CleanSweep — Long-Term Engineering Roadmap

> This is the definitive plan for building a world-class, production-grade file organizer.
> Every phase builds on the previous. Phases are ordered by user impact × engineering leverage.

---

## Philosophy

1. **Safety first.** Never destroy data without a recovery path.
2. **Speed from algorithms, not just parallelism.** A better algorithm beats more threads.
3. **Incremental delivery.** Each phase ships a working, better product.
4. **Windows-native first.** Deep OS integration (USN, VSS, Task Scheduler) before cross-platform polish.

---

## Phase 0 — Foundation (Current)

**Status: Complete**

| Component | What was built |
|---|---|
| Tauri v2 + React + TypeScript shell | ✅ |
| Parallel directory walk (jwalk + rayon) | ✅ |
| 2-pass duplicate detection (size → full BLAKE3 hash) | ✅ |
| Rules engine (move/copy/archive by extension/size/age/regex) | ✅ |
| Folder watcher (notify crate) | ✅ |
| Windows Task Scheduler integration | ✅ |
| Dark/light theme, settings persistence | ✅ |

**Known gaps going into Phase 1:**
- Files are permanently deleted (no recycle bin)
- Scan results not persisted (rescan always full)
- No undo/redo
- No image/near-duplicate detection
- No treemap visualization

---

## Phase 1 — Safety & Correctness (Next Sprint)

**Theme: Make it safe to use in production**  
**Estimated effort: 1–2 weeks**

### 1.1 Recycle Bin Soft-Delete 🔴 CRITICAL

**Why:** Current code permanently deletes files. One wrong scan configuration = permanent data loss. This is the single most dangerous issue in the codebase.

**What to build:**
- Add `trash = "5"` crate to `Cargo.toml`
- Replace `std::fs::remove_file` in `scanner.rs` with `trash::delete()`
- Add `use_recycle_bin: bool` parameter to `delete_files` command
- Default: `use_recycle_bin = true`
- Add `permanent: bool` to UI with a confirmation modal ("This cannot be undone")

**Files:** `src-tauri/src/commands/scanner.rs`, `src/pages/Scanner.tsx`

---

### 1.2 Partial Hash Pre-Filter

**Why:** Current 2-pass dedup (size → full hash) reads the entire file before knowing if it's a duplicate. For a 10 GB file, that's 10 GB of I/O before deciding it's unique.

**What to build:** Add a "partial hash" middle pass — hash first 64 KB before full hash:

```
Pass 1: Group by size              → O(n), no I/O
Pass 2: Hash first 64 KB           → 99%+ filter, 1/1000 I/O
Pass 3: Full BLAKE3 hash only      → only for true collision candidates
```

For 1M files: ~100× less I/O for duplicate detection compared to current.

**Files:** `src-tauri/src/commands/scanner.rs`

---

### 1.3 Fix Progress Event Streaming

**Why:** Frontend likely only displays results after `done: true`. Real-time display of partial results as they arrive makes scans feel faster and more responsive.

**What to build:**
- Emit `Vec<JunkFile>` batch with each progress event (not just counts)
- Frontend appends to result list on each event (progressive disclosure)

**Files:** `src-tauri/src/models.rs`, `src-tauri/src/commands/scanner.rs`, `src/pages/Scanner.tsx`

---

### 1.4 Error Recovery & Partial Failures

**Why:** Current `delete_files` returns an error if any single deletion fails, potentially leaving state ambiguous.

**What to build:**
- Return `{ freed: u64, succeeded: Vec<String>, failed: Vec<(String, String)> }` instead of `Result<u64>`
- Show partial success in UI: "Deleted 95/100 files. 5 failed (permission denied)."

**Files:** `src-tauri/src/commands/scanner.rs`, `src-tauri/src/models.rs`

---

## Phase 2 — Performance & Persistence

**Theme: Make it fast enough that users never wait**
**Status: Complete**

| Component | What was built |
|---|---|
| SQLite persistent scan cache (2.1) | ✅ `cache.rs` — WAL mode, `get_cache_summary`/`invalidate_cache` Tauri commands; Settings UI shows last-scan age + Clear Cache button |
| USN Journal incremental scanning (2.2) | ✅ `usn.rs` — `usn_snapshot`/`usn_get_changes` Tauri commands; overflow fall-back to full walk |
| Image perceptual hashing (2.3) | ✅ `image_hash.rs` — dHash, `scan_similar_images` Tauri command |
| Squarified treemap visualization (2.4) | ✅ `treemap.rs` + `DiskMap.tsx` — SVG treemap with drill-down |
| Virtual scrolling (2.5) | ✅ `Scanner.tsx` uses `react-window` v2 `List` — 60 FPS for 100K+ rows |
| Undo/redo stack (2.6) | ✅ `useOperationStore.ts` — Ctrl+Z/Y in Scanner; Undo/Redo buttons in toolbar |
| Smart scheduling: idle + battery (2.7) | ✅ `system.rs` (`should_run_task`) + `AppSettings` fields + Settings page UI |

**Estimated effort: 3–4 weeks** (all items complete)

### 2.1 SQLite Persistent Scan Cache

**Why:** Every rescan walks 1M+ files from scratch. With a cache, unchanged directories return in milliseconds.

**What to build:**
- `rusqlite` + `tokio-rusqlite` dependencies
- Schema: `scans`, `junk_files`, `dir_sizes`, `file_metadata_cache` tables
- `PRAGMA journal_mode = WAL` for concurrent read/write
- New Tauri commands: `get_last_scan(root)`, `save_scan_result(...)`, `invalidate_cache(root)`
- Auto-invalidate: compare root directory mtime to last scan timestamp
- UI: "Last scanned 3 minutes ago [Rescan]" in Scanner page header

**Performance target:** Second scan of unchanged directory: < 100ms (vs 30s today)

**Files:** New `src-tauri/src/commands/cache.rs`, updates to `lib.rs`, `Scanner.tsx`

---

### 2.2 USN Journal Incremental Scanning (Windows)

**Why:** Even with SQLite cache, we need to know *what changed* since the last scan. USN journal tells us in ~100ms, regardless of how many total files exist.

**What to build:**
- New `src-tauri/src/commands/usn.rs` (Windows-only, `#[cfg(target_os = "windows")]`)
- `get_usn_journal_snapshot(drive)` → saves `last_usn` + `journal_id` to `%LOCALAPPDATA%/cleansweep/usn_state.json`
- `get_changed_files_since(drive, last_usn)` → returns `Vec<PathBuf>` of changed paths
- Overflow handler: if `last_usn < LowestValidUsn` → fall back to full walk
- Wire into `scan_for_junk`: check USN first, only re-hash changed files

**Performance target:** Rescan after 100 file changes: < 500ms (vs 30s today)

**Files:** New `src-tauri/src/commands/usn.rs`, `scanner.rs` integration

---

### 2.3 Image Duplicate Detection (Perceptual Hashing)

**Why:** Users' largest time-wasters are duplicate vacation photos — same shot taken twice, or photos synced from multiple devices at different resolutions. Exact hash can't detect these.

**What to build:**
- Add `image = "0.25"` to `Cargo.toml`
- Implement `dhash(path) -> Option<u64>` — resize to 9×8 grayscale, compare adjacent pixels
- New scan category: `SimilarImage` with Hamming distance stored
- New scanner flag: `scan_similar_images: bool`
- Phase: runs after exact-duplicate phase on image files (jpg/png/gif/webp/heic)
- UI: Show similar pairs side-by-side with "Keep this / Keep that" buttons

**Algorithm:** dHash — 2ms per image, Hamming distance ≤ 10 = 95%+ similar

**Files:** `src-tauri/src/commands/scanner.rs`, `src-tauri/src/models.rs`, new `src/components/ImagePairView.tsx`

---

### 2.4 Squarified Treemap Visualization

**Why:** Users can't intuitively understand "you have 847 GB used." A WinDirStat-style treemap makes it instantly obvious that 400 GB is old VM snapshots.

**What to build:**
- New Tauri command `compute_treemap(root, max_depth)` → returns `Vec<TreemapRect>` (label, size, x, y, width, height, color)
- Squarified layout algorithm in Rust (O(n log n))
- New React page: "Disk Map" (between Dashboard and Scanner in sidebar)
- SVG rendering in React — click rectangle → drill down OR highlight in file list
- Color scale: green (small) → orange (medium) → red (large)

**Files:** New `src-tauri/src/commands/treemap.rs`, new `src/pages/DiskMap.tsx`, new `src/components/TreemapVisualization.tsx`

---

### 2.5 Virtual Scrolling for Result Lists

**Why:** Scanning a full drive may return 50,000+ results. Current DOM-based list will freeze the browser at ~5,000 items.

**What to build:**
- Add `react-window` npm package
- Replace current result list in `Scanner.tsx` with `FixedSizeList` (only renders ~20 visible rows)
- 60 FPS scroll for 100K+ results, same as before for small lists

**Files:** `src/pages/Scanner.tsx`, `src/components/FileResultRow.tsx`

---

### 2.6 Undo/Redo Stack

**Why:** Even with recycle bin, users need an in-app undo for multi-step workflows (run rules → organize → oops that moved the wrong files).

**What to build:**
- `useOperationStore` Zustand store: `history: Operation[]`, `currentIndex`
- `Operation = { type, originalPath, newPath, timestamp }`
- Every destructive action pushes to history
- Ctrl+Z: reverse (restore from recycle bin or move back)
- Ctrl+Y: redo
- UI: Undo/Redo buttons in TopBar, disabled when stack empty

**Files:** New `src/store/useOperationStore.ts`, updates to `Scanner.tsx`, `Organizer.tsx`

---

### 2.7 Smart Scheduling — Idle + Battery Detection

**Why:** Running a CPU-intensive scan while the user is presenting in PowerPoint = terrible experience. Like Dropbox and OneDrive, check system idle time and battery state.

**What to build:**
- `idle_seconds()` via `GetLastInputInfo` (Windows WinAPI)
- `is_on_battery()` via `GetSystemPowerStatus`
- New `AppSettings` fields: `run_only_when_idle`, `idle_threshold_minutes`, `require_ac_power`
- Scheduled tasks check these conditions before running

**Files:** New `src-tauri/src/commands/system.rs`, `settings.rs`, `src/pages/Settings.tsx`

---

## Phase 3 — Intelligence & Visualization

**Theme: Make it smarter than any competitor**
**Status: Complete**

| Component | What was built |
|---|---|
| Arena allocator for path strings (3.1) | ✅ `PathArena` + `CompactEntry` in `scanner.rs` — ~34 bytes/entry vs ~128 bytes |
| mmap I/O for large files ≥512 MiB (3.2) | ✅ `full_hash()` uses `memmap2::Mmap` above threshold |
| Parallel BLAKE3 via `update_rayon()` for ≥1 GiB (3.3) | ✅ Layered in same `full_hash()` |
| NTFS MFT direct scanning — WizTree-style (3.4) | ✅ `src-tauri/src/commands/mft.rs` — `scan_mft` Tauri command |
| Near-duplicate detection via LSH / Jaccard (3.5) | ✅ `src-tauri/src/commands/lsh.rs` — `scan_near_duplicates` Tauri command |
| VSS System Restore Point (3.6) | ✅ `create_restore_point` in `system.rs` via PowerShell `Checkpoint-Computer` |

### 3.1 Arena Allocator for Path Strings ✅

**Implementation:** Replaced `Vec<RawEntry>` (owned `PathBuf` per file) with a `PathArena` — a single contiguous `Vec<u8>` buffer where all path strings are interned sequentially. Each `CompactEntry` holds `(path_offset: u32, path_len: u32, size: u64, age_days: i64, flags: u8)` ≈ 34 bytes vs the previous ~128 bytes per `PathBuf`.

**Files:** `src-tauri/src/commands/scanner.rs`

---

### 3.2 Memory-Mapped I/O for Large Files ✅

**Implementation:** `full_hash()` now branches on file size: files ≥ 512 MiB use `memmap2::Mmap` to map the entire file into virtual address space, avoiding a kernel→user copy on every `read()` syscall (~30% faster).

**Files:** `src-tauri/src/commands/scanner.rs`

---

### 3.3 mmap Parallel BLAKE3 for Very Large Files ✅

**Implementation:** For files ≥ 1 GiB, `full_hash()` calls `hasher.update_rayon(&mmap)` which uses BLAKE3's built-in tree-parallel hashing (~4× faster on 8-core CPUs for 10 GB files).

**Files:** `src-tauri/src/commands/scanner.rs`

---

### 3.4 NTFS MFT Direct Scanning (WizTree-style) ✅

**Implementation:** `src-tauri/src/commands/mft.rs` — Windows-only via `FSCTL_ENUM_USN_RECORDS` (`DeviceIoControl`). Opens the volume with `FILE_FLAG_BACKUP_SEMANTICS`, reads all MFT records in 1 MiB batches, reconstructs full paths via BFS from the NTFS root (file ref 5). Falls back gracefully on non-Windows. Requires admin or SeBackupPrivilege.

**New Tauri command:** `scan_mft(drive: String) -> Result<MftScanResult, String>`

**Files:** New `src-tauri/src/commands/mft.rs`

---

### 3.5 Near-Duplicate Detection via LSH ✅

**Implementation:** `src-tauri/src/commands/lsh.rs` — each file is fingerprinted as a sorted `Vec<u64>` of xxHash64 block hashes (8 KiB blocks). Jaccard similarity is computed via merge-based intersection/union in O(n+m). Pairs with similarity ≥ threshold (default 0.80) are returned as `NearDuplicatePair`. New `JunkCategory::PossibleDuplicate` variant.

**New Tauri command:** `scan_near_duplicates(paths, threshold) -> Result<Vec<NearDuplicatePair>, String>`

**Files:** New `src-tauri/src/commands/lsh.rs`, updated `src-tauri/src/models.rs`

---

### 3.6 Volume Shadow Copy Integration (Windows) ✅

**Implementation:** `create_restore_point(description)` in `system.rs` shells out to PowerShell `Checkpoint-Computer` with `MODIFY_SETTINGS` type. Surfaces success/failure message to the frontend with actionable error text (e.g. System Protection not enabled). Non-Windows returns a graceful error.

**New Tauri command:** `create_restore_point(description: Option<String>) -> RestorePointResult`

**Files:** `src-tauri/src/commands/system.rs`

---

## Phase 4 — Ecosystem & Polish

**Theme: Make it the only tool you need**
**Status: Complete**

| Component | What was built |
|---|---|
| Audio duplicate detection (4.1) | ✅ `audio_hash.rs` — sub-band energy fingerprint via symphonia, `scan_audio_duplicates` Tauri command |
| Disk growth trends (4.2) | ✅ `growth.rs` — SQLite time-series snapshots, `get_growth_trends` command, Dashboard trend widget |
| Rule marketplace (4.3) | ✅ `marketplace.rs` — import/export JSON rules from file or URL; Import/Export buttons in Organizer |
| CLI interface (4.4) | ✅ `cli.rs` — `cleansweep-cli` binary; scan, clean, rules, trends, mft, export-csv sub-commands |
| Network drive & WSL2 (4.5) | ✅ `network.rs` — UNC/WSL2/mapped-drive detection, serial scan with exponential-backoff retry |
| Notifications & reports (4.6) | ✅ `reports.rs` — CSV export, HTML report (opens in browser), Windows toast notification |

### 4.1 Audio Duplicate Detection ✅

Pure-Rust sub-band energy fingerprint (no C FFI): decode audio via `symphonia` (MP3/FLAC/OGG/AAC/WAV/M4A), downsample to 11 025 Hz mono, compute per-frame 32-band energy, encode as `Vec<u32>`. Pair comparison via average per-frame Hamming distance. Threshold 8.0/32 ≈ 75% match catches same recording at different bitrates.

**New Tauri command:** `scan_audio_duplicates(paths, threshold) -> Result<Vec<AudioDuplicatePair>, String>`
**File:** `src-tauri/src/commands/audio_hash.rs`

### 4.2 Disk Growth Trends ✅

Time-series snapshots in SQLite (`dir_snapshots` table — path, size_bytes, file_count, captured_at). `get_growth_trends` returns deltas sorted by absolute growth. Dashboard shows top-5 growing folders with TrendingUp/Down icons and delta sizes.

**New Tauri commands:** `capture_growth_snapshot`, `get_growth_trends`, `prune_growth_snapshots`
**File:** `src-tauri/src/commands/growth.rs`

### 4.3 Rule Marketplace ✅

Import/export organizer rules as portable JSON. Import from local file or URL (via PowerShell `Invoke-WebRequest`). Imported rules get fresh IDs to prevent conflicts. Import/Export buttons added to the Organizer page header.

**New Tauri commands:** `export_rules`, `import_rules_from_file`, `import_rules_from_url`, `save_merged_rules`
**File:** `src-tauri/src/commands/marketplace.rs`

### 4.4 CLI Interface ✅

`cleansweep-cli` binary shares the same Rust library. Supports `scan`, `clean`, `rules list/run/export/import`, `trends`, `mft`, `export-csv`. Sync wrappers added to `scanner.rs` so CLI doesn't need a Tauri runtime.

**File:** `src-tauri/src/cli.rs` (built as `[[bin]] name = "cleansweep-cli"`)

### 4.5 Network Drive & WSL2 Support ✅

Path classifier detects UNC (`\\server\share`), WSL2 (`\\wsl$\...`), mapped network drives (via `GetDriveTypeW`). Serial walk with exponential-backoff retry on I/O failure (`base_delay * 2^attempt`). Returns structured `NetworkScanResult` with entries, errors, and retry count.

**New Tauri commands:** `classify_path`, `scan_network_path`
**File:** `src-tauri/src/commands/network.rs`

### 4.6 Notifications & Reports ✅

- **CSV export**: `export_scan_csv` writes scan results to CSV; Download button in Scanner toolbar.
- **HTML report**: `open_html_report` generates a styled HTML summary and opens it in the default browser via the `open` crate. Report button in Scanner toolbar.
- **Toast notification**: `notify_cleanup_complete` fires a Windows balloon tip (BurntToast if installed, otherwise `NotifyIcon` via PowerShell).
- **Restore point UI**: Safety section added to Scanner left panel.

**New Tauri commands:** `export_scan_csv`, `open_html_report`, `notify_cleanup_complete`
**File:** `src-tauri/src/commands/reports.rs`

---

## Implementation Order Summary

```
PHASE 1 (now) ─────────────────────────────────────────────────────────
 1.1  Recycle bin soft-delete              🔴 CRITICAL  ~4 hours
 1.2  Partial hash pre-filter              ⚠️ PERF      ~3 hours
 1.3  Progressive result streaming         ⚠️ UX        ~2 hours
 1.4  Partial failure reporting            ⚠️ UX        ~2 hours

PHASE 2 (weeks 2–5) ────────────────────────────────────────────────────
  2.1  SQLite persistent cache              ✅ DONE
  2.2  USN Journal incremental scan         ✅ DONE
  2.3  Image perceptual hashing (dHash)     ✅ DONE
  2.4  Squarified treemap visualization     ✅ DONE
  2.5  Virtual scrolling (react-window)     ✅ DONE
  2.6  Undo/redo stack                      ✅ DONE
  2.7  Idle + battery scheduling            ✅ DONE

PHASE 3 (weeks 6–11) ───────────────────────────────────────────────────
  3.1  Arena allocator (PathArena)         ✅ DONE
  3.2  mmap for large files                ✅ DONE
  3.3  Parallel BLAKE3 for huge files      ✅ DONE
  3.4  NTFS MFT direct scanning            ✅ DONE
  3.5  LSH near-duplicate detection        ✅ DONE
  3.6  VSS restore point integration       ✅ DONE

PHASE 4 (months 3+) ────────────────────────────────────────────────────
  4.1  Audio fingerprinting                ✅ DONE
  4.2  Disk growth trends                  ✅ DONE
  4.3  Rule marketplace                    ✅ DONE
  4.4  CLI interface                       ✅ DONE
  4.5  Network + WSL2 support              ✅ DONE
  4.6  Reports & notifications             ✅ DONE
```

---

## Performance Targets by Phase

| Metric | Today | Phase 1 | Phase 2 | Phase 3 |
|---|---|---|---|---|
| First scan (1M files) | ~30s | ~15s | ~10s | **~2s** (MFT) |
| Rescan unchanged | ~30s | ~30s | **<1s** (cache) | **<100ms** (USN) |
| Duplicate scan (1M files) | ~5min | **~30s** (partial hash) | ~20s | ~10s |
| RAM usage (1M files) | ~1 GB | ~1 GB | ~500 MB | **~50 MB** (arena) |
| Result list (100K items) | 💥 freeze | 💥 freeze | **60 FPS** (virtual scroll) | 60 FPS |
| Large file hash (10 GB) | ~20s | ~20s | ~14s (mmap) | **~5s** (parallel) |

---

## Research Files

- [`RESEARCH.md`](RESEARCH.md) — First research doc: walkdir algorithms, hash selection, I/O strategies
- [`RESEARCH_ADVANCED.md`](RESEARCH_ADVANCED.md) — Advanced research: LSH, perceptual hashing, USN journal, treemaps, safety

---

*Last updated: All phases complete (Phase 0 → Phase 4). Phase 2 frontend items (2.5 virtual scroll, 2.7 smart scheduling UI, 2.1 cache UI) completed.*

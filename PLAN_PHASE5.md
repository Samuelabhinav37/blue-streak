# Blue Streak — Phase 5 Master Plan
## "Blueprint First" Re-Architecture

> Synthesizes all research (RESEARCH_WIZTREE2.md, RESEARCH_PHASE5_ARCH.md,
> RESEARCH_PHASE5_VIZ.md) into a prioritised, executable build plan.
> Every item traces to a specific file + line range.

---

## The Root Problem (One Sentence)

We scan the filesystem correctly — but throw the data away after each scan,
so every feature (DiskMap, duplicates, treemap) re-walks the entire drive independently.

---

## What WizTree Does That We Don't

| WizTree | Blue Streak Today | Gap |
|---|---|---|
| Read $MFT **once** into RAM | ✅ MFT scan exists (`mft.rs`) | Data not shared |
| Keep flat array in memory | ❌ Array discarded after scan | No shared state |
| All views query that array | ❌ Each view re-scans | 3–5 independent walks |
| Store short names + parent refs | ❌ Full paths stored | 5–10× RAM waste |
| Path reconstruction via HashMap | ❌ LRU cache (random seeks) | Slow, unpredictable |
| No temp file copy | ❌ $MFT copied to disk then re-read | 3× I/O per scan |
| Dir sizes from flat array | ❌ `fs::read_dir` recursive walk | DiskMap as slow as full scan |
| Delta updates via USN | ✅ USN logic exists | Never applied to in-memory index |
| Snapshot to disk for instant reload | ❌ No snapshot | Full scan every app open |

---

## What We're Doing Wrong — 7 Specific Bugs

### Bug 1 — `mft.rs`: 3× I/O (Temp File Copy)
**Location:** `src-tauri/src/commands/mft.rs:91–116`
```rust
fn read_mft_to_temp(drive: &str) -> Result<PathBuf, String> {
    let mut src = open_mft(drive)?;   // I/O READ #1
    let tmp = temp_dir().join("Blue Streak_mft.bin");
    io::copy(&mut src, &mut dst)?;    // I/O WRITE #2 to disk
    Ok(tmp)
    // MftParser then reads tmp     // I/O READ #3
}
```
**Fix:** Read $MFT directly into `Vec<u8>` in RAM, wrap with `std::io::Cursor<Vec<u8>>`.
**Impact:** 2–3× faster MFT scan, no temp file left on disk.

---

### Bug 2 — `mft.rs`: LRU Cache Path Reconstruction (Random Seeks)
**Location:** `src-tauri/src/commands/mft.rs:240–255`
```rust
let path = parser.get_full_path_for_entry(&rd.entry)...;
// ↑ Uses 128-entry LRU — 99%+ miss rate for 1M entries
// Each miss = random seek into Cursor<Vec<u8>>
```
**Fix:** During Pass 1, build `HashMap<u64, u64>` (file_ref → parent_ref) and
`HashMap<u64, String>` (file_ref → short_name). Reconstruct paths via O(1) lookups.
**Impact:** 5–10× faster path reconstruction, predictable memory.

---

### Bug 3 — No Shared Scan State
**Location:** `src-tauri/src/lib.rs` (nothing registered for scan results)

After `scan_for_junk` returns `Vec<JunkFile>` to the frontend, the raw flat data
is gone. Every subsequent command (`compute_treemap`, `scan_near_duplicates`,
`scan_audio_duplicates`) has no data to query and re-walks from scratch.

**Fix:** Add `SharedScanIndex: Arc<RwLock<ScanIndex>>` to Tauri managed state.
After scan completes, store the full `Vec<FlatEntry>` there.
**Impact:** All views after first scan become instant queries (0 I/O).

---

### Bug 4 — `treemap.rs`: Complete Independent Filesystem Walk
**Location:** `src-tauri/src/commands/treemap.rs:208–299`
```rust
fn build_dir_tree_inner(path: &Path, depth: u32, max_depth: u32) -> DirEntry {
    if let Ok(entries) = std::fs::read_dir(path) {  // ← re-walks every time
```
And at max depth calls `quick_size(path)` which spawns `jwalk::WalkDir`.

**Fix:** Rewrite `compute_treemap` to query `SharedScanIndex`.
Aggregate directory sizes bottom-up from the flat entry array.
**Impact:** DiskMap goes from 30–60 s → **<100 ms** (300–600× speedup).

---

### Bug 5 — `FlatEntry` Has No `file_ref` or `parent_ref`
**Location:** `src-tauri/src/commands/scanner.rs:147–157`
```rust
struct CompactEntry {
    path_offset: u32,  // full reconstructed path
    path_len:    u32,
    size:        u64,
    age_days:    i64,
    flags:       u8,
}
```
No `file_ref` or `parent_ref` means there's no tree structure — can't derive
directory containment, can't build treemap, can't do bottom-up size aggregation.

**Fix:** Add `file_ref: u64` and `parent_ref: u64` to the entry struct.
Store SHORT name only (15 bytes avg) instead of full path (45+ bytes avg).
**Impact:** 5× RAM reduction, enables tree queries, enables treemap without re-scan.

---

### Bug 6 — USN Delta Not Applied to In-Memory Index
**Location:** `src-tauri/src/commands/usn.rs` (logic complete but unused)

`usn_get_changes` returns `Vec<UsnChange>` (list of changed paths). This is never
applied to update the in-memory scan index. The frontend receives the delta but
has no way to merge it.

**Fix:** Add `apply_usn_delta(index: &mut ScanIndex, changes: &[UsnChange])`.
Called after `usn_get_changes` returns; merges adds/deletes/modifications.
**Impact:** Incremental scans work end-to-end. Rescan after small changes: <500 ms.

---

### Bug 7 — No Snapshot Persistence
**Location:** `src-tauri/src/commands/cache.rs` (SQLite cache exists for metadata)

The SQLite cache stores scan metadata but not the raw `Vec<FlatEntry>` array.
Every app open requires a full scan from scratch (2–5 s with MFT, 30–60 s without).

**Fix:** Serialize `ScanIndex` to disk using `bincode` or `FlatBuffers` after scan.
On app startup, load snapshot + apply USN delta if available.
**Impact:** App startup with unchanged drive: **<200 ms** instead of 30–60 s.

---

## Phase 5 Build Plan — Ordered by Impact

### Milestone A — Foundation (1–2 days)
> Enables everything else. Must be done first.

**A1. Add `FlatEntry` with `file_ref` + `parent_ref` + short name**
- New struct in `models.rs` or `shared.rs`
- `file_ref: u64`, `parent_ref: u64`, `size: u64`, `name_offset: u32`, `name_len: u16`, `age_days: u16`, `flags: u8`
- Store short name in PathArena, NOT full path
- Files: new `src-tauri/src/models.rs` addition

**A2. Add `ScanIndex` to Tauri state**
- `pub struct ScanIndex { entries, names, by_parent, by_size, dir_sizes, root, scanned_at }`
- `pub type SharedScanIndex = Arc<RwLock<ScanIndex>>`
- Register in `lib.rs`: `.manage(Arc::new(RwLock::new(ScanIndex::default())))`
- Files: `src-tauri/src/lib.rs`, new `src-tauri/src/commands/shared.rs`

**A3. Fix `mft.rs`: Remove temp file, add HashMap path reconstruction**
- Replace `read_mft_to_temp()` with in-memory `Cursor<Vec<u8>>`
- Single pass: build `parent_map: HashMap<u64, u64>` + `name_map: HashMap<u64, String>`
- Path reconstruction: O(1) HashMap lookups, no LRU cache
- Files: `src-tauri/src/commands/mft.rs`

---

### Milestone B — Core Data Flow (1–2 days)
> After A: scan result flows to all views.

**B1. `scan_for_junk` populates `SharedScanIndex`**
- After MFT walk, store `Vec<FlatEntry>` into `SharedScanIndex` (RwLock write)
- Return `Vec<JunkFile>` to frontend as before (no API change)
- Files: `src-tauri/src/commands/scanner.rs`

**B2. `compute_treemap` queries `SharedScanIndex` instead of `fs::read_dir`**
- Check `SharedScanIndex`; if root matches, call `build_treemap_from_index()`
- `build_treemap_from_index`: filter entries by root prefix, aggregate dir sizes, squarify
- Remove `build_dir_tree()`, `build_dir_tree_inner()`, `quick_size()` from `treemap.rs`
- Files: `src-tauri/src/commands/treemap.rs`

**B3. Bottom-up directory size aggregation**
- `fn compute_dir_sizes(index: &ScanIndex) -> HashMap<u64, u64>`
- Walk entries, add file sizes to all ancestor dirs via parent_ref chain
- Called once after scan, stored in `ScanIndex::dir_sizes`
- Files: `src-tauri/src/commands/shared.rs` or `treemap.rs`

---

### Milestone C — Performance (1 day)
> Makes the fast path as fast as possible.

**C1. Background async scan**
- `scan_for_junk` spawns `tokio::spawn` so UI stays responsive
- Emits progress events as before
- Atomic swap into `SharedScanIndex` on complete
- Files: `src-tauri/src/commands/scanner.rs`

**C2. Parallel index build (rayon)**
- Build `by_parent`, `by_size`, `by_extension` using `rayon` fold+reduce
- Cuts index build from ~200 ms to ~50 ms for 1M entries
- Files: `src-tauri/src/commands/shared.rs`

**C3. Snapshot persistence (bincode)**
- After scan, serialize `ScanIndex` to `%LOCALAPPDATA%/Blue Streak/scan.bin`
- On startup: deserialize + apply USN delta
- Files: new `src-tauri/src/commands/snapshot.rs`

---

### Milestone D — USN Delta Integration (1 day)
> Makes rescans instant.

**D1. Apply USN changes to `ScanIndex`**
- `fn apply_usn_delta(index: &mut ScanIndex, changes: &[UsnChange])`
- Delete: remove entries matching changed paths
- Create/modify: stat file, build FlatEntry, insert
- Rebuild affected parts of `by_parent` + `dir_sizes`
- Files: `src-tauri/src/commands/usn.rs`

**D2. Wire into rescan flow**
- On "Rescan" click: check USN first; if delta available, merge; else full scan
- Files: `src-tauri/src/commands/scanner.rs`

---

### Milestone E — DiskMap UX (1–2 days)
> Makes the visualization useful.

**E1. Hover tooltips**
- `onMouseEnter` → show `{name, size, fileCount}` floating tooltip
- Files: `src/pages/DiskMap.tsx`

**E2. Clickable breadcrumb**
- Each crumb is a `<button>` that calls `jumpToLevel(index)`
- Files: `src/pages/DiskMap.tsx`

**E3. File list panel**
- Below/beside treemap: virtualized `List` of files in selected directory
- Uses `scanResults` from Zustand store (already available)
- Sort by size/name/age
- Files: `src/pages/DiskMap.tsx`, possibly new `src/components/DirFileList.tsx`

**E4. Right-click context menu**
- `onContextMenu` → menu: Open in Explorer | Copy Path | Delete | Properties
- Files: `src/pages/DiskMap.tsx`, new `src/components/ContextMenu.tsx`

**E5. "Other" bucket for tiny files**
- Merge items < 2% of parent size into neutral gray "Other" rectangle
- Prevents System32 from creating 100K unreadable dots
- Files: `src/pages/DiskMap.tsx`

---

### Milestone F — Polish (0.5 day)
> Nice-to-haves, do last.

- Continuous HSL color gradient (instead of 4 discrete buckets)
- Tighter text label thresholds (40×25 px instead of 60×30 px)
- Keyboard navigation (Enter = drill in, Backspace = drill up, arrows = siblings)
- Canvas renderer auto-switch at 5K rectangles

---

## Expected Performance After Phase 5

| Metric | Before Phase 5 | After Phase 5 | Gain |
|---|---|---|---|
| First scan (1M files, admin) | 5–30 s | 2–5 s | 2–6× |
| First scan (no admin, jwalk) | 30–60 s | 30–60 s | — |
| DiskMap open after scan | 30–60 s | **<100 ms** | **300–600×** |
| Duplicate scan after scan | 30–60 s | **<500 ms** | **60–120×** |
| Rescan (snapshot + USN delta) | 30–60 s | **<200 ms** | **150–300×** |
| RAM (1M files) | ~1 GB | **~140 MB** | **7× less** |
| MFT scan I/O operations | 3 (temp file) | 1 | 3× |

---

## File Change Map

| File | Change Type | Milestone |
|---|---|---|
| `src-tauri/src/models.rs` | Add `FlatEntry`, `ScanIndex` structs | A1 |
| `src-tauri/src/lib.rs` | Register `SharedScanIndex` in `.manage()` | A2 |
| `src-tauri/src/commands/mft.rs` | Remove temp file, HashMap path recon | A3 |
| `src-tauri/src/commands/scanner.rs` | Populate `SharedScanIndex` after scan | B1 |
| `src-tauri/src/commands/treemap.rs` | Query index, remove `fs::read_dir` walk | B2 |
| `src-tauri/src/commands/shared.rs` | New: `ScanIndex`, `compute_dir_sizes()` | A2, B3 |
| `src-tauri/src/commands/snapshot.rs` | New: bincode serialize/deserialize | C3 |
| `src-tauri/src/commands/usn.rs` | Add `apply_usn_delta()` | D1 |
| `src/pages/DiskMap.tsx` | Tooltips, clickable crumb, file list, ctx menu | E1–E5 |
| `src/components/DirFileList.tsx` | New: virtualized file list for DiskMap | E3 |
| `src/components/ContextMenu.tsx` | New: right-click menu component | E4 |

---

## Research Documents (for reference during implementation)

| Document | Covers |
|---|---|
| `RESEARCH_WIZTREE2.md` | WizTree architecture, 6 specific bugs, FSCTL vs $MFT |
| `RESEARCH_PHASE5_ARCH.md` | FlatEntry layout, ScanIndex design, pipeline, Everything tricks |
| `RESEARCH_PHASE5_VIZ.md` | DiskMap UX, SVG vs Canvas, color schemes, interaction patterns |
| `RESEARCH_ADVANCED.md` | Phase 1–4 research (LSH, dHash, USN, treemaps) |
| `ROADMAP.md` | Phase 0–4 status, performance targets |

---

*Last updated: Research complete. Ready to implement starting from Milestone A.*

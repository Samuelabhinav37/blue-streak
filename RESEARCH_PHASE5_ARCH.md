# CleanSweep — Phase 5: Production Architecture Research

> Deep research on in-memory index design, Tauri state management,
> incremental scanning, memory layout, and pipeline design.
> Based on Everything (VoidTools), WizTree, and production Rust patterns.

---

## Part 1 — Optimal In-Memory Index Design

### 1.1 Current FlatEntry vs Optimal

**Current** (`scanner.rs` ArenaEntries):
```rust
struct CompactEntry {
    path_offset: u32,   // offset into PathArena
    path_len:    u32,   // FULL path length stored
    size:        u64,
    age_days:    i64,
    flags:       u8,
}
// ~34 bytes, but full paths duplicated everywhere
```

**Problem:** No `file_ref` or `parent_ref`. Path is already reconstructed and stored.
When `compute_treemap` needs a tree it has to re-walk `fs::read_dir` because there's
no parent-child relationship in the stored entries.

**Optimal** (WizTree / Everything pattern):
```rust
#[repr(C)]
pub struct FlatEntry {
    pub file_ref:    u64,   // 8 — NTFS file reference number
    pub parent_ref:  u64,   // 8 — parent directory reference
    pub size:        u64,   // 8
    pub name_offset: u32,   // 4 — offset into PathArena (SHORT name only)
    pub name_len:    u16,   // 2 — short name length
    pub age_days:    u16,   // 2 — max 179 years as u16
    pub flags:       u8,    // 1 — is_dir | is_system | is_hidden
    pub _pad:        u8,    // 1
}
// 34 bytes, stores SHORT names only. Full paths reconstructed on demand.
```

**Key difference:** Store `file_ref` + `parent_ref` + SHORT name.
Full paths are O(depth) reconstructions from the parent map — done once, cached.

### 1.2 Memory Budget

| Component | Calculation | Size |
|---|---|---|
| FlatEntry array | 34 bytes × 1M | 34 MB |
| PathArena (short names ~15 chars) | 15 bytes × 1M | 15 MB |
| by_parent HashMap | ~40 bytes × 1M | 40 MB |
| by_size BTreeMap | ~48 bytes × 1M | 48 MB |
| by_extension HashMap | ~32 bytes × 100K unique | 3 MB |
| Full-path cache (top 10K dirs) | ~50 bytes × 10K | 0.5 MB |
| **Total** | | **~140 MB** |

Compared to:
- **Current CleanSweep:** ~1 GB (full paths everywhere, multiple structures)
- **Everything (VoidTools):** ~20 MB (minimal, lazy path reconstruction)
- **WizTree:** ~50–100 MB (names + sizes + refs)
- **CleanSweep optimized:** ~140 MB ← target

### 1.3 Secondary Index Structures

Built eagerly after scan completes (~200 ms for 1M files):

```rust
pub struct ScanIndex {
    // Primary: one entry per file/directory
    pub entries:       Vec<FlatEntry>,
    pub names:         PathArena,             // short name strings

    // Secondary: built on scan complete
    pub by_parent:     HashMap<u64, Vec<u32>>,      // file_ref → child indices
    pub by_size:       BTreeMap<u64, Vec<u32>>,     // size → indices (for dedup)
    pub by_extension:  HashMap<String, Vec<u32>>,   // ext → indices
    pub by_age_sorted: Vec<(u16, u32)>,             // (age_days, idx) sorted

    // Dir size cache: built bottom-up in one pass
    pub dir_sizes:     HashMap<u64, u64>,           // file_ref → cumulative size
}
```

**Index build is parallelizable (rayon):**
```rust
let by_parent: HashMap<u64, Vec<u32>> = entries
    .par_iter()
    .enumerate()
    .fold(HashMap::new, |mut acc, (i, e)| {
        acc.entry(e.parent_ref).or_default().push(i as u32);
        acc
    })
    .reduce(HashMap::new, merge_hashmaps);
```

---

## Part 2 — The Critical Missing Piece: Shared ScanIndex in Tauri State

### 2.1 Why Every Command Re-Scans Today

`scan_for_junk` → returns `Vec<JunkFile>` to frontend → **data discarded**.
`compute_treemap` → walks `fs::read_dir` from scratch — has no access to prior scan.
`scan_near_duplicates` → another independent walk.

Root cause: there is no **shared mutable state** holding the scan result.

### 2.2 The Fix: SharedScanIndex

```rust
// models.rs or a new shared.rs
pub struct ScanIndex {
    pub entries:    Vec<FlatEntry>,
    pub names:      PathArena,
    pub by_parent:  HashMap<u64, Vec<u32>>,
    pub by_size:    BTreeMap<u64, Vec<u32>>,
    pub dir_sizes:  HashMap<u64, u64>,
    pub root:       String,
    pub scanned_at: std::time::SystemTime,
}

impl Default for ScanIndex {
    fn default() -> Self { /* zero state */ }
}

pub type SharedScanIndex = Arc<RwLock<ScanIndex>>;
```

**Registered in `lib.rs`:**
```rust
let scan_index: SharedScanIndex = Arc::new(RwLock::new(ScanIndex::default()));
tauri::Builder::default()
    .manage(scan_index)
    // ...
```

**Used in commands:**
```rust
#[tauri::command]
pub fn compute_treemap(
    root_path: String,
    index: State<'_, SharedScanIndex>,
    // ...
) -> Result<TreemapNode, String> {
    let idx = index.read().map_err(|e| e.to_string())?;
    if idx.root == root_path {
        return build_treemap_from_index(&idx, &root_path, ...);
        // ← 0 I/O, <100 ms
    }
    Err("No cached scan for this path — run Scan first".into())
}
```

### 2.3 RwLock vs Mutex

**Our workload:** Many concurrent reads (UI queries) + rare writes (scan completes).

| | Mutex | RwLock |
|---|---|---|
| Multiple readers | Blocked | ✅ Concurrent |
| Single writer | ✅ | ✅ |
| Uncontended cost | ~2 ns | ~5 ns |
| Recommendation | Only if simple | **Use this** |

**Use `RwLock`** — the UI can query results while a background scan is still running.

### 2.4 Async Scan — Don't Block the Main Thread

```rust
#[tauri::command]
pub async fn scan_for_junk(
    app: AppHandle,
    paths: Vec<String>,
    index: State<'_, SharedScanIndex>,
) -> Result<(), String> {
    let index_arc = Arc::clone(&*index);
    let app_clone = app.clone();

    tokio::spawn(async move {
        // 1. Build new index (runs in background)
        let new_index = build_scan_index(&paths, &app_clone).await;

        // 2. Atomic swap — UI immediately sees new results
        {
            let mut guard = index_arc.write().unwrap();
            *guard = new_index;
        }

        // 3. Notify frontend
        let _ = app_clone.emit("scan-complete", ());
    });

    Ok(())  // Returns immediately; scan runs in background
}
```

---

## Part 3 — Directory Size Accumulation (One Pass, No fs::read_dir)

### 3.1 Current Problem

`treemap.rs::build_dir_tree_inner` calls `fs::read_dir(path)` recursively.
At max depth it calls `quick_size(path)` which spawns a `jwalk::WalkDir`.
**This is a full re-scan every time DiskMap is opened.**

### 3.2 Bottom-Up Aggregation From Flat Entries

Once `ScanIndex` has `by_parent` (children by file_ref), dir sizes accumulate in one BFS:

```rust
pub fn compute_dir_sizes(index: &ScanIndex) -> HashMap<u64, u64> {
    let mut dir_sizes: HashMap<u64, u64> = HashMap::new();

    // Process leaves first, then propagate upward (post-order traversal)
    // Use topological sort via parent_ref (files always have dir parents)

    for entry in &index.entries {
        if !entry.is_dir() {
            // Add file size to all ancestor directories
            let mut cur_ref = entry.parent_ref;
            let mut size_to_add = entry.size;
            loop {
                let prev = dir_sizes.entry(cur_ref).or_insert(0);
                *prev += size_to_add;
                // Walk up to parent
                match index.parent_of(cur_ref) {
                    Some(p) if p != cur_ref => cur_ref = p,
                    _ => break,
                }
            }
        }
    }

    dir_sizes
}
```

**This runs in O(n × average_depth) ≈ O(n × 7) ≈ O(7n) for typical Windows drives.**
For 1M files: ~7M operations = **~50 ms**. No I/O.

### 3.3 Treemap From Index (No fs::read_dir)

```rust
pub fn build_treemap_from_index(
    index: &ScanIndex,
    root_ref: u64,
    canvas_w: f64,
    canvas_h: f64,
) -> TreemapNode {
    // Get direct children of root_ref
    let children: Vec<(String, u64, u64, u64)> = // (name, file_ref, size, file_count)
        index.by_parent.get(&root_ref)
            .unwrap_or(&vec![])
            .iter()
            .map(|&idx| {
                let e = &index.entries[idx as usize];
                let name = index.names.get(e.name_offset, e.name_len).to_string();
                let size = if e.is_dir() {
                    index.dir_sizes.get(&e.file_ref).copied().unwrap_or(0)
                } else {
                    e.size
                };
                let count = index.by_parent.get(&e.file_ref).map(|c| c.len() as u64).unwrap_or(1);
                (name, e.file_ref, size, count)
            })
            .collect();

    // Sort by size descending (squarified requires this)
    let mut items: Vec<_> = children.iter()
        .map(|(name, fref, size, count)| (name.clone(), fref.to_string(), *size, *count))
        .collect();
    items.sort_by(|a, b| b.2.cmp(&a.2));

    // Layout with squarify — pure computation, no I/O
    let rects = squarify(&items, 0.0, 0.0, canvas_w, canvas_h, 0);

    TreemapNode { children: rects, ... }
}
```

---

## Part 4 — Incremental Scanning (USN Journal Delta)

### 4.1 Current State

`usn.rs` implements `usn_snapshot` and `usn_get_changes` correctly.
`cache.rs` persists `usn_state` table with `(drive, last_usn, journal_id)`.

**The problem:** USN delta is computed but **never applied to the ScanIndex**.
The delta is returned to the frontend as raw paths; there's no merge logic.

### 4.2 Delta Merge Algorithm

```rust
pub fn apply_usn_delta(index: &mut ScanIndex, changes: &[UsnChange]) {
    for change in changes {
        let path = &change.path;
        let reason = &change.reason;

        if reason.contains("delete") || reason.contains("rename_old") {
            // Remove entry from index
            index.entries.retain(|e| {
                index.names.get(e.name_offset, e.name_len) != path.as_str()
            });
        }

        if reason.contains("create") || reason.contains("modify") || reason.contains("rename_new") {
            // Re-stat and re-add the entry
            if let Ok(meta) = std::fs::metadata(path) {
                // Build new FlatEntry and push
                index.entries.push(flat_entry_from_meta(path, &meta));
            }
        }
    }

    // Rebuild indices after delta (200 ms, but delta is usually small)
    // Optimization: only rebuild affected sub-trees
    index.rebuild_indices();
}
```

### 4.3 Journal Wrap Detection

**Critical:** NTFS periodically clears old USN entries. If `journal_id` changes,
the `last_usn` cursor is invalid — must fall back to full scan.

```rust
pub fn usn_get_changes_safe(
    drive: &str,
    saved_usn: i64,
    saved_journal_id: u64,
) -> Result<Vec<UsnChange>, UsnError> {
    let (current_low_usn, current_journal_id) = query_journal(drive)?;

    if current_journal_id != saved_journal_id {
        return Err(UsnError::JournalWrapped);  // Caller does full scan
    }

    if saved_usn < current_low_usn {
        return Err(UsnError::CursorExpired);   // Saved USN was cleared
    }

    read_changes_since(drive, saved_usn)
}
```

---

## Part 5 — Serialization: FlatBuffers for Instant Rescans

### 5.1 Format Comparison

| Format | Read speed | File size | Schema evolution |
|---|---|---|---|
| JSON | 10 MB/s | 3× memory | ✅ Easy |
| bincode | 500 MB/s | 1× | ❌ Breaks on struct change |
| FlatBuffers | 1000 MB/s (zero-copy) | 1× | ✅ Versioned |
| SQLite WAL | 50 MB/s (ACID) | 1.2× | ✅ SQL migrations |
| Raw mmap binary | N/A (no copy) | 1× | ❌ Platform-specific |

### 5.2 Recommended Strategy

- **FlatBuffers** for snapshot (`Vec<FlatEntry>` → disk → instant reload)
- **SQLite** stays for USN state, growth trends, rule history (change journal)

```rust
// Save after scan (~200 MB write, ~400 ms)
pub fn save_snapshot(index: &ScanIndex) -> io::Result<()> {
    let path = snapshot_path();
    let buf = flatbuffers_serialize(index);
    std::fs::write(&path, buf)
}

// Load on startup (~200 MB read, ~100 ms — avoids full rescan)
pub fn load_snapshot() -> io::Result<ScanIndex> {
    let bytes = std::fs::read(snapshot_path())?;
    flatbuffers_deserialize(&bytes)
}
```

**With snapshot + USN delta:**
- First run: 3–5 s (full MFT scan) + 400 ms (write snapshot)
- Subsequent runs: 100 ms (load snapshot) + 50 ms (apply USN delta)
- **10–50× faster rescans**

---

## Part 6 — The 6-Stage Scan Pipeline

```
Stage 1: Acquisition (single-threaded I/O)
  → MFT read (admin) or jwalk (no admin)
  → Output: raw (path, size, age, parent_ref, flags) per entry

Stage 2: Parsing (rayon parallel)
  → Decode UTF-16 names, normalize paths, compute age
  → Output: Vec<FlatEntry>

Stage 3: Dedup + sort (single-threaded O(n log n))
  → Deduplicate by file_ref (multi-root scans may include same file twice)
  → Sort by parent_ref for cache-efficient traversal
  → Output: Vec<FlatEntry> sorted, unique

Stage 4: Index build (rayon parallel)
  → Build by_parent, by_size, by_extension, by_age_sorted
  → Compute dir_sizes bottom-up
  → Output: complete ScanIndex

Stage 5: Junk classification (rayon parallel)
  → Apply rules: temp patterns, large files, duplicate detection, old downloads
  → Output: Vec<JunkFile> with categories

Stage 6: Store + notify (single-threaded)
  → Atomic swap into SharedScanIndex (RwLock write)
  → Emit "scan-complete" event to frontend
  → Save FlatBuffers snapshot to disk
  → Output: frontend notified, all views can now query
```

### Parallelism by Stage

| Stage | Method | Why |
|---|---|---|
| 1: Acquisition | Single-thread | Sequential I/O — kernel manages disk |
| 2: Parsing | rayon par_iter | CPU-bound, no shared state |
| 3: Dedup/sort | Single-thread | sort needs full array; par sort degrades |
| 4: Index build | rayon fold+reduce | Each index independent |
| 5: Classification | rayon par_iter | Rules stateless per-entry |
| 6: Store/notify | Single-thread | Atomic swap + event emit |

---

## Part 7 — Error Handling Patterns

### File Deleted Between Scan and Hash
```rust
match std::fs::File::open(&path) {
    Ok(f) => compute_hash(f),
    Err(e) if e.kind() == ErrorKind::NotFound => {
        // Race condition — file gone. Remove from results, continue.
        return None;
    }
    Err(e) => Err(e.to_string()),
}
```

### System-Locked Files to Skip
```rust
pub fn is_system_file(path: &str) -> bool {
    let name = path.split(['/', '\\']).last().unwrap_or("");
    matches!(name.to_lowercase().as_str(),
        "pagefile.sys" | "hiberfil.sys" | "swapfile.sys"
        | "$mft" | "$logfile" | "$bitmap" | "$badclus"
    )
}
```

### Locked Files That Cause Hash Failures

**Don't fail the scan.** Return `FailedHash` category and continue:
```rust
pub enum HashResult { Ok([u8; 32]), Locked, TooLarge, Deleted }
```

---

## Part 8 — Everything (VoidTools) Memory Tricks

### How Everything Stores 1M Files in 20 MB

1. **No full paths** — only short names + parent_ref
2. **Lazy path reconstruction** — only when user queries a specific file
3. **Bit-packed flags** — hidden/system/dir/archive in one u8
4. **Custom allocator** — slab allocator, no per-entry malloc overhead
5. **Name deduplication** — common names ("System32", ".exe") interned once
6. **No per-entry metadata** — no size, no timestamps in primary index
   (fetched on-demand via GetFileInformationByHandle by file_ref)

### Can CleanSweep Use Same Tricks?

**Partially.** We need size + age for junk detection, so can't go as minimal as Everything.
But we can:
- Store SHORT names (not full paths) — save 30 bytes/file
- Use u16 for age_days (not i64) — save 6 bytes/file
- PathArena already implemented ✅

**These two changes alone: 34→16 bytes/file = 50% RAM reduction.**

---

## Part 9 — What "Everything" Does That We Can Learn From

| Technique | Everything | CleanSweep Today | Action |
|---|---|---|---|
| USN journal delta | ✅ Core feature | ✅ Implemented but not applied to index | Wire USN delta into ScanIndex |
| Short name storage | ✅ 20 MB/1M | ⚠️ Full paths (1 GB) | Store short names, reconstruct |
| Background indexing | ✅ Always running | ❌ Manual scan only | Watcher already exists → connect |
| File ref as primary key | ✅ O(1) lookup | ❌ Path string matching | Use file_ref in FlatEntry |
| Instant filename search | ✅ <100 ms for 1M | ❌ Not implemented | Add suffix array or trie |
| Change propagation | ✅ USN journal | ❌ Full rescan | Apply delta to ScanIndex |
| NTFS-only fast path | ✅ | ✅ MFT scan exists | Connect to SharedScanIndex |

---

## Summary Table: What to Build in Phase 5

| Item | Problem It Solves | Effort | Impact |
|---|---|---|---|
| `SharedScanIndex` in Tauri state | All views re-scan independently | 4 hrs | ★★★★★ |
| FlatEntry with file_ref/parent_ref | Can't build treemap from scan data | 3 hrs | ★★★★★ |
| `compute_treemap` queries SharedScanIndex | DiskMap re-scans every open | 2 hrs | ★★★★★ |
| Dir size bottom-up aggregation | treemap.rs uses fs::read_dir | 2 hrs | ★★★★★ |
| Remove temp file copy in mft.rs | 3× I/O on every MFT scan | 2 hrs | ★★★★☆ |
| HashMap path recon (drop LRU cache) | Slow random-access path building | 2 hrs | ★★★★☆ |
| FlatBuffers snapshot | Full rescan on every app open | 4 hrs | ★★★★☆ |
| USN delta → ScanIndex merge | Incremental scan not applied | 3 hrs | ★★★☆☆ |
| DiskMap hover tooltips | No feedback on hover | 1 hr | ★★★☆☆ |
| DiskMap clickable breadcrumb | Can't jump to parent levels | 1 hr | ★★★☆☆ |
| DiskMap file list panel | Can't see files in a directory | 3 hrs | ★★★☆☆ |
| DiskMap right-click menu | Can't act on files from treemap | 2 hrs | ★★★☆☆ |
| Canvas renderer (>5K rects) | SVG freezes at 50K nodes | 4 hrs | ★★☆☆☆ |
| "Other" bucket for tiny files | Treemap unusable for System32 | 1 hr | ★★☆☆☆ |
| Continuous color gradient | Info loss at bucket boundaries | 1 hr | ★★☆☆☆ |

---

## References

- VoidTools Everything Architecture: https://www.voidtools.com/support/everything/
- NTFS USN Journal: https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_read_usn_journal
- FlatBuffers Rust: https://github.com/google/flatbuffers/tree/master/rust/flatbuffers
- WizTree technical: https://diskanalyzer.com/guide
- NTFS MFT format: https://flatcap.github.io/linux-ntfs/ntfs/files/mft.html
- rayon parallel patterns: https://docs.rs/rayon/latest/rayon/

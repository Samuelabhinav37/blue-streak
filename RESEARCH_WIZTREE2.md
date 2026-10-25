# WizTree Architecture Deep-Dive: The "Blueprint First" Pattern
## Why WizTree Is 100× Faster — And What We're Doing Wrong

> **Executive Summary:** WizTree achieves 1–2 s for 1 M files by reading the NTFS
> $MFT sequentially **once**, building a flat in-memory array, then having every view
> (disk map, treemap, duplicate scanner, junk detector) query that array without
> any further I/O. Our code scans the MFT correctly but **throws the data away after
> each scan** — so clicking DiskMap re-scans from scratch.

---

## Part 1 — The Two Architectures Side-by-Side

### 1.1 WizTree: "Blueprint First"

```
┌─────────────────────────────────────────────────────┐
│  ONE MFT READ  (~1–2 s for 1 M files)               │
│  Sequential contiguous I/O (disk-bandwidth limited)  │
│  Output: Vec<FlatEntry> in RAM  (~100–200 MB)        │
└──────────┬──────────────────────────────────────────┘
           │
     ┌─────┴──────┬───────────────┬───────────────┬──────────────┐
     ▼            ▼               ▼               ▼              ▼
  DiskMap      Treemap      DuplicateFinder  JunkScanner    FileList
  query RAM    query RAM    query RAM        query RAM      query RAM
  0 I/O        0 I/O        0 I/O            0 I/O          0 I/O
```

### 1.2 Our Current Code: "Re-scan Per View"

```
User: "Scan Now"
  → scan_for_junk()
    → parallel_walk() / MFT walk  ← WALK #1 (2–60 s)
    → Vec<JunkFile> displayed
    → data discarded from memory

User: "DiskMap tab"
  → compute_treemap()
    → build_dir_tree() → fs::read_dir()  ← WALK #2 (30–60 s)
    → TreemapNode returned

User: (future) "Duplicates tab"
  → scan_duplicates()
    → jwalk(root)  ← WALK #3 (30–60 s)
```

**Three independent filesystem walks. 1–3 minutes total. All avoidable.**

---

## Part 2 — How WizTree Actually Works

### 2.1 Step 1 — Open the Volume (Admin Required)

```c
HANDLE h = CreateFileW(
    L"\\\\.\\C:",
    GENERIC_READ,
    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
    NULL,
    OPEN_EXISTING,
    FILE_FLAG_BACKUP_SEMANTICS,  // SeBackupPrivilege bypass
    NULL
);
```

`FILE_FLAG_BACKUP_SEMANTICS` lets the kernel bypass all per-file ACL checks.
Without it Windows does a full ACL check per `FindNextFile` call → 10× slowdown.

### 2.2 Step 2 — Enumerate the MFT via FSCTL_ENUM_USN_RECORDS

```c
MFT_ENUM_DATA_V0 med = { 0, 0, MAXLONGLONG };
BYTE buf[1 * 1024 * 1024];  // 1 MB buffer

while (DeviceIoControl(
    h,
    FSCTL_ENUM_USN_RECORDS,
    &med, sizeof(med),
    buf, sizeof(buf),
    &bytesReturned, NULL
)) {
    med.StartFileReferenceNumber = *(DWORDLONG*)buf;  // advance cursor

    // parse USN_RECORD_V2 from buf[8 .. bytesReturned]
    // each record: file_ref, parent_ref, filename, timestamps, flags
    // NO file size here — separate step needed
}
```

The kernel reads $MFT sequentially from disk in large chunks.
A 500 MB $MFT at 200 MB/s = **~2.5 seconds**. No random seeks, no ACL checks.

### 2.3 What Each USN_RECORD_V2 Contains

| Field | Size | Notes |
|---|---|---|
| `FileReferenceNumber` | u64 | MFT entry index (unique file ID) |
| `ParentFileReferenceNumber` | u64 | **Key for path reconstruction** |
| `FileAttributes` | u32 | `FILE_ATTRIBUTE_DIRECTORY` etc. |
| `FileName` | UTF-16 | Short name ("notepad.exe") |
| `TimeStamp` | i64 | 100 ns intervals since 1601 |

**File size is NOT in USN records.** WizTree gets it from a secondary source
(NtQueryInformationFile by file ref, or direct $DATA attribute parsing).

### 2.4 Step 3 — Path Reconstruction in O(n) with a HashMap

```rust
// Build two maps in one pass
let mut parent_map: HashMap<u64, u64>    = HashMap::new();  // ref → parent_ref
let mut name_map:   HashMap<u64, String> = HashMap::new();  // ref → short_name

for record in usn_records {
    parent_map.insert(record.file_ref, record.parent_ref);
    name_map.insert(record.file_ref, record.name.clone());
}

// Reconstruct paths: walk up parent chain (O(depth) per file)
fn full_path(file_ref: u64) -> String {
    let mut parts = vec![];
    let mut cur = file_ref;
    loop {
        let name = &name_map[&cur];
        parts.push(name.clone());
        let parent = parent_map[&cur];
        if parent == 5 || parent == cur { break; }  // 5 = NTFS root
        cur = parent;
    }
    parts.reverse();
    parts.join("\\")
}
```

**Total: O(n) one-pass map build + O(n) path reconstruction with O(1) lookups.**
No LRU cache, no random seeks back into the MFT data.

### 2.5 Step 4 — Build the Flat Array

```rust
pub struct FlatEntry {
    pub path:       String,
    pub file_ref:   u64,
    pub parent_ref: u64,
    pub size_bytes: u64,
    pub age_days:   i64,
    pub is_dir:     bool,
}

// One allocation, ~100–200 MB for 1M files
let blueprint: Vec<FlatEntry> = reconstruct_all_paths(usn_records);
```

### 2.6 Step 5 — All Views Query the Array

```rust
// Treemap: sum directory sizes bottom-up from flat array
fn build_dir_sizes(entries: &[FlatEntry]) -> HashMap<String, u64> {
    entries.iter().fold(HashMap::new(), |mut acc, e| {
        if !e.is_dir {
            // Walk up parent chain, adding size to each ancestor
            let parts = e.path.split('\\').collect::<Vec<_>>();
            for depth in 1..parts.len() {
                *acc.entry(parts[..depth].join("\\")).or_insert(0) += e.size_bytes;
            }
        }
        acc
    })
}

// Duplicates: group by size, then hash — zero I/O
fn find_duplicates(entries: &[FlatEntry]) -> Vec<DuplicateGroup> {
    let by_size: HashMap<u64, Vec<&FlatEntry>> = group_by_size(entries);
    // only need to actually open files for hashing, not re-walk them
}

// Junk detector: filter the flat array
fn find_junk(entries: &[FlatEntry]) -> Vec<JunkFile> {
    entries.iter().filter(|e| is_junk(e)).cloned().collect()
}
```

---

## Part 3 — Why WizTree Is ~100× Faster Than `FindFirstFile`

### 3.1 I/O Profile Comparison

**FindFirstFile / jwalk (what we fall back to without admin):**
```
for each directory (1000 dirs on a typical drive):
    FindFirstFileW(dir\*)      ← kernel call #1  (open dir $INDEX_ROOT)
    loop:
        FindNextFileW(handle)  ← kernel call per entry
        [ACL check on each file, even as admin if no backup priv]

Total: ~1 M kernel calls × ~50 µs each = 50 seconds
```

**Sequential MFT read (WizTree / us with admin):**
```
DeviceIoControl(FSCTL_ENUM_USN_RECORDS, 1 MB buf) × 500 iters
= 500 sequential reads × 5 ms each = 2.5 seconds
```

**Ratio: 50 s / 2.5 s = 20×. With ACL overhead on non-admin: up to 100×.**

### 3.2 Why Sequential I/O Wins

The $MFT is guaranteed contiguous on NTFS (it does not fragment because NTFS
reserves space for it in the MFT Zone). Reading it is:
- **SSD:** 200–500 MB/s sustained → 2–5 seconds for 1 GB $MFT
- **HDD:** 50–100 MB/s with OS read-ahead → 10–20 seconds

Compare to random directory walks which are IOPS-limited:
- SSD: ~100 K IOPS / 5–10 calls per file = 10–20 K files/s → 50–100 s for 1 M files
- HDD: ~100–300 IOPS → 100–300 s for 1 M files

---

## Part 4 — The Six Critical Bugs in Our Code

### Bug #1 — `mft.rs`: Double I/O (Temp File Copy)

**File:** `src-tauri/src/commands/mft.rs` lines 91–116

```rust
fn read_mft_to_temp(drive: &str) -> Result<PathBuf, String> {
    let mut src = open_mft(drive)?;       // I/O READ #1: from volume
    let tmp = temp_dir().join("Blue Streak_mft.bin");
    let mut dst = File::create(&tmp)?;
    io::copy(&mut src, &mut dst)?;        // I/O WRITE #2: copy to disk
    Ok(tmp)
    // then MftParser reads tmp from disk  I/O READ #3
}
```

**Why it was done:** `mft::MftParser` requires `Seek`. The raw volume file handle
is not seekable. The fix is to read into RAM via `Vec<u8>` + `std::io::Cursor`:

```rust
fn mft_parser_for_drive(drive: &str) -> Result<mft::MftParser<Cursor<Vec<u8>>>, String> {
    let mft_path = format!("\\\\?\\{}:\\$MFT", drive.trim_end_matches([':', '\\']));
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(0x02000000)  // FILE_FLAG_BACKUP_SEMANTICS
        .open(&mft_path)
        .map_err(|e| format!("Cannot open $MFT: {}", e))?;

    let mut buf = Vec::new();
    file.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    // buf is now in RAM — no temp file on disk

    mft::MftParser::from_read_seek(Cursor::new(buf))
        .map_err(|e| format!("MftParser error: {}", e))
}
```

**Impact:** Removes 2 of 3 I/O operations → **2–3× faster MFT scan.**

---

### Bug #2 — `mft.rs`: Two-Pass Path Reconstruction with Random Seeks

**File:** `src-tauri/src/commands/mft.rs` lines 192–267

Pass 1 collects `(entry, size, age, is_dir)` into `raw: Vec<RawData>`.
Pass 2 calls `parser.get_full_path_for_entry(&entry)` which uses an internal
LRU cache that **randomly re-accesses the parser's index** to walk parent refs.

For 1 M entries at depth 5–10:
- 5–10 M cache lookups per reconstruction
- LRU size = 128 → most are cache misses
- Each miss = seek into the in-memory `Cursor` + re-parse

**Fix:** Build parent/name maps in Pass 1, reconstruct paths in Pass 2
using O(1) HashMap lookups — no seeks:

```rust
let mut parent_map: HashMap<u64, u64>    = HashMap::with_capacity(total);
let mut name_map:   HashMap<u64, String> = HashMap::with_capacity(total);
let mut meta_map:   HashMap<u64, (u64, i64, bool)> = HashMap::with_capacity(total);

for entry_result in parser.iter_entries() {
    let e = entry_result?;
    if !e.header.is_valid() { continue; }
    // extract parent_ref, name, size, age, is_dir
    parent_map.insert(e.header.record_number, parent_ref);
    name_map.insert(e.header.record_number, name);
    meta_map.insert(e.header.record_number, (size, age, is_dir));
}

// O(1) path walk — no random seeks
fn reconstruct_path(
    file_ref: u64,
    drive: &str,
    parent_map: &HashMap<u64, u64>,
    name_map: &HashMap<u64, String>,
) -> String {
    let mut parts = vec![];
    let mut cur = file_ref;
    while cur != 5 {
        if let Some(name) = name_map.get(&cur) { parts.push(name.as_str()); }
        match parent_map.get(&cur) {
            Some(&p) if p != cur => cur = p,
            _ => break,
        }
    }
    parts.reverse();
    format!("{}:\\{}", drive, parts.join("\\"))
}
```

**Impact:** Eliminates random-access pattern → **3–5× faster path reconstruction.**

---

### Bug #3 — `treemap.rs`: Completely Independent Filesystem Walk

**File:** `src-tauri/src/commands/treemap.rs` lines 208–299

`compute_treemap()` calls `build_dir_tree(root)` which calls `fs::read_dir()` in a
recursive tree — **completely ignoring any previous MFT scan**:

```rust
fn build_dir_tree_inner(path: &Path, depth: u32, max_depth: u32) -> DirEntry {
    if let Ok(entries) = std::fs::read_dir(path) {  // ← separate fs walk!
        // recurse...
    }
}
```

And at max depth it falls back to `quick_size()` which calls `jwalk::WalkDir`.

**Fix:** Pass the flat `Vec<FlatEntry>` from the MFT scan. Aggregate sizes
by filtering entries whose path starts with the directory prefix:

```rust
pub fn build_treemap_from_flat(
    entries: &[FlatEntry],
    root: &str,
    max_depth: u32,
    canvas_w: f64,
    canvas_h: f64,
) -> TreemapNode {
    // Group files by their immediate parent directory
    let mut dir_sizes: HashMap<String, u64> = HashMap::new();
    let mut dir_counts: HashMap<String, u64> = HashMap::new();

    for e in entries.iter().filter(|e| e.path.starts_with(root) && !e.is_dir) {
        // Walk up the ancestor chain (up to max_depth levels above root)
        let rel = &e.path[root.len()..];
        let parts: Vec<&str> = rel.trim_matches('\\').split('\\').collect();
        for depth in 0..parts.len().min(max_depth as usize) {
            let dir_path = format!("{}\\{}", root, parts[..depth].join("\\"));
            *dir_sizes.entry(dir_path.clone()).or_insert(0) += e.size_bytes;
            *dir_counts.entry(dir_path).or_insert(0) += 1;
        }
    }

    // Build TreemapNode tree from dir_sizes (no I/O)
    layout_from_aggregated(&dir_sizes, &dir_counts, root, canvas_w, canvas_h)
}
```

**Impact:** Treemap goes from 30–60 s → **<100 ms.** 300–600× speedup.

---

### Bug #4 — No Shared Scan State Between Commands

`scan_for_junk`, `compute_treemap`, `scan_near_duplicates`, `scan_audio_duplicates`
each independently walk the filesystem. The result of `scan_for_junk` is returned
to the frontend but not stored in app state for other commands to reuse.

**Fix:** Store `Vec<FlatEntry>` in Tauri app state after each scan:

```rust
pub struct ScanBlueprint {
    pub entries: Vec<FlatEntry>,
    pub scanned_at: std::time::SystemTime,
    pub root: String,
}

pub type SharedBlueprint = Arc<Mutex<Option<ScanBlueprint>>>;
```

Register it in `lib.rs` via `.manage(...)` and inject via `State<'_, SharedBlueprint>`.

After `scan_for_junk` populates it, every other command can call:

```rust
let blueprint = state.lock().unwrap();
let entries = &blueprint.as_ref().ok_or("No scan yet")?.entries;
```

**Impact:** All views after first scan are instant queries. 100–1000× speedup for
every view after the initial scan.

---

### Bug #5 — `scanner.rs` and `treemap.rs` Use Different Data Models

`scanner.rs` returns `Vec<JunkFile>` (only junk, stripped down).
`treemap.rs` builds its own `Vec<DirEntry>` tree from scratch.

Neither has access to the other's data. The canonical representation should be
`Vec<FlatEntry>` containing ALL files (not just junk), which both can filter.

---

### Bug #6 — `quick_size()` in `treemap.rs` Reverts to `jwalk`

**File:** `src-tauri/src/commands/treemap.rs` lines 301–317

Called at max_depth to sum directory sizes:

```rust
fn quick_size(path: &Path) -> (u64, u64) {
    use jwalk::WalkDir;
    for entry in WalkDir::new(path) ...
}
```

If the outer scan used MFT (fast), this falls back to `jwalk` (slow).
Inconsistent I/O profile.

---

## Part 5 — The Fix: Three-Step Plan

### Step 1 — Fix `mft.rs`: In-Memory MFT + HashMap Path Reconstruction

- Remove `read_mft_to_temp()` → use `Cursor<Vec<u8>>`
- Single pass over MFT: build `parent_map` + `name_map` + `meta_map`
- Path reconstruction via O(1) HashMap lookups
- Result: `Vec<FlatEntry>` (all files, not just junk)

### Step 2 — Add `SharedBlueprint` to App State

```rust
// lib.rs
let blueprint: SharedBlueprint = Arc::new(Mutex::new(None));
tauri::Builder::default()
    .manage(blueprint)
    // ...
```

After `scan_for_junk` runs, store the full `Vec<FlatEntry>` in `SharedBlueprint`.

### Step 3 — Rewrite `compute_treemap` to Query `SharedBlueprint`

```rust
#[tauri::command]
pub fn compute_treemap(
    root_path: String,
    blueprint: State<'_, SharedBlueprint>,
    // ...
) -> Result<TreemapNode, String> {
    let guard = blueprint.lock().map_err(|e| e.to_string())?;
    let bp = guard.as_ref().ok_or("No scan available — run Scan first")?;

    // Filter to this root, aggregate sizes, layout — no I/O
    build_treemap_from_flat(&bp.entries, &root_path, ...)
}
```

---

## Part 6 — Performance Targets After Fix

| Operation | Current | After Fix | Improvement |
|---|---|---|---|
| First scan (1 M files, admin) | 5–30 s | 2–5 s | 6–10× |
| First scan (1 M files, no admin) | 30–60 s | 30–60 s | — (jwalk fallback) |
| MFT temp file I/O | 3 reads/writes | 1 read | 3× |
| Path reconstruction | Random seeks | O(1) HashMap | 5–10× |
| DiskMap after scan | 30–60 s | <100 ms | **300–600×** |
| Duplicate finder after scan | 30–60 s | <500 ms | **100–120×** |
| Total cycle (scan + treemap + dupes) | 90–180 s | 3–6 s | **30–50×** |

---

## Part 7 — WizTree Technology Choices

| Choice | WizTree | Our Current | Recommended Fix |
|---|---|---|---|
| MFT access method | `FSCTL_ENUM_USN_RECORDS` | `\\?\C:\$MFT` file open | Either works; switch to FSCTL to get timestamps + avoid temp copy |
| Path reconstruction | In-memory HashMap | LRU cache in mft crate | Build own HashMap from one pass |
| Data sharing | Global in-memory array | None | `SharedBlueprint` in Tauri state |
| File size source | `NtQueryInformationFile` by file ref | `$DATA` attribute parsing | Both fine; current is OK |
| Non-admin fallback | GUI shows "admin required" message | `jwalk` (slow but works) | Keep jwalk fallback |

---

## References

- Microsoft: [FSCTL_ENUM_USN_RECORDS](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_enum_usn_records)
- Microsoft: [USN_RECORD_V2 structure](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ns-winioctl-usn_record_v2)
- NTFS layout: [FlatCap NTFS docs](https://flatcap.github.io/linux-ntfs/ntfs/files/mft.html)
- WizTree: [diskanalyzer.com — How WizTree Works](https://diskanalyzer.com/guide)
- Everything (VoidTools): [USN journal architecture](https://www.voidtools.com/support/everything/)
- SpaceSniffer architecture notes: [uderzo.it](https://www.uderzo.it/main_products/space_sniffer/)

---

*Last updated: Phase 2 complete. This document defines the Phase 5 re-architecture work.*

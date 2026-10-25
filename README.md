<p align="center">
  <img src="src-tauri/icons/128x128.png" width="80" alt="CleanSweep icon" />
</p>

<h1 align="center">CleanSweep</h1>

<p align="center">
  A fast, Windows-native disk analyzer and file organizer<br/>
  built with <strong>Tauri v2</strong> · <strong>Rust</strong> · <strong>React 18</strong> · <strong>TypeScript</strong>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/platform-Windows-blue?style=flat-square" />
  <img src="https://img.shields.io/badge/Tauri-v2-purple?style=flat-square" />
  <img src="https://img.shields.io/badge/Rust-1.85%2B-orange?style=flat-square" />
  <img src="https://img.shields.io/badge/license-MIT-green?style=flat-square" />
</p>

---

## What it does

CleanSweep finds and removes the files you forgot you had — old downloads, duplicate photos, leftover temp files, large backups that never got cleaned up — and shows you exactly where every gigabyte on your drive went.

| Feature | Detail |
|---|---|
| **Junk scanner** | Temp files, stale downloads (>90 days), empty directories, large files (>500 MB) |
| **Duplicate detection** | 3-pass: group by size → partial BLAKE3 hash (64 KB) → full BLAKE3 hash |
| **Image deduplication** | Perceptual dHash — catches same photo at different resolutions |
| **Audio deduplication** | Sub-band energy fingerprint via symphonia — catches same track at different bitrates |
| **Near-duplicate detection** | LSH / Jaccard similarity on 8 KB block hashes |
| **Disk Map** | Interactive squarified treemap — see where your space went at a glance |
| **File Organizer** | Rule engine: move/copy/archive by extension, size, age, regex |
| **Watcher** | Real-time folder monitor with event log |
| **Scheduler** | Windows Task Scheduler integration — scan on login, daily, or weekly |
| **CLI** | `cleansweep-cli` — scriptable scan, clean, rules, trends, MFT export |

---

## Why it is fast

CleanSweep is designed around a single idea: **read the filesystem once, query it forever**.

### NTFS MFT direct scan (admin)
Instead of calling `FindFirstFile` / `FindNextFile` for every directory, CleanSweep reads the raw NTFS `$MFT` directly — one sequential I/O pass that returns every file and directory on the volume in 2–5 seconds, regardless of how many files there are.

```
FindFirstFile walk  ~1M files  →  30–60 s
MFT direct scan     ~1M files  →   2–5 s   (6–15× faster)
```

The `$MFT` is read entirely into RAM (no temp-file copy), and paths are reconstructed with two in-memory `HashMap`s — O(1) per hop instead of the LRU-cache thrashing used by earlier tools.

### SharedScanIndex — scan once, query everything
After every scan the full flat entry array is stored in a shared in-memory `Arc<RwLock<ScanIndex>>`. Every subsequent view (Disk Map, duplicates, near-duplicates, audio fingerprinting) queries that structure — **zero additional filesystem I/O**.

```
DiskMap open after scan      →  < 100 ms   (was 30–60 s: 300–600× speedup)
Duplicate scan after scan    →  < 500 ms   (was 30–60 s)
```

### Snapshot + USN delta
After a scan, `ScanIndex` is serialised to `%LOCALAPPDATA%\cleansweep\scan.bin` using `bincode`. On the next app launch:

1. Load snapshot (< 200 ms for 1 M files)
2. Call `FSCTL_READ_USN_JOURNAL` to get every file that changed since the snapshot
3. Apply the delta — add/update/remove affected entries
4. All views are ready with no full scan

```
Rescan (unchanged drive)     →  < 200 ms   (was 30–60 s)
Rescan after 100 changes     →  < 500 ms
```

### Arena allocator
Path strings are interned into a single contiguous `Vec<u8>` buffer (`PathArena`). Each entry is ~34 bytes instead of ~128 bytes for an owned `PathBuf`.

```
RAM for 1 M files:  ~1 GB (PathBuf)  →  ~140 MB (arena)   (7× less)
```

### Large-file hashing
| File size | Strategy |
|---|---|
| < 512 MiB | Streaming 64 KB buffered `BufReader` |
| ≥ 512 MiB | `memmap2::Mmap` — eliminates kernel→user copies (~30% faster) |
| ≥ 1 GiB | `blake3::Hasher::update_rayon()` — tree-parallel BLAKE3 across all cores (~4× faster) |

### Secondary index build (rayon)
`by_parent`, `by_size` maps are built with a parallel rayon `fold + reduce` — ~50 ms for 1 M entries vs ~200 ms single-threaded.

---

## Architecture

```
cleansweep/
├── src/                         # React 18 + TypeScript frontend
│   ├── pages/
│   │   ├── Scanner.tsx          # Junk scan, virtual list (react-window), undo/redo
│   │   ├── DiskMap.tsx          # Squarified treemap, canvas/SVG renderer, drill-down
│   │   ├── Organizer.tsx        # Rule editor, marketplace import/export
│   │   ├── Watcher.tsx          # Real-time folder watcher
│   │   ├── Scheduler.tsx        # Windows Task Scheduler UI
│   │   ├── Dashboard.tsx        # Disk usage ring, growth trends
│   │   └── Settings.tsx         # Theme, exclusions, idle/battery scheduling
│   ├── components/              # FileResultRow, ProgressBar, RuleModal, TopBar …
│   └── store/
│       ├── useAppStore.ts       # Zustand — scan results, settings
│       └── useOperationStore.ts # Undo/redo stack (Ctrl+Z / Ctrl+Y)
│
└── src-tauri/                   # Rust backend
    └── src/
        ├── models.rs            # ScanIndex, IndexEntry, NameArena, SharedScanIndex
        ├── lib.rs               # Tauri state registration + command handler list
        ├── cli.rs               # cleansweep-cli binary
        └── commands/
            ├── scanner.rs       # scan_for_junk, delete_files, 3-pass dedup, PathArena
            ├── mft.rs           # NTFS $MFT direct scan (WizTree-style)
            ├── treemap.rs       # Squarified layout, compute_treemap_cached
            ├── usn.rs           # USN journal snapshot + delta apply
            ├── snapshot.rs      # bincode serialize/deserialize ScanIndex
            ├── image_hash.rs    # dHash perceptual image hashing
            ├── audio_hash.rs    # Sub-band energy audio fingerprinting
            ├── lsh.rs           # LSH / Jaccard near-duplicate detection
            ├── organizer.rs     # Rule engine (move/copy/archive)
            ├── watcher.rs       # notify-based folder watcher
            ├── scheduler.rs     # Windows Task Scheduler XML API
            ├── growth.rs        # Disk growth time-series (SQLite)
            ├── cache.rs         # SQLite scan metadata cache
            ├── marketplace.rs   # Rule import/export (JSON + URL)
            ├── network.rs       # UNC / WSL2 / mapped drive detection
            ├── reports.rs       # CSV export, HTML report, toast notification
            └── system.rs        # Idle detection, battery check, VSS restore point
```

---

## Getting started

### Prerequisites

| Tool | Version | Notes |
|---|---|---|
| [Rust](https://rustup.rs/) | 1.85+ | `rustup update` |
| [Node.js](https://nodejs.org/) | 18+ | LTS recommended |
| [Tauri CLI prerequisites](https://v2.tauri.app/start/prerequisites/) | — | WebView2 on Windows |

### Install & run

```powershell
git clone https://github.com/Samuelabhinav37/cleansweep.git
cd cleansweep
npm install
npm run tauri dev
```

### Build for release

```powershell
npm run tauri build
# Installer: src-tauri/target/release/bundle/nsis/CleanSweep_0.1.0_x64-setup.exe
```

### CLI

The `cleansweep-cli` binary is built alongside the main app:

```powershell
# After `cargo build` in src-tauri/
.\src-tauri\target\debug\cleansweep-cli.exe --help

USAGE:
    cleansweep-cli <COMMAND>

COMMANDS:
    scan          Scan for junk files
    clean         Delete files from a list
    rules list    List organizer rules
    rules run     Run rules against a path
    rules export  Export rules to JSON
    rules import  Import rules from JSON file or URL
    trends        Show disk growth trends
    mft           Scan NTFS $MFT directly (requires admin)
    export-csv    Export last scan to CSV
```

---

## Disk Map keyboard shortcuts

| Key | Action |
|---|---|
| `←` / `↑` | Select previous sibling rectangle |
| `→` / `↓` | Select next sibling rectangle |
| `Enter` | Drill into selected directory |
| `Backspace` | Go up one level |
| Right-click | Open in Explorer · Copy path · Delete |

---

## Performance summary

| Operation | Before | After |
|---|---|---|
| First scan — 1 M files, admin | 30–60 s | **2–5 s** (MFT) |
| First scan — no admin | 30–60 s | 30–60 s (jwalk) |
| Disk Map after scan | 30–60 s | **< 100 ms** |
| Duplicate scan after scan | 30–60 s | **< 500 ms** |
| Rescan — snapshot + USN delta | 30–60 s | **< 200 ms** |
| RAM — 1 M files | ~1 GB | **~140 MB** |
| Hash — 10 GB file | ~20 s | **~5 s** (mmap + parallel) |

---

## Tech stack

| Layer | Technologies |
|---|---|
| UI | React 18, TypeScript, Tailwind CSS, Zustand, react-window, lucide-react |
| App shell | Tauri v2 (WebView2 on Windows) |
| Core engine | Rust 1.85, rayon, jwalk, blake3, memmap2 |
| Storage | SQLite (rusqlite, WAL mode), bincode snapshots |
| Filesystem | mft crate (NTFS $MFT), notify (watcher), winapi (USN journal, Task Scheduler) |
| Media | image 0.25 (dHash), symphonia (audio fingerprinting) |

---

## License

MIT

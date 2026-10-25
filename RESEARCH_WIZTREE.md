# WizTree — How It Works (Technical Deep-Dive)

> Research for the Blue Streak performance rewrite.
> Goal: understand exactly why WizTree scans a 1 TB drive in 1–2 seconds
> and why our current `jwalk`-based scanner takes 10+ minutes.

---

## The Core Answer in One Sentence

WizTree reads the **NTFS Master File Table (MFT) as a raw sequential byte stream** using
`FSCTL_ENUM_USN_RECORDS` to get filenames/parent-refs, then for each file reference it
calls `GetFileInformationByHandleEx` (or equivalent) in bulk to get sizes — all without
opening individual files or triggering per-entry ACL checks.

---

## Why jwalk / FindFirstFile Is Slow

Our current code calls `jwalk::WalkDir`, which under the hood calls:

```
FindFirstFileW(dir\*)     ← opens dir, reads first entry (1 kernel call)
FindNextFileW(handle)     ← reads next entry  (1 kernel call per file)
FindNextFileW(handle)     ← ...
CloseHandle(handle)
```

**Problems:**

| Problem | Cost |
|---|---|
| One `opendir()` per directory | 100k folders = 100k kernel transitions |
| Per-file ACL check on every `FindNextFile` | Verified against security descriptor each call |
| Random I/O pattern | Directories scattered across disk, cache misses |
| Sequential by path | Can't exploit disk read-ahead on MFT |

For a drive with 1M files spread across 100k directories: ~100k `opendir` calls ×
~10 `FindNextFile` calls each = ~1M kernel transitions, each with an ACL check.
On a spinning disk this is catastrophic. On SSD it still takes minutes because it's
CPU-bound on ACL verification in the kernel.

---

## What WizTree Actually Does

### Step 1 — Open the volume (not a directory)

```c
HANDLE h = CreateFileW(
    L"\\\\.\\C:",                    // volume path, NOT a directory
    GENERIC_READ,
    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
    NULL,
    OPEN_EXISTING,
    FILE_FLAG_BACKUP_SEMANTICS,      // required for volume/backup access
    NULL
);
```

Requires **Administrator** or `SeBackupPrivilege`. This opens the raw NTFS volume.

### Step 2 — Enumerate ALL MFT records in one sequential pass

```c
MFT_ENUM_DATA_V0 med = { 0, 0, MAXLONGLONG };

BYTE buf[1024 * 1024]; // 1 MB buffer
DWORD bytesReturned;

while (DeviceIoControl(
    h,
    FSCTL_ENUM_USN_RECORDS,   // 0x000900B3
    &med, sizeof(med),
    buf, sizeof(buf),
    &bytesReturned, NULL
)) {
    // First 8 bytes = next StartFileReferenceNumber (advance cursor)
    med.StartFileReferenceNumber = *(DWORDLONG*)buf;
    
    // Parse USN_RECORD_V2 structs starting at offset 8
    // Each record contains: file_ref, parent_ref, file_attributes, filename
    // Does NOT contain file size
}
```

This is a **single sequential read of the MFT**. The kernel reads the MFT
(which is stored as a contiguous file `$MFT` on disk) in large chunks directly
into your buffer. No per-file ACL checks. No random I/O. On an SSD this runs
at near-RAM speed — the MFT for a 1 TB drive is typically 200–500 MB.

**Speed:** Enumerating 1M file names takes ~0.5–1 second.

### Step 3 — Get file sizes

`USN_RECORD_V2` does NOT include file size. WizTree gets sizes one of two ways:

**Option A (what WizTree likely does):** After collecting all file refs,
call `NtQueryInformationFile` with `FileStandardInformation` in a tight loop.
This is cheaper than `stat()` because it skips the path lookup — you already
have the file reference number (which maps directly to an MFT entry index).

**Option B:** Parse the MFT `$DATA` attribute directly from raw sector reads.
This requires raw sector access and MFT boot sector parsing — very complex.

**Option C (fast fallback):** Use `GetFileAttributesExW` on the reconstructed
full path. This is one syscall per file but still cheaper than `FindFirstFile`
because it's a direct MFT lookup by path (no directory enumeration).

### Step 4 — Reconstruct the directory tree

Each `USN_RECORD_V2` contains `file_reference_number` and `parent_file_reference_number`.
WizTree builds a `HashMap<file_ref, (name, parent_ref, size, is_dir)>` and then
does a BFS from root (file ref = 5, the NTFS root directory `$MFT`) to
reconstruct full paths.

```
file_ref=5   → "C:\"         (root, parent=5)
file_ref=42  → "Windows"     (parent=5)  → "C:\Windows"
file_ref=137 → "System32"    (parent=42) → "C:\Windows\System32"
file_ref=999 → "notepad.exe" (parent=137)→ "C:\Windows\System32\notepad.exe"
```

This BFS is O(n) and runs in ~100ms for 1M files.

---

## The Problem With Our Current `mft.rs`

Our Phase 3 `scan_mft` command does the MFT enumeration correctly BUT:

1. **It is never called by `scan_for_junk`** — the main scanner still uses `jwalk`
2. **It returns `size_bytes: 0` for all files** — `USN_RECORD_V2` has no size field
3. **It is a separate command** — the UI calls `scan_for_junk`, not `scan_mft`

The `scan_for_junk` function that runs when the user clicks "Scan Now" still goes through:
```
parallel_walk() → jwalk::WalkDir → FindFirstFile per directory
```

This is why it takes 10 minutes. The fast MFT path exists but is disconnected.

---

## The Fix: Replace the jwalk Walk With MFT Enumeration

The rewrite plan:

1. **In `scan_for_junk`**: detect if running as admin → if yes, use MFT enumeration
   to get all file refs + names + parent refs in one pass. Then batch-stat the
   file refs to get sizes (using `GetFileAttributesExW` on the full paths, or
   `NtQueryInformationFile` by file ref). Fall back to `jwalk` if not admin.

2. **Size from MFT**: The fastest approach that doesn't require raw sector parsing:
   after building the path map, call `GetFileAttributesExW` **in parallel** (rayon)
   on the reconstructed paths. This is O(n) stat calls but they hit the MFT
   directly by path (no directory scan) and can be parallelised.

3. **Even faster**: Use `NtQueryDirectoryFile` (NT native API). This returns
   `FILE_FULL_DIR_INFORMATION` per directory, which includes size AND modified time
   in a single kernel call per directory entry — same cost as `FindFirstFile` but
   with far less overhead because it's the raw NT layer below Win32.

---

## WizTree Competitive Analysis

| Tool | API | Full scan (1M files, SSD) | Admin required |
|---|---|---|---|
| **WizTree** | `FSCTL_ENUM_USN_RECORDS` + size batch | **~1–2s** | Yes |
| TreeSize Pro | `NtQueryDirectoryFile` | ~10–30s | No |
| WinDirStat | `FindFirstFile` | ~2–5 min | No |
| **Blue Streak (current)** | `jwalk` → `FindFirstFile` | **~10 min** | No |
| **Blue Streak (target)** | MFT enumeration + parallel stat | **~2–5s** | Yes (fallback: 30s) |

---

## Why Not Just Use `NtQueryDirectoryFile`?

`NtQueryDirectoryFile` is the NT native API beneath `FindFirstFile`. It returns
`FILE_FULL_DIR_INFORMATION` which includes sizes. It's faster than `FindFirstFile`
because:
- No string conversion overhead
- Larger buffer fills possible
- No redundant handle work

TreeSize uses this and achieves ~10–30s. Still not WizTree speed because it's still
per-directory, not per-volume-in-one-pass.

For Blue Streak, this is a good **no-admin fallback**: replace `jwalk` with
`NtQueryDirectoryFile` and get ~3–10× speedup without requiring elevation.

---

## Implementation Plan for Blue Streak

### Path A: Admin path (WizTree speed)
1. Try `FSCTL_ENUM_USN_RECORDS` on `\\.\C:`
2. Build `HashMap<u64, (name, parent_u64, is_dir)>`
3. BFS to reconstruct full paths → `Vec<(String, bool)>`
4. Parallel `GetFileAttributesExW` on all file paths → sizes + mtimes
5. Total: ~2–5 seconds for 1M files

### Path B: Non-admin path (still much faster than current)
1. Replace `jwalk::WalkDir` with `NtQueryDirectoryFile` wrapper
2. Buffer 64KB per directory, get name+size+mtime in one call
3. Rayon parallel at depth ≤ 2 (top directories)
4. Total: ~15–30 seconds for 1M files (vs 10 minutes today)

### The critical bug to fix first
`scan_for_junk` must attempt the MFT path and fall back automatically.
Currently `scan_mft` is a separate command the user has to call manually.

---

## Sources

- Microsoft Docs: [FSCTL_ENUM_USN_RECORDS control code](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_enum_usn_records)
- Microsoft Docs: [USN_RECORD_V2 structure](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ns-winioctl-usn_record_v2)
- Microsoft Docs: [MFT_ENUM_DATA_V0 structure](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ns-winioctl-mft_enum_data_v0)
- WizTree website: [How WizTree Works](https://diskanalyzer.com/guide) — "WizTree reads the Master File Table (MFT) directly from the hard drive"
- WizTree FAQ: "WizTree requires administrator access to read the MFT directly"
- GitHub: [mft crate (Rust)](https://github.com/omerbenamram/mft) — pure Rust MFT parser
- Raymond Chen (Old New Thing): [How does Windows know which directories to include in a folder scan](https://devblogs.microsoft.com/oldnewthing/) — covers FindFirstFile overhead

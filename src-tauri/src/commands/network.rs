/// Network drive and WSL2 path support.
///
/// Detects:
///   - SMB/UNC paths (\\server\share) — serial scan with retry + exponential backoff
///   - WSL2 paths (\\wsl$\...) — detected as a special case of UNC
///   - Mapped network drives (X:) — detected via GetDriveType on Windows
///
/// For network paths, parallelism is reduced to 1 thread (serial) to avoid
/// overwhelming the remote server and tripping rate-limits.  A configurable
/// retry policy handles transient disconnects.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

// ─── path classification ──────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PathKind {
    /// Local filesystem
    Local,
    /// SMB/NFS network share via UNC path (\\server\share)
    NetworkUnc,
    /// WSL2 virtual filesystem (\\wsl$\distro or \\wsl.localhost\distro)
    Wsl2,
    /// Windows mapped network drive (e.g. Z:) — may be local or network
    MappedDrive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathInfo {
    pub path: String,
    pub kind: PathKind,
    /// Recommended parallelism (1 for network, rayon default for local)
    pub suggested_threads: u32,
    /// Human-readable warning (e.g. "Network path — scan will be slower")
    pub warning: Option<String>,
}

/// Classify a path and return scan recommendations.
#[tauri::command]
pub fn classify_path(path: String) -> PathInfo {
    let kind = detect_kind(&path);
    let (threads, warning) = match &kind {
        PathKind::NetworkUnc => (
            1,
            Some("Network path detected — serial scan to avoid overwhelming the server.".to_string()),
        ),
        PathKind::Wsl2 => (
            1,
            Some("WSL2 path detected — scanning via 9P bridge. Performance may be limited.".to_string()),
        ),
        PathKind::MappedDrive => {
            // Try to detect if the mapped drive is remote
            if is_remote_drive(&path) {
                (
                    2,
                    Some("Mapped network drive detected — reduced parallelism.".to_string()),
                )
            } else {
                (0, None) // 0 = use rayon default
            }
        }
        PathKind::Local => (0, None),
    };
    PathInfo { path, kind, suggested_threads: threads, warning }
}

fn detect_kind(path: &str) -> PathKind {
    // Normalise slashes
    let p = path.replace('/', "\\");

    if p.starts_with("\\\\wsl$\\") || p.starts_with("\\\\wsl.localhost\\") {
        return PathKind::Wsl2;
    }
    if p.starts_with("\\\\") {
        return PathKind::NetworkUnc;
    }

    // Single-letter drive: check with WinAPI if available
    if p.len() >= 2 && p.as_bytes()[1] == b':' {
        let drive_letter = &p[..3.min(p.len())]; // e.g. "C:\"
        if is_remote_drive(drive_letter) {
            return PathKind::MappedDrive;
        }
    }

    PathKind::Local
}

fn is_remote_drive(path: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;

        let root: Vec<u16> = OsStr::new(path)
            .encode_wide()
            .chain(std::iter::once(0u16))
            .collect();

        // GetDriveTypeW: DRIVE_REMOTE = 4
        let drive_type = unsafe {
            winapi::um::fileapi::GetDriveTypeW(root.as_ptr())
        };
        return drive_type == 4; // DRIVE_REMOTE
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
        false
    }
}

// ─── resilient walk with retry ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkScanResult {
    pub entries: Vec<NetworkEntry>,
    pub total_size_bytes: u64,
    pub errors: Vec<String>,
    pub retries: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkEntry {
    pub path: String,
    pub size_bytes: u64,
    pub is_dir: bool,
}

/// Walk a network path with retry-on-disconnect logic.
/// Uses serial (single-threaded) traversal to be a good neighbour.
///
/// `max_retries`: maximum number of reconnect attempts per directory (default 3).
/// `base_delay_ms`: initial backoff delay in ms, doubles each retry (default 500).
#[tauri::command]
pub fn scan_network_path(
    path: String,
    max_retries: Option<u32>,
    base_delay_ms: Option<u64>,
) -> Result<NetworkScanResult, String> {
    let max_retries = max_retries.unwrap_or(3);
    let base_delay = base_delay_ms.unwrap_or(500);

    let root = PathBuf::from(&path);
    if !root.exists() {
        return Err(format!("Path does not exist: {path}"));
    }

    let info = classify_path(path.clone());

    let mut entries: Vec<NetworkEntry> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut total_retries = 0u32;

    // Serial walk with retry per directory
    walk_with_retry(
        &root,
        &mut entries,
        &mut errors,
        &mut total_retries,
        max_retries,
        base_delay,
    );

    let total_size: u64 = entries.iter().map(|e| e.size_bytes).sum();

    if let Some(warn) = &info.warning {
        if !errors.is_empty() {
            errors.insert(0, warn.clone());
        }
    }

    Ok(NetworkScanResult {
        entries,
        total_size_bytes: total_size,
        errors,
        retries: total_retries,
    })
}

fn walk_with_retry(
    dir: &Path,
    entries: &mut Vec<NetworkEntry>,
    errors: &mut Vec<String>,
    total_retries: &mut u32,
    max_retries: u32,
    base_delay_ms: u64,
) {
    let mut attempt = 0u32;
    loop {
        match std::fs::read_dir(dir) {
            Ok(read_dir) => {
                for entry_res in read_dir {
                    let entry = match entry_res {
                        Ok(e) => e,
                        Err(e) => {
                            errors.push(format!("{}: {}", dir.display(), e));
                            continue;
                        }
                    };
                    let path = entry.path();
                    let meta = match entry.metadata() {
                        Ok(m) => m,
                        Err(e) => {
                            errors.push(format!("{}: {}", path.display(), e));
                            continue;
                        }
                    };
                    if meta.is_dir() {
                        entries.push(NetworkEntry {
                            path: path.to_string_lossy().into_owned(),
                            size_bytes: 0,
                            is_dir: true,
                        });
                        walk_with_retry(&path, entries, errors, total_retries, max_retries, base_delay_ms);
                    } else {
                        entries.push(NetworkEntry {
                            path: path.to_string_lossy().into_owned(),
                            size_bytes: meta.len(),
                            is_dir: false,
                        });
                    }
                }
                return; // success
            }
            Err(e) => {
                if attempt >= max_retries {
                    errors.push(format!(
                        "Failed after {} retries: {} — {}",
                        max_retries,
                        dir.display(),
                        e
                    ));
                    return;
                }
                attempt += 1;
                *total_retries += 1;
                let delay = base_delay_ms * (1u64 << attempt.min(5));
                std::thread::sleep(Duration::from_millis(delay));
            }
        }
    }
}

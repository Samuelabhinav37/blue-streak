use rusqlite::{params, Connection, OptionalExtension};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::models::{JunkCategory, JunkFile};

// ─── schema ───────────────────────────────────────────────────────────────────

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
PRAGMA synchronous  = NORMAL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS scans (
    scan_id    INTEGER PRIMARY KEY AUTOINCREMENT,
    root_path  TEXT    NOT NULL,
    timestamp  INTEGER NOT NULL,
    duration_ms INTEGER,
    total_files INTEGER
);

CREATE TABLE IF NOT EXISTS junk_files (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    scan_id     INTEGER NOT NULL REFERENCES scans(scan_id) ON DELETE CASCADE,
    path        TEXT    NOT NULL,
    size_bytes  INTEGER NOT NULL DEFAULT 0,
    category    TEXT    NOT NULL,
    age_days    INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_junk_scan ON junk_files(scan_id);

CREATE TABLE IF NOT EXISTS dir_sizes (
    path        TEXT    PRIMARY KEY,
    size_bytes  INTEGER NOT NULL DEFAULT 0,
    file_count  INTEGER NOT NULL DEFAULT 0,
    computed_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS usn_state (
    drive       TEXT    PRIMARY KEY,
    last_usn    INTEGER NOT NULL DEFAULT 0,
    journal_id  INTEGER NOT NULL DEFAULT 0,
    updated_at  INTEGER NOT NULL
);
";

// ─── public API ───────────────────────────────────────────────────────────────

pub struct ScanCache {
    conn: Connection,
}

impl ScanCache {
    /// Open (or create) the cache database at `path`.
    pub fn open(db_path: &std::path::Path) -> rusqlite::Result<Self> {
        if let Some(parent) = db_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(db_path)?;
        conn.execute_batch(SCHEMA)?;
        Ok(ScanCache { conn })
    }

    /// Borrow the underlying SQLite connection for modules that need to
    /// manage their own schema (e.g., growth trends, network drives).
    pub fn raw_conn(&self) -> &Connection {
        &self.conn
    }

    // ── scans ─────────────────────────────────────────────────────────────────

    /// Persist a completed scan result.
    pub fn save_scan(
        &self,
        root_path: &str,
        results: &[JunkFile],
        duration_ms: u64,
    ) -> rusqlite::Result<i64> {
        let now = unix_now();

        // Delete any previous scan for this root (keep only the latest)
        self.conn.execute(
            "DELETE FROM scans WHERE root_path = ?1",
            params![root_path],
        )?;

        self.conn.execute(
            "INSERT INTO scans (root_path, timestamp, duration_ms, total_files)
             VALUES (?1, ?2, ?3, ?4)",
            params![root_path, now as i64, duration_ms as i64, results.len() as i64],
        )?;
        let scan_id = self.conn.last_insert_rowid();

        // Batch insert via a single prepared statement
        let mut stmt = self.conn.prepare(
            "INSERT INTO junk_files (scan_id, path, size_bytes, category, age_days)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for f in results {
            stmt.execute(params![
                scan_id,
                f.path,
                f.size_bytes as i64,
                category_to_str(&f.category),
                f.age_days,
            ])?;
        }

        Ok(scan_id)
    }

    /// Load the latest cached scan for `root_path`.
    /// Returns `None` if there is no cached result.
    pub fn load_latest_scan(
        &self,
        root_path: &str,
    ) -> rusqlite::Result<Option<CachedScan>> {
        let row = self.conn.query_row(
            "SELECT scan_id, timestamp, duration_ms, total_files
             FROM scans WHERE root_path = ?1
             ORDER BY timestamp DESC LIMIT 1",
            params![root_path],
            |r| {
                Ok(CachedScanMeta {
                    scan_id: r.get(0)?,
                    timestamp: r.get::<_, i64>(1)? as u64,
                    duration_ms: r.get::<_, Option<i64>>(2)?.unwrap_or(0) as u64,
                    total_files: r.get::<_, Option<i64>>(3)?.unwrap_or(0) as u64,
                })
            },
        )
        .optional()?;

        let meta = match row {
            None => return Ok(None),
            Some(m) => m,
        };

        let mut stmt = self.conn.prepare(
            "SELECT path, size_bytes, category, age_days
             FROM junk_files WHERE scan_id = ?1",
        )?;
        let files: Vec<JunkFile> = stmt
            .query_map(params![meta.scan_id], |r| {
                Ok(JunkFile {
                    path: r.get(0)?,
                    size_bytes: r.get::<_, i64>(1)? as u64,
                    category: str_to_category(&r.get::<_, String>(2)?),
                    age_days: r.get(3)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(Some(CachedScan { meta, files }))
    }

    /// Return the Unix timestamp of the last scan for `root_path`, or 0.
    pub fn last_scan_timestamp(&self, root_path: &str) -> u64 {
        self.conn
            .query_row(
                "SELECT timestamp FROM scans WHERE root_path = ?1
                 ORDER BY timestamp DESC LIMIT 1",
                params![root_path],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0) as u64
    }

    /// Delete all cached scans (used when user forces a full fresh scan).
    pub fn invalidate(&self, root_path: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "DELETE FROM scans WHERE root_path = ?1",
            params![root_path],
        )?;
        Ok(())
    }

    // ── directory sizes ───────────────────────────────────────────────────────

    pub fn save_dir_size(&self, path: &str, size_bytes: u64, file_count: u64) -> rusqlite::Result<()> {
        let now = unix_now() as i64;
        self.conn.execute(
            "INSERT INTO dir_sizes (path, size_bytes, file_count, computed_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(path) DO UPDATE SET
               size_bytes  = excluded.size_bytes,
               file_count  = excluded.file_count,
               computed_at = excluded.computed_at",
            params![path, size_bytes as i64, file_count as i64, now],
        )?;
        Ok(())
    }

    pub fn get_dir_size(&self, path: &str) -> Option<(u64, u64)> {
        self.conn
            .query_row(
                "SELECT size_bytes, file_count FROM dir_sizes WHERE path = ?1",
                params![path],
                |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64)),
            )
            .ok()
    }

    // ── USN journal state (Windows incremental scanning) ─────────────────────

    pub fn save_usn_state(&self, drive: &str, last_usn: i64, journal_id: u64) -> rusqlite::Result<()> {
        let now = unix_now() as i64;
        self.conn.execute(
            "INSERT INTO usn_state (drive, last_usn, journal_id, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(drive) DO UPDATE SET
               last_usn   = excluded.last_usn,
               journal_id = excluded.journal_id,
               updated_at = excluded.updated_at",
            params![drive, last_usn, journal_id as i64, now],
        )?;
        Ok(())
    }

    pub fn load_usn_state(&self, drive: &str) -> Option<(i64, u64)> {
        self.conn
            .query_row(
                "SELECT last_usn, journal_id FROM usn_state WHERE drive = ?1",
                params![drive],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)? as u64)),
            )
            .ok()
    }
}

// ─── helpers ──────────────────────────────────────────────────────────────────

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn category_to_str(c: &JunkCategory) -> &'static str {
    match c {
        JunkCategory::Temp             => "temp",
        JunkCategory::Download         => "download",
        JunkCategory::Duplicate        => "duplicate",
        JunkCategory::EmptyDir         => "emptyDir",
        JunkCategory::Large            => "large",
        JunkCategory::PossibleDuplicate => "possibleDuplicate",
    }
}

fn str_to_category(s: &str) -> JunkCategory {
    match s {
        "download"          => JunkCategory::Download,
        "duplicate"         => JunkCategory::Duplicate,
        "emptyDir"          => JunkCategory::EmptyDir,
        "large"             => JunkCategory::Large,
        "possibleDuplicate" => JunkCategory::PossibleDuplicate,
        _                   => JunkCategory::Temp,
    }
}

// ─── returned types ───────────────────────────────────────────────────────────

pub struct CachedScanMeta {
    pub scan_id: i64,
    pub timestamp: u64,
    pub duration_ms: u64,
    pub total_files: u64,
}

pub struct CachedScan {
    pub meta: CachedScanMeta,
    pub files: Vec<JunkFile>,
}

// ─── Tauri commands ───────────────────────────────────────────────────────────

use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanCacheSummary {
    pub root_path: String,
    pub timestamp: u64,
    pub total_files: u64,
    pub duration_ms: u64,
    pub age_seconds: u64,
}

pub type SharedCache = std::sync::Mutex<ScanCache>;

pub fn new_shared_cache() -> SharedCache {
    let db_path = cache_db_path();
    let cache = ScanCache::open(&db_path)
        .expect("Failed to open scan cache database");
    std::sync::Mutex::new(cache)
}

fn cache_db_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("blue-streak")
        .join("cache.db")
}

#[tauri::command]
pub fn get_cache_summary(
    root_path: String,
    cache: State<'_, SharedCache>,
) -> Option<ScanCacheSummary> {
    let cache = cache.lock().ok()?;
    let ts = cache.last_scan_timestamp(&root_path);
    if ts == 0 {
        return None;
    }
    let now = unix_now();
    Some(ScanCacheSummary {
        root_path,
        timestamp: ts,
        total_files: 0,
        duration_ms: 0,
        age_seconds: now.saturating_sub(ts),
    })
}

#[tauri::command]
pub fn invalidate_cache(
    root_path: String,
    cache: State<'_, SharedCache>,
) -> Result<(), String> {
    let cache = cache.lock().map_err(|e| e.to_string())?;
    cache.invalidate(&root_path).map_err(|e| e.to_string())
}

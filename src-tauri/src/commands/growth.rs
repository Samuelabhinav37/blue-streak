/// Disk growth trend tracking.
///
/// Periodically snapshots directory sizes to SQLite and surfaces "this folder
/// grew 2 GB in the last week" insights on the Dashboard.
///
/// Schema: `dir_snapshots(path, size_bytes, file_count, captured_at)`
/// The existing `dir_sizes` table in cache.rs holds the *current* size; this
/// module adds time-series rows so we can compute deltas.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::State;

use crate::commands::cache::SharedCache;

// ─── SQLite schema (applied lazily on first use) ─────────────────────────────

const TREND_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS dir_snapshots (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    path        TEXT    NOT NULL,
    size_bytes  INTEGER NOT NULL DEFAULT 0,
    file_count  INTEGER NOT NULL DEFAULT 0,
    captured_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_snap_path_time
    ON dir_snapshots(path, captured_at);
";

fn ensure_trend_schema(conn: &Connection) {
    let _ = conn.execute_batch(TREND_SCHEMA);
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ─── public types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotPoint {
    /// Unix timestamp in seconds
    pub captured_at: u64,
    pub size_bytes: u64,
    pub file_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthTrend {
    pub path: String,
    /// All data points, oldest first
    pub points: Vec<SnapshotPoint>,
    /// Size delta between oldest and newest point in bytes (positive = grew)
    pub delta_bytes: i64,
    /// Number of days spanned by the points
    pub span_days: u64,
}

// ─── Tauri commands ───────────────────────────────────────────────────────────

/// Capture a size snapshot for each of `paths` right now.
/// Call this after every scan to build up the time-series.
#[tauri::command]
pub fn capture_growth_snapshot(
    paths: Vec<String>,
    cache: State<'_, SharedCache>,
) -> Result<u32, String> {
    let cache = cache.lock().map_err(|e| e.to_string())?;
    let conn = cache.raw_conn();
    ensure_trend_schema(conn);

    let now = unix_now() as i64;
    let mut count = 0u32;

    for root in &paths {
        let p = std::path::Path::new(root);
        if !p.exists() {
            continue;
        }
        let (size_bytes, file_count) = quick_dir_size(p);

        conn.execute(
            "INSERT INTO dir_snapshots (path, size_bytes, file_count, captured_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![root, size_bytes as i64, file_count as i64, now],
        )
        .map_err(|e| e.to_string())?;
        count += 1;
    }

    Ok(count)
}

/// Return growth trend data for `paths` over the last `days` days.
/// Returns one `GrowthTrend` per path that has at least 2 data points.
#[tauri::command]
pub fn get_growth_trends(
    paths: Vec<String>,
    days: Option<u64>,
    cache: State<'_, SharedCache>,
) -> Result<Vec<GrowthTrend>, String> {
    let days = days.unwrap_or(30);
    let since = unix_now().saturating_sub(days * 86_400);

    let cache = cache.lock().map_err(|e| e.to_string())?;
    let conn = cache.raw_conn();
    ensure_trend_schema(conn);

    let mut trends: Vec<GrowthTrend> = Vec::new();

    for path in &paths {
        let mut stmt = conn
            .prepare(
                "SELECT captured_at, size_bytes, file_count
                 FROM dir_snapshots
                 WHERE path = ?1 AND captured_at >= ?2
                 ORDER BY captured_at ASC",
            )
            .map_err(|e| e.to_string())?;

        let points: Vec<SnapshotPoint> = stmt
            .query_map(params![path, since as i64], |row| {
                Ok(SnapshotPoint {
                    captured_at: row.get::<_, i64>(0)? as u64,
                    size_bytes:  row.get::<_, i64>(1)? as u64,
                    file_count:  row.get::<_, i64>(2)? as u64,
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        if points.len() < 2 {
            continue;
        }

        let oldest = &points[0];
        let newest = &points[points.len() - 1];
        let delta_bytes = newest.size_bytes as i64 - oldest.size_bytes as i64;
        let span_days = (newest.captured_at.saturating_sub(oldest.captured_at)) / 86_400;

        trends.push(GrowthTrend {
            path: path.clone(),
            points,
            delta_bytes,
            span_days,
        });
    }

    // Sort by absolute delta descending (biggest growers first)
    trends.sort_by_key(|t| std::cmp::Reverse(t.delta_bytes.unsigned_abs()));

    Ok(trends)
}

/// Prune snapshots older than `keep_days` to avoid unbounded growth.
#[tauri::command]
pub fn prune_growth_snapshots(
    keep_days: Option<u64>,
    cache: State<'_, SharedCache>,
) -> Result<u64, String> {
    let keep_days = keep_days.unwrap_or(90);
    let cutoff = unix_now().saturating_sub(keep_days * 86_400) as i64;

    let cache = cache.lock().map_err(|e| e.to_string())?;
    let conn = cache.raw_conn();
    ensure_trend_schema(conn);

    let deleted = conn
        .execute(
            "DELETE FROM dir_snapshots WHERE captured_at < ?1",
            params![cutoff],
        )
        .map_err(|e| e.to_string())?;

    Ok(deleted as u64)
}

// ─── helper ───────────────────────────────────────────────────────────────────

fn quick_dir_size(path: &std::path::Path) -> (u64, u64) {
    let mut size = 0u64;
    let mut count = 0u64;
    for entry in jwalk::WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if let Ok(meta) = entry.metadata() {
            if meta.is_file() {
                size += meta.len();
                count += 1;
            }
        }
    }
    (size, count)
}

/// Squarified Treemap layout algorithm.
///
/// Reference: Bruls, Huizing, van Wijk (2000) "Squarified Treemaps"
/// https://www.win.tue.nl/~vanwijk/stm.pdf
///
/// Produces a list of rectangles where area ∝ size, with aspect ratios
/// kept as close to 1 (square) as possible for readability.

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tauri::State;
use crate::models::SharedScanIndex;

// ─── public types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreemapNode {
    pub label: String,
    pub path: String,
    pub size_bytes: u64,
    pub file_count: u64,
    /// Layout rectangle in the canvas coordinate space
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// Hex colour derived from size bucket
    pub color: String,
    pub depth: u32,
    pub children: Vec<TreemapNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreemapInput {
    pub label: String,
    pub path: String,
    pub size_bytes: u64,
    pub file_count: u64,
    pub children: Vec<TreemapInput>,
}

// ─── layout algorithm ─────────────────────────────────────────────────────────

/// Lay out `items` into the rectangle `(x, y, w, h)` using the squarified algorithm.
/// Items must already be sorted by size descending when passed in.
pub fn squarify(
    items: &[(String, String, u64, u64)], // (label, path, size, file_count)
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    depth: u32,
) -> Vec<TreemapNode> {
    if items.is_empty() || w <= 0.0 || h <= 0.0 {
        return vec![];
    }

    let total: u64 = items.iter().map(|(_, _, s, _)| s).sum();
    if total == 0 {
        return vec![];
    }

    let mut result: Vec<TreemapNode> = Vec::new();
    let mut remaining = items;
    let mut rx = x;
    let mut ry = y;
    let mut rw = w;
    let mut rh = h;

    while !remaining.is_empty() {
        // Determine layout direction: lay along the shorter side
        let horizontal = rw >= rh;
        let strip_size: u64 = remaining.iter().map(|(_, _, s, _)| s).sum();

        // Find how many items to fit in the current strip to minimise aspect ratio
        let mut best_end = 1usize;
        let mut best_ratio = f64::MAX;

        let mut strip_sum = 0u64;
        for (i, (_, _, size, _)) in remaining.iter().enumerate() {
            strip_sum += size;
            let ratio = worst_ratio(remaining[..=i].iter().map(|(_, _, s, _)| *s),
                                    strip_sum, strip_size, rw, rh, horizontal);
            if ratio < best_ratio {
                best_ratio = ratio;
                best_end = i + 1;
            } else if i > 0 {
                // Ratio is getting worse — stop
                break;
            }
        }

        // Place the chosen strip
        let strip_items = &remaining[..best_end];
        remaining = &remaining[best_end..];

        let strip_sum: u64 = strip_items.iter().map(|(_, _, s, _)| s).sum();
        let strip_fraction = strip_sum as f64 / strip_size as f64;

        let (strip_w, strip_h) = if horizontal {
            (rw * strip_fraction, rh)
        } else {
            (rw, rh * strip_fraction)
        };

        let mut cursor = if horizontal { ry } else { rx };

        for (label, path, size, file_count) in strip_items {
            let item_fraction = *size as f64 / strip_sum as f64;
            let (item_w, item_h, item_x, item_y) = if horizontal {
                let h_item = strip_h * item_fraction;
                (strip_w, h_item, rx, cursor)
            } else {
                let w_item = strip_w * item_fraction;
                (w_item, strip_h, cursor, ry)
            };

            result.push(TreemapNode {
                label: label.clone(),
                path: path.clone(),
                size_bytes: *size,
                file_count: *file_count,
                x: item_x,
                y: item_y,
                width: item_w.max(0.0),
                height: item_h.max(0.0),
                color: size_color(*size),
                depth,
                children: vec![],
            });

            cursor += if horizontal { item_h } else { item_w };
        }

        // Advance remaining canvas
        if horizontal {
            rx += strip_w;
            rw -= strip_w;
        } else {
            ry += strip_h;
            rh -= strip_h;
        }
    }

    result
}

fn worst_ratio<I: Iterator<Item = u64>>(
    sizes: I,
    strip_sum: u64,
    total_sum: u64,
    w: f64,
    h: f64,
    horizontal: bool,
) -> f64 {
    if strip_sum == 0 || total_sum == 0 { return f64::MAX; }

    let strip_fraction = strip_sum as f64 / total_sum as f64;
    let (strip_dim, perp_dim) = if horizontal {
        (w * strip_fraction, h)
    } else {
        (h * strip_fraction, w)
    };

    if strip_dim <= 0.0 { return f64::MAX; }

    sizes
        .map(|s| {
            let item_frac = s as f64 / strip_sum as f64;
            let item_len = perp_dim * item_frac;
            if item_len <= 0.0 {
                return f64::MAX;
            }
            let ratio = if strip_dim > item_len {
                strip_dim / item_len
            } else {
                item_len / strip_dim
            };
            ratio
        })
        .fold(0.0f64, f64::max)
}

/// Continuous HSL colour gradient: green (small) → yellow → red (large).
/// Matches the client-side `sizeColor()` function in DiskMap.tsx so that
/// server-pre-computed colours (used in the `TreemapNode` struct) are
/// consistent with the canvas/SVG renderer overrides.
///
/// Mapping: 0 GB → hue 120 (green), ≥1 GB → hue 0 (red), linearly interpolated.
fn size_color(size_bytes: u64) -> String {
    let gb = size_bytes as f64 / (1024.0_f64.powi(3));
    let hue = (120.0 - gb * 120.0).max(0.0).min(120.0).round() as u32;
    format!("hsl({},65%,45%)", hue)
}

// ─── directory size accumulation ──────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct DirEntry {
    pub path: String,
    pub name: String,
    pub size_bytes: u64,
    pub file_count: u64,
    pub children: Vec<DirEntry>,
}

/// Walk `root` and compute recursive directory sizes bottom-up.
/// Returns a tree structure suitable for treemap layout.
pub fn build_dir_tree(root: &Path, max_depth: u32) -> DirEntry {
    build_dir_tree_inner(root, 0, max_depth)
}

fn build_dir_tree_inner(path: &Path, depth: u32, max_depth: u32) -> DirEntry {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();

    let mut size_bytes = 0u64;
    let mut file_count = 0u64;
    let mut children: Vec<DirEntry> = Vec::new();

    if let Ok(entries) = std::fs::read_dir(path) {
        let entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();

        if depth < max_depth {
            // Recurse in parallel at top levels
            if depth < 2 {
                let sub: Vec<DirEntry> = entries
                    .par_iter()
                    .filter_map(|e| {
                        let p = e.path();
                        let meta = e.metadata().ok()?;
                        if meta.is_dir() {
                            Some(build_dir_tree_inner(&p, depth + 1, max_depth))
                        } else {
                            // Leaf file — count directly
                            Some(DirEntry {
                                path: p.to_string_lossy().into_owned(),
                                name: p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string(),
                                size_bytes: meta.len(),
                                file_count: 1,
                                children: vec![],
                            })
                        }
                    })
                    .collect();
                for child in sub {
                    size_bytes += child.size_bytes;
                    file_count += child.file_count;
                    if depth + 1 < max_depth || child.children.is_empty() {
                        children.push(child);
                    } else {
                        children.push(child);
                    }
                }
            } else {
                // Sequential at deeper levels to avoid thread pool thrashing
                for entry in &entries {
                    let p = entry.path();
                    if let Ok(meta) = entry.metadata() {
                        if meta.is_dir() {
                            let child = build_dir_tree_inner(&p, depth + 1, max_depth);
                            size_bytes += child.size_bytes;
                            file_count += child.file_count;
                            children.push(child);
                        } else {
                            size_bytes += meta.len();
                            file_count += 1;
                        }
                    }
                }
            }
        } else {
            // At max depth: just sum files, no children
            for entry in &entries {
                if let Ok(meta) = entry.metadata() {
                    if meta.is_file() {
                        size_bytes += meta.len();
                        file_count += 1;
                    } else if meta.is_dir() {
                        // Quick recursive sum without building tree
                        let (s, c) = quick_size(&entry.path());
                        size_bytes += s;
                        file_count += c;
                    }
                }
            }
        }
    }

    DirEntry {
        path: path.to_string_lossy().into_owned(),
        name,
        size_bytes,
        file_count,
        children,
    }
}

fn quick_size(path: &Path) -> (u64, u64) {
    use jwalk::WalkDir;
    let mut size = 0u64;
    let mut count = 0u64;
    for entry in WalkDir::new(path)
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

fn dir_entry_to_items(entry: &DirEntry) -> Vec<(String, String, u64, u64)> {
    let mut items: Vec<(String, String, u64, u64)> = entry
        .children
        .iter()
        .map(|c| (c.name.clone(), c.path.clone(), c.size_bytes, c.file_count))
        .collect();
    // Sort descending by size for best squarified layout
    items.sort_by(|a, b| b.2.cmp(&a.2));
    items
}

fn layout_tree(entry: &DirEntry, x: f64, y: f64, w: f64, h: f64, depth: u32) -> TreemapNode {
    let items = dir_entry_to_items(entry);
    let mut children = squarify(&items, x, y, w, h, depth + 1);

    // Recursively layout children that themselves have sub-children
    if depth < 2 {
        let entry_children_map: std::collections::HashMap<&str, &DirEntry> = entry
            .children
            .iter()
            .map(|c| (c.path.as_str(), c))
            .collect();

        for node in &mut children {
            if let Some(child_entry) = entry_children_map.get(node.path.as_str()) {
                if !child_entry.children.is_empty() && node.width > 20.0 && node.height > 20.0 {
                    let sub = layout_tree(
                        child_entry,
                        node.x + 1.0,
                        node.y + 1.0,
                        (node.width - 2.0).max(0.0),
                        (node.height - 2.0).max(0.0),
                        depth + 1,
                    );
                    node.children = sub.children;
                }
            }
        }
    }

    TreemapNode {
        label: entry.name.clone(),
        path: entry.path.clone(),
        size_bytes: entry.size_bytes,
        file_count: entry.file_count,
        x,
        y,
        width: w,
        height: h,
        color: size_color(entry.size_bytes),
        depth,
        children,
    }
}

// ─── Tauri commands ────────────────────────────────────────────────────────────

/// Compute a squarified treemap for `root_path` up to `max_depth` directory levels.
/// Runs the directory walk on a blocking thread so the async executor stays free.
#[tauri::command]
pub async fn compute_treemap(
    root_path: String,
    max_depth: Option<u32>,
    canvas_width: Option<f64>,
    canvas_height: Option<f64>,
) -> Result<TreemapNode, String> {
    if !std::path::Path::new(&root_path).exists() {
        return Err(format!("Path does not exist: {}", root_path));
    }

    let depth = max_depth.unwrap_or(3).min(5);
    let w = canvas_width.unwrap_or(800.0);
    let h = canvas_height.unwrap_or(600.0);

    tokio::task::spawn_blocking(move || {
        let tree = build_dir_tree(std::path::Path::new(&root_path), depth);
        Ok(layout_tree(&tree, 0.0, 0.0, w, h, 0))
    })
    .await
    .map_err(|e| format!("Treemap task panicked: {}", e))?
}

/// Phase 5 (Milestone B2): compute treemap from the shared in-memory scan index.
///
/// If the index covers `root_path` (i.e. a scan was already performed), this
/// returns in <100 ms without any filesystem I/O.
/// Falls back to the filesystem walk if the index doesn't cover `root_path`.
#[tauri::command]
pub async fn compute_treemap_cached(
    root_path: String,
    canvas_width: Option<f64>,
    canvas_height: Option<f64>,
    scan_index: State<'_, SharedScanIndex>,
) -> Result<TreemapNode, String> {
    let w = canvas_width.unwrap_or(800.0);
    let h = canvas_height.unwrap_or(600.0);

    // Try the in-memory index first (zero I/O — no blocking needed)
    {
        let index = scan_index.read().map_err(|e| e.to_string())?;
        if index.is_valid_for(&root_path) {
            return build_treemap_from_index(&index, &root_path, w, h);
        }
    }

    // Index miss — fall back to blocking filesystem walk on a worker thread
    compute_treemap(root_path, Some(3), Some(w), Some(h)).await
}

/// Build a treemap by aggregating sizes from the shared `ScanIndex`.
///
/// Uses path-prefix matching to find immediate children of `root_path`,
/// then sums file sizes under each child directory.  Works for both the
/// MFT fast-path (real file_refs) and the jwalk fallback (all file_refs = 0).
fn build_treemap_from_index(
    index: &crate::models::ScanIndex,
    root_path: &str,
    w: f64,
    h: f64,
) -> Result<TreemapNode, String> {
    use std::collections::HashMap as HMap;

    // Normalise root so we can do prefix matching reliably
    let root_norm = root_path.trim_end_matches(['\\', '/']).to_ascii_lowercase();
    let sep = if root_norm.contains('\\') { '\\' } else { '/' };

    // key = direct child path (e.g. "C:\Users\Alice")
    // value = (total_size_bytes, file_count)
    let mut child_map: HMap<String, (u64, u64)> = HMap::new();

    for entry in &index.entries {
        if entry.is_dir() { continue; }
        if entry.size == 0 { continue; }

        let path_lc = entry.full_path.to_ascii_lowercase();
        if !path_lc.starts_with(&root_norm) { continue; }

        // Get the portion of the path after the root prefix
        let rest = &entry.full_path[root_norm.len()..].trim_start_matches(['\\', '/']);
        if rest.is_empty() { continue; }

        // Direct child = first path component after root
        let child_name: &str = rest.split(sep).next().unwrap_or(rest);
        if child_name.is_empty() { continue; }

        let child_path = format!("{}{}{}", root_path.trim_end_matches(['\\', '/']), sep, child_name);

        let e = child_map.entry(child_path).or_insert((0, 0));
        e.0 += entry.size;
        e.1 += 1;
    }

    let mut items: Vec<(String, String, u64, u64)> = child_map
        .into_iter()
        .filter(|(_, (size, _))| *size > 0)
        .map(|(path, (size, count))| {
            let label = path.rsplit(['\\', '/']).next().unwrap_or(&path).to_owned();
            (label, path, size, count)
        })
        .collect();

    // Merge items < 2% of total into an "Other" bucket (Phase 5 E5)
    let total_size: u64 = items.iter().map(|(_, _, s, _)| s).sum();
    let threshold = total_size / 50; // 2%
    let mut other_size = 0u64;
    let mut other_count = 0u64;
    items.retain(|(_, _, size, count)| {
        if *size < threshold {
            other_size += size;
            other_count += count;
            false
        } else {
            true
        }
    });
    if other_size > 0 {
        items.push(("Other".to_owned(), root_path.to_owned(), other_size, other_count));
    }

    // Sort descending for optimal squarified layout
    items.sort_by(|a, b| b.2.cmp(&a.2));

    let children = squarify(&items, 0.0, 0.0, w, h, 1);

    Ok(TreemapNode {
        label: root_path.rsplit(['\\', '/']).next().unwrap_or(root_path).to_owned(),
        path: root_path.to_owned(),
        size_bytes: total_size,
        file_count: items.iter().map(|(_, _, _, c)| c).sum(),
        x: 0.0,
        y: 0.0,
        width: w,
        height: h,
        color: size_color(total_size),
        depth: 0,
        children,
    })
}

/// Phase 5: return a summary of the current `SharedScanIndex` state.
/// Used by the frontend to decide whether to show "Scan first" or "View DiskMap".
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanIndexSummary {
    pub entry_count: usize,
    pub root: String,
    pub scanned_at: u64,
    pub is_ready: bool,
}

#[tauri::command]
pub fn get_scan_index_summary(
    scan_index: State<SharedScanIndex>,
) -> Result<ScanIndexSummary, String> {
    let index = scan_index.read().map_err(|e| e.to_string())?;
    Ok(ScanIndexSummary {
        entry_count: index.entries.len(),
        root: index.root.clone(),
        scanned_at: index.scanned_at,
        is_ready: !index.entries.is_empty(),
    })
}

/// LSH (Locality-Sensitive Hashing) near-duplicate file detection.
///
/// Catches files that are "close but not identical" — a document saved twice
/// with minor edits, a photo processed with different settings, a binary
/// slightly modified between builds.
///
/// Algorithm:
///   1. Split each file into 8 KiB blocks.
///   2. Hash each block with xxHash64 (fast, non-cryptographic).
///   3. Represent the file as a set of block hashes (its "fingerprint").
///   4. Compare fingerprints via Jaccard similarity: |A ∩ B| / |A ∪ B|.
///   5. Files with Jaccard ≥ threshold (default 0.80) are "near-duplicates".
///
/// Complexity: O(n²) pair comparisons on fingerprints — fast because
/// fingerprints are tiny (a 1 MB file = 128 u64s).
///
/// For very large collections a MinHash + LSH band structure can reduce this
/// to O(n), but O(n²) is fine for < 100K files.

use std::io::Read;
use std::path::Path;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use twox_hash::XxHash64;
use std::hash::Hasher;

use crate::models::{JunkCategory, JunkFile};

/// Default Jaccard similarity threshold (0.0–1.0).
/// 0.80 = at least 80% of 8 KiB blocks are shared → strong near-duplicate signal.
pub const DEFAULT_JACCARD_THRESHOLD: f32 = 0.80;

/// Block size for fingerprinting.
const BLOCK_SIZE: usize = 8 * 1024; // 8 KiB

// ─── core fingerprinting ──────────────────────────────────────────────────────

/// Compute an LSH fingerprint for `path`: a sorted Vec of xxHash64 block hashes.
/// Returns `None` on I/O error or empty file.
pub fn lsh_fingerprint(path: &Path) -> Option<Vec<u64>> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut buf = [0u8; BLOCK_SIZE];
    let mut hashes: Vec<u64> = Vec::new();
    // xxHash seed = 0 (deterministic, reproducible)
    const SEED: u64 = 0;

    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let mut h = XxHash64::with_seed(SEED);
                h.write(&buf[..n]);
                hashes.push(h.finish());
            }
            Err(_) => return None,
        }
    }

    if hashes.is_empty() {
        return None;
    }

    hashes.sort_unstable(); // sort for fast intersection via merge
    Some(hashes)
}

/// Jaccard similarity between two sorted fingerprint slices.
/// Uses a merge-based intersection/union for O(n+m) complexity.
pub fn jaccard(a: &[u64], b: &[u64]) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let mut ai = 0usize;
    let mut bi = 0usize;
    let mut intersection = 0u64;
    let mut union = 0u64;

    while ai < a.len() && bi < b.len() {
        union += 1;
        match a[ai].cmp(&b[bi]) {
            std::cmp::Ordering::Equal => {
                intersection += 1;
                ai += 1;
                bi += 1;
            }
            std::cmp::Ordering::Less  => { ai += 1; }
            std::cmp::Ordering::Greater => { bi += 1; }
        }
    }
    // Remaining elements contribute to union only
    union += (a.len() - ai) as u64 + (b.len() - bi) as u64;

    if union == 0 { 1.0 } else { intersection as f32 / union as f32 }
}

// ─── public types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NearDuplicatePair {
    pub path_a: String,
    pub path_b: String,
    /// Jaccard similarity (0.0–1.0)
    pub similarity: f32,
    pub size_a: u64,
    pub size_b: u64,
}

// ─── Tauri command ─────────────────────────────────────────────────────────────

/// Scan `paths` for near-duplicate files using LSH fingerprinting.
///
/// Returns pairs of files with similarity ≥ `threshold` (default 0.80).
/// The smaller file in each pair is the suggested deletion candidate.
///
/// Skips files < 4 KiB (too small for meaningful block comparison) and files
/// > 512 MiB (too large for in-memory fingerprint collection; use exact hash instead).
#[tauri::command]
pub async fn scan_near_duplicates(
    paths: Vec<String>,
    threshold: Option<f32>,
) -> Result<Vec<NearDuplicatePair>, String> {
    let threshold = threshold.unwrap_or(DEFAULT_JACCARD_THRESHOLD).clamp(0.0, 1.0);

    const MIN_SIZE: u64 =   4 * 1024;          // 4 KiB — too small to fingerprint
    const MAX_SIZE: u64 = 512 * 1024 * 1024;   // 512 MiB — use exact hash instead

    // Collect candidate file paths
    let candidates: Vec<std::path::PathBuf> = paths
        .iter()
        .flat_map(|root| {
            jwalk::WalkDir::new(root)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| {
                    if !e.path().is_file() { return false; }
                    let sz = e.metadata().map(|m| m.len()).unwrap_or(0);
                    sz >= MIN_SIZE && sz <= MAX_SIZE
                })
                .map(|e| e.path())
                .collect::<Vec<_>>()
        })
        .collect();

    if candidates.len() < 2 {
        return Ok(vec![]);
    }

    // Compute fingerprints in parallel
    let fingerprints: Vec<(std::path::PathBuf, Vec<u64>)> = candidates
        .par_iter()
        .filter_map(|p| lsh_fingerprint(p).map(|fp| (p.clone(), fp)))
        .collect();

    // O(n²) pair comparison — fast because fingerprints are small (~128 u64s for 1 MB)
    let pairs: Vec<NearDuplicatePair> = fingerprints
        .par_iter()
        .enumerate()
        .flat_map(|(i, (path_a, fp_a))| {
            let mut local: Vec<NearDuplicatePair> = Vec::new();
            for (path_b, fp_b) in &fingerprints[i + 1..] {
                let sim = jaccard(fp_a, fp_b);
                if sim >= threshold {
                    let size_a = path_a.metadata().map(|m| m.len()).unwrap_or(0);
                    let size_b = path_b.metadata().map(|m| m.len()).unwrap_or(0);
                    local.push(NearDuplicatePair {
                        path_a: path_a.to_string_lossy().into_owned(),
                        path_b: path_b.to_string_lossy().into_owned(),
                        similarity: sim,
                        size_a,
                        size_b,
                    });
                }
            }
            local
        })
        .collect();

    Ok(pairs)
}

/// Convert near-duplicate pairs into `JunkFile` entries (smaller of each pair).
pub fn near_dupe_pairs_to_junk(pairs: &[NearDuplicatePair]) -> Vec<JunkFile> {
    pairs
        .iter()
        .map(|pair| {
            let (junk_path, junk_size) = if pair.size_a <= pair.size_b {
                (pair.path_a.clone(), pair.size_a)
            } else {
                (pair.path_b.clone(), pair.size_b)
            };
            JunkFile {
                path: junk_path,
                size_bytes: junk_size,
                category: JunkCategory::PossibleDuplicate,
                age_days: 0,
            }
        })
        .collect()
}

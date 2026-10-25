/// Perceptual image hashing (dHash) for finding similar/duplicate images
/// regardless of resize, re-compression, or minor edits.
///
/// Algorithm (dHash):
///   1. Resize image to 9×8 pixels (grayscale)
///   2. For each row, compare each pixel to the one on its right (8 comparisons × 8 rows = 64 bits)
///   3. Output: one u64 fingerprint
///
/// Two images with Hamming distance ≤ 10 are considered similar (≥ 95% match).

use std::path::Path;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

const DHASH_WIDTH: u32 = 9;
const DHASH_HEIGHT: u32 = 8;

/// Default Hamming distance threshold for "similar" images (0–64).
/// 10 = ~95% match (same photo, different resize/compression).
pub const DEFAULT_THRESHOLD: u32 = 10;

pub static IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "bmp", "tiff", "tif",
];

// ─── core hash ────────────────────────────────────────────────────────────────

/// Compute the dHash fingerprint of an image file.
/// Returns `None` if the file cannot be opened or decoded.
pub fn dhash(path: &Path) -> Option<u64> {
    use image::imageops::FilterType;
    use image::GenericImageView;

    let img = image::open(path).ok()?;
    // Resize to 9×8 grayscale
    let small = img
        .resize_exact(DHASH_WIDTH, DHASH_HEIGHT, FilterType::Lanczos3)
        .grayscale();

    let mut hash: u64 = 0;
    let mut bit: u32 = 0;

    for row in 0..DHASH_HEIGHT {
        for col in 0..DHASH_WIDTH - 1 {
            let left = small.get_pixel(col, row)[0] as u64;
            let right = small.get_pixel(col + 1, row)[0] as u64;
            if right > left {
                hash |= 1u64 << bit;
            }
            bit += 1;
        }
    }

    Some(hash)
}

/// Hamming distance between two dHash fingerprints (0 = identical, 64 = opposite).
#[inline]
pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| IMAGE_EXTENSIONS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

// ─── Tauri commands ───────────────────────────────────────────────────────────

use crate::models::{JunkCategory, JunkFile};
use jwalk::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimilarImagePair {
    pub path_a: String,
    pub path_b: String,
    pub distance: u32,
    pub size_a: u64,
    pub size_b: u64,
}

/// Scan `roots` for similar images and return pairs + JunkFile entries
/// for the lower-quality (smaller) duplicate in each pair.
#[tauri::command]
pub async fn scan_similar_images(
    paths: Vec<String>,
    threshold: Option<u32>,
) -> Result<Vec<SimilarImagePair>, String> {
    let threshold = threshold.unwrap_or(DEFAULT_THRESHOLD);

    // Collect all image paths
    let image_paths: Vec<std::path::PathBuf> = paths
        .iter()
        .flat_map(|root| {
            WalkDir::new(root)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_file() && is_image(&e.path()))
                .map(|e| e.path())
                .collect::<Vec<_>>()
        })
        .collect();

    if image_paths.is_empty() {
        return Ok(vec![]);
    }

    // Hash all images in parallel — dHash is fast (~2ms per image)
    let hashed: Vec<(std::path::PathBuf, u64)> = image_paths
        .par_iter()
        .filter_map(|p| dhash(p).map(|h| (p.clone(), h)))
        .collect();

    // O(n²) comparison — acceptable for < 50K images
    // For larger sets a BK-tree or LSH index would be needed
    let pairs: Vec<SimilarImagePair> = hashed
        .par_iter()
        .enumerate()
        .flat_map(|(i, (path_a, hash_a))| {
            let mut local: Vec<SimilarImagePair> = Vec::new();
            for (path_b, hash_b) in &hashed[i + 1..] {
                let dist = hamming(*hash_a, *hash_b);
                if dist <= threshold {
                    let size_a = path_a.metadata().map(|m| m.len()).unwrap_or(0);
                    let size_b = path_b.metadata().map(|m| m.len()).unwrap_or(0);
                    local.push(SimilarImagePair {
                        path_a: path_a.to_string_lossy().into_owned(),
                        path_b: path_b.to_string_lossy().into_owned(),
                        distance: dist,
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

/// Convert similar-image pairs into `JunkFile` entries (the smaller of each pair).
pub fn pairs_to_junk(pairs: &[SimilarImagePair]) -> Vec<JunkFile> {
    pairs
        .iter()
        .map(|pair| {
            // Keep the larger file; mark the smaller as junk
            let (junk_path, junk_size) = if pair.size_a <= pair.size_b {
                (pair.path_a.clone(), pair.size_a)
            } else {
                (pair.path_b.clone(), pair.size_b)
            };
            JunkFile {
                path: junk_path,
                size_bytes: junk_size,
                category: JunkCategory::Duplicate,
                age_days: 0,
            }
        })
        .collect()
}

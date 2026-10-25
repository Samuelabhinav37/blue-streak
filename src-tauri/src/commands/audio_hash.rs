/// Acoustic audio fingerprinting for duplicate detection across formats/bitrates.
///
/// Algorithm: Sub-band energy fingerprint
///   1. Decode audio to raw PCM samples using `symphonia` (supports MP3, FLAC,
///      AAC, OGG Vorbis, WAV, M4A).
///   2. Downsample to mono 11 025 Hz (telephony rate — enough for fingerprinting).
///   3. Divide samples into 512-sample frames (≈46 ms each).
///   4. For each frame, compute 32-band energy spectrum via a simple DFT.
///   5. For each of the 32 bands, compare energy to the previous frame.
///      If energy increased → set corresponding bit. Output: one u32 per frame.
///   6. Fingerprint = the first 120 frames (≈5.5 seconds), stored as Vec<u32>.
///
/// Two audio files are "similar" when their fingerprints have Hamming distance
/// per-frame below threshold averaged across all frames (≤ 8 bits/frame = 75%
/// match). This reliably detects the same recording at different bitrates,
/// re-encoded into different containers, or with minor edits.
///
/// This is a pure-Rust approximation of Chromaprint/AcoustID, trading some
/// accuracy for zero C-FFI dependencies.

use std::path::Path;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub static AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "ogg", "m4a", "aac", "wav", "wma", "opus", "aiff", "aif",
];

/// Number of frequency bands in the sub-band energy fingerprint
const NUM_BANDS: usize = 32;
/// Frames to fingerprint (120 × 512 samples ≈ 5.5 s at 11 025 Hz)
const NUM_FRAMES: usize = 120;
/// Samples per frame
const FRAME_SIZE: usize = 512;
/// Target mono sample rate for fingerprinting
const TARGET_SR: u32 = 11_025;

// ─── fingerprint ─────────────────────────────────────────────────────────────

/// Decode `path`, downsample to TARGET_SR mono, return raw f32 samples.
/// Returns `None` on decode failure or unsupported format.
fn decode_mono_samples(path: &Path) -> Option<Vec<f32>> {
    let file = std::fs::File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .ok()?;

    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)?;

    let track_id = track.id;
    let codec_params = track.codec_params.clone();
    let mut decoder = symphonia::default::get_codecs()
        .make(&codec_params, &DecoderOptions::default())
        .ok()?;

    let src_rate = codec_params.sample_rate.unwrap_or(44_100);
    let channels = codec_params.channels.map(|c| c.count()).unwrap_or(1);

    let mut raw: Vec<f32> = Vec::with_capacity(TARGET_SR as usize * 10); // pre-alloc ~10s

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::IoError(_)) => break,
            Err(SymphoniaError::ResetRequired) => break,
            Err(_) => break,
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(_) => continue,
        };

        let spec = *decoded.spec();
        let mut sample_buf: SampleBuffer<f32> =
            SampleBuffer::new(decoded.capacity() as u64, spec);
        sample_buf.copy_interleaved_ref(decoded);
        let samples = sample_buf.samples();

        // Mix down to mono and simple nearest-neighbour downsample
        let ch = channels.max(1);
        let step = (src_rate as f64 / TARGET_SR as f64).max(1.0) as usize * ch;
        let mut i = 0;
        while i + ch <= samples.len() {
            let mono: f32 = samples[i..i + ch].iter().sum::<f32>() / ch as f32;
            raw.push(mono);
            i += step;

            // We only need NUM_FRAMES * FRAME_SIZE samples
            if raw.len() >= NUM_FRAMES * FRAME_SIZE {
                return Some(raw);
            }
        }
    }

    if raw.len() < FRAME_SIZE {
        None
    } else {
        Some(raw)
    }
}

/// Compute a 32-band sub-band energy fingerprint.
/// Returns a Vec<u32> of NUM_FRAMES frames, each encoding 32 bits.
pub fn audio_fingerprint(path: &Path) -> Option<Vec<u32>> {
    let samples = decode_mono_samples(path)?;
    let frame_count = (samples.len() / FRAME_SIZE).min(NUM_FRAMES);
    if frame_count < 2 {
        return None;
    }

    // Compute per-band energies for each frame via rectangular DFT on FRAME_SIZE samples
    let mut frame_energies: Vec<[f32; NUM_BANDS]> = Vec::with_capacity(frame_count);
    for f in 0..frame_count {
        let start = f * FRAME_SIZE;
        let frame = &samples[start..start + FRAME_SIZE];
        let mut bands = [0.0f32; NUM_BANDS];
        let band_width = FRAME_SIZE / (NUM_BANDS * 2); // frequency bins per band

        for (band_idx, energy) in bands.iter_mut().enumerate() {
            // DFT bins for this band: [band_idx*band_width .. (band_idx+1)*band_width)
            let bin_start = band_idx * band_width;
            let bin_end = bin_start + band_width;
            let mut real_sum = 0.0f32;
            let mut imag_sum = 0.0f32;
            for bin in bin_start..bin_end {
                let freq = std::f64::consts::TAU * bin as f64 / FRAME_SIZE as f64;
                for (n, &s) in frame.iter().enumerate() {
                    real_sum += s * (freq * n as f64).cos() as f32;
                    imag_sum -= s * (freq * n as f64).sin() as f32;
                }
            }
            *energy = real_sum * real_sum + imag_sum * imag_sum;
        }
        frame_energies.push(bands);
    }

    // Encode: for each consecutive frame pair and each band,
    // set bit if energy[frame][band] > energy[frame-1][band]
    let mut fp: Vec<u32> = Vec::with_capacity(frame_count - 1);
    for f in 1..frame_count {
        let mut word: u32 = 0;
        for band in 0..NUM_BANDS {
            if frame_energies[f][band] > frame_energies[f - 1][band] {
                word |= 1u32 << band;
            }
        }
        fp.push(word);
    }

    Some(fp)
}

/// Average per-frame Hamming distance between two fingerprints (0.0–32.0).
/// ≤ 8.0 bits/frame → similar audio (same recording at different quality).
pub fn fingerprint_distance(a: &[u32], b: &[u32]) -> f32 {
    let len = a.len().min(b.len());
    if len == 0 {
        return 32.0;
    }
    let total_bits: u32 = a.iter().zip(b.iter()).map(|(x, y)| (x ^ y).count_ones()).sum();
    total_bits as f32 / len as f32
}

// ─── public types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDuplicatePair {
    pub path_a: String,
    pub path_b: String,
    /// Average Hamming distance per 32-bit frame (0 = identical, 32 = opposite)
    pub distance: f32,
    pub size_a: u64,
    pub size_b: u64,
}

fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTENSIONS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

// ─── Tauri command ─────────────────────────────────────────────────────────────

/// Scan `paths` for duplicate/near-duplicate audio files using sub-band
/// energy fingerprinting.
///
/// `threshold` is the maximum average per-frame Hamming distance to consider
/// two files "similar" (default 8.0, range 0–32).
/// Lower = stricter match (identical), higher = looser (different bitrate/format).
#[tauri::command]
pub async fn scan_audio_duplicates(
    paths: Vec<String>,
    threshold: Option<f32>,
) -> Result<Vec<AudioDuplicatePair>, String> {
    let threshold = threshold.unwrap_or(8.0).clamp(0.0, 32.0);

    // Collect audio file paths
    let audio_paths: Vec<std::path::PathBuf> = paths
        .iter()
        .flat_map(|root| {
            jwalk::WalkDir::new(root)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_file() && is_audio(&e.path()))
                .map(|e| e.path())
                .collect::<Vec<_>>()
        })
        .collect();

    if audio_paths.len() < 2 {
        return Ok(vec![]);
    }

    // Fingerprint in parallel (decode + DFT is CPU-bound)
    let fingerprinted: Vec<(std::path::PathBuf, Vec<u32>)> = audio_paths
        .par_iter()
        .filter_map(|p| audio_fingerprint(p).map(|fp| (p.clone(), fp)))
        .collect();

    // O(n²) comparison — fingerprint comparison is O(120) per pair, very fast
    let pairs: Vec<AudioDuplicatePair> = fingerprinted
        .par_iter()
        .enumerate()
        .flat_map(|(i, (path_a, fp_a))| {
            let mut local: Vec<AudioDuplicatePair> = Vec::new();
            for (path_b, fp_b) in &fingerprinted[i + 1..] {
                let dist = fingerprint_distance(fp_a, fp_b);
                if dist <= threshold {
                    let size_a = path_a.metadata().map(|m| m.len()).unwrap_or(0);
                    let size_b = path_b.metadata().map(|m| m.len()).unwrap_or(0);
                    local.push(AudioDuplicatePair {
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

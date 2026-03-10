//! Offline audio analysis: BPM detection and musical key detection.
//!
//! All functions are pure — no persistent state. They take `&[f32]` PCM data
//! and return results. Analysis is meant to run once after `load_mp3()`.

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use std::f32::consts::PI;
use std::sync::Arc;

// ── Result type ──

/// Analysis result returned to JS via serde_wasm_bindgen.
#[derive(serde::Serialize, Clone, Debug)]
pub struct AnalysisResult {
    pub bpm: f64,
    pub bpm_confidence: f64,
    pub key: String,
    pub key_confidence: f64,
    pub first_beat_secs: f64,
    /// Beat positions in seconds (source time). Tracks tempo changes within the song.
    pub beat_times: Vec<f64>,
}

// ── Constants ──

const BPM_FFT_SIZE: usize = 1024;
const BPM_HOP_SIZE: usize = 256;
const KEY_FFT_SIZE: usize = 2048;
const KEY_HOP_SIZE: usize = 1024;

/// Temperley major key profile (starting from C).
const MAJOR_PROFILE: [f64; 12] = [5.0, 2.0, 3.5, 2.0, 4.5, 4.0, 2.0, 4.5, 2.0, 3.5, 1.5, 4.0];

/// Temperley minor key profile (starting from C).
const MINOR_PROFILE: [f64; 12] = [5.0, 2.0, 3.5, 4.5, 2.0, 4.0, 2.0, 4.5, 3.5, 2.0, 1.5, 4.0];

const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B",
];

/// Minimum number of IOIs for reliable BPM detection.
const MIN_IOIS: usize = 8;

// ── Top-level entry point ──

/// Analyze a loaded track for BPM and musical key.
///
/// `samples` is interleaved PCM, `channels` is 1 or 2, `sample_rate` is the
/// output sample rate (typically 44100).
pub fn analyze_track(
    samples: &[f32],
    channels: usize,
    sample_rate: u32,
) -> AnalysisResult {
    let fallback = AnalysisResult {
        bpm: 0.0,
        bpm_confidence: 0.0,
        key: "---".to_string(),
        key_confidence: 0.0,
        first_beat_secs: 0.0,
        beat_times: Vec::new(),
    };

    if samples.is_empty() || channels == 0 || sample_rate == 0 {
        return fallback;
    }

    // 1. Downmix to mono.
    let mono = downmix_to_mono(samples, channels);
    if mono.len() < BPM_FFT_SIZE * 4 {
        return fallback; // Too short for analysis.
    }

    // 2. Downsample to ~1/4 original rate.
    let decimation = 4usize;
    let downsampled = downsample(&mono, decimation);
    let ds_rate = sample_rate / decimation as u32;

    // 3. Plan FFTs for analysis (separate from vocoder's real-time FFTs).
    let mut planner = FftPlanner::<f32>::new();
    let fft_bpm = planner.plan_fft_forward(BPM_FFT_SIZE);
    let fft_key = planner.plan_fft_forward(KEY_FFT_SIZE);

    // 4. Onset detection → peak picking → IOI histogram → BPM.
    let onset_signal =
        compute_onset_signal(&downsampled, &fft_bpm, BPM_FFT_SIZE, BPM_HOP_SIZE);

    if onset_signal.len() < 16 {
        // Key detection can still run.
        let (key, key_confidence) = detect_key(&downsampled, ds_rate, &fft_key);
        return AnalysisResult {
            bpm: 0.0,
            bpm_confidence: 0.0,
            key,
            key_confidence,
            first_beat_secs: 0.0,
            beat_times: Vec::new(),
        };
    }

    let peak_indices = pick_peaks(&onset_signal, 8);

    let (bpm, bpm_confidence) = if peak_indices.len() >= 2 {
        compute_bpm_histogram(&peak_indices, BPM_HOP_SIZE, ds_rate)
    } else {
        (0.0, 0.0)
    };

    let first_beat_secs = if bpm > 0.0 {
        find_first_beat(&onset_signal, bpm, BPM_HOP_SIZE, ds_rate)
    } else {
        0.0
    };

    // 5. Beat grid: adaptive beat tracking that handles tempo changes.
    let beat_times = if bpm > 0.0 {
        build_beat_grid(
            &onset_signal,
            &peak_indices,
            bpm,
            first_beat_secs,
            BPM_HOP_SIZE,
            ds_rate,
        )
    } else {
        Vec::new()
    };

    // 6. Key detection.
    let (key, key_confidence) = detect_key(&downsampled, ds_rate, &fft_key);

    AnalysisResult {
        bpm,
        bpm_confidence,
        key,
        key_confidence,
        first_beat_secs,
        beat_times,
    }
}

// ── Preprocessing ──

fn downmix_to_mono(samples: &[f32], channels: usize) -> Vec<f32> {
    if channels == 1 {
        return samples.to_vec();
    }
    let frames = samples.len() / channels;
    let mut mono = Vec::with_capacity(frames);
    for f in 0..frames {
        let mut sum = 0.0f32;
        for ch in 0..channels {
            sum += samples[f * channels + ch];
        }
        mono.push(sum / channels as f32);
    }
    mono
}

/// Downsample by an integer factor with a simple low-pass anti-aliasing filter.
/// Uses a 7-tap Hann-windowed sinc filter at cutoff = 1/(2*factor).
fn downsample(mono: &[f32], factor: usize) -> Vec<f32> {
    if factor <= 1 {
        return mono.to_vec();
    }

    // Design a small low-pass FIR: 7-tap Hann-windowed sinc, cutoff = 1/(2*factor).
    let taps = 7usize;
    let center = taps / 2;
    let cutoff = 1.0 / (2.0 * factor as f64);
    let mut kernel = vec![0.0f64; taps];
    for i in 0..taps {
        let n = i as f64 - center as f64;
        let sinc = if n.abs() < 1e-10 {
            2.0 * cutoff
        } else {
            (2.0 * std::f64::consts::PI * cutoff * n).sin() / (std::f64::consts::PI * n)
        };
        let hann = 0.5 * (1.0 - (2.0 * std::f64::consts::PI * i as f64 / (taps - 1) as f64).cos());
        kernel[i] = sinc * hann;
    }
    // Normalize kernel.
    let sum: f64 = kernel.iter().sum();
    if sum > 1e-10 {
        for k in kernel.iter_mut() {
            *k /= sum;
        }
    }

    // Apply filter and decimate.
    let out_len = mono.len() / factor;
    let mut result = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src_idx = i * factor;
        let mut val = 0.0f64;
        for (t, &k) in kernel.iter().enumerate() {
            let idx = src_idx as isize + t as isize - center as isize;
            if idx >= 0 && (idx as usize) < mono.len() {
                val += mono[idx as usize] as f64 * k;
            }
        }
        result.push(val as f32);
    }
    result
}

fn hann_window(size: usize) -> Vec<f32> {
    (0..size)
        .map(|i| 0.5 * (1.0 - (2.0 * PI * i as f32 / size as f32).cos()))
        .collect()
}

// ── BPM Detection ──

/// Compute spectral flux onset signal.
/// Reuses the same half-wave rectified spectral flux formula from vocoder.rs.
fn compute_onset_signal(
    audio: &[f32],
    fft_forward: &Arc<dyn Fft<f32>>,
    fft_size: usize,
    hop_size: usize,
) -> Vec<f32> {
    let window = hann_window(fft_size);
    let half = fft_size / 2 + 1;
    let mut prev_magnitudes = vec![0.0f32; half];
    let mut frame_buf = vec![Complex::new(0.0f32, 0.0); fft_size];
    let mut onset_signal = Vec::new();
    let mut has_prev = false;

    let mut pos = 0;
    while pos + fft_size <= audio.len() {
        // Window the frame.
        for i in 0..fft_size {
            frame_buf[i] = Complex::new(audio[pos + i] * window[i], 0.0);
        }

        // Forward FFT.
        fft_forward.process(&mut frame_buf);

        // Half-wave rectified spectral flux.
        let mut flux = 0.0f32;
        if has_prev {
            for k in 0..half {
                let mag = frame_buf[k].norm();
                let diff = mag - prev_magnitudes[k];
                if diff > 0.0 {
                    flux += diff;
                }
                prev_magnitudes[k] = mag;
            }
            flux /= half as f32;
        } else {
            for k in 0..half {
                prev_magnitudes[k] = frame_buf[k].norm();
            }
            has_prev = true;
        }

        onset_signal.push(flux);
        pos += hop_size;
    }

    onset_signal
}

/// Adaptive peak picking with local mean threshold.
fn pick_peaks(onset_signal: &[f32], window_size: usize) -> Vec<usize> {
    let len = onset_signal.len();
    let min_spacing = 5; // Minimum frames between peaks (~116ms at 11025/256).
    let threshold_multiplier = 1.4f32;

    let mut peaks = Vec::new();
    let mut last_peak: Option<usize> = None;

    for i in 1..len.saturating_sub(1) {
        // Must be a local maximum.
        if onset_signal[i] <= onset_signal[i - 1] || onset_signal[i] <= onset_signal[i + 1] {
            continue;
        }

        // Enforce minimum spacing.
        if let Some(lp) = last_peak {
            if i - lp < min_spacing {
                continue;
            }
        }

        // Compute local mean over a window centered on i.
        let start = i.saturating_sub(window_size);
        let end = (i + window_size + 1).min(len);
        let local_mean: f32 =
            onset_signal[start..end].iter().sum::<f32>() / (end - start) as f32;

        // Peak must exceed threshold.
        if onset_signal[i] > local_mean * threshold_multiplier && onset_signal[i] > 1e-6 {
            peaks.push(i);
            last_peak = Some(i);
        }
    }

    peaks
}

/// Build IOI histogram with octave folding to determine BPM.
/// Uses multi-beat intervals (1-beat, 2-beat, 3-beat gaps) for robustness.
fn compute_bpm_histogram(
    peak_indices: &[usize],
    hop_size: usize,
    sample_rate: u32,
) -> (f64, f64) {
    let secs_per_frame = hop_size as f64 / sample_rate as f64;

    // Collect IOIs from consecutive peaks, 2-apart, and 3-apart for robustness.
    let mut iois = Vec::new();
    for gap in 1..=3usize {
        if peak_indices.len() <= gap {
            continue;
        }
        for i in 0..peak_indices.len() - gap {
            let interval_secs =
                (peak_indices[i + gap] - peak_indices[i]) as f64 * secs_per_frame;
            let bpm = 60.0 * gap as f64 / interval_secs;
            if (30.0..=300.0).contains(&bpm) {
                iois.push(bpm);
            }
        }
    }

    if iois.len() < MIN_IOIS {
        return (0.0, 0.0);
    }

    // Build histogram: 1-BPM bins from 60 to 200.
    let bin_min = 60.0f64;
    let bin_max = 200.0f64;
    let num_bins = (bin_max - bin_min) as usize + 1;
    let mut histogram = vec![0.0f64; num_bins];

    for &bpm in &iois {
        // Fold into 60-200 range via octave multiples.
        let mut b = bpm;
        while b > bin_max {
            b /= 2.0;
        }
        while b < bin_min {
            b *= 2.0;
        }
        if b >= bin_min && b <= bin_max {
            let idx = (b - bin_min).round() as usize;
            if idx < num_bins {
                histogram[idx] += 1.0;
            }
        }
    }

    // Smooth histogram with a small window (±2 bins).
    let smoothed = smooth_histogram(&histogram, 2);

    // Find peak.
    let mut best_idx = 0;
    let mut best_val = 0.0f64;
    for (i, &v) in smoothed.iter().enumerate() {
        if v > best_val {
            best_val = v;
            best_idx = i;
        }
    }

    let bpm = bin_min + best_idx as f64;
    let total: f64 = smoothed.iter().sum();
    let confidence = if total > 0.0 {
        (best_val / total).min(1.0)
    } else {
        0.0
    };

    (bpm, confidence)
}

fn smooth_histogram(hist: &[f64], radius: usize) -> Vec<f64> {
    let len = hist.len();
    let mut smoothed = vec![0.0f64; len];
    for i in 0..len {
        let start = i.saturating_sub(radius);
        let end = (i + radius + 1).min(len);
        let sum: f64 = hist[start..end].iter().sum();
        smoothed[i] = sum / (end - start) as f64;
    }
    smoothed
}

/// Find the first beat offset by testing all candidate phases.
fn find_first_beat(
    onset_signal: &[f32],
    bpm: f64,
    hop_size: usize,
    sample_rate: u32,
) -> f64 {
    let secs_per_frame = hop_size as f64 / sample_rate as f64;
    let period_frames = 60.0 / bpm / secs_per_frame;

    if period_frames < 1.0 || onset_signal.is_empty() {
        return 0.0;
    }

    let period = period_frames.round() as usize;
    if period == 0 {
        return 0.0;
    }

    // Test each candidate phase offset.
    let mut best_offset = 0usize;
    let mut best_energy = 0.0f64;

    for offset in 0..period.min(onset_signal.len()) {
        let mut energy = 0.0f64;
        let mut pos = offset;
        while pos < onset_signal.len() {
            energy += onset_signal[pos] as f64;
            pos += period;
        }
        if energy > best_energy {
            best_energy = energy;
            best_offset = offset;
        }
    }

    best_offset as f64 * secs_per_frame
}

// ── Beat Grid Construction ──

/// Estimate the local beat period at a given position using onset signal autocorrelation.
///
/// Returns the period in frames, or 0.0 if no clear periodicity is found.
fn local_tempo_period(onset_signal: &[f32], center: usize, secs_per_frame: f64) -> f64 {
    // Window: ~6 seconds centered on position.
    let window_half = (3.0 / secs_per_frame) as usize;
    let start = center.saturating_sub(window_half);
    let end = (center + window_half).min(onset_signal.len());

    if end <= start + 20 {
        return 0.0;
    }
    let window = &onset_signal[start..end];
    let wlen = window.len();

    // Autocorrelation for lags corresponding to 60–200 BPM.
    let min_lag = (60.0 / 200.0 / secs_per_frame).round() as usize;
    let max_lag = ((60.0 / 60.0 / secs_per_frame).round() as usize).min(wlen / 2);

    if min_lag >= max_lag {
        return 0.0;
    }

    // Subtract mean for proper autocorrelation.
    let mean: f64 = window.iter().map(|&x| x as f64).sum::<f64>() / wlen as f64;
    let var: f64 = window.iter().map(|&x| (x as f64 - mean).powi(2)).sum::<f64>();

    if var < 1e-10 {
        return 0.0;
    }

    // Compute autocorrelation at all candidate lags.
    let lag_count = max_lag - min_lag + 1;
    let mut acf_values = vec![0.0f64; lag_count];

    for (idx, lag) in (min_lag..=max_lag).enumerate() {
        let mut acf = 0.0f64;
        for i in 0..wlen - lag {
            acf += (window[i] as f64 - mean) * (window[i + lag] as f64 - mean);
        }
        acf_values[idx] = acf / var; // Normalized correlation coefficient.
    }

    // Find global maximum for threshold reference.
    let global_max = acf_values
        .iter()
        .cloned()
        .fold(f64::NEG_INFINITY, f64::max);

    if global_max < 0.05 {
        return 0.0;
    }

    // Find the FIRST local maximum that's at least 80% of global max.
    // This prefers shorter periods (higher BPM) over their sub-harmonics.
    let threshold = global_max * 0.80;
    let mut best_idx = 0;

    for idx in 1..lag_count.saturating_sub(1) {
        if acf_values[idx] >= threshold
            && acf_values[idx] >= acf_values[idx - 1]
            && acf_values[idx] >= acf_values[idx + 1]
        {
            best_idx = idx;
            break;
        }
    }

    // Fallback to global max if no peak found above threshold.
    if best_idx == 0 {
        best_idx = acf_values
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, _)| i)
            .unwrap_or(0);
    }

    (min_lag + best_idx) as f64
}

/// Build a beat grid using autocorrelation-based local tempo estimation.
///
/// At each step, the local tempo is queried from the onset signal's autocorrelation,
/// making the tracker robust to tempo changes within the song.
fn build_beat_grid(
    onset_signal: &[f32],
    peak_indices: &[usize],
    initial_bpm: f64,
    first_beat_secs: f64,
    hop_size: usize,
    sample_rate: u32,
) -> Vec<f64> {
    let secs_per_frame = hop_size as f64 / sample_rate as f64;
    let n = onset_signal.len();
    let initial_period = 60.0 / initial_bpm / secs_per_frame;
    let first_beat_frame = (first_beat_secs / secs_per_frame).round() as usize;

    let min_period = 60.0 / 200.0 / secs_per_frame;
    let max_period = 60.0 / 60.0 / secs_per_frame;

    // Track forward from first beat.
    let forward = track_with_local_tempo(
        onset_signal,
        peak_indices,
        first_beat_frame,
        initial_period,
        true,
        min_period,
        max_period,
        secs_per_frame,
        n,
    );

    // Track backward from first beat.
    let backward = track_with_local_tempo(
        onset_signal,
        peak_indices,
        first_beat_frame,
        initial_period,
        false,
        min_period,
        max_period,
        secs_per_frame,
        n,
    );

    // Merge: backward (reversed) + first beat + forward.
    let mut beats = Vec::with_capacity(backward.len() + 1 + forward.len());
    for &b in backward.iter().rev() {
        beats.push(b);
    }
    beats.push(first_beat_secs);
    beats.extend_from_slice(&forward);

    beats
}

/// Track beats in one direction, using onset autocorrelation for local tempo.
fn track_with_local_tempo(
    onset_signal: &[f32],
    peak_indices: &[usize],
    start_frame: usize,
    initial_period: f64,
    forward: bool,
    min_period: f64,
    max_period: f64,
    secs_per_frame: f64,
    n: usize,
) -> Vec<f64> {
    /// Snap window: ±20% of expected position.
    const SNAP_TOLERANCE: f64 = 0.20;

    let mut beats = Vec::new();
    let mut period = initial_period;
    let mut current = start_frame as f64;

    loop {
        // Query local tempo from onset autocorrelation.
        let local_period =
            local_tempo_period(onset_signal, current.round() as usize, secs_per_frame);
        if local_period > 0.0 {
            // Blend: favour autocorrelation estimate (70%) over running period (30%).
            period = period * 0.3 + local_period * 0.7;
            period = period.clamp(min_period, max_period);
        }

        let expected = if forward {
            current + period
        } else {
            current - period
        };

        if expected < 0.0 || expected >= n as f64 {
            break;
        }

        // Snap to nearest onset peak.
        let expected_idx = expected.round() as usize;
        let window_half = (period * SNAP_TOLERANCE).round() as usize;
        let search_start = expected_idx.saturating_sub(window_half);
        let search_end = (expected_idx + window_half + 1).min(n);

        let mut best_peak: Option<usize> = None;
        let mut best_strength = 0.0f32;

        for &p in peak_indices {
            if p >= search_start && p < search_end && onset_signal[p] > best_strength {
                best_strength = onset_signal[p];
                best_peak = Some(p);
            }
        }

        let beat_frame = best_peak.unwrap_or(expected_idx);
        beats.push(beat_frame as f64 * secs_per_frame);
        current = beat_frame as f64;
    }

    beats
}

// ── Key Detection ──

fn detect_key(
    audio: &[f32],
    sample_rate: u32,
    fft_forward: &Arc<dyn Fft<f32>>,
) -> (String, f64) {
    let chroma = compute_chromagram(audio, sample_rate, fft_forward, KEY_FFT_SIZE, KEY_HOP_SIZE);

    // Check if chromagram is essentially silent.
    let total: f64 = chroma.iter().sum();
    if total < 1e-10 {
        return ("---".to_string(), 0.0);
    }

    correlate_key_profiles(&chroma)
}

/// Compute a 12-bin chromagram from the audio.
fn compute_chromagram(
    audio: &[f32],
    sample_rate: u32,
    fft_forward: &Arc<dyn Fft<f32>>,
    fft_size: usize,
    hop_size: usize,
) -> [f64; 12] {
    let window = hann_window(fft_size);
    let half = fft_size / 2 + 1;
    let mut frame_buf = vec![Complex::new(0.0f32, 0.0); fft_size];
    let mut chroma = [0.0f64; 12];

    // Frequency range for chromagram: ~65 Hz (C2) to ~2000 Hz.
    let min_freq = 65.0f64;
    let max_freq = 2000.0f64;
    let reference_freq = 440.0f64; // A4.

    let mut pos = 0;
    let mut frame_count = 0u64;

    while pos + fft_size <= audio.len() {
        // Window the frame.
        for i in 0..fft_size {
            frame_buf[i] = Complex::new(audio[pos + i] * window[i], 0.0);
        }

        // Forward FFT.
        fft_forward.process(&mut frame_buf);

        // Map FFT bins to pitch classes.
        for k in 1..half {
            let freq = k as f64 * sample_rate as f64 / fft_size as f64;
            if freq < min_freq || freq > max_freq {
                continue;
            }

            // Frequency → pitch class (0-11, where 0=C, 9=A).
            let midi_note = 12.0 * (freq / reference_freq).log2() + 69.0;
            let pitch_class = ((midi_note.round() as i64 % 12) + 12) % 12;

            let power = (frame_buf[k].norm() as f64).powi(2);
            chroma[pitch_class as usize] += power;
        }

        frame_count += 1;
        pos += hop_size;
    }

    // Normalize to average per frame.
    if frame_count > 0 {
        for c in chroma.iter_mut() {
            *c /= frame_count as f64;
        }
    }

    // Normalize to sum to 1.0.
    let total: f64 = chroma.iter().sum();
    if total > 1e-10 {
        for c in chroma.iter_mut() {
            *c /= total;
        }
    }

    chroma
}

/// Correlate chromagram against 24 Temperley key profiles.
fn correlate_key_profiles(chroma: &[f64; 12]) -> (String, f64) {
    let mut best_key = String::new();
    let mut best_corr = f64::NEG_INFINITY;

    for shift in 0..12 {
        let major_corr = pearson_correlation(chroma, &rotate_profile(&MAJOR_PROFILE, shift));
        let minor_corr = pearson_correlation(chroma, &rotate_profile(&MINOR_PROFILE, shift));

        if major_corr > best_corr {
            best_corr = major_corr;
            best_key = format!("{} major", NOTE_NAMES[shift]);
        }
        if minor_corr > best_corr {
            best_corr = minor_corr;
            best_key = format!("{} minor", NOTE_NAMES[shift]);
        }
    }

    // Clamp confidence to [0, 1].
    let confidence = best_corr.max(0.0).min(1.0);
    (best_key, confidence)
}

fn rotate_profile(profile: &[f64; 12], shift: usize) -> [f64; 12] {
    let mut rotated = [0.0f64; 12];
    for i in 0..12 {
        rotated[i] = profile[(i + 12 - shift) % 12];
    }
    rotated
}

fn pearson_correlation(x: &[f64; 12], y: &[f64; 12]) -> f64 {
    let n = 12.0f64;
    let mean_x: f64 = x.iter().sum::<f64>() / n;
    let mean_y: f64 = y.iter().sum::<f64>() / n;

    let mut cov = 0.0f64;
    let mut var_x = 0.0f64;
    let mut var_y = 0.0f64;

    for i in 0..12 {
        let dx = x[i] - mean_x;
        let dy = y[i] - mean_y;
        cov += dx * dy;
        var_x += dx * dx;
        var_y += dy * dy;
    }

    let denom = (var_x * var_y).sqrt();
    if denom < 1e-10 {
        return 0.0;
    }
    cov / denom
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    fn make_mono_sine(freq: f32, sample_rate: f32, duration_secs: f32) -> Vec<f32> {
        let n = (sample_rate * duration_secs) as usize;
        (0..n)
            .map(|i| (2.0 * PI * freq * i as f32 / sample_rate).sin() * 0.5)
            .collect()
    }

    /// Generate a synthetic click track at a given BPM.
    fn make_click_track(bpm: f64, sample_rate: u32, duration_secs: f64) -> Vec<f32> {
        let total = (sample_rate as f64 * duration_secs) as usize;
        let beat_period = (60.0 / bpm * sample_rate as f64) as usize;
        let click_len = (sample_rate as f64 * 0.005) as usize; // 5ms click.
        let mut samples = vec![0.0f32; total];
        let mut pos = 0;
        while pos < total {
            for i in 0..click_len.min(total - pos) {
                samples[pos + i] = (2.0 * std::f64::consts::PI * 1000.0 * i as f64
                    / sample_rate as f64)
                    .sin() as f32
                    * 0.8;
            }
            pos += beat_period;
        }
        samples
    }

    #[test]
    fn test_downmix_stereo_to_mono() {
        let stereo = vec![0.5f32, -0.5, 1.0, -1.0, 0.0, 0.0];
        let mono = downmix_to_mono(&stereo, 2);
        assert_eq!(mono.len(), 3);
        assert!((mono[0] - 0.0).abs() < 1e-6);
        assert!((mono[1] - 0.0).abs() < 1e-6);
        assert!((mono[2] - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_downmix_mono_passthrough() {
        let mono = vec![1.0f32, 2.0, 3.0];
        let result = downmix_to_mono(&mono, 1);
        assert_eq!(result.len(), 3);
        assert!((result[0] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_downsample_length() {
        let mono = vec![1.0f32; 44100];
        let ds = downsample(&mono, 4);
        assert_eq!(ds.len(), 44100 / 4);
    }

    #[test]
    fn test_bpm_detection_120bpm() {
        let track = make_click_track(120.0, 44100, 15.0);
        let result = analyze_track(&track, 1, 44100);
        assert!(
            (result.bpm - 120.0).abs() < 5.0,
            "Expected ~120 BPM, got {}",
            result.bpm
        );
        assert!(result.bpm_confidence > 0.1, "Confidence too low: {}", result.bpm_confidence);
    }

    #[test]
    fn test_bpm_detection_140bpm() {
        let track = make_click_track(140.0, 44100, 15.0);
        let result = analyze_track(&track, 1, 44100);
        assert!(
            (result.bpm - 140.0).abs() < 5.0,
            "Expected ~140 BPM, got {}",
            result.bpm
        );
    }

    #[test]
    fn test_bpm_detection_90bpm() {
        let track = make_click_track(90.0, 44100, 20.0);
        let result = analyze_track(&track, 1, 44100);
        assert!(
            (result.bpm - 90.0).abs() < 5.0,
            "Expected ~90 BPM, got {}",
            result.bpm
        );
    }

    #[test]
    fn test_key_detection_a_sine() {
        // A4 = 440 Hz — strong A should detect A-something.
        let track = make_mono_sine(440.0, 44100.0, 10.0);
        let result = analyze_track(&track, 1, 44100);
        assert!(
            result.key.starts_with('A'),
            "Expected A key, got {}",
            result.key
        );
    }

    #[test]
    fn test_first_beat_within_one_period() {
        let track = make_click_track(120.0, 44100, 15.0);
        let result = analyze_track(&track, 1, 44100);
        let beat_period = 60.0 / 120.0;
        assert!(
            result.first_beat_secs < beat_period,
            "First beat {} should be < beat period {}",
            result.first_beat_secs,
            beat_period
        );
    }

    #[test]
    fn test_stereo_click_track() {
        let mono = make_click_track(128.0, 44100, 15.0);
        let stereo: Vec<f32> = mono.iter().flat_map(|&s| [s, s]).collect();
        let result = analyze_track(&stereo, 2, 44100);
        assert!(
            (result.bpm - 128.0).abs() < 5.0,
            "Expected ~128 BPM, got {}",
            result.bpm
        );
    }

    #[test]
    fn test_empty_track() {
        let result = analyze_track(&[], 2, 44100);
        assert!((result.bpm - 0.0).abs() < 1e-6);
        assert_eq!(result.key, "---");
    }

    #[test]
    fn test_chromagram_sums_to_one() {
        let track = make_mono_sine(440.0, 44100.0, 5.0);
        let ds = downsample(&track, 4);
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(KEY_FFT_SIZE);
        let chroma = compute_chromagram(&ds, 44100 / 4, &fft, KEY_FFT_SIZE, KEY_HOP_SIZE);
        let total: f64 = chroma.iter().sum();
        assert!(
            (total - 1.0).abs() < 0.01,
            "Chromagram should sum to ~1.0, got {}",
            total
        );
    }

    #[test]
    fn test_pearson_perfect_match() {
        let x = [1.0, 0.0, 0.5, 0.0, 1.0, 0.5, 0.0, 1.0, 0.0, 0.5, 0.0, 0.5];
        let corr = pearson_correlation(&x, &x);
        assert!(
            (corr - 1.0).abs() < 1e-6,
            "Self-correlation should be 1.0, got {}",
            corr
        );
    }

    #[test]
    fn test_rotate_profile() {
        let p = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0];
        let rotated = rotate_profile(&p, 1);
        // shift=1 means C profile → C# key, so index 0 should get the last element.
        assert!((rotated[0] - 12.0).abs() < 1e-6);
        assert!((rotated[1] - 1.0).abs() < 1e-6);
    }

    // ── Beat grid tests ──

    #[test]
    fn test_beat_grid_120bpm() {
        let track = make_click_track(120.0, 44100, 15.0);
        let result = analyze_track(&track, 1, 44100);

        assert!(
            !result.beat_times.is_empty(),
            "Should have beat times for a 120 BPM click track"
        );

        // Should have roughly 15s × 2 beats/s = 30 beats (±5).
        assert!(
            result.beat_times.len() > 20 && result.beat_times.len() < 40,
            "Expected ~30 beats, got {}",
            result.beat_times.len()
        );

        // Beat intervals should be close to 0.5s.
        let intervals: Vec<f64> = result
            .beat_times
            .windows(2)
            .map(|w| w[1] - w[0])
            .collect();
        let avg_interval: f64 = intervals.iter().sum::<f64>() / intervals.len() as f64;
        assert!(
            (avg_interval - 0.5).abs() < 0.05,
            "Expected ~0.5s intervals for 120 BPM, got {:.4}",
            avg_interval
        );
    }

    #[test]
    fn test_beat_grid_monotonic() {
        let track = make_click_track(128.0, 44100, 15.0);
        let result = analyze_track(&track, 1, 44100);

        // Beat times must be strictly increasing.
        for w in result.beat_times.windows(2) {
            assert!(
                w[1] > w[0],
                "Beat times not monotonic: {} >= {}",
                w[0],
                w[1]
            );
        }
    }

    /// Click track that changes tempo mid-way — like "Take Me Out".
    fn make_tempo_change_track(
        bpm1: f64,
        bpm2: f64,
        sample_rate: u32,
        switch_secs: f64,
        total_secs: f64,
    ) -> Vec<f32> {
        let total = (sample_rate as f64 * total_secs) as usize;
        let switch = (sample_rate as f64 * switch_secs) as usize;
        let beat_period1 = (60.0 / bpm1 * sample_rate as f64) as usize;
        let beat_period2 = (60.0 / bpm2 * sample_rate as f64) as usize;
        let click_len = (sample_rate as f64 * 0.005) as usize;
        let mut samples = vec![0.0f32; total];

        let mut pos = 0;
        while pos < switch {
            for i in 0..click_len.min(total - pos) {
                samples[pos + i] = (2.0 * std::f64::consts::PI * 1000.0 * i as f64
                    / sample_rate as f64)
                    .sin() as f32
                    * 0.8;
            }
            pos += beat_period1;
        }

        pos = switch;
        while pos < total {
            for i in 0..click_len.min(total - pos) {
                samples[pos + i] = (2.0 * std::f64::consts::PI * 1000.0 * i as f64
                    / sample_rate as f64)
                    .sin() as f32
                    * 0.8;
            }
            pos += beat_period2;
        }

        samples
    }

    #[test]
    fn test_beat_grid_tempo_change() {
        // Simulate "Take Me Out": 117.5 BPM for first 33s, then 95.7 BPM.
        let track = make_tempo_change_track(117.5, 95.7, 44100, 33.0, 60.0);
        let result = analyze_track(&track, 1, 44100);

        assert!(
            !result.beat_times.is_empty(),
            "Should have beat times for tempo-change track"
        );

        // Check that beats in the steady first section (~5–28s) have intervals near 0.511s.
        let early_beats: Vec<f64> = result
            .beat_times
            .iter()
            .filter(|&&t| t > 5.0 && t < 28.0)
            .cloned()
            .collect();
        if early_beats.len() >= 4 {
            let avg_interval: f64 = early_beats
                .windows(2)
                .map(|w| w[1] - w[0])
                .sum::<f64>()
                / (early_beats.len() - 1) as f64;
            assert!(
                (avg_interval - 0.511).abs() < 0.12,
                "Early section should have ~0.511s intervals, got {:.3}",
                avg_interval
            );
        }

        // Check that beats in the steady second section (~40–55s) have intervals near 0.627s.
        let late_beats: Vec<f64> = result
            .beat_times
            .iter()
            .filter(|&&t| t > 40.0 && t < 55.0)
            .cloned()
            .collect();
        if late_beats.len() >= 4 {
            let avg_interval: f64 = late_beats
                .windows(2)
                .map(|w| w[1] - w[0])
                .sum::<f64>()
                / (late_beats.len() - 1) as f64;
            assert!(
                (avg_interval - 0.627).abs() < 0.12,
                "Late section should have ~0.627s intervals, got {:.3}",
                avg_interval
            );
        }
    }

    #[test]
    fn test_empty_track_has_no_beats() {
        let result = analyze_track(&[], 2, 44100);
        assert!(result.beat_times.is_empty());
    }
}

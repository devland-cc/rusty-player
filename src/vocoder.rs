use std::f32::consts::PI;
use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

/// Streaming phase vocoder for real-time time-stretching.
///
/// Processes mono audio incrementally via ring buffer I/O.
/// Each channel needs its own instance.
pub struct StreamingPhaseVocoder {
    fft_size: usize,
    analysis_hop: usize,
    fft_forward: Arc<dyn Fft<f32>>,
    fft_inverse: Arc<dyn Fft<f32>>,
    window: Vec<f32>,

    // Phase state.
    prev_phase: Vec<f32>,
    synth_phase: Vec<f32>,
    bin_freq: Vec<f32>,
    frame_buf: Vec<Complex<f32>>,
    has_state: bool,

    // Input ring buffer.
    input_ring: Vec<f32>,
    input_write: usize,
    input_read: usize,
    input_available: usize,

    // Output ring buffer with overlap-add.
    output_ring: Vec<f32>,
    window_sum_ring: Vec<f32>,
    output_write: usize,
    output_read: usize,
    output_available: usize,

    // Current stretch ratio.
    current_stretch: f64,

    // Phase increments from last processed frame (for linked-phase stereo).
    last_phase_increments: Vec<f32>,

    // Identity phase locking.
    phase_lock_enabled: bool,
    magnitudes: Vec<f32>,
    peak_flags: Vec<bool>,
    nearest_peak_buf: Vec<usize>,

    // Transient detection.
    transient_detect_enabled: bool,
    transient_sensitivity: f32,
    prev_magnitudes: Vec<f32>,
    has_prev_magnitudes: bool,
    running_flux: f32,
    last_frame_was_transient: bool,
}

impl StreamingPhaseVocoder {
    pub fn new(fft_size: usize, overlap: usize) -> Self {
        let analysis_hop = fft_size / overlap;

        let mut planner = FftPlanner::new();
        let fft_forward = planner.plan_fft_forward(fft_size);
        let fft_inverse = planner.plan_fft_inverse(fft_size);

        let window = hann_window(fft_size);

        let bin_freq: Vec<f32> = (0..fft_size)
            .map(|k| 2.0 * PI * k as f32 * analysis_hop as f32 / fft_size as f32)
            .collect();

        let input_cap = fft_size * 8;
        let output_cap = fft_size * 16;
        let half = fft_size / 2 + 1;

        Self {
            fft_size,
            analysis_hop,
            fft_forward,
            fft_inverse,
            window,
            prev_phase: vec![0.0; fft_size],
            synth_phase: vec![0.0; fft_size],
            bin_freq,
            frame_buf: vec![Complex::new(0.0, 0.0); fft_size],
            has_state: false,
            input_ring: vec![0.0; input_cap],
            input_write: 0,
            input_read: 0,
            input_available: 0,
            output_ring: vec![0.0; output_cap],
            window_sum_ring: vec![0.0; output_cap],
            output_write: 0,
            output_read: 0,
            output_available: 0,
            current_stretch: 1.0,
            last_phase_increments: vec![0.0; fft_size],
            phase_lock_enabled: false,
            magnitudes: vec![0.0; fft_size],
            peak_flags: vec![false; half],
            nearest_peak_buf: vec![0; half],
            transient_detect_enabled: false,
            transient_sensitivity: 0.5,
            prev_magnitudes: vec![0.0; half],
            has_prev_magnitudes: false,
            running_flux: 0.0,
            last_frame_was_transient: false,
        }
    }

    pub fn reset(&mut self) {
        self.prev_phase.fill(0.0);
        self.synth_phase.fill(0.0);
        self.has_state = false;

        self.input_ring.fill(0.0);
        self.input_write = 0;
        self.input_read = 0;
        self.input_available = 0;

        self.output_ring.fill(0.0);
        self.window_sum_ring.fill(0.0);
        self.output_write = 0;
        self.output_read = 0;
        self.output_available = 0;

        self.magnitudes.fill(0.0);
        self.prev_magnitudes.fill(0.0);
        self.has_prev_magnitudes = false;
        self.running_flux = 0.0;
        self.last_frame_was_transient = false;
    }

    pub fn set_stretch(&mut self, ratio: f64) {
        self.current_stretch = ratio.max(0.1).min(10.0);
    }

    pub fn set_phase_lock(&mut self, enabled: bool) {
        self.phase_lock_enabled = enabled;
    }

    pub fn phase_lock(&self) -> bool {
        self.phase_lock_enabled
    }

    pub fn set_transient_detect(&mut self, enabled: bool) {
        self.transient_detect_enabled = enabled;
    }

    pub fn transient_detect(&self) -> bool {
        self.transient_detect_enabled
    }

    pub fn set_transient_sensitivity(&mut self, sensitivity: f32) {
        self.transient_sensitivity = sensitivity.clamp(0.0, 1.0);
    }

    pub fn transient_sensitivity(&self) -> f32 {
        self.transient_sensitivity
    }

    pub fn last_was_transient(&self) -> bool {
        self.last_frame_was_transient
    }

    /// Get the phase increments from the last processed frame.
    /// Used for linked-phase stereo: L channel computes these, R channel applies them.
    pub fn last_phase_increments(&self) -> &[f32] {
        &self.last_phase_increments
    }

    /// How many output samples are currently available without processing.
    pub fn output_available(&self) -> usize {
        self.output_available
    }

    /// Whether there's enough input to process at least one more frame.
    pub fn can_process(&self) -> bool {
        self.input_available >= self.fft_size
    }

    /// Read available output WITHOUT triggering frame processing.
    /// Use with manual frame processing (try_process_frame / process_frame_linked).
    pub fn drain_output(&mut self, output: &mut [f32]) -> usize {
        let to_read = output.len().min(self.output_available);
        let cap = self.output_ring.len();

        for i in 0..to_read {
            output[i] = self.output_ring[self.output_read];
            self.output_ring[self.output_read] = 0.0;
            self.window_sum_ring[self.output_read] = 0.0;
            self.output_read = (self.output_read + 1) % cap;
        }
        self.output_available -= to_read;

        for s in output[to_read..].iter_mut() {
            *s = 0.0;
        }

        to_read
    }

    /// Detect if current frame is a transient via spectral flux.
    /// Must be called after FFT (Step 2), reads magnitudes from frame_buf.
    fn detect_transient(&mut self) -> bool {
        if !self.transient_detect_enabled {
            return false;
        }

        let half = self.fft_size / 2 + 1;

        if !self.has_prev_magnitudes {
            // First frame: store magnitudes, no detection possible.
            for k in 0..half {
                self.prev_magnitudes[k] = self.frame_buf[k].norm();
            }
            self.has_prev_magnitudes = true;
            return false;
        }

        // Compute half-wave rectified spectral flux.
        let mut flux: f32 = 0.0;
        for k in 0..half {
            let current_mag = self.frame_buf[k].norm();
            let diff = current_mag - self.prev_magnitudes[k];
            if diff > 0.0 {
                flux += diff;
            }
            self.prev_magnitudes[k] = current_mag;
        }

        flux /= half as f32;

        // Update running average.
        let alpha = 0.05;
        self.running_flux = self.running_flux * (1.0 - alpha) + flux * alpha;

        // Avoid false positives when running_flux is near zero.
        if self.running_flux < 1e-8 {
            return false;
        }

        // sensitivity 0 = very sensitive (threshold = 1.5x), 1 = hard to trigger (6x).
        let threshold_multiplier = 1.5 + self.transient_sensitivity * 4.5;
        let threshold = self.running_flux * threshold_multiplier;

        flux > threshold
    }

    /// Identity phase locking: lock non-peak bins to nearest spectral peak.
    /// Modifies synth_phase in-place. Called after phase accumulation, before spectrum rebuild.
    fn apply_phase_locking(&mut self) {
        let half = self.fft_size / 2 + 1;

        // Extract magnitudes.
        for k in 0..self.fft_size {
            self.magnitudes[k] = self.frame_buf[k].norm();
        }

        // Find peaks: local maxima in bins 1..half-1.
        self.peak_flags.fill(false);
        for k in 1..half - 1 {
            if self.magnitudes[k] >= self.magnitudes[k - 1]
                && self.magnitudes[k] >= self.magnitudes[k + 1]
            {
                self.peak_flags[k] = true;
            }
        }

        // Forward pass: propagate nearest peak from left.
        let mut last_peak = 0;
        for k in 0..half {
            if self.peak_flags[k] {
                last_peak = k;
            }
            self.nearest_peak_buf[k] = last_peak;
        }

        // Backward pass: choose closer peak.
        let mut next_peak = half - 1;
        for k in (0..half).rev() {
            if self.peak_flags[k] {
                next_peak = k;
            }
            let dist_left = k.saturating_sub(self.nearest_peak_buf[k]);
            let dist_right = next_peak.saturating_sub(k);
            if dist_right < dist_left {
                self.nearest_peak_buf[k] = next_peak;
            }
        }

        // Apply identity phase locking for non-peak bins.
        // synth_phase[k] = synth_phase[p] + (analysis_phase[k] - analysis_phase[p])
        for k in 1..half - 1 {
            if !self.peak_flags[k] {
                let p = self.nearest_peak_buf[k];
                let analysis_phase_k = self.frame_buf[k].arg();
                let analysis_phase_p = self.frame_buf[p].arg();
                self.synth_phase[k] =
                    self.synth_phase[p] + (analysis_phase_k - analysis_phase_p);
            }
        }

        // Mirror to negative frequencies (conjugate symmetry).
        for k in 1..self.fft_size / 2 {
            self.synth_phase[self.fft_size - k] = -self.synth_phase[k];
        }
    }

    /// Process one frame using phase increments from a reference channel.
    /// The reference channel's instantaneous frequency estimates are used instead of
    /// this channel's own, preserving inter-channel phase relationships.
    pub fn process_frame_linked(&mut self, ref_phase_increments: &[f32], is_transient: bool) -> bool {
        if self.input_available < self.fft_size {
            return false;
        }

        let fft_size = self.fft_size;
        let analysis_hop = self.analysis_hop;
        let input_cap = self.input_ring.len();
        let output_cap = self.output_ring.len();

        let synthesis_hop = (analysis_hop as f64 * self.current_stretch)
            .round()
            .max(1.0) as usize;

        // Step 1: Copy windowed input frame.
        for i in 0..fft_size {
            let idx = (self.input_read + i) % input_cap;
            self.frame_buf[i] = Complex::new(
                self.input_ring[idx] * self.window[i],
                0.0,
            );
        }

        // Step 2: Forward FFT.
        self.fft_forward.process(&mut self.frame_buf);

        // Step 3: Use reference channel's phase increments.
        if !self.has_state {
            // First frame: initialize from analysis phase.
            for k in 0..fft_size {
                let phase = self.frame_buf[k].arg();
                self.prev_phase[k] = phase;
                self.synth_phase[k] = phase;
            }
            self.has_state = true;
        } else if is_transient && self.transient_detect_enabled {
            // Transient: reset synth_phase to analysis phase.
            for k in 0..fft_size {
                let phase = self.frame_buf[k].arg();
                self.synth_phase[k] = phase;
                self.prev_phase[k] = phase;
            }
        } else {
            // Apply the reference channel's phase increments to this channel's synth_phase.
            for k in 0..fft_size {
                self.prev_phase[k] = self.frame_buf[k].arg();
                self.synth_phase[k] += ref_phase_increments[k];
            }
        }

        // Step 3.5: Identity phase locking (optional).
        if self.phase_lock_enabled {
            self.apply_phase_locking();
        }

        // Step 4: Rebuild spectrum with modified phases (use this channel's magnitudes).
        for k in 0..fft_size {
            let mag = self.frame_buf[k].norm();
            self.frame_buf[k] = Complex::from_polar(mag, self.synth_phase[k]);
        }

        // Step 5: Inverse FFT.
        self.fft_inverse.process(&mut self.frame_buf);

        // Step 6: Overlap-add into output ring.
        let norm = 1.0 / fft_size as f32;
        for i in 0..fft_size {
            let out_idx = (self.output_write + i) % output_cap;
            let w = self.window[i];
            self.output_ring[out_idx] += self.frame_buf[i].re * norm * w;
            self.window_sum_ring[out_idx] += w * w;
        }

        // Advance input.
        self.input_read = (self.input_read + analysis_hop) % input_cap;
        self.input_available -= analysis_hop;

        // Normalize newly ready samples.
        let normalize_start = (self.output_read + self.output_available) % output_cap;
        for i in 0..synthesis_hop {
            let idx = (normalize_start + i) % output_cap;
            if self.window_sum_ring[idx] > 1e-6 {
                self.output_ring[idx] /= self.window_sum_ring[idx];
            }
        }
        self.output_available += synthesis_hop;
        self.output_write = (self.output_write + synthesis_hop) % output_cap;

        true
    }

    /// Feed mono input samples into the input ring buffer.
    /// Returns how many samples were consumed.
    pub fn write_input(&mut self, samples: &[f32]) -> usize {
        let cap = self.input_ring.len();
        let space = cap - self.input_available;
        let to_write = samples.len().min(space);

        for i in 0..to_write {
            self.input_ring[self.input_write] = samples[i];
            self.input_write = (self.input_write + 1) % cap;
        }
        self.input_available += to_write;
        to_write
    }

    /// Pull processed samples. Runs vocoder frames as needed.
    pub fn read_output(&mut self, output: &mut [f32]) -> usize {
        let requested = output.len();

        while self.output_available < requested {
            if !self.try_process_frame() {
                break;
            }
        }

        let to_read = requested.min(self.output_available);
        let cap = self.output_ring.len();

        for i in 0..to_read {
            output[i] = self.output_ring[self.output_read];
            self.output_ring[self.output_read] = 0.0;
            self.window_sum_ring[self.output_read] = 0.0;
            self.output_read = (self.output_read + 1) % cap;
        }
        self.output_available -= to_read;

        for s in output[to_read..].iter_mut() {
            *s = 0.0;
        }

        to_read
    }

    pub fn try_process_frame(&mut self) -> bool {
        if self.input_available < self.fft_size {
            return false;
        }

        let fft_size = self.fft_size;
        let analysis_hop = self.analysis_hop;
        let input_cap = self.input_ring.len();
        let output_cap = self.output_ring.len();

        let synthesis_hop = (analysis_hop as f64 * self.current_stretch)
            .round()
            .max(1.0) as usize;

        // Step 1: Copy windowed input frame.
        for i in 0..fft_size {
            let idx = (self.input_read + i) % input_cap;
            self.frame_buf[i] = Complex::new(
                self.input_ring[idx] * self.window[i],
                0.0,
            );
        }

        // Step 2: Forward FFT.
        self.fft_forward.process(&mut self.frame_buf);

        // Step 2.5: Transient detection.
        let is_transient = self.detect_transient();
        self.last_frame_was_transient = is_transient;

        // Step 3: Phase accumulation + store increments for linked-phase stereo.
        if !self.has_state {
            for k in 0..fft_size {
                let phase = self.frame_buf[k].arg();
                self.prev_phase[k] = phase;
                self.synth_phase[k] = phase;
                self.last_phase_increments[k] = phase; // Initial: use raw phase.
            }
            self.has_state = true;
        } else if is_transient {
            // Transient: reset synth_phase to analysis phase for coherence.
            // Still compute & store increments for the linked R channel.
            let hop_ratio = synthesis_hop as f32 / analysis_hop as f32;
            for k in 0..fft_size {
                let phase = self.frame_buf[k].arg();
                let mut dp = phase - self.prev_phase[k] - self.bin_freq[k];
                dp -= (dp / (2.0 * PI)).round() * 2.0 * PI;
                let inst_freq = self.bin_freq[k] + dp;
                self.last_phase_increments[k] = inst_freq * hop_ratio;
                self.synth_phase[k] = phase; // RESET instead of accumulate.
                self.prev_phase[k] = phase;
            }
        } else {
            let hop_ratio = synthesis_hop as f32 / analysis_hop as f32;
            for k in 0..fft_size {
                let phase = self.frame_buf[k].arg();
                let mut dp = phase - self.prev_phase[k] - self.bin_freq[k];
                dp -= (dp / (2.0 * PI)).round() * 2.0 * PI;
                let inst_freq = self.bin_freq[k] + dp;
                let increment = inst_freq * hop_ratio;
                self.synth_phase[k] += increment;
                self.prev_phase[k] = phase;
                self.last_phase_increments[k] = increment;
            }
        }

        // Step 3.5: Identity phase locking (optional).
        if self.phase_lock_enabled {
            self.apply_phase_locking();
        }

        // Step 4: Rebuild spectrum with modified phases.
        for k in 0..fft_size {
            let mag = self.frame_buf[k].norm();
            self.frame_buf[k] = Complex::from_polar(mag, self.synth_phase[k]);
        }

        // Step 5: Inverse FFT.
        self.fft_inverse.process(&mut self.frame_buf);

        // Step 6: Overlap-add into output ring.
        let norm = 1.0 / fft_size as f32;
        for i in 0..fft_size {
            let out_idx = (self.output_write + i) % output_cap;
            let w = self.window[i];
            self.output_ring[out_idx] += self.frame_buf[i].re * norm * w;
            self.window_sum_ring[out_idx] += w * w;
        }

        // Advance input.
        self.input_read = (self.input_read + analysis_hop) % input_cap;
        self.input_available -= analysis_hop;

        // Normalize newly ready samples.
        let normalize_start = (self.output_read + self.output_available) % output_cap;
        for i in 0..synthesis_hop {
            let idx = (normalize_start + i) % output_cap;
            if self.window_sum_ring[idx] > 1e-6 {
                self.output_ring[idx] /= self.window_sum_ring[idx];
            }
        }
        self.output_available += synthesis_hop;
        self.output_write = (self.output_write + synthesis_hop) % output_cap;

        true
    }
}

fn hann_window(size: usize) -> Vec<f32> {
    (0..size)
        .map(|i| 0.5 * (1.0 - (2.0 * PI * i as f32 / size as f32).cos()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sine(freq: f32, sample_rate: f32, duration_secs: f32) -> Vec<f32> {
        let n = (sample_rate * duration_secs) as usize;
        (0..n)
            .map(|i| (2.0 * PI * freq * i as f32 / sample_rate).sin() * 0.5)
            .collect()
    }

    fn stream_all(sv: &mut StreamingPhaseVocoder, input: &[f32], write_chunk: usize) -> Vec<f32> {
        let read_chunk = 1024;
        let mut output = Vec::new();
        let mut input_pos = 0;
        let mut read_buf = vec![0.0f32; read_chunk];

        loop {
            if input_pos < input.len() {
                let end = (input_pos + write_chunk).min(input.len());
                let written = sv.write_input(&input[input_pos..end]);
                input_pos += written;
            }

            let read = sv.read_output(&mut read_buf);
            if read > 0 {
                output.extend_from_slice(&read_buf[..read]);
            }

            if input_pos >= input.len() && read == 0 {
                break;
            }
        }
        output
    }

    #[test]
    fn test_stretch_1x_preserves_length() {
        let mut sv = StreamingPhaseVocoder::new(1024, 4);
        sv.set_stretch(1.0);

        let input = make_sine(440.0, 44100.0, 1.0);
        let output = stream_all(&mut sv, &input, 2048);

        let ratio = output.len() as f64 / input.len() as f64;
        assert!(
            (ratio - 1.0).abs() < 0.15,
            "Expected ~1.0x, got {ratio}"
        );
    }

    #[test]
    fn test_stretch_2x_doubles_length() {
        let mut sv = StreamingPhaseVocoder::new(1024, 4);
        sv.set_stretch(2.0);

        let input = make_sine(440.0, 44100.0, 1.0);
        let output = stream_all(&mut sv, &input, 2048);

        let ratio = output.len() as f64 / input.len() as f64;
        assert!(
            (ratio - 2.0).abs() < 0.25,
            "Expected ~2.0x, got {ratio}"
        );
    }

    #[test]
    fn test_stretch_half_halves_length() {
        let mut sv = StreamingPhaseVocoder::new(1024, 4);
        sv.set_stretch(0.5);

        let input = make_sine(440.0, 44100.0, 1.0);
        let output = stream_all(&mut sv, &input, 2048);

        let ratio = output.len() as f64 / input.len() as f64;
        assert!(
            (ratio - 0.5).abs() < 0.15,
            "Expected ~0.5x, got {ratio}"
        );
    }

    #[test]
    fn test_hann_window_shape() {
        let w = hann_window(1024);
        assert_eq!(w.len(), 1024);

        // Endpoints should be near zero.
        assert!(w[0].abs() < 1e-6, "window[0] should be ~0, got {}", w[0]);
        assert!(
            w[1023] < 0.01,
            "window[last] should be near 0, got {}",
            w[1023]
        );

        // Center should be near 1.0.
        assert!(
            (w[512] - 1.0).abs() < 0.01,
            "window[center] should be ~1.0, got {}",
            w[512]
        );

        // All values in [0, 1].
        for (i, &v) in w.iter().enumerate() {
            assert!(v >= 0.0 && v <= 1.0001, "window[{i}] = {v} out of range");
        }

        // Symmetry: w[i] ≈ w[N-1-i] (approximately for even-length periodic Hann).
        for i in 0..512 {
            let diff = (w[i] - w[1023 - i]).abs();
            assert!(
                diff < 0.01,
                "asymmetry at {i}: w[{i}]={} vs w[{}]={}",
                w[i],
                1023 - i,
                w[1023 - i]
            );
        }
    }

    #[test]
    fn test_vocoder_new_sizes() {
        let sv = StreamingPhaseVocoder::new(2048, 4);

        // analysis_hop = fft_size / overlap = 2048 / 4 = 512.
        assert_eq!(sv.analysis_hop, 512);
        assert_eq!(sv.fft_size, 2048);
        assert_eq!(sv.input_ring.len(), 2048 * 8);
        assert_eq!(sv.output_ring.len(), 2048 * 16);
        assert!((sv.current_stretch - 1.0).abs() < 1e-6);
        assert_eq!(sv.prev_phase.len(), 2048);
        assert_eq!(sv.synth_phase.len(), 2048);
        assert_eq!(sv.last_phase_increments.len(), 2048);
        assert_eq!(sv.window.len(), 2048);
    }

    #[test]
    fn test_vocoder_reset() {
        let mut sv = StreamingPhaseVocoder::new(1024, 4);
        let input = make_sine(440.0, 44100.0, 0.5);

        // Feed data and process.
        sv.write_input(&input[..4096]);
        while sv.can_process() {
            sv.try_process_frame();
        }
        assert!(sv.output_available() > 0, "should have output before reset");

        // Reset.
        sv.reset();
        assert_eq!(sv.output_available(), 0, "output_available should be 0 after reset");
        assert!(!sv.can_process(), "can_process should be false after reset");
        assert!(!sv.has_state, "has_state should be false after reset");
    }

    #[test]
    fn test_vocoder_set_stretch_clamp() {
        let mut sv = StreamingPhaseVocoder::new(1024, 4);

        // Below minimum → clamped to 0.1.
        sv.set_stretch(0.01);
        assert!(
            (sv.current_stretch - 0.1).abs() < 1e-6,
            "expected 0.1, got {}",
            sv.current_stretch
        );

        // Above maximum → clamped to 10.0.
        sv.set_stretch(100.0);
        assert!(
            (sv.current_stretch - 10.0).abs() < 1e-6,
            "expected 10.0, got {}",
            sv.current_stretch
        );

        // Normal value stays.
        sv.set_stretch(2.5);
        assert!(
            (sv.current_stretch - 2.5).abs() < 1e-6,
            "expected 2.5, got {}",
            sv.current_stretch
        );
    }

    #[test]
    fn test_last_phase_increments_populated() {
        let mut sv = StreamingPhaseVocoder::new(1024, 4);
        let input = make_sine(440.0, 44100.0, 0.5);

        // Before processing, increments are all zero.
        assert!(
            sv.last_phase_increments().iter().all(|&v| v == 0.0),
            "increments should be zero before processing"
        );

        // Feed and process.
        sv.write_input(&input[..4096]);
        assert!(sv.can_process());
        sv.try_process_frame();

        // After processing, some increments should be non-zero.
        let non_zero = sv
            .last_phase_increments()
            .iter()
            .filter(|&&v| v.abs() > 1e-10)
            .count();
        assert!(
            non_zero > 0,
            "expected non-zero phase increments after processing"
        );
        assert_eq!(sv.last_phase_increments().len(), 1024);
    }

    #[test]
    fn test_can_process_threshold() {
        let mut sv = StreamingPhaseVocoder::new(1024, 4);

        // Initially empty → can't process.
        assert!(!sv.can_process());

        // Feed less than fft_size → still can't.
        let partial = vec![0.0f32; 512];
        sv.write_input(&partial);
        assert!(!sv.can_process());

        // Feed more to reach fft_size → can process.
        let more = vec![0.0f32; 512];
        sv.write_input(&more);
        assert!(sv.can_process(), "should be able to process with exactly fft_size samples");
    }

    #[test]
    fn test_write_input_fills_ring() {
        let mut sv = StreamingPhaseVocoder::new(1024, 4);
        let cap = sv.input_ring.len(); // 1024 * 8 = 8192

        // Fill to capacity.
        let data = vec![1.0f32; cap];
        let written = sv.write_input(&data);
        assert_eq!(written, cap);

        // Trying to write more should return 0 (ring full).
        let extra = vec![1.0f32; 100];
        let written = sv.write_input(&extra);
        assert_eq!(written, 0, "should not write when ring buffer is full");
    }

    #[test]
    fn test_try_process_frame_identity() {
        // At stretch=1.0, output should closely resemble input.
        let mut sv = StreamingPhaseVocoder::new(1024, 4);
        sv.set_stretch(1.0);

        let input = make_sine(440.0, 44100.0, 0.5);
        let output = stream_all(&mut sv, &input, 2048);

        // Check that output has reasonable amplitude (not silent, not clipped).
        let max_abs = output.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        assert!(max_abs > 0.1, "output is too quiet: max_abs={max_abs}");
        assert!(max_abs < 2.0, "output is too loud: max_abs={max_abs}");

        // Length should be ~1.0x.
        let ratio = output.len() as f64 / input.len() as f64;
        assert!(
            (ratio - 1.0).abs() < 0.15,
            "expected ~1.0x length, got {ratio:.3}"
        );
    }

    #[test]
    fn test_linked_phase_matches_independent_mags() {
        let mut sv_l = StreamingPhaseVocoder::new(1024, 4);
        let mut sv_r = StreamingPhaseVocoder::new(1024, 4);
        sv_l.set_stretch(1.5);
        sv_r.set_stretch(1.5);

        let input = make_sine(440.0, 44100.0, 0.5);

        // Feed same data to both.
        sv_l.write_input(&input[..4096]);
        sv_r.write_input(&input[..4096]);

        // Process L normally.
        assert!(sv_l.can_process());
        sv_l.try_process_frame();
        let increments = sv_l.last_phase_increments().to_vec();
        let is_transient = sv_l.last_was_transient();

        // Process R with linked phase.
        assert!(sv_r.can_process());
        let ok = sv_r.process_frame_linked(&increments, is_transient);
        assert!(ok, "process_frame_linked should succeed");

        // Both should have output available.
        assert!(sv_l.output_available() > 0, "L should have output");
        assert!(sv_r.output_available() > 0, "R should have output");

        // Drain and verify R has reasonable amplitude.
        let mut out_r = vec![0.0f32; sv_r.output_available()];
        let read = sv_r.drain_output(&mut out_r);
        assert!(read > 0);
        let max_r = out_r[..read].iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        assert!(max_r > 0.01, "linked R output is too quiet: {max_r}");
    }

    #[test]
    fn test_drain_vs_read() {
        let mut sv = StreamingPhaseVocoder::new(1024, 4);
        sv.set_stretch(1.0);

        let input = make_sine(440.0, 44100.0, 0.5);
        sv.write_input(&input[..4096]);

        // drain_output should NOT trigger processing.
        let mut out = vec![0.0f32; 1024];
        let drained = sv.drain_output(&mut out);
        assert_eq!(drained, 0, "drain should produce 0 without prior processing");

        // read_output SHOULD trigger processing.
        let mut out2 = vec![0.0f32; 1024];
        let read = sv.read_output(&mut out2);
        assert!(read > 0, "read_output should trigger processing and produce output");
    }

    #[test]
    fn test_phase_lock_produces_output() {
        let mut sv = StreamingPhaseVocoder::new(1024, 4);
        sv.set_stretch(1.5);
        sv.set_phase_lock(true);

        let input = make_sine(440.0, 44100.0, 0.5);
        let output = stream_all(&mut sv, &input, 2048);

        let max_abs = output.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        assert!(max_abs > 0.05, "phase-locked output is too quiet: {max_abs}");

        let ratio = output.len() as f64 / input.len() as f64;
        assert!(
            (ratio - 1.5).abs() < 0.25,
            "phase-locked: expected ~1.5x length, got {ratio:.3}"
        );
    }

    #[test]
    fn test_transient_detect_produces_output() {
        let mut sv = StreamingPhaseVocoder::new(1024, 4);
        sv.set_stretch(1.0);
        sv.set_transient_detect(true);
        sv.set_transient_sensitivity(0.5);

        // Create audio with a transient: silence, then sudden burst.
        let mut input = vec![0.0f32; 2048];
        let burst = make_sine(440.0, 44100.0, 0.3);
        input.extend_from_slice(&burst);

        let output = stream_all(&mut sv, &input, 2048);

        let max_abs = output.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        assert!(max_abs > 0.01, "transient-detect output should not be silent: {max_abs}");
    }
}

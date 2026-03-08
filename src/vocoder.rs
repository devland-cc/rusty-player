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
    }

    pub fn set_stretch(&mut self, ratio: f64) {
        self.current_stretch = ratio.max(0.1).min(10.0);
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

    /// Process one frame using phase increments from a reference channel.
    /// The reference channel's instantaneous frequency estimates are used instead of
    /// this channel's own, preserving inter-channel phase relationships.
    pub fn process_frame_linked(&mut self, ref_phase_increments: &[f32]) -> bool {
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
        } else {
            // Apply the reference channel's phase increments to this channel's synth_phase.
            for k in 0..fft_size {
                self.prev_phase[k] = self.frame_buf[k].arg();
                self.synth_phase[k] += ref_phase_increments[k];
            }
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

        // Step 3: Phase accumulation + store increments for linked-phase stereo.
        if !self.has_state {
            for k in 0..fft_size {
                let phase = self.frame_buf[k].arg();
                self.prev_phase[k] = phase;
                self.synth_phase[k] = phase;
                self.last_phase_increments[k] = phase; // Initial: use raw phase.
            }
            self.has_state = true;
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
}

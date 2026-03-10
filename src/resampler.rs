/// Streaming resampler with linear or cubic Hermite interpolation.
///
/// Processes mono audio sample-by-sample with zero latency.
/// The ratio can be changed at any time for real-time pitch correction.
pub struct StreamingResampler {
    /// Output-to-input ratio: `output_rate / input_rate`.
    /// ratio > 1.0 produces more output samples (lower pitch).
    /// ratio < 1.0 produces fewer output samples (higher pitch).
    ratio: f64,
    frac_pos: f64,
    prev_sample: f32,
    has_prev: bool,
    prev_prev_sample: f32,
    has_prev_prev: bool,
    cubic_enabled: bool,
}

impl StreamingResampler {
    pub fn new() -> Self {
        Self {
            ratio: 1.0,
            frac_pos: 0.0,
            prev_sample: 0.0,
            has_prev: false,
            prev_prev_sample: 0.0,
            has_prev_prev: false,
            cubic_enabled: false,
        }
    }

    pub fn set_ratio(&mut self, ratio: f64) {
        self.ratio = ratio.max(0.05).min(20.0);
    }

    pub fn set_cubic(&mut self, enabled: bool) {
        self.cubic_enabled = enabled;
    }

    pub fn cubic(&self) -> bool {
        self.cubic_enabled
    }

    pub fn reset(&mut self) {
        self.frac_pos = 0.0;
        self.prev_sample = 0.0;
        self.has_prev = false;
        self.prev_prev_sample = 0.0;
        self.has_prev_prev = false;
    }

    /// Process input samples, producing resampled output.
    /// Returns `(input_consumed, output_produced)`.
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) -> (usize, usize) {
        if input.is_empty() || output.is_empty() {
            return (0, 0);
        }

        // Bypass when ratio ≈ 1.0.
        if (self.ratio - 1.0).abs() < 1e-6 {
            let n = input.len().min(output.len());
            output[..n].copy_from_slice(&input[..n]);
            if n >= 2 {
                self.prev_prev_sample = input[n - 2];
                self.has_prev_prev = true;
            } else if n == 1 && self.has_prev {
                self.prev_prev_sample = self.prev_sample;
                self.has_prev_prev = true;
            }
            if n > 0 {
                self.prev_sample = input[n - 1];
                self.has_prev = true;
            }
            return (n, n);
        }

        let step = 1.0 / self.ratio; // input advance per output sample
        let mut out_pos = 0usize;

        if self.cubic_enabled {
            // 4-point Hermite cubic interpolation.
            while out_pos < output.len() {
                let int_pos = self.frac_pos as usize;
                let frac = (self.frac_pos - int_pos as f64) as f32;

                // Need int_pos + 2 in bounds for y2.
                if int_pos + 2 >= input.len() {
                    break;
                }

                // ym1: sample at int_pos - 1.
                let ym1 = if int_pos == 0 {
                    if self.has_prev { self.prev_sample } else { input[0] }
                } else {
                    input[int_pos - 1]
                };
                let y0 = input[int_pos];
                let y1 = input[int_pos + 1];
                let y2 = input[int_pos + 2];

                output[out_pos] = hermite4(frac, ym1, y0, y1, y2);
                out_pos += 1;
                self.frac_pos += step;
            }
        } else {
            // 2-point linear interpolation (original).
            while out_pos < output.len() {
                let int_pos = self.frac_pos as usize;
                let frac = self.frac_pos - int_pos as f64;

                if int_pos + 1 >= input.len() {
                    break;
                }

                let s0 = if int_pos == 0 && self.has_prev && self.frac_pos < 1.0 {
                    self.prev_sample
                } else if int_pos < input.len() {
                    input[int_pos]
                } else {
                    break;
                };

                let s1 = if int_pos + 1 < input.len() {
                    input[int_pos + 1]
                } else {
                    input[int_pos]
                };

                output[out_pos] = s0 + (s1 - s0) * frac as f32;
                out_pos += 1;
                self.frac_pos += step;
            }
        }

        let consumed = self.frac_pos as usize;
        self.frac_pos -= consumed as f64;

        // Update prev_prev and prev samples for cross-buffer continuity.
        if consumed >= 2 && consumed <= input.len() {
            self.prev_prev_sample = input[consumed - 2];
            self.has_prev_prev = true;
        } else if consumed == 1 {
            self.prev_prev_sample = self.prev_sample;
            self.has_prev_prev = self.has_prev;
        }

        if consumed > 0 && consumed <= input.len() {
            self.prev_sample = input[consumed - 1];
            self.has_prev = true;
        } else if !input.is_empty() {
            self.prev_sample = input[input.len() - 1];
            self.has_prev = true;
        }

        (consumed.min(input.len()), out_pos)
    }
}

/// 4-point Hermite cubic interpolation.
fn hermite4(frac: f32, ym1: f32, y0: f32, y1: f32, y2: f32) -> f32 {
    let c0 = y0;
    let c1 = 0.5 * (y1 - ym1);
    let c2 = ym1 - 2.5 * y0 + 2.0 * y1 - 0.5 * y2;
    let c3 = 0.5 * (y2 - ym1) + 1.5 * (y0 - y1);
    ((c3 * frac + c2) * frac + c1) * frac + c0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn make_sine(freq: f32, sample_rate: f32, duration_secs: f32) -> Vec<f32> {
        let n = (sample_rate * duration_secs) as usize;
        (0..n)
            .map(|i| (2.0 * PI * freq * i as f32 / sample_rate).sin() * 0.5)
            .collect()
    }

    #[test]
    fn test_ratio_1_passthrough() {
        let mut r = StreamingResampler::new();
        r.set_ratio(1.0);

        let input = make_sine(440.0, 44100.0, 0.1);
        let mut output = vec![0.0f32; input.len()];
        let (consumed, produced) = r.process(&input, &mut output);

        assert_eq!(consumed, input.len());
        assert_eq!(produced, input.len());
        for i in 0..input.len() {
            assert!((output[i] - input[i]).abs() < 1e-6);
        }
    }

    #[test]
    fn test_ratio_2_doubles_output() {
        let mut r = StreamingResampler::new();
        r.set_ratio(2.0);

        let input = make_sine(440.0, 44100.0, 0.5);
        let mut output = vec![0.0f32; input.len() * 3];
        let (consumed, produced) = r.process(&input, &mut output);

        let ratio = produced as f64 / consumed as f64;
        assert!(
            (ratio - 2.0).abs() < 0.1,
            "Expected ~2.0x, got {ratio}"
        );
    }

    #[test]
    fn test_ratio_half_halves_output() {
        let mut r = StreamingResampler::new();
        r.set_ratio(0.5);

        let input = make_sine(440.0, 44100.0, 0.5);
        let mut output = vec![0.0f32; input.len()];
        let (consumed, produced) = r.process(&input, &mut output);

        let ratio = produced as f64 / consumed as f64;
        assert!(
            (ratio - 0.5).abs() < 0.1,
            "Expected ~0.5x, got {ratio}"
        );
    }

    #[test]
    fn test_new_defaults() {
        let r = StreamingResampler::new();
        // Verify identity state: ratio=1.0 passthrough.
        let mut r = r;
        let input = [1.0f32, 2.0, 3.0, 4.0, 5.0];
        let mut output = [0.0f32; 5];
        let (consumed, produced) = r.process(&input, &mut output);
        assert_eq!(consumed, 5);
        assert_eq!(produced, 5);
        for i in 0..5 {
            assert!((output[i] - input[i]).abs() < 1e-6, "sample {i} mismatch");
        }
    }

    #[test]
    fn test_set_ratio_clamping() {
        let mut r = StreamingResampler::new();

        // Below minimum → clamped to 0.05.
        r.set_ratio(0.001);
        let input = make_sine(440.0, 44100.0, 0.1);
        let mut output = vec![0.0f32; input.len()];
        let (_consumed, produced) = r.process(&input, &mut output);
        // ratio=0.05 → very few output samples.
        assert!(produced < input.len() / 10, "expected very few outputs at ratio=0.05, got {produced}");

        // Above maximum → clamped to 20.0.
        r.reset();
        r.set_ratio(100.0);
        let mut output_big = vec![0.0f32; input.len() * 25];
        let (consumed, produced) = r.process(&input, &mut output_big);
        let ratio = produced as f64 / consumed as f64;
        assert!(
            (ratio - 20.0).abs() < 1.0,
            "expected ~20.0x at clamped max, got {ratio:.2}"
        );
    }

    #[test]
    fn test_reset_clears_state() {
        let mut r = StreamingResampler::new();
        r.set_ratio(2.0);

        // Process some data to build up internal state.
        let input = make_sine(440.0, 44100.0, 0.1);
        let mut output = vec![0.0f32; input.len() * 3];
        r.process(&input, &mut output);

        // Reset should clear frac_pos and prev_sample but preserve ratio.
        r.reset();

        // After reset, processing the same input should give the same result
        // as a fresh resampler with ratio=2.0.
        let mut r2 = StreamingResampler::new();
        r2.set_ratio(2.0);

        let mut out1 = vec![0.0f32; input.len() * 3];
        let mut out2 = vec![0.0f32; input.len() * 3];
        let (c1, p1) = r.process(&input, &mut out1);
        let (c2, p2) = r2.process(&input, &mut out2);

        assert_eq!(c1, c2, "consumed should match after reset");
        assert_eq!(p1, p2, "produced should match after reset");
        for i in 0..p1 {
            assert!(
                (out1[i] - out2[i]).abs() < 1e-6,
                "sample {i} differs after reset: {} vs {}",
                out1[i],
                out2[i]
            );
        }
    }

    #[test]
    fn test_process_cross_buffer_continuity() {
        // Processing in two chunks should produce the same result as one big chunk.
        let mut r_chunked = StreamingResampler::new();
        r_chunked.set_ratio(1.5);

        let input = make_sine(440.0, 44100.0, 0.1);
        let mid = input.len() / 2;

        // Process in two halves.
        let mut out1 = vec![0.0f32; input.len() * 2];
        let (_, p1) = r_chunked.process(&input[..mid], &mut out1);
        let mut out2 = vec![0.0f32; input.len() * 2];
        let (_, p2) = r_chunked.process(&input[mid..], &mut out2);

        let total_chunked = p1 + p2;

        // Process all at once.
        let mut r_single = StreamingResampler::new();
        r_single.set_ratio(1.5);
        let mut out_single = vec![0.0f32; input.len() * 2];
        let (_, p_single) = r_single.process(&input, &mut out_single);

        // Output counts should be very close (within a sample or two due to boundary).
        let diff = (total_chunked as i64 - p_single as i64).unsigned_abs();
        assert!(
            diff <= 2,
            "chunked ({total_chunked}) vs single ({p_single}) differ by {diff}"
        );
    }

    #[test]
    fn test_process_empty_input() {
        let mut r = StreamingResampler::new();
        r.set_ratio(1.5);

        let mut output = vec![0.0f32; 100];
        let (consumed, produced) = r.process(&[], &mut output);
        assert_eq!(consumed, 0);
        assert_eq!(produced, 0);

        // Also test empty output buffer.
        let input = [1.0f32; 10];
        let (consumed, produced) = r.process(&input, &mut []);
        assert_eq!(consumed, 0);
        assert_eq!(produced, 0);
    }

    #[test]
    fn test_cubic_ratio_2_output_count() {
        let mut r = StreamingResampler::new();
        r.set_ratio(2.0);
        r.set_cubic(true);

        let input = make_sine(440.0, 44100.0, 0.5);
        let mut output = vec![0.0f32; input.len() * 3];
        let (consumed, produced) = r.process(&input, &mut output);

        let ratio = produced as f64 / consumed as f64;
        assert!(
            (ratio - 2.0).abs() < 0.15,
            "Cubic: Expected ~2.0x, got {ratio}"
        );
    }

    #[test]
    fn test_hermite4_known_values() {
        // When frac=0, should return y0.
        let val = hermite4(0.0, 1.0, 2.0, 3.0, 4.0);
        assert!((val - 2.0).abs() < 1e-6, "hermite4(0) should be y0, got {val}");

        // When frac=1, should return y1.
        let val = hermite4(1.0, 1.0, 2.0, 3.0, 4.0);
        assert!((val - 3.0).abs() < 1e-6, "hermite4(1) should be y1, got {val}");

        // Linear data: hermite should produce exact linear interpolation.
        let val = hermite4(0.5, 0.0, 1.0, 2.0, 3.0);
        assert!((val - 1.5).abs() < 1e-6, "hermite4(0.5) on linear data should be 1.5, got {val}");
    }
}

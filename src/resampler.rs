/// Streaming resampler using linear interpolation.
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
}

impl StreamingResampler {
    pub fn new() -> Self {
        Self {
            ratio: 1.0,
            frac_pos: 0.0,
            prev_sample: 0.0,
            has_prev: false,
        }
    }

    pub fn set_ratio(&mut self, ratio: f64) {
        self.ratio = ratio.max(0.05).min(20.0);
    }

    pub fn reset(&mut self) {
        self.frac_pos = 0.0;
        self.prev_sample = 0.0;
        self.has_prev = false;
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
            if n > 0 {
                self.prev_sample = input[n - 1];
                self.has_prev = true;
            }
            return (n, n);
        }

        let step = 1.0 / self.ratio; // input advance per output sample
        let mut out_pos = 0usize;

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

        let consumed = self.frac_pos as usize;
        self.frac_pos -= consumed as f64;

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
}

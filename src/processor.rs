use crate::resampler::StreamingResampler;
use crate::vocoder::StreamingPhaseVocoder;

use std::f32::consts::PI;

const FFT_SIZE: usize = 4096;
const OVERLAP: usize = 8;
const FEED_CHUNK: usize = 512;

pub struct AudioProcessor {
    vocoders: Vec<StreamingPhaseVocoder>,
    resamplers: Vec<StreamingResampler>,
    channels: usize,
    source_samples: Vec<f32>,
    source_pos: usize,
    output_sample_rate: u32,

    // Target values (set by user via set_tempo/set_pitch).
    target_tempo: f64,
    target_pitch: f64,

    // Smoothed current values (interpolated toward targets).
    tempo_ratio: f64,
    pitch_semitones: f64,

    playing: bool,
    vocoder_primed: bool,  // Whether vocoders have been pre-filled with data.
    mid_side_mode: bool,    // Post-processing stereo width correction.
    stereo_correction: f64, // Smoothed stereo width correction factor.
    gain_comp_amount: f64,  // Fixed makeup gain amount (0.0 = 0dB, 1.0 = +6dB).

    // Per-channel temp buffers.
    mono_in: Vec<f32>,
    mono_stretched: Vec<f32>,
    mono_resampled: Vec<f32>,
}

impl AudioProcessor {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            vocoders: Vec::new(),
            resamplers: Vec::new(),
            channels: 2,
            source_samples: Vec::new(),
            source_pos: 0,
            output_sample_rate: sample_rate,
            target_tempo: 1.0,
            target_pitch: 0.0,
            tempo_ratio: 1.0,
            pitch_semitones: 0.0,
            playing: false,
            vocoder_primed: false,
            mid_side_mode: true,
            stereo_correction: 1.0,
            gain_comp_amount: 0.5,
            mono_in: Vec::new(),
            mono_stretched: Vec::new(),
            mono_resampled: Vec::new(),
        }
    }

    pub fn load(
        &mut self,
        samples: Vec<f32>,
        channels: usize,
        source_sample_rate: u32,
    ) {
        // Resample to output rate if source rate differs.
        let resampled = if source_sample_rate != self.output_sample_rate {
            let ratio = self.output_sample_rate as f64 / source_sample_rate as f64;
            resample_buffer(&samples, channels, ratio)
        } else {
            samples
        };

        self.source_samples = resampled;
        self.channels = channels;
        self.source_pos = 0;
        self.playing = false;
        self.vocoder_primed = false;
        self.stereo_correction = 1.0;

        self.vocoders.clear();
        self.resamplers.clear();
        for _ in 0..channels {
            self.vocoders.push(StreamingPhaseVocoder::new(FFT_SIZE, OVERLAP));
            self.resamplers.push(StreamingResampler::new());
        }
        self.apply_dsp_params();
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Generate a test tone (stereo 440Hz sine wave) to verify audio pipeline.
    pub fn load_test_tone(&mut self, duration_secs: f64) {
        let sr = self.output_sample_rate as f64;
        let channels = 2usize;
        let total_frames = (sr * duration_secs) as usize;
        let mut samples = vec![0.0f32; total_frames * channels];

        for f in 0..total_frames {
            let t = f as f64 / sr;
            let val = (2.0 * PI as f64 * 440.0 * t).sin() as f32 * 0.3;
            samples[f * channels] = val;     // L
            samples[f * channels + 1] = val; // R
        }

        self.source_samples = samples;
        self.channels = channels;
        self.source_pos = 0;
        self.playing = false;
        self.vocoder_primed = false;
        self.stereo_correction = 1.0;

        self.vocoders.clear();
        self.resamplers.clear();
        for _ in 0..channels {
            self.vocoders.push(StreamingPhaseVocoder::new(FFT_SIZE, OVERLAP));
            self.resamplers.push(StreamingResampler::new());
        }
        self.apply_dsp_params();
    }

    pub fn set_tempo(&mut self, ratio: f64) {
        self.target_tempo = ratio.max(0.25).min(4.0);
    }

    pub fn set_pitch(&mut self, semitones: f64) {
        self.target_pitch = semitones.max(-12.0).min(12.0);
    }

    pub fn set_mid_side_mode(&mut self, enabled: bool) {
        self.mid_side_mode = enabled;
    }

    pub fn mid_side_mode(&self) -> bool {
        self.mid_side_mode
    }

    pub fn set_gain_comp_amount(&mut self, amount: f64) {
        self.gain_comp_amount = amount.clamp(0.0, 1.0);
    }

    pub fn gain_comp_amount(&self) -> f64 {
        self.gain_comp_amount
    }

    pub fn play(&mut self) {
        self.playing = true;
    }

    pub fn pause(&mut self) {
        self.playing = false;
    }

    pub fn seek(&mut self, position_secs: f64) {
        let frame = (position_secs * self.output_sample_rate as f64) as usize;
        self.source_pos = (frame * self.channels).min(self.source_samples.len());
        self.vocoder_primed = false;
        self.stereo_correction = 1.0;
        for v in &mut self.vocoders {
            v.reset();
        }
        for r in &mut self.resamplers {
            r.reset();
        }
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn position_secs(&self) -> f64 {
        let frame = self.source_pos / self.channels.max(1);
        frame as f64 / self.output_sample_rate as f64
    }

    pub fn duration_secs(&self) -> f64 {
        let frames = self.source_samples.len() / self.channels.max(1);
        frames as f64 / self.output_sample_rate as f64
    }

    pub fn is_loaded(&self) -> bool {
        !self.source_samples.is_empty()
    }

    pub fn sample_rate(&self) -> u32 {
        self.output_sample_rate
    }

    fn is_bypass(&self) -> bool {
        (self.tempo_ratio - 1.0).abs() < 0.005
            && self.pitch_semitones.abs() < 0.05
            && (self.target_tempo - 1.0).abs() < 0.005
            && self.target_pitch.abs() < 0.05
    }

    /// Smoothly interpolate current params toward targets, then update DSP.
    fn smooth_and_update_params(&mut self) {
        const ALPHA: f64 = 0.5; // Fast convergence: 50% per step, ~3 steps to settle.

        self.tempo_ratio += (self.target_tempo - self.tempo_ratio) * ALPHA;
        self.pitch_semitones += (self.target_pitch - self.pitch_semitones) * ALPHA;

        // Snap to target when very close.
        if (self.tempo_ratio - self.target_tempo).abs() < 0.001 {
            self.tempo_ratio = self.target_tempo;
        }
        if (self.pitch_semitones - self.target_pitch).abs() < 0.01 {
            self.pitch_semitones = self.target_pitch;
        }

        self.apply_dsp_params();
    }

    fn apply_dsp_params(&mut self) {
        let pitch_ratio = 2.0f64.powf(self.pitch_semitones / 12.0);
        let vocoder_stretch = pitch_ratio / self.tempo_ratio;
        let resample_ratio = 1.0 / pitch_ratio;

        for v in &mut self.vocoders {
            v.set_stretch(vocoder_stretch);
        }
        for r in &mut self.resamplers {
            r.set_ratio(resample_ratio);
        }
    }

    /// Pre-fill vocoders with lookback data so they're ready to produce output
    /// immediately. Uses source data behind current position (already played).
    fn prime_vocoders(&mut self) {
        let channels = self.channels.max(1);
        let lookback_frames = FFT_SIZE + FEED_CHUNK; // Need fft_size to produce first frame.
        let lookback_samples = lookback_frames * channels;

        // Find start of lookback window.
        let start = if self.source_pos >= lookback_samples {
            self.source_pos - lookback_samples
        } else {
            0
        };
        let available_frames = (self.source_pos - start) / channels;

        if available_frames == 0 {
            return;
        }

        // Feed lookback data to each vocoder (always L/R).
        for ch in 0..channels {
            self.mono_in.resize(available_frames, 0.0);
            for f in 0..available_frames {
                self.mono_in[f] = self.source_samples[start + f * channels + ch];
            }
            self.vocoders[ch].write_input(&self.mono_in[..available_frames]);
        }

        // Process frames in lockstep for stereo (linked-phase priming).
        if channels == 2 {
            while self.vocoders[0].can_process() && self.vocoders[1].can_process() {
                self.vocoders[0].try_process_frame();
                let increments = self.vocoders[0].last_phase_increments().to_vec();
                self.vocoders[1].process_frame_linked(&increments);
            }
        } else {
            while self.vocoders[0].can_process() {
                self.vocoders[0].try_process_frame();
            }
        }

        // Drain and discard any output produced during priming.
        let mut discard = vec![0.0f32; FFT_SIZE * 4];
        for ch in 0..channels {
            self.vocoders[ch].drain_output(&mut discard);
        }

        self.vocoder_primed = true;
    }

    /// Fill interleaved output buffer.
    pub fn fill_output(&mut self, n_frames: usize) -> Vec<f32> {
        let channels = self.channels.max(1);
        let total_samples = n_frames * channels;
        let mut output = vec![0.0f32; total_samples];

        if !self.playing || self.source_samples.is_empty() {
            return output;
        }

        if self.source_pos >= self.source_samples.len() {
            self.playing = false;
            return output;
        }

        // Smooth parameters toward targets before processing.
        self.smooth_and_update_params();

        if self.is_bypass() {
            // Invalidate vocoder priming when entering bypass (position diverges).
            self.vocoder_primed = false;
            return self.fill_bypass(n_frames);
        }

        // Prime vocoders with lookback data on first entry to vocoder mode.
        if !self.vocoder_primed {
            self.prime_vocoders();
        }

        self.fill_vocoder(n_frames, &mut output);
        output
    }

    /// Direct copy bypass — no DSP, used when tempo=1.0 and pitch=0.
    fn fill_bypass(&mut self, n_frames: usize) -> Vec<f32> {
        let channels = self.channels.max(1);
        let available = (self.source_samples.len() - self.source_pos) / channels;
        let frames = n_frames.min(available);
        let count = frames * channels;

        let mut output = vec![0.0f32; n_frames * channels];
        output[..count].copy_from_slice(
            &self.source_samples[self.source_pos..self.source_pos + count],
        );
        self.source_pos += count;

        if self.source_pos >= self.source_samples.len() {
            self.playing = false;
        }

        output
    }

    /// Full vocoder + resampler pipeline.
    fn fill_vocoder(&mut self, n_frames: usize, output: &mut [f32]) {
        let channels = self.channels.max(1);
        let total_samples = n_frames * channels;
        let mut out_pos: usize = 0;
        let is_stereo = channels == 2;

        // Compute how many vocoder samples per resampled output sample.
        // When pitch is shifted up, the resampler needs MORE vocoder input
        // per output sample (step > 1). We must read enough from the vocoder
        // to keep the resampler fed.
        let pitch_ratio = 2.0f64.powf(self.pitch_semitones / 12.0);
        let resample_ratio = (1.0 / pitch_ratio).max(0.05);

        let max_iterations = (n_frames / FEED_CHUNK + 1) * 10;
        let mut iterations = 0;

        let source_pos_start = self.source_pos;

        while out_pos < n_frames && iterations < max_iterations {
            iterations += 1;

            // Feed a chunk of source to each vocoder (always L/R deinterleave).
            let source_frames_left =
                (self.source_samples.len() - self.source_pos) / channels;
            let fed = if source_frames_left > 0 {
                let to_feed = FEED_CHUNK.min(source_frames_left);
                for ch in 0..channels {
                    self.mono_in.resize(to_feed, 0.0);
                    for f in 0..to_feed {
                        self.mono_in[f] =
                            self.source_samples[self.source_pos + f * channels + ch];
                    }
                    self.vocoders[ch].write_input(&self.mono_in[..to_feed]);
                }
                self.source_pos += to_feed * channels;
                to_feed
            } else {
                0
            };

            // Process vocoder frames in lockstep for stereo (linked-phase).
            // L channel processes normally and computes phase increments.
            // R channel uses L's phase increments to preserve inter-channel phase.
            if is_stereo {
                while self.vocoders[0].can_process() && self.vocoders[1].can_process() {
                    self.vocoders[0].try_process_frame();
                    let increments = self.vocoders[0].last_phase_increments().to_vec();
                    self.vocoders[1].process_frame_linked(&increments);
                }
            } else {
                // Mono: process normally.
                while self.vocoders[0].can_process() {
                    self.vocoders[0].try_process_frame();
                }
            }

            // Read output from each vocoder (drain without processing), resample, interleave.
            let space = n_frames - out_pos;

            // Size the vocoder read based on what the resampler needs.
            // If resample_ratio < 1 (pitch up), we need more vocoder samples
            // than output frames because each output sample consumes >1 input.
            let voc_needed =
                ((space as f64 / resample_ratio) + 4.0).ceil() as usize;
            let read_size = voc_needed.max(2).min(FEED_CHUNK * 4);
            let mut ch0_produced = 0usize;

            for ch in 0..channels {
                self.mono_stretched.resize(read_size, 0.0);
                let voc_read =
                    self.vocoders[ch].drain_output(&mut self.mono_stretched[..read_size]);

                if voc_read == 0 {
                    continue;
                }

                self.mono_resampled.resize(space, 0.0);
                let (_consumed, produced) = self.resamplers[ch].process(
                    &self.mono_stretched[..voc_read],
                    &mut self.mono_resampled[..space],
                );

                for f in 0..produced {
                    let idx = (out_pos + f) * channels + ch;
                    if idx < total_samples {
                        output[idx] = self.mono_resampled[f];
                    }
                }

                if ch == 0 {
                    ch0_produced = produced;
                }
            }

            out_pos += ch0_produced;

            if fed == 0 && ch0_produced == 0 {
                break;
            }
        }

        // Post-processing: gain compensation + optional stereo width correction.
        if channels == 2 && out_pos > 0 {
            self.apply_post_processing(output, out_pos, source_pos_start);
        }

        if self.source_pos >= self.source_samples.len() {
            self.playing = false;
        }
    }

    /// Post-processing: fixed makeup gain + optional stereo width correction.
    /// Gain is a fixed boost from the slider (0–6 dB). Stereo correction is measurement-based.
    fn apply_post_processing(
        &mut self,
        output: &mut [f32],
        out_frames: usize,
        src_start: usize,
    ) {
        // --- Fixed makeup gain from slider ---
        // amount=0 → 0 dB (1.0x), amount=0.5 → +3 dB (1.41x), amount=1 → +6 dB (2.0x)
        let gain = 10.0_f64.powf(self.gain_comp_amount * 6.0 / 20.0);

        // --- Stereo width correction (when M/S enabled) ---
        let stereo_corr = if self.mid_side_mode {
            let src_end = self.source_pos;
            let src_frames = (src_end - src_start) / 2;

            if src_frames >= 64 {
                // Measure source M/S energy.
                let mut src_m_energy = 0.0f64;
                let mut src_s_energy = 0.0f64;
                for f in 0..src_frames {
                    let l = self.source_samples[src_start + f * 2] as f64;
                    let r = self.source_samples[src_start + f * 2 + 1] as f64;
                    let m = (l + r) * 0.5;
                    let s = (l - r) * 0.5;
                    src_m_energy += m * m;
                    src_s_energy += s * s;
                }

                // Measure output M/S energy.
                let mut out_m_energy = 0.0f64;
                let mut out_s_energy = 0.0f64;
                for f in 0..out_frames {
                    let l = output[f * 2] as f64;
                    let r = output[f * 2 + 1] as f64;
                    let m = (l + r) * 0.5;
                    let s = (l - r) * 0.5;
                    out_m_energy += m * m;
                    out_s_energy += s * s;
                }

                let src_width = if src_m_energy > 1e-10 {
                    (src_s_energy / src_m_energy).sqrt()
                } else {
                    0.0
                };
                let out_width = if out_m_energy > 1e-10 {
                    (out_s_energy / out_m_energy).sqrt()
                } else {
                    0.0
                };

                let target = if out_width > 1e-6 {
                    (src_width / out_width).clamp(0.5, 3.0)
                } else {
                    1.0
                };

                let stereo_alpha = if target > self.stereo_correction { 0.3 } else { 0.08 };
                self.stereo_correction += (target - self.stereo_correction) * stereo_alpha;
                self.stereo_correction
            } else {
                self.stereo_correction
            }
        } else {
            1.0
        };

        // --- Apply both corrections in a single pass ---
        for f in 0..out_frames {
            let l = output[f * 2] as f64;
            let r = output[f * 2 + 1] as f64;

            if stereo_corr != 1.0 {
                // Decompose to M/S, scale Side, recompose, then apply gain.
                let m = (l + r) * 0.5;
                let s = (l - r) * 0.5;
                let s_corrected = s * stereo_corr;
                output[f * 2] = ((m + s_corrected) * gain) as f32;
                output[f * 2 + 1] = ((m - s_corrected) * gain) as f32;
            } else {
                // Gain only.
                output[f * 2] = (l * gain) as f32;
                output[f * 2 + 1] = (r * gain) as f32;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_stereo_sine(freq: f64, sample_rate: u32, duration_secs: f64) -> Vec<f32> {
        let frames = (sample_rate as f64 * duration_secs) as usize;
        let mut samples = vec![0.0f32; frames * 2];
        for f in 0..frames {
            let t = f as f64 / sample_rate as f64;
            let val = (2.0 * PI as f64 * freq * t).sin() as f32 * 0.3;
            samples[f * 2] = val;
            samples[f * 2 + 1] = val;
        }
        samples
    }

    fn run_processor(tempo: f64, pitch: f64, duration: f64) -> (usize, usize) {
        let sr = 44100;
        let mut proc = AudioProcessor::new(sr);
        let samples = make_stereo_sine(440.0, sr, duration);
        let source_frames = samples.len() / 2;
        proc.load(samples, 2, sr);

        // Set target AND current directly (skip smoothing).
        proc.target_tempo = tempo;
        proc.target_pitch = pitch;
        proc.tempo_ratio = tempo;
        proc.pitch_semitones = pitch;
        proc.apply_dsp_params();
        proc.play();

        let mut total_output_frames = 0usize;
        let chunk = 4096;
        for _ in 0..1000 {
            if !proc.is_playing() { break; }
            let out = proc.fill_output(chunk);
            let frames = out.len() / 2;
            total_output_frames += frames;
        }
        (source_frames, total_output_frames)
    }

    #[test]
    fn test_tempo_half_produces_longer_output() {
        let (source, output) = run_processor(0.5, 0.0, 2.0);
        let ratio = output as f64 / source as f64;
        println!("tempo=0.5x: source={source}, output={output}, ratio={ratio:.3}");
        assert!(ratio > 1.5, "Expected ratio > 1.5, got {ratio:.3}");
    }

    #[test]
    fn test_tempo_double_produces_shorter_output() {
        let (source, output) = run_processor(2.0, 0.0, 2.0);
        let ratio = output as f64 / source as f64;
        println!("tempo=2.0x: source={source}, output={output}, ratio={ratio:.3}");
        assert!(ratio < 0.75, "Expected ratio < 0.75, got {ratio:.3}");
    }

    /// Focused test: trace every iteration of fill_vocoder for pitch-only case.
    #[test]
    fn test_pitch_only_trace() {
        let sr = 44100u32;
        let mut proc = AudioProcessor::new(sr);
        let samples = make_stereo_sine(440.0, sr, 2.0);
        proc.load(samples, 2, sr);

        proc.target_tempo = 1.0;
        proc.target_pitch = 6.0;
        proc.tempo_ratio = 1.0;
        proc.pitch_semitones = 6.0;
        proc.apply_dsp_params();
        proc.play();

        // Run multiple fill_output calls and track per-call consumption.
        let mut per_call = Vec::new();
        let chunk = 4096;
        for _ in 0..100 {
            if !proc.is_playing() { break; }
            let pos_before = proc.source_pos;
            let out = proc.fill_output(chunk);
            let pos_after = proc.source_pos;
            let consumed = (pos_after - pos_before) / 2;
            let produced = out.len() / 2;
            per_call.push((consumed, produced));
        }

        let total_consumed: usize = per_call.iter().map(|x| x.0).sum();
        let total_produced: usize = per_call.iter().map(|x| x.1).sum();
        let calls = per_call.len();

        println!("pitch_only: {calls} calls");
        println!("  total consumed={total_consumed}, total produced={total_produced}");
        println!("  effective ratio = {:.4}", total_produced as f64 / total_consumed as f64);
        println!("  first 5 consumed/call: {:?}", &per_call[..5.min(calls)]);
        if calls > 5 {
            println!("  last 5 consumed/call: {:?}", &per_call[calls-5..]);
        }

        let ratio = total_produced as f64 / total_consumed as f64;
        assert!(ratio > 0.8, "output/source ratio {ratio:.4} is too low");
    }

    /// Detailed per-call trace to diagnose tempo accuracy.
    #[test]
    fn test_tempo_accuracy_detailed() {
        let sr = 44100u32;
        let chunk = 4096usize;

        // Test multiple tempo/pitch combos.
        let cases: Vec<(f64, f64, &str)> = vec![
            (0.5, 0.0, "0.5x tempo, 0 pitch"),
            (0.75, 0.0, "0.75x tempo, 0 pitch"),
            (1.5, 0.0, "1.5x tempo, 0 pitch"),
            (2.0, 0.0, "2.0x tempo, 0 pitch"),
            (1.0, 6.0, "1.0x tempo, +6st"),
            (1.0, -6.0, "1.0x tempo, -6st"),
            (0.75, 3.0, "0.75x tempo, +3st"),
            (1.5, -3.0, "1.5x tempo, -3st"),
        ];

        for (tempo, pitch, label) in &cases {
            let mut proc = AudioProcessor::new(sr);
            let samples = make_stereo_sine(440.0, sr, 5.0);
            let source_frames = samples.len() / 2;
            proc.load(samples, 2, sr);

            // Set directly (skip smoothing).
            proc.target_tempo = *tempo;
            proc.target_pitch = *pitch;
            proc.tempo_ratio = *tempo;
            proc.pitch_semitones = *pitch;
            proc.apply_dsp_params();
            proc.play();

            let mut total_output = 0usize;
            let mut calls = 0;
            let mut source_consumed_per_call = Vec::new();

            for _ in 0..2000 {
                if !proc.is_playing() { break; }
                let pos_before = proc.source_pos;
                let out = proc.fill_output(chunk);
                let pos_after = proc.source_pos;
                let consumed = (pos_after - pos_before) / 2; // stereo
                let produced = out.len() / 2;
                total_output += produced;
                calls += 1;
                source_consumed_per_call.push(consumed);
            }

            let effective_ratio = total_output as f64 / source_frames as f64;
            let expected_ratio = 1.0 / tempo;

            // Show per-call stats for first 10 and last 5 calls.
            let first: Vec<_> = source_consumed_per_call.iter().take(10).collect();
            let last: Vec<_> = source_consumed_per_call.iter().rev().take(5).collect();
            println!("\n--- {label} ---");
            println!("  source_frames={source_frames}, total_output={total_output}, calls={calls}");
            println!("  effective_ratio={effective_ratio:.4}, expected={expected_ratio:.4}, error={:.1}%",
                (effective_ratio - expected_ratio).abs() / expected_ratio * 100.0);
            println!("  first 10 source/call: {first:?}");
            println!("  last 5 source/call: {last:?}");

            let error_pct = (effective_ratio - expected_ratio).abs() / expected_ratio * 100.0;
            assert!(error_pct < 15.0, "{label}: error {error_pct:.1}% exceeds 15%");
        }
    }
}

/// Resample an interleaved buffer by the given ratio (output_rate / input_rate).
fn resample_buffer(samples: &[f32], channels: usize, ratio: f64) -> Vec<f32> {
    let frames = samples.len() / channels;
    let out_frames = (frames as f64 * ratio) as usize;
    let mut output = Vec::with_capacity(out_frames * channels);

    for ch in 0..channels {
        let mut resampler = StreamingResampler::new();
        resampler.set_ratio(ratio);

        // Deinterleave.
        let mono: Vec<f32> = (0..frames).map(|f| samples[f * channels + ch]).collect();

        // Resample.
        let mut out_mono = vec![0.0f32; out_frames + 1024];
        let (_consumed, produced) = resampler.process(&mono, &mut out_mono);

        // Interleave into output on first channel, expand output.
        if ch == 0 {
            output.resize(produced * channels, 0.0);
        }
        for f in 0..produced.min(output.len() / channels) {
            output[f * channels + ch] = out_mono[f];
        }
    }

    output
}

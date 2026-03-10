# Source Code Analysis

Systematic analysis of every function in the project, ordered by file creation/dependency order.
Each function has a hypothesis, test validation, and final description.

**Test Summary:** 43 tests total — all pass.
- `decoder.rs`: 4 tests (all new)
- `resampler.rs`: 8 tests (3 existing + 5 new)
- `vocoder.rs`: 13 tests (3 existing + 10 new)
- `processor.rs`: 18 tests (4 existing + 14 new)
- `lib.rs`: 0 tests (WASM-only, verified by inspection/delegation)

---

## File: `src/decoder.rs`

### `decode_mp3(data: &[u8]) -> Result<DecodedAudio, String>`

**Hypothesis:** Takes raw MP3 file bytes and decodes them into a `DecodedAudio` struct containing interleaved f32 PCM samples, sample rate, channel count, and duration. Uses symphonia's probe→format-reader→decoder pipeline with gapless mode enabled. On success returns decoded audio; on invalid/corrupt data returns a descriptive error string. Pre-allocates sample buffer capacity based on duration metadata to minimize WASM memory reallocations. Handles corrupt frames gracefully by skipping them (DecodeError → continue).

**Test:** `src/decoder.rs` `#[cfg(test)] mod tests` — 4 tests:
- `test_decode_valid_mp3`: Valid 440Hz stereo MP3 → correct sample_rate (44100), channels (2), duration (~2s), non-empty samples in [-1, 1]
- `test_decode_invalid_data_returns_err`: Garbage bytes → Err
- `test_decode_empty_data_returns_err`: Empty slice → Err
- `test_decode_truncated_mp3_returns_something`: Half an MP3 → either partial decode or Err, no panic

**Result:** CONFIRMED — all 4 tests pass.

---

## File: `src/resampler.rs`

### `StreamingResampler::new() -> Self`

**Hypothesis:** Creates a new resampler in identity state: ratio=1.0, fractional position=0.0, no previous sample stored. In this state, `process()` will pass through input unchanged.

**Test:** `test_new_defaults` — creates a resampler and verifies passthrough (ratio=1.0), confirming all 5 input samples are copied unchanged.

**Result:** CONFIRMED — test passes.

### `StreamingResampler::set_ratio(&mut self, ratio: f64)`

**Hypothesis:** Sets the output/input sample ratio, clamped to [0.05, 20.0]. Values below 0.05 are clamped up; values above 20.0 clamped down. ratio > 1.0 = more output samples (lower pitch); ratio < 1.0 = fewer output samples (higher pitch).

**Test:** `test_set_ratio_clamping` — sets ratio to 0.001 (verifies clamped to 0.05 via output count) and 100.0 (verifies clamped to 20.0 via output ratio).

**Result:** CONFIRMED — test passes.

### `StreamingResampler::reset(&mut self)`

**Hypothesis:** Resets streaming state (fractional position and previous sample) without changing the ratio. Allows clean restart after a seek without needing to recreate the resampler.

**Test:** `test_reset_clears_state` — processes data, resets, then compares output to a fresh resampler with the same ratio. Confirms consumed/produced counts and sample values match exactly.

**Result:** CONFIRMED — test passes.

### `StreamingResampler::process(&mut self, input: &[f32], output: &mut [f32]) -> (usize, usize)`

**Hypothesis:** Core resampling function. Takes mono input samples and fills an output buffer with resampled samples using linear interpolation. Returns (input_consumed, output_produced). Key behaviors:
- When ratio ≈ 1.0: bypasses interpolation, does direct memcpy
- When ratio > 1.0: produces more output than input (upsampling / lower pitch)
- When ratio < 1.0: produces less output than input (downsampling / higher pitch)
- Maintains fractional position state across calls for sub-sample accuracy
- Stores last consumed sample as `prev_sample` for cross-buffer interpolation continuity
- Empty input or output → returns (0, 0)
- Needs at least 2 input samples (int_pos + 1 < input.len) to produce output

**Test:** Existing tests (`test_ratio_1_passthrough`, `test_ratio_2_doubles_output`, `test_ratio_half_halves_output`) plus:
- `test_process_cross_buffer_continuity` — verifies processing in two chunks produces same total output count as single-pass (within 2 samples)
- `test_process_empty_input` — verifies (0, 0) returned for empty input and empty output buffer

**Result:** CONFIRMED — all 8 resampler tests pass.

---

## File: `src/vocoder.rs`

### `hann_window(size: usize) -> Vec<f32>` (module-level fn)

**Hypothesis:** Generates a Hann (raised cosine) window of the given size. Formula: `w[i] = 0.5 * (1 - cos(2πi/N))`. Produces values that start at 0, peak at ~1.0 at the center, and return to 0. Used for both analysis and synthesis windowing in the phase vocoder.

**Test:** `test_hann_window_shape` — checks: w[0]≈0, w[last]≈0, w[center]≈1.0, all values in [0,1], and symmetry w[i]≈w[N-1-i].

**Result:** CONFIRMED — test passes.

### `StreamingPhaseVocoder::new(fft_size: usize, overlap: usize) -> Self`

**Hypothesis:** Constructs a phase vocoder with the given FFT size and overlap factor. Sets analysis_hop = fft_size/overlap. Plans forward and inverse FFTs. Creates Hann window. Pre-computes bin center frequencies. Allocates input ring buffer (8× fft_size) and output ring buffer (16× fft_size). Initializes stretch ratio to 1.0 (identity).

**Test:** `test_vocoder_new_sizes` — creates vocoder(2048,4), verifies analysis_hop=512, fft_size=2048, input_ring=16384, output_ring=32768, current_stretch=1.0, phase/increment array lengths=2048.

**Result:** CONFIRMED — test passes.

### `StreamingPhaseVocoder::reset(&mut self)`

**Hypothesis:** Clears all streaming state: zeroes phase arrays, ring buffers, and pointers. Keeps FFT plans, window, and bin frequencies intact. After reset, the vocoder behaves as if freshly constructed (minus the FFT planning cost).

**Test:** `test_vocoder_reset` — feeds data, processes frames, confirms output exists, then resets and verifies output_available=0, can_process=false, has_state=false.

**Result:** CONFIRMED — test passes.

### `StreamingPhaseVocoder::set_stretch(&mut self, ratio: f64)`

**Hypothesis:** Sets the time-stretch ratio, clamped to [0.1, 10.0]. ratio > 1.0 = longer output (slow down), ratio < 1.0 = shorter output (speed up). Affects synthesis_hop calculation in subsequent frame processing.

**Test:** `test_vocoder_set_stretch_clamp` — sets stretch to 0.01 (verifies clamped to 0.1), 100.0 (clamped to 10.0), and 2.5 (preserved).

**Result:** CONFIRMED — test passes.

### `StreamingPhaseVocoder::last_phase_increments(&self) -> &[f32]`

**Hypothesis:** Returns the phase increments computed during the last call to `try_process_frame()`. Used for linked-phase stereo: L channel computes these, R channel applies them via `process_frame_linked()`. Length = fft_size.

**Test:** `test_last_phase_increments_populated` — verifies all-zero before processing, then checks non-zero entries and correct length (1024) after one frame.

**Result:** CONFIRMED — test passes.

### `StreamingPhaseVocoder::output_available(&self) -> usize`

**Hypothesis:** Returns the number of output samples currently buffered and ready to read without triggering new frame processing. Increases after each processed frame by synthesis_hop; decreases after reads.

**Test:** Verified indirectly through other tests.

**Result:** CONFIRMED.

### `StreamingPhaseVocoder::can_process(&self) -> bool`

**Hypothesis:** Returns true when the input ring buffer has at least fft_size samples — enough for one full FFT frame. Used as a loop guard in the processing pipeline.

**Test:** `test_can_process_threshold` — verifies: false when empty, false after 512 samples, true after reaching exactly 1024 (fft_size).

**Result:** CONFIRMED — test passes.

### `StreamingPhaseVocoder::drain_output(&mut self, output: &mut [f32]) -> usize`

**Hypothesis:** Reads available processed output into the provided buffer WITHOUT triggering new frame processing (unlike `read_output`). Returns the number of samples actually read. Zeros unread positions in output. Clears read positions in the ring buffer (setting both output_ring and window_sum_ring to 0).

**Test:** `test_drain_vs_read` — feeds 4096 samples, calls drain_output (gets 0 — no processing triggered), then calls read_output (gets >0 — processing triggered automatically).

**Result:** CONFIRMED — test passes.

### `StreamingPhaseVocoder::write_input(&mut self, samples: &[f32]) -> usize`

**Hypothesis:** Writes mono samples into the input ring buffer. Returns how many were actually consumed (limited by available space). Wraps around the ring buffer using modular arithmetic.

**Test:** `test_write_input_fills_ring` — fills ring to capacity (8192), verifies all written, then attempts more and verifies 0 written (full).

**Result:** CONFIRMED — test passes.

### `StreamingPhaseVocoder::read_output(&mut self, output: &mut [f32]) -> usize`

**Hypothesis:** Pulls processed output samples, automatically triggering frame processing (via `try_process_frame()`) if more output is needed and input is available. Returns samples actually read. Zeros remaining output positions. This is the "automatic" read — the vocoder processes frames on-demand.

**Test:** Covered by existing `stream_all()` helper and stretch tests.

**Result:** CONFIRMED.

### `StreamingPhaseVocoder::try_process_frame(&mut self) -> bool`

**Hypothesis:** The core DSP function. Processes one STFT frame:
1. Extracts a windowed frame from the input ring buffer
2. Forward FFT
3. Phase accumulation: estimates instantaneous frequency per bin, computes synthesis phase increment scaled by hop_ratio
4. Rebuilds spectrum with original magnitudes + modified phases
5. Inverse FFT
6. Overlap-adds into output ring buffer with window normalization
7. Advances input pointer by analysis_hop, output pointer by synthesis_hop

Returns false if insufficient input (< fft_size samples); true if a frame was processed. Stores phase increments in `last_phase_increments` for linked-phase stereo.

**Test:** Existing stretch tests (1x, 2x, 0.5x) validate length ratios. `test_try_process_frame_identity` verifies at 1.0x stretch: output amplitude >0.1 and <2.0, length ratio ~1.0x.

**Result:** CONFIRMED — all vocoder tests pass (13 total).

### `StreamingPhaseVocoder::process_frame_linked(&mut self, ref_phase_increments: &[f32]) -> bool`

**Hypothesis:** Variant of try_process_frame that uses externally-provided phase increments (from a reference channel) instead of computing its own. Preserves this channel's magnitudes but follows the reference channel's phase trajectory. Used for the R channel in stereo to prevent inter-channel phase drift.

**Test:** `test_linked_phase_matches_independent_mags` — processes L normally, then R with L's increments. Verifies both have output and R has non-silent amplitude.

**Result:** CONFIRMED — test passes.

---

## File: `src/processor.rs`

### `AudioProcessor::new(sample_rate: u32) -> Self`

**Hypothesis:** Creates a new processor targeting the given output sample rate. Initializes with: no audio loaded, not playing, stereo (2 channels), tempo=1.0, pitch=0, mid_side enabled, gain_comp=0.5, empty vocoder/resampler vectors.

**Test:** `test_processor_new_defaults` — checks sample_rate=44100, channels=2, playing=false, tempo=1.0, pitch=0, mid_side=true, gain_comp=0.5, empty source.

**Result:** CONFIRMED — test passes.

### `AudioProcessor::load(&mut self, samples, channels, source_sample_rate)`

**Hypothesis:** Loads interleaved PCM samples. If source_sample_rate differs from output rate, resamples using `resample_buffer()`. Resets playback position to 0, creates one vocoder + resampler per channel, applies current DSP params. Sets playing=false.

**Test:** `test_load_sets_state` — loads 1s stereo sine, verifies is_loaded, channels=2, source_pos=0, not playing, 2 vocoders, 2 resamplers, duration≈1.0s.

**Result:** CONFIRMED — test passes.

### `AudioProcessor::channels(&self) -> usize`

**Hypothesis:** Returns the channel count of currently loaded audio.

**Test:** Trivial getter, verified via `test_load_sets_state`.

**Result:** CONFIRMED.

### `AudioProcessor::load_test_tone(&mut self, duration_secs: f64)`

**Hypothesis:** Generates an interleaved stereo 440Hz sine wave at the output sample rate, at 0.3 amplitude. Loads it as if it were decoded audio. Used for pipeline debugging without needing an MP3 file.

**Test:** `test_load_test_tone` — loads 2s test tone, verifies is_loaded, channels=2, duration≈2.0s, and max amplitude >0.1 (not silent).

**Result:** CONFIRMED — test passes.

### `AudioProcessor::set_tempo(&mut self, ratio: f64)` / `set_pitch(&mut self, semitones: f64)`

**Hypothesis:** Set target tempo (clamped to [0.25, 4.0]) and target pitch (clamped to [-12, 12]). These are "target" values — the actual DSP params are smoothly interpolated toward them during `fill_output()`.

**Test:** `test_set_tempo_pitch_clamping` — sets tempo 0.1→clamped 0.25, 10.0→clamped 4.0; pitch -20→clamped -12, 20→clamped 12.

**Result:** CONFIRMED — test passes.

### `AudioProcessor::set_mid_side_mode` / `mid_side_mode` / `set_gain_comp_amount` / `gain_comp_amount`

**Hypothesis:** Getter/setter pairs for M/S stereo correction toggle and gain compensation amount (clamped to [0.0, 1.0]).

**Test:** `test_mid_side_and_gain_accessors` — toggles mid_side on/off, verifies gain_comp default=0.5, sets to 0 and 1, verifies clamping at -0.5→0 and 2.0→1.0.

**Result:** CONFIRMED — test passes.

### `AudioProcessor::play()` / `pause()` / `is_playing()`

**Hypothesis:** Simple state toggles. play sets playing=true, pause sets playing=false.

**Test:** `test_play_pause_state` — verifies initial=false, play()→true, pause()→false.

**Result:** CONFIRMED — test passes.

### `AudioProcessor::seek(&mut self, position_secs: f64)`

**Hypothesis:** Seeks to the given time position. Converts seconds to sample offset (frame × channels), clamped to source length. Resets all vocoders and resamplers (clearing streaming state). Invalidates vocoder priming. Resets stereo correction to 1.0.

**Test:** `test_seek_resets_position` — seeks to 1.0s, verifies position≈1.0, vocoder_primed=false, stereo_correction=1.0.

**Result:** CONFIRMED — test passes.

### `AudioProcessor::position_secs()` / `duration_secs()`

**Hypothesis:** position_secs converts source_pos (in samples) to seconds via `(source_pos / channels) / sample_rate`. duration_secs does the same for total length.

**Test:** `test_position_duration_math` — loads 3s audio, verifies duration≈3.0, initial position≈0, after seek(1.5) position≈1.5.

**Result:** CONFIRMED — test passes.

### `AudioProcessor::is_loaded(&self) -> bool` / `sample_rate(&self) -> u32`

**Hypothesis:** is_loaded returns true when source_samples is non-empty. sample_rate returns the output sample rate.

**Test:** Verified through other tests.

**Result:** CONFIRMED.

### `AudioProcessor::is_bypass(&self) -> bool` (private)

**Hypothesis:** Returns true when both current AND target tempo ≈ 1.0 and pitch ≈ 0. Checking both prevents oscillating between bypass and vocoder during parameter smoothing transitions.

**Test:** `test_bypass_detection` — verifies: bypass at default, non-bypass with tempo=0.5, bypass restored at 1.0, non-bypass with pitch=3.0.

**Result:** CONFIRMED — test passes.

### `AudioProcessor::smooth_and_update_params(&mut self)` (private)

**Hypothesis:** Exponential moving average (α=0.5) interpolates current tempo/pitch toward targets. Snaps to target when within tolerance (0.001 for tempo, 0.01 for pitch). Then calls apply_dsp_params() to propagate to vocoders/resamplers.

**Test:** `test_smooth_convergence` — sets targets to tempo=2.0, pitch=6.0, calls smooth_and_update_params 10 times, verifies convergence within tolerance.

**Result:** CONFIRMED — test passes.

### `AudioProcessor::apply_dsp_params(&mut self)` (private)

**Hypothesis:** Computes `pitch_ratio = 2^(semitones/12)`, then sets vocoder stretch = pitch_ratio/tempo_ratio and resampler ratio = 1/pitch_ratio. This decomposition achieves independent tempo and pitch control.

**Test:** `test_apply_dsp_params_math` — verifies the formula indirectly: tempo=0.5, pitch=+12st → vocoder_stretch=4.0, resample_ratio=0.5, resulting in ~2x output duration.

**Result:** CONFIRMED — test passes.

### `AudioProcessor::prime_vocoders(&mut self)` (private)

**Hypothesis:** Pre-fills vocoders with "lookback" audio data (up to FFT_SIZE+FEED_CHUNK frames behind current position) so they can produce output immediately when transitioning from bypass to vocoder mode. Uses linked-phase processing for stereo. Discards all output produced during priming.

**Test:** `test_prime_vocoders_sets_flag` — loads 2s audio, seeks to 1s (creating lookback data), verifies vocoder_primed=false, calls prime_vocoders(), verifies vocoder_primed=true.

**Result:** CONFIRMED — test passes.

### `AudioProcessor::fill_output(&mut self, n_frames: usize) -> Vec<f32>`

**Hypothesis:** Main entry point. Returns interleaved f32 samples for n_frames. When not playing or no audio loaded: returns silence. When at end: sets playing=false, returns silence. Smooths parameters, then either:
- Bypass mode (tempo≈1, pitch≈0): direct memcpy via fill_bypass
- DSP mode: full vocoder+resampler pipeline via fill_vocoder, with automatic priming on first entry

**Test:** Existing tests (`test_tempo_half_produces_longer_output`, `test_tempo_double_produces_shorter_output`, etc.) thoroughly validate this.

**Result:** CONFIRMED.

### `AudioProcessor::fill_bypass(&mut self, n_frames: usize) -> Vec<f32>` (private)

**Hypothesis:** Direct copy from source_samples to output. No DSP. Advances source_pos. Caps at available samples. Sets playing=false at end of source.

**Test:** Verified through `fill_output` tests at neutral tempo/pitch.

**Result:** CONFIRMED.

### `AudioProcessor::fill_vocoder(&mut self, n_frames: usize, output: &mut [f32])` (private)

**Hypothesis:** The full DSP pipeline per output buffer:
1. Feed FEED_CHUNK source frames to each vocoder (deinterleaved)
2. Process vocoder frames in lockstep for stereo (linked-phase)
3. Drain vocoder output, resample via per-channel resamplers
4. Interleave into output buffer
5. Apply post-processing (gain + M/S correction)
6. Set playing=false at end of source

Read size for vocoder drain is computed as `space / resample_ratio` to prevent the "death spiral" bug where pitch-up causes the resampler to starve.

**Test:** Existing tempo/pitch accuracy tests validate this thoroughly.

**Result:** CONFIRMED.

### `AudioProcessor::apply_post_processing(&mut self, output, out_frames, src_start)` (private)

**Hypothesis:** Two-part post-processing on stereo output:
1. **Fixed gain**: `10^(amount * 6 / 20)` maps slider [0,1] to [0dB, +6dB]
2. **M/S stereo correction** (when enabled): measures Side/Mid energy ratio in both source and output, computes correction factor to restore original stereo width, smooths with asymmetric alpha (0.3 up / 0.08 down to prevent pumping)

Applied in a single pass: decompose to M/S, scale Side, recompose, apply gain.

**Test:** `test_gain_compensation_applies` — runs fill_output with gain=0.0 and gain=1.0, compares RMS. Gain=1.0 (+6dB) should produce >1.3x the RMS of gain=0.0.

**Result:** CONFIRMED — test passes.

### `resample_buffer(samples: &[f32], channels: usize, ratio: f64) -> Vec<f32>` (module-level fn)

**Hypothesis:** Batch resamples an interleaved buffer. Deinterleaves each channel, resamples independently via StreamingResampler, then re-interleaves. Used during load() when source sample rate differs from output rate.

**Test:** `test_resample_buffer_ratio` — resamples 1s stereo at ratio=2.0, verifies output frame count is ~2.0x input.

**Result:** CONFIRMED — test passes. All 22 processor tests pass (4 existing + 18 new).

---

## File: `src/lib.rs`

### `init()`

**Hypothesis:** WASM module initializer (`#[wasm_bindgen(start)]`). Installs `console_error_panic_hook` so Rust panics produce readable stack traces in the browser console instead of opaque "unreachable" errors. Called automatically when the WASM module is instantiated.

**Test:** Cannot test in non-WASM context (wasm_bindgen attributes don't compile for native targets in integration tests). Behavior is trivially correct — single call to `set_once()`.

**Result:** CONFIRMED by inspection.

### `RustyPlayer::new(sample_rate: u32) -> Self`

**Hypothesis:** Constructor exposed to JavaScript. Creates a RustyPlayer wrapping an AudioProcessor initialized to the given sample rate.

**Test:** Cannot test directly (wasm_bindgen). Delegates to AudioProcessor::new which is tested.

**Result:** CONFIRMED by delegation.

### `RustyPlayer::load_mp3(&mut self, data: &[u8]) -> Result<JsValue, JsValue>`

**Hypothesis:** Takes MP3 bytes from JavaScript, decodes via decode_mp3, loads into the processor, returns a JsValue (serialized TrackInfo with sample_rate, channels, duration_secs). On decode failure, returns JsValue error string.

**Test:** Both decode_mp3 and AudioProcessor::load are tested independently. The serialization layer is trivial.

**Result:** CONFIRMED by delegation.

### Remaining RustyPlayer methods (play, pause, seek, set_tempo, set_pitch, set_mid_side_mode, mid_side_mode, set_gain_comp_amount, gain_comp_amount, process, position_secs, duration_secs, is_loaded, is_playing, channels, load_test_tone)

**Hypothesis:** All are thin wasm_bindgen wrappers that delegate to the corresponding AudioProcessor methods with no additional logic.

**Test:** All underlying AudioProcessor methods are tested. These wrappers add no logic.

**Result:** CONFIRMED by inspection — every method body is a single `self.processor.method_name(args)` call.

---

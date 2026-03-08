# Real-Time Tempo & Pitch Shifting — Technical Reference

Lessons learned building a WASM-based audio player with independent tempo (BPM) and pitch (key) control. All DSP in Rust, compiled to WebAssembly.

---

## Architecture

```
Source PCM → [Phase Vocoder] → [Resampler] → Output
              time-stretch      pitch-correct
```

Two-stage pipeline where each stage has one job:
1. **Phase Vocoder**: stretches/compresses audio in time WITHOUT changing pitch
2. **Linear Resampler**: shifts pitch by resampling WITHOUT changing duration

Independent tempo and pitch control emerges from how you set each stage's parameter.

---

## The Critical Math

```
pitch_ratio    = 2^(semitones / 12)
vocoder_stretch = pitch_ratio / tempo_ratio
resample_ratio  = 1.0 / pitch_ratio
```

This is the single most important formula. Get this wrong and nothing works.

### Why it works

- **Tempo only** (pitch=0, tempo=0.5x):
  - pitch_ratio = 1.0, vocoder_stretch = 1.0/0.5 = 2.0, resample_ratio = 1.0
  - Vocoder doubles duration. Resampler does nothing. Song plays at half speed. Correct.

- **Pitch only** (pitch=+6st, tempo=1.0):
  - pitch_ratio = 1.414, vocoder_stretch = 1.414/1.0 = 1.414, resample_ratio = 0.707
  - Vocoder stretches 1.414x (preserving pitch). Resampler shrinks by 0.707x (raising pitch).
  - Net duration: 1.414 * 0.707 = 1.0x. Duration unchanged, pitch shifted. Correct.

- **Combined** (pitch=+6st, tempo=0.5x):
  - vocoder_stretch = 1.414/0.5 = 2.828, resample_ratio = 0.707
  - Net: 2.828 * 0.707 = 2.0x duration. Half speed + pitch up. Correct.

### Common mistakes

- Inverting tempo_ratio direction (is 2.0 faster or slower?)
- Forgetting that resampler ratio is the INVERSE of pitch_ratio
- Applying pitch shift as a vocoder parameter instead of splitting across both stages

---

## Phase Vocoder Implementation

### Constants that matter

| Constant | Value | Why |
|----------|-------|-----|
| FFT_SIZE | 4096 | Good frequency resolution for music. Smaller = worse quality, larger = more latency |
| OVERLAP | 8 | Higher = smoother overlap-add = fewer artifacts. 4 is minimum viable, 8 is good |
| ANALYSIS_HOP | FFT_SIZE/OVERLAP = 512 | How many new input samples per frame |
| SYNTHESIS_HOP | round(ANALYSIS_HOP * stretch) | Adapts dynamically per frame |

### Algorithm (per frame)

1. Extract windowed frame (Hann window) from input ring buffer
2. Forward FFT
3. Phase accumulation:
   ```
   bin_freq[k] = 2*PI * k * analysis_hop / fft_size
   dp = current_phase[k] - prev_phase[k] - bin_freq[k]
   dp -= round(dp / 2*PI) * 2*PI          // unwrap to [-PI, PI]
   inst_freq = bin_freq[k] + dp
   synth_phase[k] += inst_freq * (synthesis_hop / analysis_hop)
   ```
4. Rebuild spectrum: magnitude from FFT, phase from synth_phase
5. Inverse FFT
6. Overlap-add into output ring buffer with Hann window
7. Normalize by sum of squared windows (prevents amplitude distortion)

### Critical details

- **Phase unwrapping** (`dp -= round(dp/2PI)*2PI`): Without this, phase accumulates errors and output sounds metallic/robotic.
- **Window-sum normalization**: Track `window_sum[i] += w*w` for each output position. Divide output by this. Without it, amplitude varies with overlap.
- **First frame**: Set synth_phase = current phase (don't accumulate from zero).
- **Ring buffer sizing**: Input ring = 8x FFT, Output ring = 16x FFT. Too small causes overflow with high stretch ratios.

---

## Resampler Implementation

Simple linear interpolation with fractional position tracking:

```
step = 1.0 / ratio    // input advance per output sample
```

Per output sample:
```
int_pos = floor(frac_pos)
frac = frac_pos - int_pos
output = input[int_pos] * (1-frac) + input[int_pos+1] * frac
frac_pos += step
```

### Critical details

- **Cross-buffer continuity**: Store `prev_sample` from the end of each process() call. Use it for interpolation at the start of the next call when frac_pos < 1.0.
- **Consumed tracking**: After loop, `consumed = floor(frac_pos)`, then `frac_pos -= consumed`. This maintains sub-sample accuracy across calls.
- **Minimum input**: The resampler needs at least 2 input samples (`int_pos + 1 < input.len()`). Feeding 1 sample produces 0 output. This matters — see "Death Spiral Bug" below.
- **Bypass**: When ratio ~= 1.0, skip interpolation and memcpy directly.

---

## Processor: Orchestrating the Pipeline

### Feed-Read Loop

The processor runs a loop per output buffer:
1. Feed FEED_CHUNK (= analysis_hop = 512) source frames to vocoder
2. Read processed output from vocoder
3. Pass through resampler
4. Write to interleaved output buffer

### BUG: Vocoder Read Size Death Spiral (the big one)

**Symptom**: Song plays way too fast. Source consumed 10-90x faster than expected.

**Root cause**: The vocoder read size was capped by remaining output space:
```rust
// BROKEN
let read_size = space.min(FEED_CHUNK * 4);
```

When pitch is shifted up, the resampler has `step > 1.0` — it consumes MORE input per output sample. As the output buffer fills and `space` shrinks:
1. read_size shrinks (e.g., to 1)
2. Resampler needs 2+ input samples but gets 1 → produces 0 output
3. Loop continues feeding 512 source frames per iteration with 0 output
4. Hits max_iterations, having consumed tens of thousands of source frames for nothing

**Fix**: Size the vocoder read based on what the resampler actually needs:
```rust
// CORRECT
let voc_needed = ((space as f64 / resample_ratio) + 4.0).ceil() as usize;
let read_size = voc_needed.max(2).min(FEED_CHUNK * 4);
```

When resample_ratio < 1.0 (pitch up), this reads MORE from the vocoder, ensuring the resampler always has enough input to fill the remaining output space.

**This is the most impactful bug we found.** Without it, any pitch shift causes massive source overconsumption. Pure tempo changes (resample_ratio = 1.0) are unaffected.

### Parameter Smoothing

Abrupt parameter changes cause clicks/pops. Exponential smoothing:
```
current += (target - current) * ALPHA
```
- ALPHA = 0.5 works well (converges in ~3 steps = ~0.3 seconds at 10 Hz update rate)
- Too slow (0.1): user sees "0.5x" on display but audio takes seconds to actually slow down
- Too fast (1.0): no smoothing, clicks on every parameter change
- Snap to target when within tolerance (0.001 for tempo, 0.01 for pitch)

### Bypass Mode

When tempo ~= 1.0 AND pitch ~= 0, skip the vocoder entirely and memcpy source to output. The phase vocoder always introduces some coloration — bypass preserves pristine audio at center position.

Check both current AND target values to prevent oscillating between bypass and vocoder during smoothing transitions.

### Vocoder Priming

When transitioning from bypass to vocoder mode (user moves XY pad from center), the vocoder's input ring buffer is empty. It needs FFT_SIZE (4096) input samples before it can produce its first output frame. This causes a latency burst.

Fix: Pre-fill the vocoder with "lookback" data — audio behind the current playback position that was already played in bypass mode:
```
lookback = FFT_SIZE + FEED_CHUNK frames of source behind source_pos
feed to vocoder → discard output (priming only)
```

This eliminates the startup latency when entering vocoder mode.

---

## Web Audio Transport

All DSP runs in WASM. The browser is only used to route samples to speakers.

### AudioBufferSourceNode Scheduling (current approach)

```javascript
function scheduleChunks() {
    while (nextStartTime < audioCtx.currentTime + 0.3) {
        const samples = player.process(4096);  // WASM call
        const buf = audioCtx.createBuffer(2, frames, sampleRate);
        // deinterleave into buf channels
        const source = audioCtx.createBufferSource();
        source.buffer = buf;
        source.connect(audioCtx.destination);
        source.start(nextStartTime);
        nextStartTime += frames / sampleRate;
    }
    setTimeout(scheduleChunks, 50);
}
```

### BUG: Duplicate Scheduler Loops

**Symptom**: Audio sounds chopped/doubled, plays too fast.

**Root cause**: Each Play click starts a new setTimeout chain. Play→Pause→Play creates two chains running simultaneously, both consuming from the same WASM processor.

**Fix**: Track the timer ID and cancel it before starting a new one:
```javascript
let schedulerTimer = null;

function stopPlayback() {
    isPlaying = false;
    if (schedulerTimer !== null) {
        clearTimeout(schedulerTimer);
        schedulerTimer = null;
    }
}
```

Also guard against double-play: `if (isPlaying) return;`

---

## Testing Strategy

### Unit tests that catch real bugs

1. **Tempo ratio test**: Set tempo to 0.5x, run processor until done, verify output is ~2x source length
2. **Pitch-only test**: Set pitch to +6st at 1.0x tempo, verify output/source ratio ~= 1.0 (duration preserved)
3. **Per-call consumption test**: Track `source_pos` before/after each `fill_output()` call. The first call will overconsume (vocoder warmup), subsequent calls should be stable.

```rust
// Example: verify steady-state source consumption
for _ in 0..100 {
    let pos_before = proc.source_pos;
    proc.fill_output(4096);
    let consumed = (proc.source_pos - pos_before) / channels;
    // After warmup, consumed should be approximately:
    // 4096 * tempo_ratio (for pure tempo change)
    // 4096 (for pitch-only change, since duration is preserved)
}
```

### Accuracy targets

- Tempo-only: <2% error over full song duration
- Pitch-only: <2% error (duration should be ~unchanged)
- Combined: <2% error
- First fill_output call will have higher error due to vocoder warmup — this is expected

---

## Stereo Quality Preservation

### Problem: Phase Vocoder Destroys Stereo Image

Processing L and R channels through independent phase vocoders causes inter-channel phase divergence. Each vocoder accumulates its own synthesis phase independently. Over time, the L and R phase states drift apart, collapsing the stereo image to mono or producing phasing artifacts.

This degradation is cumulative — audio sounds fine at first, then progressively loses stereo width and spatial detail.

### Solution 1: Linked-Phase Stereo Processing

Use the **left channel as the phase reference** for both channels:

1. L channel processes normally via `try_process_frame()` — computes instantaneous frequencies and phase increments
2. L channel stores its phase increments in `last_phase_increments[]`
3. R channel calls `process_frame_linked(ref_phase_increments)` — uses L's phase increments instead of computing its own

```rust
// In vocoder: store increments during normal processing
let increment = inst_freq * hop_ratio;
self.synth_phase[k] += increment;
self.last_phase_increments[k] = increment;

// Linked processing: apply reference channel's increments
pub fn process_frame_linked(&mut self, ref_phase_increments: &[f32]) {
    // Use this channel's magnitudes but reference channel's phase increments
    self.synth_phase[k] += ref_phase_increments[k];
}
```

**Critical**: Both vocoders must be processed in **lockstep** — one frame at a time, alternating. L processes frame → extracts increments → R processes frame with those increments. You cannot batch-process L then batch-process R.

```rust
while vocoders[0].can_process() && vocoders[1].can_process() {
    vocoders[0].try_process_frame();
    let increments = vocoders[0].last_phase_increments().to_vec();
    vocoders[1].process_frame_linked(&increments);
}
```

This also applies to **vocoder priming** — the lookback data fed during bypass→vocoder transitions must also use linked-phase processing, not independent processing.

### Solution 2: M/S Stereo Width Correction (Post-Processing)

Even with linked-phase, some stereo width loss can occur. Mid/Side post-processing measures and corrects this:

1. Convert source and output to M/S: `M = (L+R)/2`, `S = (L-R)/2`
2. Measure energy ratio: `width = sqrt(S_energy / M_energy)`
3. Compute correction factor: `correction = source_width / output_width`
4. Scale the Side channel by correction, reconstruct L/R

```rust
let src_width = (src_s_energy / src_m_energy).sqrt();
let out_width = (out_s_energy / out_m_energy).sqrt();
let target = (src_width / out_width).clamp(0.5, 3.0);

// Smooth correction (fast rise, slow fall to avoid pumping)
let alpha = if target > self.stereo_correction { 0.3 } else { 0.08 };
self.stereo_correction += (target - self.stereo_correction) * alpha;
```

The asymmetric smoothing (0.3 up, 0.08 down) prevents the correction factor from oscillating or pumping with transients.

---

## Gain Compensation

### Problem: Perceived Volume Loss in Vocoder Mode

Phase vocoder processing preserves RMS energy (theoretically), but perceptually the output sounds quieter than bypass mode. This is because:
- Spectral smearing spreads energy across bins, reducing peak amplitudes
- Transient softening reduces perceived loudness even at same RMS
- The overlap-add window normalization can slightly attenuate depending on stretch ratio

### Solution: Fixed Makeup Gain (Slider-Controlled)

**Don't use measurement-based auto-gain** — it converges to ~1.0 because the vocoder does preserve RMS energy. The volume loss is perceptual, not measurable by simple RMS comparison.

Instead, provide a user-controlled fixed gain slider:

```
gain = 10^(amount * 6 / 20)
```

| Slider | dB    | Linear Gain |
|--------|-------|-------------|
| 0%     | 0 dB  | 1.0x        |
| 50%    | +3 dB | 1.41x       |
| 70%    | +4.2 dB | 1.62x     |
| 100%   | +6 dB | 2.0x        |

**Important**: Gain compensation is applied ONLY in vocoder mode (not bypass). Bypass is the reference — it should be untouched. The gain compensates for the perceptual difference between bypass and vocoder output.

In practice, ~70% (+4.2 dB) works well for most music content.

---

## Current Quality Limitations

Even with all optimizations (linked-phase, M/S correction, gain compensation), vocoder mode has inherent quality trade-offs vs bypass:

| Artifact | Cause | Potential Improvement |
|----------|-------|----------------------|
| **Spectral smearing** | FFT bins average frequency content within each bin | Identity phase locking (propagate phase from spectral peaks to surrounding bins) |
| **Transient softening** | Overlap-add smooths sharp attacks | Transient detection → bypass vocoder during attacks |
| **Loss of detail** | Phase modification alters fine temporal structure | Higher overlap (16 or 32, at CPU cost), or hybrid time-domain methods |
| **Metallic artifacts at extreme stretch** | Phase accumulation errors at high stretch ratios | Spectral envelope preservation, formant correction |

These are fundamental limitations of FFT-based phase vocoders. Professional DJ software (e.g., Algoriddim djay, Traktor) uses proprietary algorithms that likely combine phase vocoding with time-domain methods (WSOLA, synchronized overlap-add) and sophisticated transient handling.

---

## Build & Deployment

### WASM Build (wasm-pack)

```bash
wasm-pack build --target web --out-dir www/pkg --release
```

Outputs to `www/pkg/`: `rusty_player.js`, `rusty_player_bg.wasm`, `rusty_player.d.ts`

### Docker Multi-Stage Build

```dockerfile
FROM rust:1.86-slim-bookworm AS builder
RUN wasm-pack build --target web --out-dir www/pkg --release
COPY www/ www/

FROM nginx:alpine
COPY --from=builder /app/www /usr/share/nginx/html
COPY nginx.conf /etc/nginx/conf.d/default.conf
```

### BUG: Dockerfile Overwrites Fresh WASM with Stale Local Files

**Symptom**: All Rust code changes have zero effect. WASM file size doesn't change between builds. Extremely confusing because the build log shows successful compilation.

**Root cause**: `COPY www/ www/` on line 16 copies the LOCAL `www/` directory — including `www/pkg/` with the old/stale WASM files — OVER the freshly compiled WASM from line 14's `wasm-pack build`.

The `COPY` directive replaces files with the same name, so the freshly-built `.wasm` and `.js` files are silently replaced with whatever exists locally in `www/pkg/`.

**Fix**: Add `www/pkg/` to `.dockerignore`:
```
target/
www/pkg/
```

This ensures the `COPY www/ www/` skips the local pkg directory, preserving the freshly-compiled WASM from the builder stage.

**This bug can silently invalidate ALL Rust-side changes.** If you modify Rust code and see no effect in the browser, check the WASM file size — if it hasn't changed, the build output is being overwritten.

### COOP/COEP Headers (for SharedArrayBuffer)

The nginx config must include these headers for SharedArrayBuffer to work (required by AudioWorklet):

```nginx
add_header Cross-Origin-Opener-Policy same-origin always;
add_header Cross-Origin-Embedder-Policy require-corp always;
```

---

## Summary of Bugs (in order of severity)

| Bug | Impact | Fix |
|-----|--------|-----|
| Dockerfile WASM overwrite | ALL Rust changes silently ignored | `.dockerignore` with `www/pkg/` |
| Vocoder read size death spiral | 90% tempo error with any pitch shift | Size read by `space / resample_ratio` |
| Inter-channel phase divergence | Stereo image collapses over time | Linked-phase processing (L drives R) |
| Duplicate scheduler loops | Audio doubled/chopped | Track setTimeout ID, cancel on pause |
| Slow parameter smoothing | Display says 0.5x, audio still at 1.0x for seconds | ALPHA = 0.5, not 0.1 |
| No vocoder priming | Burst of fast audio on bypass→vocoder transition | Pre-fill with lookback data |
| Missing bypass mode | Vocoder coloration at center position | Direct memcpy when params ~= neutral |
| Measurement-based gain comp | Converges to 1.0x, does nothing | Fixed slider-controlled makeup gain (0–6 dB) |

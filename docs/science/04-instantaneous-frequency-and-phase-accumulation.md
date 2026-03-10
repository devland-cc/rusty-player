# Instantaneous Frequency & Phase Accumulation

**Relevance:** The core phase processing step that makes time-stretching work without pitch artifacts.
**Files:** `src/vocoder.rs` (Step 3 of `try_process_frame()`)

## The Problem

When time-stretching with a phase vocoder, you change the spacing between frames (synthesis_hop ≠ analysis_hop). If you simply use the raw FFT phases, the output has severe phase discontinuities between frames, causing metallic/robotic artifacts.

The solution is to estimate the **true (instantaneous) frequency** of each bin and use it to compute what the phase **should be** at the new synthesis hop spacing.

## Instantaneous Frequency Estimation

For bin k, the expected phase advance between consecutive analysis frames (spaced by analysis_hop samples) is:

```
expected_advance = 2π * k * analysis_hop / fft_size
```

This is the phase advance that a pure sinusoid at the bin center frequency would make. In the code, this is precomputed as `bin_freq[k]`.

The **actual** phase advance is:
```
actual_advance = current_phase[k] - prev_phase[k]
```

The difference (deviation from expected) tells us the **true frequency offset** within the bin:
```
dp = actual_advance - expected_advance
dp = wrap_to_pi(dp)  // unwrap to [-π, π]
instantaneous_freq = expected_advance + dp
```

## Phase Unwrapping

Phase values from `atan2()` are in `(-π, π]`. The difference between consecutive phases can jump by ±2π (or multiples). Phase unwrapping removes these jumps:

```rust
let mut dp = phase - self.prev_phase[k] - self.bin_freq[k];
dp -= (dp / (2.0 * PI)).round() * 2.0 * PI;  // wrap to [-π, π]
```

This is equivalent to: `dp = dp - 2π * round(dp / 2π)`

Without unwrapping, the instantaneous frequency estimate is wrong, and the output sounds "phasey" or metallic.

## Phase Accumulation

Once we know the instantaneous frequency, we compute how much phase should advance over the **synthesis** hop (which differs from the analysis hop during time-stretching):

```rust
let hop_ratio = synthesis_hop as f32 / analysis_hop as f32;
let increment = inst_freq * hop_ratio;
self.synth_phase[k] += increment;
```

The `hop_ratio` scales the frequency-domain phase advance to match the new time spacing. This is what preserves pitch during time-stretching:
- The frequencies stay the same (same instantaneous frequency)
- Only the time spacing changes (synthesis_hop)
- Phase advances by the correct amount for the new spacing

## First Frame Handling

```rust
if !self.has_state {
    for k in 0..fft_size {
        let phase = self.frame_buf[k].arg();
        self.prev_phase[k] = phase;
        self.synth_phase[k] = phase;
    }
    self.has_state = true;
}
```

The first frame has no previous phase to compare against, so synthesis phase is initialized directly from the analysis phase. This avoids a startup transient.

## Why This Is Hard to Get Right

### 1. Frequency Leakage Between Bins
A real-world signal rarely has components exactly at bin center frequencies. A 445 Hz tone at 44100/4096 resolution falls between bins 41 (441.1 Hz) and 42 (451.8 Hz), spreading energy across both. The instantaneous frequency estimate for each bin reflects this leaked energy, not a pure sinusoid.

### 2. Multiple Sinusoids Per Bin
When two frequencies are close together (e.g., two notes in a chord), they may fall in the same bin. The instantaneous frequency estimate becomes meaningless — it's the weighted average of both frequencies.

### 3. Phase Accumulation Drift
Over many frames, small errors in instantaneous frequency estimation accumulate in `synth_phase`. This causes gradual phase coherence loss, which manifests as:
- Slight detuning of harmonics
- Loss of stereo image (when L and R drift independently)
- Increased "phasiness" or chorus-like effect

## Connection to Linked-Phase Stereo

Phase accumulation drift is why independent L/R vocoder processing destroys the stereo image. See [18-linked-phase-stereo-processing.md](18-linked-phase-stereo-processing.md) for the solution.

## Potential Improvements

### Phase Locking to Spectral Peaks
Instead of accumulating phase independently per bin, identify magnitude peaks and propagate their phase to surrounding bins:

```
For peak bin p:
  phase_advance_p = compute normally
  For nearby bin k:
    synth_phase[k] = synth_phase[p] + (analysis_phase[k] - analysis_phase[p])
```

This maintains harmonic phase relationships and is the most impactful quality improvement for phase vocoders (Laroche & Dolson, 1999).

### Frequency Reassignment
Instead of using the bin center frequency + deviation, compute the exact frequency via the derivative of the spectrum. This gives more accurate instantaneous frequency estimates but requires computing the FFT of the time-derivative of the windowed signal (additional FFT cost).

---

## Learned Notes

<!-- Add notes here -->

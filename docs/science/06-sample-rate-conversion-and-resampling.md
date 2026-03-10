# Sample Rate Conversion & Resampling

**Relevance:** The second stage of the pitch-shifting pipeline. Resamples vocoder output to shift pitch.
**Files:** `src/resampler.rs`, `src/processor.rs`

## What It Is

Resampling (sample rate conversion) changes the number of samples representing an audio signal. In this project, it serves two purposes:

1. **Pitch correction** in the DSP pipeline: After the vocoder time-stretches, the resampler adjusts the sample rate to shift pitch while maintaining the stretched duration.
2. **Source rate matching** on load: If the MP3's sample rate differs from the output device rate, the source is resampled during `load()`.

## How Resampling Changes Pitch

Playing back samples faster (fewer output samples) raises the pitch. Playing slower (more output samples) lowers the pitch.

```
resample_ratio = 1.0 / pitch_ratio
pitch_ratio = 2^(semitones / 12)
```

- **Pitch up (+6 semitones)**: pitch_ratio = 1.414, resample_ratio = 0.707 → fewer output samples → higher pitch
- **Pitch down (-6 semitones)**: pitch_ratio = 0.707, resample_ratio = 1.414 → more output samples → lower pitch

## Project Implementation: Linear Interpolation

The `StreamingResampler` uses **linear interpolation** between adjacent samples:

```rust
let step = 1.0 / self.ratio;  // input advance per output sample

// For each output sample:
let int_pos = self.frac_pos as usize;
let frac = self.frac_pos - int_pos as f64;
output[out_pos] = s0 + (s1 - s0) * frac as f32;
self.frac_pos += step;
```

The `frac_pos` tracks the exact (sub-sample) position in the input. Between each pair of input samples, linear interpolation draws a straight line and reads the value at the fractional position.

### Cross-Buffer Continuity
```rust
self.prev_sample = input[consumed - 1];  // remember last sample
// Next call: use prev_sample when frac_pos < 1.0
```

This ensures smooth output across chunk boundaries — the resampler maintains state between `process()` calls.

## Quality of Linear Interpolation

Linear interpolation is the simplest resampling method. It works by drawing straight lines between samples.

### Frequency Response
- Perfect at 0 Hz (DC)
- -3 dB at Nyquist/2 (11025 Hz at 44100)
- Rolls off to -∞ at Nyquist
- Introduces imaging artifacts (aliased frequencies) above Nyquist/2

### When It's Adequate
For this project, linear interpolation is acceptable because:
- Pitch shifts are typically ≤ 12 semitones (ratio 0.5–2.0)
- The source material is MP3 (already bandlimited, typically lacking content above 16 kHz)
- The phase vocoder's spectral smearing already masks subtle interpolation artifacts

### When It's Not Enough
- Large pitch shifts (> 1 octave) introduce noticeable aliasing
- High-quality source material (lossless, with content near Nyquist) will show the rolloff
- Professional applications demand higher-order interpolation

## Interpolation Quality Hierarchy

| Method | Complexity | Quality | Suitable For |
|--------|-----------|---------|-------------|
| **Linear** | 2 samples | Low-medium | **MP3 player, ≤12st shift (current)** |
| Cubic (Hermite) | 4 samples | Medium | General purpose |
| Cubic (Catmull-Rom) | 4 samples | Medium-high | Most audio applications |
| Sinc (windowed) | 16-64 samples | High | Professional quality |
| Polyphase FIR | Varies | Highest | Broadcast, studio |

## Potential Improvements

### Cubic Interpolation
Replace the 2-point linear interpolation with 4-point cubic (Hermite or Catmull-Rom spline). This provides:
- Smoother frequency response (less rolloff)
- Better alias rejection
- Minimal additional CPU cost (2 extra multiplies per sample)

```rust
// Hermite cubic interpolation
fn hermite(frac: f32, s_m1: f32, s0: f32, s1: f32, s2: f32) -> f32 {
    let c0 = s0;
    let c1 = 0.5 * (s1 - s_m1);
    let c2 = s_m1 - 2.5 * s0 + 2.0 * s1 - 0.5 * s2;
    let c3 = 0.5 * (s2 - s_m1) + 1.5 * (s0 - s1);
    ((c3 * frac + c2) * frac + c1) * frac + c0
}
```

This requires storing the previous 2 samples (not just 1) for cross-buffer continuity.

### Windowed Sinc
For maximum quality, use a sinc function windowed by a Kaiser or Lanczos window. Typically 8-32 taps. Provides near-perfect reconstruction but at significant CPU cost.

### Anti-Aliasing Filter
When downsampling (resample_ratio < 1.0, i.e., pitch up), frequencies above the new Nyquist should be filtered before resampling to prevent aliasing. Currently, the project doesn't apply an anti-aliasing filter, relying on the vocoder's inherent spectral processing to attenuate high frequencies.

---

## Learned Notes

<!-- Add notes here -->

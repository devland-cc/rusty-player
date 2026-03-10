# Gain, Loudness & Decibel Scale

**Relevance:** Gain compensation for perceived volume loss in vocoder mode.
**Files:** `src/processor.rs` (`apply_post_processing()`)

## Decibel Scale

The decibel (dB) is a logarithmic unit for expressing ratios. In audio:

```
dB = 20 * log10(amplitude_ratio)
amplitude_ratio = 10^(dB / 20)
```

| dB | Linear Gain | Perceptual |
|----|------------|------------|
| -6 | 0.50x | Noticeably quieter |
| -3 | 0.71x | Slightly quieter |
| 0 | 1.00x | No change |
| +3 | 1.41x | Slightly louder |
| +6 | 2.00x | Noticeably louder |
| +10 | 3.16x | About "twice as loud" (perceptual) |
| +20 | 10.0x | Very loud |

## Project Implementation

```rust
// amount: 0.0 = 0 dB, 0.5 = +3 dB, 1.0 = +6 dB
let gain = 10.0_f64.powf(self.gain_comp_amount * 6.0 / 20.0);
```

This maps the slider range [0.0, 1.0] to [0 dB, +6 dB]:
- 0% → 10^(0/20) = 1.0x
- 50% → 10^(3/20) = 1.41x
- 70% → 10^(4.2/20) = 1.62x (recommended default)
- 100% → 10^(6/20) = 2.0x

## Why Fixed Gain, Not Auto-Gain

The phase vocoder preserves RMS energy — a measurement-based auto-gain system converges to ~1.0x because the actual energy is preserved. But the output sounds perceptually quieter because:

1. **Spectral smearing** spreads energy across more frequency bins, reducing peak amplitudes
2. **Transient softening** reduces perceived loudness even at the same RMS level
3. **Phasiness** can cause destructive interference that reduces instantaneous peaks

These are **perceptual** effects, not measurable by simple RMS comparison. Hence the project uses a user-controlled fixed gain slider rather than automated measurement.

## Perceived Loudness vs Measured Loudness

Human hearing is not a simple energy detector. Loudness perception depends on:

### Equal-Loudness Contours (Fletcher-Munson)
Humans are most sensitive to frequencies around 2–5 kHz. Low bass and very high frequencies need more energy to sound equally loud.

### Temporal Integration
Sustained sounds are perceived as louder than brief sounds of the same peak amplitude. The vocoder's transient softening removes brief peaks, reducing perceived loudness even though RMS is similar.

### LUFS (Loudness Units Full Scale)
The modern standard for loudness measurement (EBU R128, ITU-R BS.1770) applies K-weighting (frequency weighting matching human sensitivity) and temporal integration. A LUFS-based gain compensation would be more perceptually accurate than RMS, but more complex.

## Clipping Considerations

Applying +6 dB gain doubles the amplitude. If the vocoder output peaks at 0.7, the compensated output peaks at 1.4 — exceeding the [-1.0, 1.0] range and causing **clipping** (hard distortion).

The project does not currently apply a limiter. For safety:
- The default gain (50% = +3 dB = 1.41x) is modest enough that clipping is rare with typical MP3 content
- MP3 files are usually mastered with some headroom
- A future improvement would be a soft limiter or compressor

## Potential Improvements

### Soft Limiter
Apply a soft-clipping function after gain compensation:
```rust
fn soft_clip(x: f32) -> f32 {
    if x.abs() < 0.9 { x }
    else { x.signum() * (0.9 + 0.1 * ((x.abs() - 0.9) / 0.1).tanh()) }
}
```

### LUFS-Based Measurement
For more accurate auto-gain:
1. Apply K-weighting filter to both source and output
2. Measure gated loudness (ignoring silent passages)
3. Compute gain difference in LUFS
4. This would capture the perceptual difference that RMS misses

### Per-Band Gain Compensation
Apply different gain corrections in different frequency bands. The vocoder may attenuate high frequencies more than low frequencies, and a multi-band approach could compensate more precisely.

---

## Learned Notes

<!-- Add notes here -->

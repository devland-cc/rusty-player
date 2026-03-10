# Mid/Side (M/S) Stereo Processing

**Relevance:** Post-processing correction to preserve stereo image after phase vocoder processing.
**Files:** `src/processor.rs` (`apply_post_processing()`)

## What It Is

Mid/Side encoding decomposes a stereo signal (Left/Right) into two orthogonal components:

```
Mid  = (L + R) / 2    // the mono-compatible center content
Side = (L - R) / 2    // the stereo difference (spatial information)
```

Reconstruction:
```
L = Mid + Side
R = Mid - Side
```

The Mid channel contains everything common to both channels (vocals, bass, centered instruments). The Side channel contains everything that differs between channels (stereo width, panning, room ambience).

## Why the Phase Vocoder Damages Stereo

When L and R channels are processed through independent phase vocoders, each accumulates synthesis phase independently. Even with identical input signals, floating-point rounding and the non-linear nature of phase accumulation cause the L and R phases to drift apart over time.

This drift:
1. Reduces Side energy (L and R become more similar)
2. Increases Mid energy (more content moves to center)
3. Result: **stereo image collapses toward mono**

The linked-phase processing (see [18-linked-phase-stereo-processing.md](18-linked-phase-stereo-processing.md)) is the primary fix, but residual width loss can still occur.

## Project Implementation: Energy-Based Width Correction

The post-processing in `processor.rs` measures and corrects stereo width:

### Step 1: Measure Source Width
```rust
for f in 0..src_frames {
    let l = source_samples[src_start + f * 2] as f64;
    let r = source_samples[src_start + f * 2 + 1] as f64;
    let m = (l + r) * 0.5;
    let s = (l - r) * 0.5;
    src_m_energy += m * m;
    src_s_energy += s * s;
}
let src_width = (src_s_energy / src_m_energy).sqrt();
```

### Step 2: Measure Output Width
Same computation on the processed output.

### Step 3: Compute Correction
```rust
let target = (src_width / out_width).clamp(0.5, 3.0);
```

If the output is narrower than the source (width loss), `target > 1.0` and Side is boosted. Clamped to [0.5, 3.0] to prevent extreme corrections.

### Step 4: Smooth & Apply
```rust
// Asymmetric smoothing (fast rise, slow fall)
let alpha = if target > self.stereo_correction { 0.3 } else { 0.08 };
self.stereo_correction += (target - self.stereo_correction) * alpha;

// Apply: scale Side channel
let s_corrected = s * stereo_corr;
output[f * 2] = ((m + s_corrected) * gain) as f32;
output[f * 2 + 1] = ((m - s_corrected) * gain) as f32;
```

## Asymmetric Smoothing Rationale

- **Fast rise (α=0.3):** When width is lost (most common), correct quickly to prevent audible narrowing
- **Slow fall (α=0.08):** When width appears to increase (transient), don't overcorrect — it may be a momentary fluctuation

This prevents "pumping" where the stereo width oscillates noticeably on transients.

## Width Metric: Side/Mid Energy Ratio

```
width = sqrt(Side_energy / Mid_energy)
```

| Width Value | Meaning |
|-------------|---------|
| 0.0 | Pure mono (L = R) |
| ~0.3–0.5 | Typical centered pop/rock |
| ~0.7–1.0 | Wide stereo mix |
| > 1.0 | Side-heavy (unusual, out-of-phase content) |

## Potential Improvements

### Per-Band M/S Correction
Instead of a single correction factor for all frequencies, apply different corrections in different frequency bands. Phase vocoder artifacts often affect high frequencies more than low frequencies, so a multi-band approach could be more accurate.

### Correlation-Based Width
Instead of energy ratio, measure the cross-correlation between L and R:
```
correlation = Σ(L[i] * R[i]) / sqrt(Σ(L[i]²) * Σ(R[i]²))
```
correlation = 1.0 is mono, 0.0 is fully decorrelated stereo, -1.0 is out of phase.

### Phase Coherence Measurement
Directly measure the inter-channel phase difference in the frequency domain rather than the time domain. This could provide more precise correction tied to specific frequency regions where the vocoder caused phase drift.

---

## Learned Notes

<!-- Add notes here -->

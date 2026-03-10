# Windowing Functions (Hann Window)

**Relevance:** Applied before FFT analysis and during overlap-add synthesis. Critical for spectral quality.
**Files:** `src/vocoder.rs` (`hann_window()` function, Steps 1 and 6)

## What It Is

A window function is a smooth taper applied to each audio frame before FFT analysis. Without windowing, the abrupt start/end of each frame creates spectral leakage — energy spreads from the true frequency into adjacent bins, smearing the spectrum.

## Why Windowing Is Necessary

The FFT assumes the input signal is periodic with period N. A raw audio frame is NOT periodic — the first and last samples don't match. This discontinuity creates broadband spectral artifacts (leakage) that corrupt frequency and phase estimates.

A window function smoothly tapers the signal to zero at both ends, eliminating the discontinuity:

```
windowed_sample[i] = sample[i] * window[i]
```

## Hann Window

The project uses the Hann (also called Hanning) window:

```rust
fn hann_window(size: usize) -> Vec<f32> {
    (0..size)
        .map(|i| 0.5 * (1.0 - (2.0 * PI * i as f32 / size as f32).cos()))
        .collect()
}
```

Formula: `w[n] = 0.5 * (1 - cos(2πn/N))`

### Properties
- Shape: raised cosine, zero at both ends, maximum (1.0) at center
- Main lobe width: 4 bins (moderate frequency resolution)
- First sidelobe: -31.5 dB (good suppression)
- Sidelobe rolloff: -18 dB/octave
- Satisfies COLA condition for overlap ≥ 4

## Dual Application in the Project

### Analysis Window (Step 1)
```rust
self.frame_buf[i] = Complex::new(
    self.input_ring[idx] * self.window[i],  // windowed input
    0.0,
);
```

### Synthesis Window (Step 6)
```rust
let w = self.window[i];
self.output_ring[out_idx] += self.frame_buf[i].re * norm * w;  // windowed output
self.window_sum_ring[out_idx] += w * w;  // track window² sum
```

Using the same Hann window for both analysis and synthesis means the effective window is Hann² (a raised cosine squared). The window-sum normalization in the overlap-add step compensates for this.

## Window Comparison for Phase Vocoders

| Window | Main Lobe Width | Sidelobe Level | COLA Overlap | Notes |
|--------|----------------|----------------|--------------|-------|
| Rectangular | 2 bins | -13 dB | 1 | Terrible leakage, never use |
| **Hann** | **4 bins** | **-31.5 dB** | **≥4** | **Good all-around (current choice)** |
| Hamming | 4 bins | -42.7 dB | ≥4 | Better sidelobe suppression, doesn't reach zero at edges |
| Blackman | 6 bins | -58 dB | ≥6 | Excellent suppression, wider main lobe |
| Kaiser-Bessel | Tunable | Tunable | Varies | Optimal tradeoff via β parameter |

The Hann window is the standard choice for phase vocoders because:
1. It reaches exactly zero at both ends (no edge discontinuity)
2. It satisfies COLA with reasonable overlap (4+)
3. Good balance of frequency resolution vs leakage suppression
4. Simple to compute

## Impact on Phase Vocoder Quality

### Frequency Resolution
The main lobe width determines the minimum frequency separation that can be resolved. At 4 bins × 10.77 Hz/bin = 43.1 Hz, this means:
- Two harmonics closer than ~43 Hz will interfere in the same bins
- Low bass notes (fundamental < 50 Hz) only occupy a few bins → less precise phase estimation

### Leakage and Phase Estimation
Sidelobe leakage contaminates phase estimates in neighboring bins. At -31.5 dB, the Hann window's leakage is typically below the noise floor of MP3 audio, so this is adequate for the project.

## Potential Improvements

### Kaiser-Bessel Window
A Kaiser window with tunable β parameter allows optimizing the resolution/leakage tradeoff for specific use cases. β ≈ 6 gives similar performance to Hann; higher β improves sidelobe suppression at the cost of frequency resolution.

### Asymmetric Analysis/Synthesis Windows
Use a wider analysis window (better frequency resolution) paired with the Hann synthesis window. The analysis window doesn't need to satisfy COLA — only the synthesis window does.

---

## Learned Notes

<!-- Add notes here -->

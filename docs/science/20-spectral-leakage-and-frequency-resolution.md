# Spectral Leakage & Frequency Resolution

**Relevance:** Fundamental FFT tradeoff that determines phase vocoder quality and parameter choices.
**Files:** `src/vocoder.rs` (FFT_SIZE choice, window function)

## What Is Spectral Leakage

When the FFT analyzes a frame of N samples, it assumes the signal is periodic with period N. If the signal's frequency doesn't align exactly with a bin center frequency (`k * fs / N`), the energy "leaks" from the true frequency into adjacent bins.

### Example
A 445 Hz sine at fs=44100, N=4096:
- Bin spacing: 44100/4096 = 10.77 Hz
- Nearest bins: bin 41 = 441.1 Hz, bin 42 = 451.8 Hz
- 445 Hz falls between them → energy spreads across both bins and their neighbors

Without windowing, the leakage extends across the entire spectrum (the rectangular window has -13 dB sidelobes). With the Hann window, leakage is suppressed to -31.5 dB.

## Frequency Resolution

The ability to resolve two closely-spaced frequencies depends on the window's **main lobe width**:

```
resolution = main_lobe_width * fs / N
```

For the Hann window (main lobe width = 4 bins):
```
resolution = 4 * 44100 / 4096 = 43.1 Hz
```

Two sinusoids closer than ~43 Hz will merge into a single peak in the spectrum. Their individual phases cannot be estimated separately.

## Impact on the Phase Vocoder

### Phase Estimation Accuracy
The instantaneous frequency estimation relies on measuring phase changes between frames. Leakage corrupts phase estimates for bins near strong spectral peaks — the leaked energy has a different phase than the bin's "true" content.

### Harmonic Resolution
Musical notes have harmonics spaced at multiples of the fundamental:
- A4 (440 Hz): harmonics at 880, 1320, 1760, 2200, ...
- Spacing = 440 Hz (well above 43.1 Hz resolution) → easily resolved
- A2 (110 Hz): harmonics at 220, 330, 440, 550, ...
- Spacing = 110 Hz → resolved
- A1 (55 Hz): harmonics at 110, 165, 220, ...
- Spacing = 55 Hz → barely resolved (43.1 Hz resolution)

Low bass notes push the limits of resolution at FFT_SIZE=4096. A larger FFT would help but increases latency.

## The Time-Frequency Tradeoff

This is the fundamental uncertainty principle of signal analysis:

```
Δt * Δf ≥ 1/(4π)  (approximately)
```

You cannot simultaneously have perfect time resolution AND perfect frequency resolution. Increasing the FFT size improves frequency resolution at the cost of time resolution:

| FFT Size | Freq Resolution (Hann) | Time Resolution | Frame Duration |
|----------|----------------------|-----------------|----------------|
| 1024 | 172.3 Hz | 5.8 ms | 23.2 ms |
| 2048 | 86.1 Hz | 11.6 ms | 46.4 ms |
| **4096** | **43.1 Hz** | **23.2 ms** | **92.9 ms** |
| 8192 | 21.5 Hz | 46.4 ms | 185.8 ms |

- **Better frequency resolution** → more accurate phase estimation → fewer tonal artifacts
- **Better time resolution** → sharper transients → better drum/percussion quality

The project's choice of 4096 favors frequency resolution (good for music with sustained harmonics) at the cost of transient preservation.

## Zero-Padding

Adding zeros to the end of a frame before FFT interpolates the spectrum (more bins between the same frequencies) but does **not** improve true frequency resolution — the main lobe width is unchanged.

Zero-padding can be useful for:
- Smoother spectral peak detection
- More precise magnitude estimation
- But adds computational cost (larger FFT)

## Reducing Leakage

### Window Function Choice
See [05-windowing-functions.md](05-windowing-functions.md). Better windows (Blackman, Kaiser) have lower sidelobes but wider main lobes.

### Reassignment Methods
Instead of assuming each bin represents its center frequency, compute the actual centroid of energy within each bin using the derivative of the spectrum. This provides sub-bin frequency accuracy without increasing FFT size.

### Multi-Resolution Analysis
Use different FFT sizes for different frequency ranges:
- Large FFT (8192+) for low frequencies where frequency resolution matters
- Small FFT (1024) for high frequencies where time resolution matters

This is more complex to implement but addresses the time-frequency tradeoff directly.

## Practical Implications for the Project

1. **Bass content** (< 200 Hz) has marginally resolved harmonics → phase estimation less accurate → more artifacts on bass-heavy content at extreme stretch ratios

2. **Percussion** shares frequency content across many bins → leakage is less of an issue (it's already broadband), but time smearing from large FFT is the bigger problem

3. **Solo instruments/vocals** benefit most from the current 4096 FFT → well-resolved harmonics → clean phase vocoder output

4. **Complex mixes** fall in between — harmonics from multiple instruments may collide in the same bins, degrading phase estimation

---

## Learned Notes

<!-- Add notes here -->

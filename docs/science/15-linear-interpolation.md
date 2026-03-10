# Linear Interpolation

**Relevance:** The method used by the resampler to compute sample values at non-integer positions.
**Files:** `src/resampler.rs`

## What It Is

Linear interpolation estimates a value between two known data points by drawing a straight line between them:

```
y = y0 + (y1 - y0) * t
```

Where `t ∈ [0, 1)` is the fractional position between samples `y0` and `y1`.

In the project:
```rust
output[out_pos] = s0 + (s1 - s0) * frac as f32;
```

## Geometric Interpretation

```
Signal:  ...--*----------*--...
              s0         s1
              |←-- frac →|
              ↕
         interpolated value
```

The resampler walks along the input signal at a rate determined by the pitch ratio, reading values at fractional positions. Linear interpolation fills in the gaps.

## Why It Introduces Error

### The Sinc Ideal
The theoretically perfect reconstruction of a bandlimited signal from its samples uses the **sinc function** (sin(πx)/(πx)), which extends infinitely in both directions. Every sample contributes to every reconstructed point.

Linear interpolation only uses **2 adjacent samples**, ignoring all others. This is equivalent to convolving with a triangular (tent) function instead of a sinc function.

### Frequency Response
The frequency response of linear interpolation is:
```
H(f) = sinc²(f / fs)
```

Where `fs` is the sample rate. This gives:
- 0 dB at DC (perfect)
- -0.9 dB at fs/4 (11025 Hz at 44100)
- -3.5 dB at fs/3 (14700 Hz)
- -∞ at fs/2 (Nyquist)

### Aliasing
When downsampling (pitch up), frequencies above the new Nyquist are folded back (aliased). Linear interpolation provides no anti-aliasing — high-frequency content wraps around as audible artifacts.

## When Linear Interpolation Is Good Enough

For this project:
1. **MP3 source material**: Already bandlimited. MP3 at 128 kbps typically has negligible content above 16 kHz. The rolloff at 14.7 kHz affects almost nothing.
2. **Moderate pitch shifts**: Within ±12 semitones (0.5x–2.0x ratio). The aliasing is mild.
3. **Phase vocoder masking**: The vocoder's inherent spectral smearing (see [01-phase-vocoder.md](01-phase-vocoder.md)) is typically more noticeable than interpolation artifacts.

## Comparison of Interpolation Methods

### Nearest Neighbor (0th order)
```
y = y_nearest
```
- Uses 1 sample. Fastest but produces audible stepping artifacts.

### Linear (1st order) — Current
```
y = y0 + (y1 - y0) * t
```
- Uses 2 samples. Smooth but rolls off high frequencies.

### Cubic Hermite (3rd order)
```
y = ((c3*t + c2)*t + c1)*t + c0
```
where coefficients use 4 samples: `y[-1], y[0], y[1], y[2]`
- Preserves derivative continuity (no "corners" at sample points)
- Much flatter frequency response up to Nyquist/2
- Minimal additional CPU (2 more multiplies per sample)

### Windowed Sinc (Nth order)
```
y = Σ y[k] * sinc(t - k) * window(t - k)    for k in range
```
- Uses 8–64 samples. Near-perfect reconstruction.
- Significant CPU cost, but still feasible for real-time at 44.1 kHz.

## Practical Upgrade Path

The highest-impact improvement would be switching from linear to **4-point Hermite cubic**:

```rust
fn hermite4(frac: f32, ym1: f32, y0: f32, y1: f32, y2: f32) -> f32 {
    let c0 = y0;
    let c1 = 0.5 * (y1 - ym1);
    let c2 = ym1 - 2.5 * y0 + 2.0 * y1 - 0.5 * y2;
    let c3 = 0.5 * (y2 - ym1) + 1.5 * (y0 - y1);
    ((c3 * frac + c2) * frac + c1) * frac + c0
}
```

This requires:
- Storing 2 previous samples instead of 1 (`prev_prev_sample`, `prev_sample`)
- Accessing `input[int_pos - 1]`, `input[int_pos]`, `input[int_pos + 1]`, `input[int_pos + 2]`
- Handling the boundary condition at the start of the buffer

---

## Learned Notes

<!-- Add notes here -->

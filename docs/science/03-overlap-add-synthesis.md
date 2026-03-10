# Overlap-Add Synthesis (OLA)

**Relevance:** The reconstruction method that converts processed FFT frames back into a continuous audio signal.
**Files:** `src/vocoder.rs` (Step 6 of `try_process_frame()`)

## What It Is

Overlap-Add is a method for reconstructing a time-domain signal from overlapping windowed frames. After each frame is processed (FFT → modify → IFFT), the output frames are summed (overlapped) into the output buffer at positions determined by the synthesis hop size.

```
Frame 0:  [----window----]
Frame 1:       [----window----]
Frame 2:            [----window----]
Output:   [===sum of overlapping windows===]
```

## Why It's Necessary

The STFT analysis chops audio into overlapping frames, each multiplied by a window function (Hann). To reconstruct perfectly, the overlapping windows must sum to a constant:

```
Σ window(n - k*hop)² = constant  (for all n)
```

This is called the **Constant Overlap-Add (COLA) condition**. The Hann window satisfies COLA when the overlap factor is ≥ 4 (75% overlap).

## Project Implementation

In `vocoder.rs`, overlap-add happens in two parts:

### 1. Accumulation (during frame processing)
```rust
// Step 6: Overlap-add into output ring
let norm = 1.0 / fft_size as f32;
for i in 0..fft_size {
    let out_idx = (self.output_write + i) % output_cap;
    let w = self.window[i];
    self.output_ring[out_idx] += self.frame_buf[i].re * norm * w;
    self.window_sum_ring[out_idx] += w * w;
}
```

Each output position accumulates contributions from multiple overlapping frames. The `window_sum_ring` tracks the sum of squared window values at each position.

### 2. Normalization (before reading output)
```rust
for i in 0..synthesis_hop {
    let idx = (normalize_start + i) % output_cap;
    if self.window_sum_ring[idx] > 1e-6 {
        self.output_ring[idx] /= self.window_sum_ring[idx];
    }
}
```

Dividing by the window sum ensures constant amplitude regardless of the overlap factor or stretch ratio.

## Why Window-Sum Normalization Matters

Without normalization, the output amplitude depends on how many frames overlap at each position:
- At 1.0x stretch with overlap 8: each position has ~8 overlapping frames → amplitude ≈ 8x too loud
- At 2.0x stretch: synthesis_hop is larger → fewer overlapping frames → different amplitude
- At 0.5x stretch: synthesis_hop is smaller → more overlapping frames → even louder

The window-sum normalization automatically compensates for all of these cases. The threshold `> 1e-6` prevents division by zero at the edges where fewer frames contribute.

## Synthesis Hop Calculation

```rust
let synthesis_hop = (analysis_hop as f64 * self.current_stretch)
    .round()
    .max(1.0) as usize;
```

- `stretch = 1.0`: synthesis_hop = analysis_hop → same duration
- `stretch = 2.0`: synthesis_hop = 2 * analysis_hop → frames spaced further apart → longer output
- `stretch = 0.5`: synthesis_hop = 0.5 * analysis_hop → frames closer together → shorter output

## Ring Buffer Considerations

The output ring buffer must be large enough to hold the maximum number of simultaneously overlapping frames:

```
max_overlap_samples = fft_size + max_synthesis_hop * (fft_size / analysis_hop)
```

The project uses `output_cap = fft_size * 16 = 65536` samples, which provides ample headroom for stretch ratios up to 10x.

## Quality Impact

### Overlap Factor Effects

| Overlap | Frames per position | Quality | CPU Cost |
|---------|-------------------|---------|----------|
| 2 | ~2 | Audible amplitude modulation | Lowest |
| 4 | ~4 | Acceptable, slight modulation | Low |
| **8** | **~8** | **Smooth, minimal artifacts** | **Moderate (current)** |
| 16 | ~16 | Very smooth | High |
| 32 | ~32 | Near-perfect | Very high |

The current overlap of 8 is a good balance. Increasing to 16 would improve quality at the cost of ~2x more FFT computations per second.

## Potential Improvements

### Asymmetric Windows
Use a different window for analysis (wide main lobe for better frequency resolution) and synthesis (narrow for better time resolution). The Hann-Hann pair used currently is symmetric and works well, but a Kaiser-Bessel or Gaussian analysis window with a Hann synthesis window can improve frequency resolution.

### Adaptive Overlap
Increase overlap dynamically for extreme stretch ratios where artifacts are more audible. At 1.0x stretch, overlap 4 is sufficient; at 4.0x, overlap 16 may be worth the CPU cost.

---

## Learned Notes

<!-- Add notes here -->

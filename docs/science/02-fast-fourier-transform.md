# Fast Fourier Transform (FFT)

**Relevance:** Computational foundation of the phase vocoder. Called twice per frame (forward + inverse).
**Files:** `src/vocoder.rs`

## What It Is

The FFT is an efficient algorithm for computing the Discrete Fourier Transform (DFT), converting a time-domain signal into its frequency-domain representation. It reduces the complexity from O(N²) to O(N log N).

For a signal of N samples, the FFT produces N complex-valued frequency bins:
- **Magnitude** = amplitude of each frequency component
- **Phase** = timing/position of each frequency component's cycle

## The DFT Equation

```
X[k] = Σ(n=0 to N-1) x[n] * e^(-j*2π*k*n/N)
```

Where:
- `x[n]` = input sample at index n
- `X[k]` = complex frequency bin at index k
- `k` = frequency bin index (0 to N-1)
- `N` = FFT size (4096 in this project)

The inverse (IFFT) reconstructs the time-domain signal:
```
x[n] = (1/N) * Σ(k=0 to N-1) X[k] * e^(j*2π*k*n/N)
```

**Note:** The 1/N normalization must be applied manually with rustfft (it does NOT normalize).

## Frequency Bin Interpretation

For FFT size N at sample rate fs:

| Property | Formula | Value (N=4096, fs=44100) |
|----------|---------|--------------------------|
| Bin spacing | fs / N | 10.77 Hz |
| Nyquist bin | N/2 | 2048 |
| Nyquist frequency | fs / 2 | 22050 Hz |
| Bin k frequency | k * fs / N | k * 10.77 Hz |

### Bin Layout
- Bin 0: DC (0 Hz)
- Bins 1..N/2: Positive frequencies (10.77 Hz to 22050 Hz)
- Bins N/2+1..N-1: Negative frequencies (mirror of positive, conjugate-symmetric for real input)

For real-valued input, `X[k] = conj(X[N-k])`. The project processes all N bins in the phase vocoder loop, which maintains this symmetry after phase modification.

## Project Usage

In `StreamingPhaseVocoder::try_process_frame()`:

```rust
// 1. Pack real samples as complex (imaginary = 0)
self.frame_buf[i] = Complex::new(input_sample * window, 0.0);

// 2. Forward FFT (in-place)
self.fft_forward.process(&mut self.frame_buf);

// 3. Extract magnitude and phase per bin
let mag = self.frame_buf[k].norm();    // sqrt(re² + im²)
let phase = self.frame_buf[k].arg();   // atan2(im, re)

// ... phase vocoder processing ...

// 4. Reconstruct with modified phase
self.frame_buf[k] = Complex::from_polar(mag, synth_phase);

// 5. Inverse FFT (in-place)
self.fft_inverse.process(&mut self.frame_buf);

// 6. Manual normalization (rustfft doesn't normalize)
output_sample = self.frame_buf[i].re * (1.0 / fft_size);
```

## FFT Size Tradeoffs

| FFT Size | Freq Resolution | Time Resolution | Latency | Best For |
|----------|----------------|-----------------|---------|----------|
| 1024 | 43.1 Hz | 23.2 ms | Low | Speech, percussion-heavy |
| 2048 | 21.5 Hz | 46.4 ms | Medium | General purpose |
| **4096** | **10.8 Hz** | **92.9 ms** | **Medium-high** | **Music (current choice)** |
| 8192 | 5.4 Hz | 185.8 ms | High | Solo instruments, high quality |

The current choice of 4096 is a good balance for music content. The frequency resolution of ~10.8 Hz can resolve individual notes down to about A1 (55 Hz), and the time resolution is acceptable for most musical content.

## Performance in WASM

- rustfft uses scalar algorithms in WASM (no SIMD)
- FFT size 4096 is a power of 2 → optimal radix-2/4 Cooley-Tukey decomposition
- Two FFT calls per frame (forward + inverse) at 86 frames/second (44100/512) = ~172 FFTs/sec
- Sub-millisecond per FFT even in WASM — not a bottleneck

## Potential Improvements

### Use `process_with_scratch()`
The current code uses `fft.process()` which allocates a scratch buffer on every call. Pre-allocating a scratch buffer eliminates per-frame heap allocation:

```rust
// Store in struct:
scratch: Vec<Complex<f32>>,

// Use:
self.fft_forward.process_with_scratch(&mut self.frame_buf, &mut self.scratch);
```

### Real FFT
Since input is real-valued, a real-to-complex FFT (N → N/2+1 bins) would halve computation and memory. However, rustfft doesn't provide this natively, and the phase vocoder needs to process all N bins to maintain conjugate symmetry after phase modification.

---

## Learned Notes

<!-- Add notes here -->

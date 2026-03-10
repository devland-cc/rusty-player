# rustfft

**Version in use:** 6.x (crate specifies `rustfft = "6"`)
**Used in:** `src/vocoder.rs`

- Crate page: https://crates.io/crates/rustfft
- API docs: https://docs.rs/rustfft/latest/rustfft/
- GitHub: https://github.com/ejmahler/RustFFT
- num-complex docs: https://docs.rs/num-complex/0.4/num_complex/struct.Complex.html

## Overview

Pure-Rust FFT library. Computes DFT and inverse DFT on buffers of `Complex<T>` where `T: FftNum` (includes `f32`, `f64`). No `unsafe` in the public API. Competitive with FFTW for common sizes.

## Core API

### FftPlanner

```rust
let mut planner = FftPlanner::<f32>::new();
let fft_forward = planner.plan_fft_forward(4096);  // -> Arc<dyn Fft<f32>>
let fft_inverse = planner.plan_fft_inverse(4096);
```

- **Planning is expensive, execution is cheap.** Plan once, reuse the `Arc<dyn Fft<f32>>`.
- The planner caches plans internally -- requesting the same size twice returns the same `Arc`.
- Auto-detects SIMD: AVX/SSE4.1 on x86_64, Neon on aarch64. **No SIMD on WASM** (scalar fallback).

### Fft Trait

```rust
pub trait Fft<T: FftNum>: Send + Sync {
    fn process(&self, buffer: &mut [Complex<T>]);
    fn process_with_scratch(&self, buffer: &mut [Complex<T>], scratch: &mut [Complex<T>]);
    fn get_inplace_scratch_len(&self) -> usize;
    fn len(&self) -> usize;
}
```

- **`process()`** - In-place FFT. Buffer must have exactly `len()` elements. **Allocates a scratch buffer internally on every call.**
- **`process_with_scratch()`** - Same but uses caller-provided scratch, avoiding allocation. Scratch must be >= `get_inplace_scratch_len()` elements.
- The trait is `Send + Sync` -- `Arc<dyn Fft<f32>>` can be shared across threads. `process()` takes `&self` (immutable).

### Complex (re-exported from num-complex)

```rust
use rustfft::num_complex::Complex;
```

| Method | Description | Project usage |
|--------|-------------|---------------|
| `Complex::new(re, im)` | Constructor | Load real samples: `Complex::new(sample * window, 0.0)` |
| `Complex::from_polar(r, theta)` | From magnitude + phase | Rebuild spectrum after phase modification |
| `.arg()` | Phase angle in radians `(-pi, pi]` | Extract phase for vocoder |
| `.norm()` | Magnitude `sqrt(re^2 + im^2)` | Extract magnitude for vocoder |
| `.norm_sqr()` | `re^2 + im^2` (no sqrt) | Useful for magnitude comparisons |
| `.re` | Real part (public field) | Extract time-domain signal after inverse FFT |
| `.im` | Imaginary part (public field) | |

## Critical: Normalization

**rustfft does NOT normalize its output.** A forward+inverse round-trip scales by N (the FFT size).

You must apply `1/N` normalization manually after the inverse FFT:
```rust
let norm = 1.0 / fft_size as f32;
output[i] = frame_buf[i].re * norm;
```

The project handles this correctly in `vocoder.rs`.

## Performance Notes

- **Power-of-2 sizes are fastest** (radix-2/radix-4 Cooley-Tukey). Size 4096 is optimal.
- Sizes with small prime factors (2, 3, 5, 7) are well-optimized. Large primes use Bluestein's (slower).
- **f32 is the right choice for audio** -- sufficient precision, faster with SIMD (8 floats/instruction vs 4 for f64 on AVX).
- **WASM**: No SIMD acceleration. Scalar only. Still sub-millisecond for size 4096. ~2-4x slower than native with AVX.
- `lto = true` and `opt-level = 3` (already configured) help significantly for WASM FFT performance.

## How the Project Uses It

In `StreamingPhaseVocoder` (`src/vocoder.rs`):

1. Plan once in `new()` -- store `Arc<dyn Fft<f32>>` for forward and inverse
2. Pack real samples as `Complex::new(sample, 0.0)` into reusable `frame_buf`
3. Forward FFT via `fft_forward.process(&mut frame_buf)`
4. Extract magnitude/phase via `.norm()` and `.arg()`
5. Phase vocoder algorithm (accumulate phase, compute instantaneous frequency)
6. Reconstruct spectrum via `Complex::from_polar(mag, new_phase)`
7. Inverse FFT via `fft_inverse.process(&mut frame_buf)`
8. Apply `1/N` normalization manually
9. Extract real part via `.re` for time-domain output

## Gotchas and Pitfalls

1. **No normalization** (repeated for emphasis). Forward+Inverse scales by N. Forgetting this causes massive clipping.

2. **`process()` allocates on every call.** For zero-allocation hot paths, use `process_with_scratch()` with a pre-allocated scratch buffer. This is a potential optimization for the project.

3. **Buffer length must exactly match planned size.** Wrong length = panic.

4. **Complex input required for real signals.** No real-to-complex FFT mode. Must pack as `Complex::new(sample, 0.0)`. Uses 2x memory/computation vs an optimal real FFT, but acceptable for phase vocoder since full complex spectrum is needed.

5. **Frequency bin ordering:** Bins 0..N/2 = positive frequencies (DC to Nyquist), bins N/2+1..N-1 = negative frequencies. For real input, spectrum is conjugate-symmetric: `X[k] = conj(X[N-k])`.

6. **Phase wrapping:** `.arg()` returns `(-pi, pi]`. Phase differences need wrapping. The project uses: `dp -= (dp / (2.0 * PI)).round() * 2.0 * PI`.

7. **Single-threaded execution.** `process()` runs on the calling thread. No internal parallelism. Fine for WASM (single-threaded per worker).

## Potential Optimization

Replace `process()` with `process_with_scratch()` using a pre-allocated scratch buffer stored in the struct, to eliminate per-frame heap allocation in WASM:

```rust
// In struct:
scratch: Vec<Complex<f32>>,

// In new():
let scratch_len = fft_forward.get_inplace_scratch_len()
    .max(fft_inverse.get_inplace_scratch_len());
let scratch = vec![Complex::new(0.0, 0.0); scratch_len];

// In processing:
self.fft_forward.process_with_scratch(&mut self.frame_buf, &mut self.scratch);
```

---

## Learned Notes

<!-- Add notes here as you learn things about rustfft through usage, debugging, forum posts, etc. -->

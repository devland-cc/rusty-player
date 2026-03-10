# Spectral Analysis: Magnitude & Phase

**Relevance:** The FFT output is complex-valued. Decomposing into magnitude and phase is fundamental to the phase vocoder.
**Files:** `src/vocoder.rs` (Steps 3–4 of `try_process_frame()`)

## Complex FFT Output

Each FFT bin k produces a complex number `X[k] = a + jb` where:
- `a` = real part (`re`)
- `b` = imaginary part (`im`)

This complex number encodes two independent properties of the frequency component at bin k:

### Magnitude (Amplitude)
```
|X[k]| = sqrt(a² + b²)
```

In rustfft: `self.frame_buf[k].norm()`

The magnitude tells you **how much** of that frequency is present. Larger magnitude = louder at that frequency.

### Phase (Angle)
```
∠X[k] = atan2(b, a)
```

In rustfft: `self.frame_buf[k].arg()`, returns radians in `(-π, π]`

The phase tells you **where in its cycle** that frequency component is at the start of the analysis frame. It determines the fine temporal alignment of the sinusoidal component.

## Why Phase Matters for Time-Stretching

Magnitude carries **what** the audio sounds like (timbre, volume). Phase carries **when** things happen (timing, transients, spatial cues).

When time-stretching:
- **Magnitudes are preserved** — the output should sound the same, just slower/faster
- **Phases must be adjusted** — because the frames are at different time positions, the phases must advance correctly for the new time spacing

If you time-stretch by keeping magnitudes and ignoring phases (using raw analysis phases), every overlapping frame has phase misalignment. The overlap-add produces cancellation and reinforcement at random, creating a "chorus" or "metallic" effect.

## Polar Reconstruction

After computing the new synthesis phase, the spectrum is rebuilt:

```rust
self.frame_buf[k] = Complex::from_polar(mag, self.synth_phase[k]);
```

This converts `(magnitude, phase)` back to `(real, imaginary)`:
```
re = mag * cos(phase)
im = mag * sin(phase)
```

## Magnitude Spectrum Properties

### Symmetry for Real Input
For real-valued audio input, the FFT output has **conjugate symmetry**:
```
X[k] = conj(X[N-k])
```

This means:
- `|X[k]| = |X[N-k]|` (magnitude is symmetric)
- `∠X[k] = -∠X[N-k]` (phase is anti-symmetric)

The project processes all N bins, which maintains this symmetry after phase modification.

### Power Spectrum
```
P[k] = |X[k]|²
```

The power spectrum is the squared magnitude. Used for energy measurements. Related to Parseval's theorem: total energy in time domain = total energy in frequency domain.

### Spectral Envelope
The smooth curve connecting the spectral peaks. Represents the overall timbral shape (formants for voice, resonances for instruments). The phase vocoder preserves magnitudes (and thus the spectral envelope) when the stretch ratio is moderate, but at extreme pitch shifts the envelope shifts, changing timbre.

## Phase-Related Artifacts

| Artifact | Cause | How Phase Is Involved |
|----------|-------|----------------------|
| Metallic sound | Phase coherence loss between harmonics | Each harmonic's phase drifts independently |
| Phasiness | Constructive/destructive interference in overlap-add | Phase errors cause unpredictable cancellation |
| Stereo collapse | L/R phase drift | Independent phase accumulation diverges |
| Transient smearing | Phase is modified uniformly regardless of signal type | Transient phase relationships are critical for sharpness |

## Potential Improvements

### Phase Locking
See [04-instantaneous-frequency-and-phase-accumulation.md](04-instantaneous-frequency-and-phase-accumulation.md) — identity phase locking maintains phase relationships between harmonics by locking nearby bins to spectral peaks.

### Phase Randomization for Extreme Stretch
At very large stretch ratios (>4x), accumulated phase errors become dominant. Some implementations randomize phases for bins below a magnitude threshold, which sounds less objectionable than coherent phase errors (the ear is less sensitive to phase in noise-like components).

---

## Learned Notes

<!-- Add notes here -->

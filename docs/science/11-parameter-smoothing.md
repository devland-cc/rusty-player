# Parameter Smoothing (Exponential Moving Average)

**Relevance:** Prevents clicks and pops when the user changes tempo or pitch in real-time.
**Files:** `src/processor.rs` (`smooth_and_update_params()`)

## The Problem

Abruptly changing DSP parameters (vocoder stretch, resampler ratio) causes discontinuities in the output waveform. These discontinuities are heard as clicks, pops, or bursts. The output at sample N was computed with one set of parameters, and sample N+1 with drastically different parameters — the waveform "jumps."

## The Solution: Exponential Smoothing

Instead of jumping to the target value instantly, interpolate toward it gradually:

```rust
const ALPHA: f64 = 0.5;
self.tempo_ratio += (self.target_tempo - self.tempo_ratio) * ALPHA;
self.pitch_semitones += (self.target_pitch - self.pitch_semitones) * ALPHA;
```

This is a **first-order IIR low-pass filter** (also called exponential moving average or EMA):
```
current = current + (target - current) * α
        = (1 - α) * current + α * target
```

## Convergence Behavior

At each step, the remaining distance to target is multiplied by `(1 - α)`:

| Step | Remaining Distance | With α=0.5 | With α=0.1 |
|------|-------------------|------------|------------|
| 0 | 100% | 100% | 100% |
| 1 | (1-α) | 50% | 90% |
| 2 | (1-α)² | 25% | 81% |
| 3 | (1-α)³ | 12.5% | 72.9% |
| 5 | (1-α)⁵ | 3.1% | 59% |
| 10 | (1-α)¹⁰ | 0.1% | 34.9% |

With α=0.5, the parameter is within 1% of the target after ~7 steps. The update rate is once per `fill_output()` call, which happens every `n_frames / sample_rate` seconds. At 4096 frames / 44100 Hz ≈ 93ms per call, convergence takes ~650ms.

## Why α=0.5

The project uses α=0.5, which provides:
- **Fast response**: User sees the change take effect within ~0.3 seconds
- **Smooth transition**: No audible clicks or discontinuities
- **Low overshoot**: First-order smoothing never overshoots

Previous attempts used α=0.1, which was too slow — the display showed "0.5x" but audio took several seconds to actually reach half speed, confusing users.

## Snap-to-Target

```rust
if (self.tempo_ratio - self.target_tempo).abs() < 0.001 {
    self.tempo_ratio = self.target_tempo;
}
```

Without snapping, exponential smoothing theoretically never reaches the target (always 50% of remaining). The snap threshold eliminates this infinite tail and ensures exact parameter values.

## Smoothing in the Context of Bypass Mode

```rust
fn is_bypass(&self) -> bool {
    (self.tempo_ratio - 1.0).abs() < 0.005
        && self.pitch_semitones.abs() < 0.05
        && (self.target_tempo - 1.0).abs() < 0.005
        && self.target_pitch.abs() < 0.05
}
```

Bypass checks BOTH current AND target values. This prevents oscillating between bypass and vocoder mode during the smoothing transition (which would cause repeated vocoder priming).

## Potential Improvements

### Per-Sample Smoothing
Currently, smoothing happens once per `fill_output()` call (~93ms). For smoother transitions, smooth per-sample or per-frame within the processing loop. This matters more at extreme parameter changes.

### Logarithmic Smoothing for Tempo
Tempo perception is logarithmic — the difference between 0.5x and 1.0x feels the same as 1.0x and 2.0x. Smoothing in log-space would provide perceptually uniform transitions:
```rust
let log_current = current.ln();
let log_target = target.ln();
log_current += (log_target - log_current) * alpha;
current = log_current.exp();
```

### Crossfade on Large Changes
For very large parameter jumps (e.g., tempo from 0.25x to 4.0x), smoothing still produces audible artifacts as the stretch ratio passes through extreme values. A short crossfade between the old and new parameter states would be cleaner.

---

## Learned Notes

<!-- Add notes here -->

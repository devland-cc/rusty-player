# Musical Pitch & Frequency Relationships

**Relevance:** The math that converts user-facing pitch controls (semitones) into DSP parameters.
**Files:** `src/processor.rs` (`apply_dsp_params()`)

## Equal Temperament

Western music divides each octave into 12 equal semitones. The frequency ratio between adjacent semitones is:

```
ratio = 2^(1/12) ≈ 1.05946
```

An octave (12 semitones) is exactly a 2:1 frequency ratio. This is the **equal temperament** tuning system.

## The Core Formula

```rust
let pitch_ratio = 2.0f64.powf(self.pitch_semitones / 12.0);
```

| Semitones | pitch_ratio | Musical Interval | Effect |
|-----------|------------|------------------|--------|
| -12 | 0.500 | Octave down | Half frequency |
| -7 | 0.667 | Perfect fifth down | |
| -5 | 0.707 | Perfect fourth down | |
| -1 | 0.944 | Semitone down | |
| 0 | 1.000 | Unison | No change |
| +1 | 1.059 | Semitone up | |
| +5 | 1.335 | Perfect fourth up | |
| +7 | 1.498 | Perfect fifth up | |
| +12 | 2.000 | Octave up | Double frequency |

## How Pitch & Tempo Are Decoupled

The project achieves independent pitch and tempo control through two cascaded stages:

```rust
let vocoder_stretch = pitch_ratio / tempo_ratio;
let resample_ratio = 1.0 / pitch_ratio;
```

The **net duration change** is:
```
net = vocoder_stretch * resample_ratio
    = (pitch_ratio / tempo_ratio) * (1 / pitch_ratio)
    = 1 / tempo_ratio
```

The pitch_ratio cancels out — the net duration depends only on tempo_ratio. Meanwhile, the pitch change comes entirely from the resampler's ratio.

### Examples

| User Setting | pitch_ratio | vocoder_stretch | resample_ratio | Net Duration |
|-------------|------------|-----------------|----------------|--------------|
| Tempo 0.5x, Pitch 0 | 1.0 | 2.0 | 1.0 | 2.0x (half speed) |
| Tempo 1.0x, Pitch +12 | 2.0 | 2.0 | 0.5 | 1.0x (same duration) |
| Tempo 2.0x, Pitch -12 | 0.5 | 0.25 | 2.0 | 0.5x (double speed) |
| Tempo 0.75x, Pitch +3 | 1.189 | 1.585 | 0.841 | 1.333x |

## Cents

For finer pitch control, musicians use **cents** (1/100 of a semitone):
```
ratio = 2^(cents / 1200)
```

The project currently uses semitone granularity. Adding cent-level control would require no code changes — just pass fractional semitones (e.g., `set_pitch(0.5)` = +50 cents).

## A440 Reference

The standard tuning reference is A4 = 440 Hz. The test tone in `processor.rs` uses this:
```rust
let val = (2.0 * PI as f64 * 440.0 * t).sin() as f32 * 0.3;
```

## Frequency to MIDI Note

For potential future features (auto-tuning, pitch detection):
```
midi_note = 69 + 12 * log2(frequency / 440)
frequency = 440 * 2^((midi_note - 69) / 12)
```

A4 = MIDI note 69. Middle C (C4) = MIDI note 60.

---

## Learned Notes

<!-- Add notes here -->

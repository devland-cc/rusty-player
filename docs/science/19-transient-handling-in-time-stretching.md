# Transient Handling in Time-Stretching

**Relevance:** A key quality limitation — the phase vocoder softens transients (drums, plucks, attacks).
**Files:** `src/vocoder.rs` (inherent limitation of the algorithm)

## The Problem

Transients are sharp, short-duration events in audio: drum hits, plucked strings, consonants in speech, percussion. They are characterized by:
- Rapid energy increase (fast attack)
- Broadband frequency content (energy across many bins simultaneously)
- Precise temporal alignment (phase coherence across all bins)

The phase vocoder inherently degrades transients because:

### 1. Overlap-Add Spreading
Each transient is captured by multiple overlapping analysis frames. After time-stretching, these frames are spaced further apart (synthesis_hop > analysis_hop), spreading the transient energy over a longer time interval. A 10ms drum hit becomes a 20ms "thud" at 2x stretch.

### 2. Phase Modification
The phase accumulation step modifies the phase relationship between frequency bins. Transients require precise phase coherence — all frequencies must arrive at the same instant. After phase modification, this coherence is partially lost, further smearing the transient.

### 3. Windowing
The Hann window tapers the frame to zero at both ends. If a transient occurs near the edge of a frame, it's attenuated. The transient's energy is distributed across adjacent frames, each of which sees only part of the event.

## Audible Effect

- Drums sound "soft" or "mushy" rather than sharp and punchy
- Percussive attacks lose their "crack"
- Plucked string/guitar sounds lose the initial pick attack
- Speech consonants become blurred (especially plosives: p, t, k, b, d, g)

This is the most common complaint about phase vocoder quality and the primary reason professional software uses hybrid methods.

## Detection Methods

### Spectral Flux
Measure the rate of change in the magnitude spectrum between consecutive frames:

```
flux = Σ max(0, |X_current[k]| - |X_prev[k]|)
```

High spectral flux indicates a transient. The `max(0, ...)` (half-wave rectification) ignores decreasing magnitudes, focusing on onset energy.

### Energy Ratio
Compare frame energy to a running average:

```
transient = frame_energy / running_average_energy > threshold
```

Threshold typically 2.0–4.0. Simple but effective.

### High-Frequency Energy Ratio (HFE)
Transients have proportionally more high-frequency energy than tonal content. Measure the ratio of energy above a cutoff (e.g., 4 kHz) to total energy.

## Mitigation Strategies

### 1. Phase Reset on Transients
When a transient is detected, reset `synth_phase = analysis_phase` for that frame. This preserves the transient's phase coherence at the cost of a potential phase discontinuity with the previous frame.

```rust
if is_transient {
    for k in 0..fft_size {
        self.synth_phase[k] = self.frame_buf[k].arg();
    }
} else {
    // normal phase accumulation
}
```

The discontinuity is usually masked by the transient itself.

### 2. Transient Bypass
Route transient frames through a simple time-domain overlap-add (no phase modification) while tonal frames go through the full phase vocoder.

### 3. Transient Separation
Pre-process the audio to separate transient and tonal components:
1. Detect transient regions
2. Extract transients into a separate signal
3. Phase-vocoder process the tonal residual
4. Time-stretch the transients via simple time-domain methods
5. Mix back together

This is the most complex but highest-quality approach, used in professional software.

### 4. Adaptive FFT Size
Use a shorter FFT size (e.g., 1024 instead of 4096) for frames containing transients. Shorter windows have better time resolution, preserving transient sharpness at the cost of frequency resolution (acceptable for broadband transients).

### 5. Waveform Similarity OLA (WSOLA)
For transient-heavy content, WSOLA is a time-domain method that:
1. Finds the best matching segment in the input for each output position
2. Cross-fades between overlapping segments

WSOLA preserves transients perfectly but can introduce pitch artifacts for tonal content. A hybrid phase-vocoder + WSOLA approach uses each method where it excels.

## Current Project Status

The project does **not** implement transient detection or preservation. All audio content (tonal and transient) goes through the same phase vocoder pipeline. This is the most significant remaining quality limitation.

## Implementation Priority

For maximum quality improvement with minimum complexity:
1. **Phase reset** (simplest): Add spectral flux measurement, reset phases when flux exceeds threshold
2. **Adaptive overlap**: Increase overlap temporarily around transients (more CPU but better preservation)
3. **Hybrid method**: Full transient separation and WSOLA would require significant new code

---

## Learned Notes

<!-- Add notes here -->

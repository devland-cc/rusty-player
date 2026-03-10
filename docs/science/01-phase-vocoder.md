# Phase Vocoder

**Relevance:** Core algorithm — the entire time-stretching capability of rusty-player depends on this.
**Files:** `src/vocoder.rs`, `src/processor.rs`

## What It Is

A phase vocoder is a frequency-domain audio processing technique that allows **time-stretching without pitch change** (and vice versa). It works by decomposing audio into overlapping spectral frames via the STFT (Short-Time Fourier Transform), modifying the time spacing between frames, and reconstructing via overlap-add synthesis.

The name "vocoder" is historical — it derives from "voice coder" but the technique is general-purpose for any audio signal.

## How It Works (High Level)

```
Input → Window → FFT → Phase Processing → IFFT → Window → Overlap-Add → Output
         ↑                                                      ↑
    analysis hop                                          synthesis hop
```

1. Extract overlapping frames from input (spaced by `analysis_hop` samples)
2. Apply a window function (Hann) to each frame
3. Forward FFT to get frequency-domain representation (magnitude + phase per bin)
4. Compute instantaneous frequency from phase differences between consecutive frames
5. Accumulate synthesis phase using the ratio `synthesis_hop / analysis_hop`
6. Reconstruct each bin as `magnitude * e^(j * synth_phase)`
7. Inverse FFT back to time domain
8. Apply synthesis window and overlap-add into output buffer

**Time-stretching** is achieved by making `synthesis_hop ≠ analysis_hop`:
- `synthesis_hop > analysis_hop` → output is longer (slow down)
- `synthesis_hop < analysis_hop` → output is shorter (speed up)

## Project Implementation

In `vocoder.rs`, the `StreamingPhaseVocoder` processes mono audio through ring buffers:

```
analysis_hop = fft_size / overlap = 4096 / 8 = 512
synthesis_hop = round(analysis_hop * stretch_ratio)
```

The stretch ratio is computed in `processor.rs`:
```
vocoder_stretch = pitch_ratio / tempo_ratio
```

This formula is the key insight: the vocoder doesn't know about "tempo" or "pitch" — it only knows a stretch ratio. The processor decomposes the user's intent (tempo + pitch) into a stretch ratio for the vocoder and a resample ratio for the pitch corrector.

## Quality Characteristics

### Strengths
- Excellent for sustained/harmonic content (vocals, pads, strings)
- Preserves pitch perfectly during time-stretch
- Smooth, artifact-free output at moderate stretch ratios (0.5x–2.0x)
- Works well with the 4096/8 configuration for music

### Weaknesses
- **Spectral smearing**: Each FFT bin averages frequency content within its bandwidth (~10.7 Hz at 44100/4096). Closely-spaced harmonics blur together.
- **Transient softening**: Sharp attacks (drums, plucks) are spread across the overlap-add window, losing their punch.
- **Phasiness at extreme ratios**: Beyond 3x stretch, accumulated phase errors become audible as metallic/robotic coloration.

## Future Quality Improvements

### Identity Phase Locking
Instead of accumulating phase independently per bin, identify **spectral peaks** (local maxima in magnitude) and lock surrounding bins' phases to the peak's phase. This reduces the "phasiness" by maintaining harmonic relationships:

```
For each peak bin p:
  For each influenced bin k near p:
    synth_phase[k] = synth_phase[p] + (analysis_phase[k] - analysis_phase[p])
```

This is the single highest-impact improvement for audio quality.

### Transient Detection & Preservation
Detect transients by measuring spectral flux (sum of positive magnitude changes between frames). When a transient is detected:
- Reset synthesis phases to analysis phases (prevents smearing)
- Or bypass the vocoder entirely for that segment
- Or use a shorter FFT window for the transient frame

### Spectral Envelope Preservation
At large pitch shifts, the spectral envelope shifts with the harmonics, changing timbre (chipmunk effect). Preserve the envelope by:
1. Estimate the spectral envelope (e.g., via cepstrum or LPC)
2. After phase modification, re-apply the original envelope
3. This is particularly important for vocal content

### Hybrid Methods (Phase Vocoder + WSOLA)
Professional tools combine frequency-domain (phase vocoder) for tonal content with time-domain methods (WSOLA — Waveform Similarity Overlap-Add) for transients. A simple hybrid:
1. Classify each frame as tonal or transient
2. Route tonal frames through the phase vocoder
3. Route transient frames through WSOLA (which preserves sharp attacks)

## Key Parameters & Their Effects

| Parameter | Current Value | Effect of Increase | Effect of Decrease |
|-----------|--------------|--------------------|--------------------|
| FFT_SIZE | 4096 | Better frequency resolution, more latency, more smearing of transients | Worse frequency resolution, less latency, better transient preservation |
| OVERLAP | 8 | Smoother output, more CPU cost | More amplitude modulation artifacts, less CPU |
| FEED_CHUNK | 512 (= analysis_hop) | N/A (tied to overlap) | N/A |

## Key References

- Dolson, M. (1986). "The Phase Vocoder: A Tutorial" — the foundational paper
- Laroche, J. & Dolson, M. (1999). "Improved Phase Vocoder Time-Scale Modification of Audio" — introduces identity phase locking
- Driedger, J. & Müller, M. (2016). "A Review of Time-Scale Modification of Music Signals" — comprehensive survey of all methods

---

## Learned Notes

<!-- Add notes here as you learn things about phase vocoders through usage, debugging, or research -->

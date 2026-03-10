# Linked-Phase Stereo Processing

**Relevance:** Preserves stereo image when processing L/R through separate phase vocoders.
**Files:** `src/vocoder.rs` (`process_frame_linked()`), `src/processor.rs` (lockstep processing)

## The Problem

When L and R channels are processed through independent phase vocoders, each accumulates its own synthesis phase:

```
L vocoder: synth_phase_L[k] += increment_L[k]
R vocoder: synth_phase_R[k] += increment_R[k]
```

Even for identical input (mono signal panned center), floating-point arithmetic and the non-linear phase accumulation cause `increment_L[k]` and `increment_R[k]` to diverge over time. After hundreds of frames:
- The inter-channel phase difference at each frequency bin drifts randomly
- Stereo content narrows toward mono
- Spatial cues (panning, reverb tails, room ambience) are destroyed
- This degradation is **cumulative** — inaudible at first, obvious after 10–30 seconds

## The Solution: Phase Linking

Use the **Left channel's phase increments as the reference** for both channels:

```
L vocoder: processes normally, computes increment[k]
R vocoder: uses L's increment[k] instead of computing its own
```

The R channel keeps its own magnitudes (preserving the amplitude difference that encodes panning) but follows L's phase trajectory.

## Implementation

### L Channel: Normal Processing (try_process_frame)
```rust
let increment = inst_freq * hop_ratio;
self.synth_phase[k] += increment;
self.last_phase_increments[k] = increment;  // save for R channel
```

### R Channel: Linked Processing (process_frame_linked)
```rust
pub fn process_frame_linked(&mut self, ref_phase_increments: &[f32]) -> bool {
    // ... FFT, get magnitudes ...

    // Use L's increments instead of computing own
    self.synth_phase[k] += ref_phase_increments[k];

    // Rebuild with R's magnitudes + linked phase
    self.frame_buf[k] = Complex::from_polar(mag, self.synth_phase[k]);

    // ... IFFT, overlap-add ...
}
```

### Lockstep Processing (processor.rs)
```rust
while self.vocoders[0].can_process() && self.vocoders[1].can_process() {
    self.vocoders[0].try_process_frame();              // L: compute
    let increments = self.vocoders[0].last_phase_increments().to_vec();
    self.vocoders[1].process_frame_linked(&increments); // R: follow
}
```

**Critical**: Both channels must process **one frame at a time in lockstep**. You cannot batch-process all L frames then all R frames — L's increments are needed per-frame.

## Why It Preserves Stereo

The stereo image is encoded in:
1. **Amplitude differences** (panning): L and R have different magnitudes → preserved because each channel keeps its own magnitudes
2. **Phase differences** (spatial cues): L and R have consistent phase relationships → preserved because R follows L's phase trajectory

By linking phases, the inter-channel phase relationship is maintained, preventing the drift that collapses the stereo image.

## Limitations

### Mono-ization of Phase Information
The R channel loses its own phase evolution. For signals where L and R have genuinely different frequency content (e.g., hard-panned instruments), the linked phase may not be ideal — R's phases should track its own frequencies, not L's.

In practice, most stereo music has similar frequency content in L and R (differing mainly in amplitude and slight phase), so this approach works well.

### Residual Width Loss
Even with phase linking, some width loss can occur because:
- The magnitudes are processed independently and may drift
- The overlap-add normalization slightly differs between channels
- At extreme stretch ratios, accumulated errors still grow

The M/S post-processing (see [08-mid-side-stereo-processing.md](08-mid-side-stereo-processing.md)) compensates for this residual loss.

## Vocoder Priming Must Also Be Linked

When transitioning from bypass to vocoder mode, the vocoders are pre-filled with lookback data. This priming must also use linked-phase processing:

```rust
// In prime_vocoders()
if channels == 2 {
    while self.vocoders[0].can_process() && self.vocoders[1].can_process() {
        self.vocoders[0].try_process_frame();
        let increments = self.vocoders[0].last_phase_increments().to_vec();
        self.vocoders[1].process_frame_linked(&increments);
    }
}
```

If priming uses independent processing, the channels start with misaligned phases, causing an audible stereo image "snap" when vocoder mode begins.

## Potential Improvements

### Mid/Side Vocoder Processing
Instead of processing L/R and linking phases, transform to M/S first:
```
M = (L + R) / 2
S = (L - R) / 2
```
Process M and S through independent vocoders, then reconstruct L/R. This naturally preserves the stereo structure because M and S are orthogonal. However, it changes the character of the processing and may introduce different artifacts.

### Adaptive Phase Linking
Use L's phases for bins where L and R are similar, but use each channel's own phases for bins where they differ significantly (hard-panned content). Measure per-bin correlation to decide dynamically.

---

## Learned Notes

<!-- Add notes here -->

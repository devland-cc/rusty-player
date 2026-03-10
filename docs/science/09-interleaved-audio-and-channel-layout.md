# Interleaved Audio & Channel Layout

**Relevance:** How multi-channel audio data is organized in memory. Affects every stage of the pipeline.
**Files:** `src/decoder.rs`, `src/processor.rs`, `src/lib.rs`

## What It Is

Audio samples for multiple channels can be stored in two layouts:

### Interleaved (used by this project)
Samples alternate between channels: `[L0, R0, L1, R1, L2, R2, ...]`

- Each "frame" is a group of samples (one per channel) at the same time instant
- `frame[i]` starts at index `i * channels`
- Channel `ch` of frame `i` is at index `i * channels + ch`

### Planar
Each channel stored contiguously: `[L0, L1, L2, ..., R0, R1, R2, ...]`

- `channel[ch]` starts at index `ch * total_frames`
- Frame `i` of channel `ch` is at index `ch * total_frames + i`

## Why Interleaved

Interleaved is the dominant format because:
1. **Web Audio API** expects interleaved data in `Float32Array`
2. **Most audio I/O APIs** (ALSA, CoreAudio, WASAPI) use interleaved
3. **symphonia's `copy_interleaved_ref()`** outputs interleaved
4. **Cache-friendly for playback**: reading frame-by-frame accesses sequential memory

## Deinterleaving for DSP

The phase vocoder processes **mono** channels independently. The processor must deinterleave before feeding the vocoder and re-interleave after:

### Deinterleave (processor.rs)
```rust
for ch in 0..channels {
    self.mono_in.resize(to_feed, 0.0);
    for f in 0..to_feed {
        self.mono_in[f] = self.source_samples[self.source_pos + f * channels + ch];
    }
    self.vocoders[ch].write_input(&self.mono_in[..to_feed]);
}
self.source_pos += to_feed * channels;
```

### Re-interleave (processor.rs)
```rust
for f in 0..produced {
    let idx = (out_pos + f) * channels + ch;
    if idx < total_samples {
        output[idx] = self.mono_resampled[f];
    }
}
```

## Frame vs Sample Counting

A consistent source of bugs is confusing **frames** (time instants) with **samples** (individual values):

```
total_samples = total_frames * channels
```

For stereo (channels=2):
- 4096 frames = 8192 samples
- `source_pos` is in **samples**, not frames
- Position in seconds: `frame = source_pos / channels`, `secs = frame / sample_rate`

The project uses `source_pos` in samples throughout `processor.rs`. Frame-based calculations divide by `channels`:
```rust
pub fn position_secs(&self) -> f64 {
    let frame = self.source_pos / self.channels.max(1);
    frame as f64 / self.output_sample_rate as f64
}
```

## Common Pitfalls

1. **Off-by-channels errors**: Advancing `source_pos` by frames instead of samples (or vice versa) causes channel misalignment — L samples read as R and vice versa, producing crackling or silence.

2. **Assuming stereo**: The code mostly handles arbitrary channel counts, but some paths are stereo-specific (M/S processing, linked-phase). The `.max(1)` guards prevent division by zero for edge cases.

3. **Buffer sizing**: Output buffers must be `n_frames * channels`, not just `n_frames`.

---

## Learned Notes

<!-- Add notes here -->

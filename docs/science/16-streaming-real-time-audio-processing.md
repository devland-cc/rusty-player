# Streaming Real-Time Audio Processing

**Relevance:** The entire architecture — processing audio in chunks with bounded latency and no gaps.
**Files:** All `src/*.rs`

## What It Is

Streaming audio processing means handling audio as a continuous flow of small chunks rather than processing an entire file at once. Key constraints:

1. **Bounded latency**: Output must be produced within a time budget (typically < 100ms)
2. **Constant throughput**: Must produce exactly as many output samples as the playback rate demands
3. **No allocation in the hot path**: Heap allocation can cause unpredictable latency spikes
4. **Deterministic CPU usage**: Processing time per chunk must be consistent

## The Project's Processing Chain

```
Source PCM → [deinterleave] → [vocoder] → [resampler] → [interleave] → Output
              per-channel       mono        mono          stereo
```

Each stage is **streaming** — it maintains internal state between calls and processes small chunks incrementally.

### Call Hierarchy
```
fill_output(n_frames)           // entry point, called by JS
  ├─ smooth_and_update_params() // once per call
  ├─ fill_bypass(n_frames)      // direct copy when no DSP needed
  └─ fill_vocoder(n_frames)     // full DSP pipeline
       ├─ deinterleave source
       ├─ vocoder.write_input()  // feed source to vocoder
       ├─ vocoder.try_process_frame()  // process FFT frames
       ├─ vocoder.drain_output() // read time-stretched audio
       ├─ resampler.process()    // pitch-correct via resampling
       └─ interleave to output
```

## Chunk Sizes and Timing Budget

| Stage | Chunk Size | Time (at 44100 Hz) |
|-------|-----------|---------------------|
| `fill_output` request | 4096 frames | 92.9 ms |
| `FEED_CHUNK` (source feed) | 512 frames | 11.6 ms |
| Vocoder analysis hop | 512 samples | 11.6 ms |
| Vocoder frame (FFT size) | 4096 samples | 92.9 ms |

The outer loop in `fill_vocoder` iterates: feed 512 source frames → process vocoder → read output → resample. Multiple iterations may be needed to fill the requested 4096 output frames.

## Latency Sources

| Source | Latency | Notes |
|--------|---------|-------|
| Vocoder startup | 4096 samples = 92.9 ms | First FFT frame needs full window. Mitigated by priming. |
| JS scheduling buffer | ~300 ms | The JS scheduler pre-buffers 0.3s of audio chunks |
| Overlap-add | ~fft_size/2 = 46 ms | Overlap-add introduces group delay |
| Resampler | ~0 ms | Linear interpolation is zero-latency |

Total system latency: ~400 ms from parameter change to audible effect.

## State Management

Each streaming component maintains state across calls:

### Vocoder State
- `prev_phase[k]`, `synth_phase[k]`: Phase continuity between frames
- `input_ring`, `output_ring`: Buffered data between calls
- `has_state`: First-frame flag

### Resampler State
- `frac_pos`: Sub-sample position continuity
- `prev_sample`: Cross-buffer interpolation

### Processor State
- `source_pos`: Current read position in source audio
- `tempo_ratio`, `pitch_semitones`: Smoothed parameter values
- `vocoder_primed`: Whether vocoders have lookback data
- `stereo_correction`: Smoothed M/S correction factor

## Reset Protocol

When seeking or re-entering vocoder mode, all streaming state must be reset:
```rust
pub fn seek(&mut self, position_secs: f64) {
    self.source_pos = ...;
    self.vocoder_primed = false;
    for v in &mut self.vocoders { v.reset(); }
    for r in &mut self.resamplers { r.reset(); }
}
```

Failing to reset causes:
- Stale phase data → metallic artifacts
- Stale ring buffer data → output from wrong position
- Wrong fractional position → pitch glitch

## Producer-Consumer Rate Matching

The vocoder produces `synthesis_hop` samples per FFT frame, but the output requests `n_frames` at a time. The processing loop in `fill_vocoder` bridges this mismatch with a feed-process-read cycle bounded by `max_iterations`:

```rust
let max_iterations = (n_frames / FEED_CHUNK + 1) * 10;
```

The safety bound prevents infinite loops when the pipeline can't produce output (e.g., end of source, or a configuration error).

## Potential Improvements

### AudioWorklet Integration
Move DSP to a Web Worker via AudioWorklet for lower latency. Currently, processing happens on the main thread and the JS scheduler buffers ~300ms. An AudioWorklet processes 128-sample blocks at native audio thread priority, reducing latency to ~3ms per block.

### Zero-Allocation Hot Path
Eliminate `Vec<f32>` allocation in `fill_output()`. Pre-allocate the output buffer in the struct and return a view into it. Similarly, use `process_with_scratch()` for FFTs.

### Adaptive Chunk Sizing
Adjust the processing chunk size based on the current stretch ratio. At 4x slow-down, the vocoder produces 4x more output per source frame — larger output chunks could be read less frequently.

---

## Learned Notes

<!-- Add notes here -->

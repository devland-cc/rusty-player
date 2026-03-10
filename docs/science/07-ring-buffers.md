# Ring Buffers (Circular Buffers)

**Relevance:** The streaming I/O mechanism for the phase vocoder and the overall audio pipeline.
**Files:** `src/vocoder.rs` (input_ring, output_ring, window_sum_ring)

## What It Is

A ring buffer (circular buffer) is a fixed-size array that wraps around. When the write pointer reaches the end, it wraps to the beginning. This provides constant-time enqueue/dequeue without memory allocation or data shifting.

```
[....RXXXXW....]
     ^    ^
     read write

After write wraps:
[XXW.....RXXXX]
   ^     ^
   write read
```

## Why Ring Buffers for Audio

Audio processing has a fundamental mismatch: the **producer** (data source) and **consumer** (DSP algorithm) operate at different rates and chunk sizes. Ring buffers decouple them:

- The vocoder needs exactly `fft_size` (4096) samples to process a frame
- The source provides `FEED_CHUNK` (512) samples at a time
- The ring buffer accumulates input until a full frame is available

Similarly for output:
- The vocoder produces `synthesis_hop` samples per frame
- The consumer requests `n_frames` samples at a time
- The output ring buffer accumulates vocoder output for smooth reading

## Project Implementation

### Input Ring Buffer
```rust
input_ring: Vec<f32>,       // capacity: fft_size * 8 = 32768
input_write: usize,         // write position (wraps)
input_read: usize,          // read position (wraps)
input_available: usize,     // current fill level
```

Write:
```rust
pub fn write_input(&mut self, samples: &[f32]) -> usize {
    let cap = self.input_ring.len();
    let space = cap - self.input_available;
    let to_write = samples.len().min(space);
    for i in 0..to_write {
        self.input_ring[self.input_write] = samples[i];
        self.input_write = (self.input_write + 1) % cap;
    }
    self.input_available += to_write;
    to_write
}
```

Read (for FFT frame extraction):
```rust
// Read fft_size samples starting at input_read (wrapping)
let idx = (self.input_read + i) % input_cap;
let sample = self.input_ring[idx];

// Advance by analysis_hop (not fft_size) — frames overlap
self.input_read = (self.input_read + analysis_hop) % input_cap;
self.input_available -= analysis_hop;
```

**Key detail:** The read pointer advances by `analysis_hop` (512), not by `fft_size` (4096). This means consecutive frames share 4096-512 = 3584 samples (87.5% overlap at overlap=8).

### Output Ring Buffer (with overlap-add)
The output ring is more complex because it supports overlap-add — multiple frames write to the same positions:

```rust
output_ring: Vec<f32>,      // capacity: fft_size * 16 = 65536
window_sum_ring: Vec<f32>,  // parallel tracking of window² sums
```

Writing is **additive** (overlap-add):
```rust
self.output_ring[out_idx] += frame_sample;      // accumulate
self.window_sum_ring[out_idx] += w * w;          // track normalization
```

Reading clears the position after reading (preparing for future frames):
```rust
output[i] = self.output_ring[self.output_read];
self.output_ring[self.output_read] = 0.0;        // clear for reuse
self.window_sum_ring[self.output_read] = 0.0;
```

## Sizing Considerations

| Buffer | Size | Rationale |
|--------|------|-----------|
| Input ring | `fft_size * 8` = 32768 | Must hold at least `fft_size` for one frame. 8x provides headroom for bursty writes. |
| Output ring | `fft_size * 16` = 65536 | Must hold overlapping frames at max stretch. At stretch=10x, synthesis_hop=5120, and fft_size=4096 frames overlap within ~40960 samples. 16x provides margin. |

### Overflow Risk
If the ring buffer fills up (available = capacity), writes are silently dropped:
```rust
let space = cap - self.input_available;
let to_write = samples.len().min(space);  // caps at available space
```

This should never happen in normal operation with proper sizing, but could occur if:
- The consumer stalls (vocoder can't process fast enough)
- Extreme stretch ratios exceed the buffer capacity

## Performance Characteristics

- **Zero allocation:** All buffers are allocated once in `new()`, no per-frame allocation
- **Cache-friendly:** Sequential access pattern (mostly). Wrapping at the boundary causes one cache miss per buffer pass.
- **Modulo operation:** `% cap` on every access. Since capacities are powers of 2 (32768, 65536), the compiler optimizes this to a bitwise AND: `& (cap - 1)`.

## Potential Improvements

### Power-of-2 Enforcement
Explicitly ensure capacities are powers of 2 and use bitwise AND instead of modulo:
```rust
let mask = cap - 1;
let idx = (pos + i) & mask;  // faster than % cap
```

### Lock-Free Ring Buffers
For a future AudioWorklet integration (where producer and consumer run on different threads), the ring buffer would need atomic read/write pointers. The current implementation is single-threaded and doesn't need this.

---

## Learned Notes

<!-- Add notes here -->

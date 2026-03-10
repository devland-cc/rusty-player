# Web Audio API Integration

**Relevance:** How processed audio gets from WASM to the speakers.
**Files:** `src/lib.rs`, JavaScript scheduling code (see TEMPO_PITCH_DSP_REFERENCE.md)

## Architecture

```
WASM (Rust DSP) → Vec<f32> → JS glue → AudioBufferSourceNode → speakers
```

All DSP runs in Rust/WASM. The Web Audio API is used only as a transport layer to route samples to the audio output device.

## AudioBufferSourceNode Scheduling

The project uses a timer-based scheduling pattern:

```javascript
function scheduleChunks() {
    while (nextStartTime < audioCtx.currentTime + 0.3) {
        const samples = player.process(4096);  // WASM call
        const buf = audioCtx.createBuffer(2, frames, sampleRate);
        // deinterleave WASM output into AudioBuffer channels
        const source = audioCtx.createBufferSource();
        source.buffer = buf;
        source.connect(audioCtx.destination);
        source.start(nextStartTime);
        nextStartTime += frames / sampleRate;
    }
    setTimeout(scheduleChunks, 50);
}
```

### How It Works
1. Pre-schedule ~300ms of audio chunks ahead of the current playback time
2. Each chunk is a separate `AudioBufferSourceNode` with a precise `start()` time
3. The Web Audio API's internal scheduler plays them seamlessly at the right times
4. A `setTimeout` at 50ms re-checks and fills the buffer when needed

### Why 300ms Pre-Buffer
The main thread is not real-time — garbage collection, layout, paint, and other tasks can delay JavaScript execution by 50–200ms. Pre-buffering ensures continuous playback even during GC pauses.

### Why Not AudioWorklet
An `AudioWorkletProcessor` runs on the audio render thread and processes 128-sample blocks at native priority. This is lower-latency and more reliable, but:
- Requires `SharedArrayBuffer` (needs COOP/COEP headers)
- WASM must be loaded in the worklet scope
- More complex setup and debugging

The current approach is simpler and adequate for a music player where 300ms latency is acceptable.

## Data Flow: WASM → Web Audio

### WASM Output
```rust
pub fn process(&mut self, n_frames: u32) -> Vec<f32>
```
Returns interleaved `[L0, R0, L1, R1, ...]` as `Vec<f32>`, which becomes a `Float32Array` in JS.

### Deinterleave in JS
Web Audio's `AudioBuffer` stores channels separately (planar). The interleaved WASM output must be deinterleaved:
```javascript
const left = buf.getChannelData(0);
const right = buf.getChannelData(1);
for (let i = 0; i < frames; i++) {
    left[i] = samples[i * 2];
    right[i] = samples[i * 2 + 1];
}
```

## AudioContext Lifecycle

### Creation
```javascript
const audioCtx = new AudioContext({ sampleRate: 44100 });
```

The sample rate must match the WASM processor's output rate (passed to `RustyPlayer::new()`).

### Suspension
Browsers require user interaction before starting audio (autoplay policy). The AudioContext starts in "suspended" state and must be resumed:
```javascript
audioCtx.resume();
```

## COOP/COEP Headers

For `SharedArrayBuffer` (needed by AudioWorklet):
```nginx
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
```

These headers isolate the page, enabling cross-origin isolation required for shared memory.

## Known Pitfalls

1. **Duplicate schedulers**: Play→Pause→Play must cancel the previous `setTimeout` chain, or two schedulers run simultaneously, consuming audio at 2x speed.

2. **AudioContext sample rate**: If the device rate differs from 44100, Web Audio silently resamples. This can introduce additional quality loss. The project handles this by resampling source to output rate in `processor.rs:load()`.

3. **GC pauses**: The Vec<f32> copy from WASM to JS creates garbage. In long sessions, GC pauses can cause audio dropouts. Pre-allocated typed arrays or zero-copy approaches can mitigate this.

## Potential Improvements

### AudioWorklet Migration
Move to AudioWorkletProcessor for lower latency and more reliable timing. Share a ring buffer via SharedArrayBuffer between the main thread (WASM processing) and the audio thread (worklet reading).

### Zero-Copy Output
Instead of returning `Vec<f32>` (which copies), expose a fixed buffer in WASM memory and create a Float32Array view directly:
```javascript
const ptr = player.get_output_ptr();
const view = new Float32Array(wasmMemory.buffer, ptr, n_frames * 2);
```
This eliminates the per-call allocation and copy.

---

## Learned Notes

<!-- Add notes here -->

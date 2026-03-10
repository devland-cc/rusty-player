# symphonia

**Version in use:** 0.5 with `default-features = false, features = ["mp3", "pcm", "isomp4"]`
**Used in:** `src/decoder.rs`

- GitHub: https://github.com/pdeljanov/Symphonia
- API docs (main): https://docs.rs/symphonia/latest/symphonia/
- API docs (core): https://docs.rs/symphonia-core/latest/symphonia_core/
- SampleBuffer docs: https://docs.rs/symphonia-core/latest/symphonia_core/audio/struct.SampleBuffer.html
- Error enum: https://docs.rs/symphonia-core/latest/symphonia_core/errors/enum.Error.html

## Overview

Pure-Rust audio decoding library. No unsafe code in core, no C library dependencies. Ideal for WASM targets. Modular architecture with feature flags for codec/format support.

## Decoding Pipeline

```
MediaSourceStream -> Probe -> FormatReader -> Decoder -> SampleBuffer
```

### 1. MediaSourceStream

Wraps any `Read + Seek` source into a buffered stream:
```rust
let cursor = Cursor::new(data.to_vec());
let mss = MediaSourceStream::new(Box::new(cursor), Default::default());
```
Internal buffer defaults to 64 KB (configurable via `MediaSourceStreamOptions`).

### 2. Probe

```rust
let mut hint = Hint::new();
hint.with_extension("mp3");

let probed = symphonia::default::get_probe()
    .format(&hint, mss, &format_opts, &MetadataOptions::default())?;
let mut reader = probed.format;  // Box<dyn FormatReader>
```

Uses the `Hint` (extension, MIME type) + header bytes to detect the container format.

### 3. Track Selection

```rust
let track = reader.tracks().iter()
    .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
    .ok_or("No audio track")?;
```

Filter out `CODEC_TYPE_NULL` tracks. Essential for multi-track containers (MP4) that may have non-audio tracks.

`CodecParameters` fields are `Option`:
- `sample_rate: Option<u32>` -- may be `None` for some streams
- `channels: Option<Channels>` -- call `.count()` to get channel count
- `n_frames: Option<u64>` -- total frames, `None` for VBR without Xing header or streaming

### 4. Decoder

```rust
let mut decoder = symphonia::default::get_codecs()
    .make(&codec_params, &DecoderOptions::default())?;
```

### 5. Decode Loop

```rust
loop {
    let packet = match reader.next_packet() {
        Ok(p) => p,
        Err(Error::IoError(ref e)) if e.kind() == UnexpectedEof => break,
        Err(e) => return Err(e),
    };
    if packet.track_id() != track_id { continue; }

    let decoded = match decoder.decode(&packet) {
        Ok(d) => d,
        Err(Error::DecodeError(_)) => continue,  // skip corrupt frame
        Err(e) => return Err(e),
    };

    // Copy samples out BEFORE next decode() call
    buf.copy_interleaved_ref(decoded);
    all_samples.extend_from_slice(buf.samples());
}
```

**Critical:** `AudioBufferRef` returned by `decode()` borrows the decoder's internal buffer. Must copy samples out before the next `decode()` call.

## SampleBuffer API

```rust
let mut buf = SampleBuffer::<f32>::new(num_frames as u64, spec);
```

| Method | Description |
|--------|-------------|
| `new(duration, spec)` | `duration` is frame count (u64), not seconds |
| `copy_interleaved_ref(decoded)` | Copy from `AudioBufferRef` in interleaved layout (L R L R...) |
| `copy_planar_ref(decoded)` | Copy in planar layout (all L, then all R) |
| `samples()` | Returns `&[f32]` of converted samples (frames * channels) |
| `capacity()` | Returns frame capacity (not sample count!) |

**Interleaved** (L R L R) is standard for Web Audio API. The project correctly uses interleaved.

**Capacity is in frames, not samples.** This asymmetry with `samples()` (which returns individual samples) can cause confusion.

## Gapless Decoding

```rust
let format_opts = FormatOptions {
    enable_gapless: true,
    ..Default::default()
};
```

MP3 inherently adds padding (encoder delay at start, padding frames at end). With `enable_gapless: true`:
- Reads LAME/Xing header for exact delay/padding values
- Automatically trims leading and trailing padding
- Result: sample-accurate playback without clicks or gaps

**Always use `enable_gapless: true` for music playback.**

## Error Handling

| Error Variant | Source | Action |
|---------------|--------|--------|
| `IoError(UnexpectedEof)` | `next_packet()` | **Break** -- stream is done (normal termination) |
| `IoError(other)` | `next_packet()` | Fatal -- return error |
| `DecodeError(&'static str)` | `decode()` | **Recoverable** -- skip packet, continue |
| `Unsupported(&'static str)` | anywhere | Fatal |
| `LimitError(&'static str)` | anywhere | Fatal |
| `ResetRequired` | `decode()` | Call `decoder.reset()`, then continue |

`DecodeError` is common in the wild -- MP3 streams frequently have minor corruption. Always skip and continue.

## Feature Flags

| Feature | Crate enabled | What it provides |
|---------|---------------|------------------|
| `"mp3"` | `symphonia-bundle-mp3` | MP3 format reader (demuxer) + codec (decoder). MPEG-1/2 Layer III. Reads LAME/Xing/VBRI headers. |
| `"pcm"` | `symphonia-codec-pcm` | PCM codec -- handles uncompressed audio in containers. Various sample formats (u8, i16, i24, i32, f32, f64, both endians). |
| `"isomp4"` | `symphonia-format-isomp4` | MP4/M4A container demuxer. Reads `moov` atoms, track metadata. Format reader only, not a codec. |

**Why these three together:** Covers standard `.mp3` files + MP3/PCM audio in MP4/M4A containers. Practical set for a web player.

Other available features: `"flac"`, `"vorbis"`, `"wav"`, `"ogg"`, `"aac"`, `"all"`, `"all-codecs"`, `"all-formats"`.

## Memory Considerations (WASM)

- **Pre-allocate based on duration metadata** to avoid Vec doubling (the project does this correctly):
  ```rust
  let capacity = (duration_secs * sample_rate as f64 * channels as f64) as usize;
  let mut all_samples = Vec::with_capacity(capacity);
  ```
- WASM linear memory can grow but **never shrinks**. Large transient allocations permanently increase the memory footprint.
- `data.to_vec()` makes a full copy of input. If caller can transfer ownership, this copy could be avoided.
- Typical 4-min stereo MP3 at 44.1kHz: ~84 MB decoded PCM + compressed size in memory simultaneously.

## Gotchas

1. **`AudioBufferRef` lifetime** -- must copy samples out before next `decode()`. Rust's borrow checker enforces this.
2. **`capacity()` is frames, `samples()` is individual samples** -- asymmetric units.
3. **SampleBuffer may need reallocation** -- MP3 can switch between 1152 frames (MPEG-1) and 576 frames (MPEG-2). Check capacity before use.
4. **`n_frames` may be `None`** -- VBR without Xing header, streaming, truncated files. Handle gracefully.
5. **`sample_rate` and `channels` may be `None`** -- the project defaults to 44100/2 which is reasonable for MP3.
6. **Filter by `track_id`** -- multi-track containers interleave packets from different tracks. Essential check.
7. **`DecodeError` string is `&'static str`** -- fixed at compile time, don't parse or match on content.

---

## Learned Notes

<!-- Add notes here as you learn things about symphonia through usage, debugging, forum posts, etc. -->

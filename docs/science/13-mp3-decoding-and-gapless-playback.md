# MP3 Decoding & Gapless Playback

**Relevance:** The input stage — converting compressed MP3 files to PCM samples for processing.
**Files:** `src/decoder.rs`

## MP3 Format Basics

MP3 (MPEG-1 Audio Layer III) is a lossy audio codec that compresses audio by:
1. Transforming to frequency domain via MDCT (Modified Discrete Cosine Transform)
2. Applying a psychoacoustic model to determine which frequencies are masked (inaudible)
3. Quantizing and encoding only the perceptually relevant data

### Key Parameters
- **Sample rates**: 32000, 44100, 48000 Hz (MPEG-1); also half rates for MPEG-2
- **Frame size**: 1152 samples (MPEG-1) or 576 samples (MPEG-2)
- **Bitrate**: 32–320 kbps (CBR) or variable (VBR)
- **Channels**: Mono, stereo, joint stereo, dual channel

## The Gapless Problem

MP3 encoding inherently adds padding:

### Encoder Delay (Start Padding)
The MDCT encoder needs a full frame of "future" samples to start encoding. This adds 576–1152 samples of silence at the beginning. The exact delay depends on the encoder.

### Frame Padding (End Padding)
The total sample count must be a multiple of the frame size (1152). The last frame is padded with silence to fill it out.

### Result Without Gapless
Playing a raw MP3 decode gives:
- ~26ms of silence at the start (1152 samples at 44100 Hz)
- Variable silence at the end
- Audible gap between tracks (breaks crossfades, live recordings, concept albums)

## Gapless Solution

MP3 encoders (LAME, iTunes) write metadata headers containing exact padding values:

### LAME/Xing Header
Stored in the first frame (which contains no audio):
- Encoder delay (start padding in samples)
- End padding (samples to trim from last frame)
- Total sample count

### iTunes iTunSMPB
An ID3 metadata tag with similar information.

### Symphonia's Gapless Support
```rust
let format_opts = FormatOptions {
    enable_gapless: true,
    ..Default::default()
};
```

With `enable_gapless: true`, symphonia:
1. Reads the LAME/Xing/VBRI header from the first MP3 frame
2. Extracts encoder delay and padding values
3. Automatically trims the padding from decoded output
4. Returns only the actual audio content

This is transparent to the project — `decode_mp3()` gets clean, sample-accurate PCM.

## Decoding Pipeline in the Project

```
MP3 bytes → Cursor → MediaSourceStream → Probe → FormatReader → Decoder → SampleBuffer → Vec<f32>
```

1. **Probe**: Detects MP3 format from header bytes + hint
2. **Track selection**: Finds first track with a valid codec (`!= CODEC_TYPE_NULL`)
3. **Decode loop**: Reads packets, decodes each to `AudioBufferRef`, copies to `SampleBuffer`
4. **Output**: Interleaved `Vec<f32>` with all samples

### Error Recovery
```rust
Err(SymphoniaError::DecodeError(_)) => continue,  // skip corrupt frame
```

MP3 streams in the wild frequently have minor corruption (bit errors, truncated frames). The standard practice is to skip corrupt frames and continue. Symphonia reports these as `DecodeError`, which are recoverable.

## Memory Considerations

For a 4-minute stereo MP3 at 44100 Hz:
- Compressed: ~4–8 MB (at 128–256 kbps)
- Decoded PCM: `4 * 60 * 44100 * 2 * 4 bytes = ~84 MB`
- Peak during decode: compressed + decoded simultaneously in memory

The project pre-allocates the output vector:
```rust
let capacity = (duration_secs * sample_rate as f64 * channels as f64) as usize;
let mut all_samples = Vec::with_capacity(capacity);
```

This avoids expensive Vec doubling in WASM's linear memory.

## Potential Improvements

### Streaming Decode
Instead of decoding the entire file upfront, decode on-demand as the playback position advances. This would reduce peak memory usage from ~84 MB to a small window around the current position. However, it adds complexity for seeking and requires the compressed data to stay in memory.

### Format Support
Adding features like `"flac"`, `"vorbis"`, `"aac"` to symphonia would expand supported formats with minimal code changes — the decode pipeline is format-agnostic.

---

## Learned Notes

<!-- Add notes here -->

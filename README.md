# rusty-player

A WebAssembly MP3 player with real-time tempo and pitch control, built entirely in Rust. Features a broadcast-quality phase vocoder, offline music analysis (BPM, key, beat grid), and two browser-based UIs.

## How It Works

Audio arrives as an MP3 file, gets decoded to PCM, then passes through a real-time DSP pipeline before reaching the Web Audio API. The entire signal path — from decoding to time-stretching to resampling — runs in Rust compiled to WASM with zero JavaScript DSP.

```
MP3 bytes
  │
  ▼
┌──────────────┐
│   Decoder    │  symphonia — MP3 → interleaved f32 PCM
└──────┬───────┘
       ▼
┌──────────────┐
│ Phase Vocoder│  rustfft — STFT overlap-add time-stretching
│  (per ch.)   │  4096-pt FFT, 8x overlap, linked-phase stereo
└──────┬───────┘
       ▼
┌──────────────┐
│  Resampler   │  Pitch shift via sample-rate conversion
│              │  Linear or cubic Hermite interpolation
└──────┬───────┘
       ▼
┌──────────────┐
│  Post-proc   │  Gain compensation, M/S stereo correction,
│              │  soft limiter
└──────┬───────┘
       ▼
  Web Audio API
```

## DSP Features

### Phase Vocoder

The core of the player is an STFT-based phase vocoder that time-stretches audio without changing pitch (or vice versa). It operates at:

- **4096-sample FFT** with a **Hann window** — large enough for clean frequency resolution at 44.1 kHz
- **8x overlap** (hop = 512 samples) — heavy overlap keeps spectral smearing low
- **Phase accumulation** — tracks instantaneous frequency per bin across frames, accumulates synthesis phase to produce the correct output hop

The vocoder supports tempo ratios from 0.25x to 4.0x and pitch shifts of ±12 semitones. Tempo and pitch are independent: tempo changes the synthesis hop (time-stretch), and pitch is achieved by combining a stretch with resampling.

### Linked-Phase Stereo

Stereo audio is processed with linked phase increments: the left channel computes phase advances normally, and the right channel reuses those same advances. This prevents the vocoder from widening or smearing the stereo image — a common artefact when channels are processed independently.

### Identity Phase Locking

Spectral peaks are identified in each frame, and non-peak bins are locked to the phase of their nearest peak. This reduces the "phasiness" and underwater quality that basic phase vocoders produce, especially on transient-heavy material.

### Transient Detection

A spectral-flux detector measures frame-to-frame energy changes. When flux exceeds an adaptive threshold (running average × sensitivity multiplier), the frame is flagged as a transient and synthesis phases are reset to analysis phases. This preserves the attack of drums and percussive sounds that vocoders typically smear.

Sensitivity is configurable from 0.0 (very sensitive, catches subtle changes) to 1.0 (only triggers on strong transients).

### Resampling

Two resampler modes are available:

- **Linear interpolation** — fast, low overhead, default
- **Cubic Hermite interpolation** — 4-point polynomial with flatter frequency response and less aliasing at extreme pitch shifts

### M/S Stereo Correction

Time-stretching tends to collapse or widen the stereo field. The M/S processor measures Mid and Side energy ratios before and after the vocoder, then applies an adaptive correction factor (clamped 0.5x–3.0x) to restore the original stereo width.

### Soft Limiter

A `tanh`-based soft clipper prevents hard clipping from gain compensation. It engages above 0.9 amplitude with a smooth saturation curve, keeping output within ±1.0 without audible distortion.

### Gain Compensation

Configurable makeup gain from 0–6 dB (slider 0.0–1.0) compensates for the slight level loss inherent in overlap-add synthesis.

### Parameter Smoothing

All real-time controls (tempo, pitch) are smoothed with an exponential filter (α = 0.5) to prevent clicks and zipper noise during adjustment.

### Bypass Mode

When tempo ≈ 1.0 and pitch ≈ 0, the entire DSP chain is bypassed — audio passes through untouched with zero processing overhead.

## Offline Analysis

The analysis module runs a complete offline pass over the decoded audio and returns:

### BPM Detection

1. Audio is downsampled 4x for efficiency
2. Onset detection via spectral energy flux (1024-pt FFT, 256 hop)
3. Peak picking on the onset signal
4. Inter-Onset Interval (IOI) histogram
5. Mode detection converts the dominant interval to BPM
6. Requires a minimum of 8 IOIs for a reliable estimate

### Key Detection

Uses **Krumhansl-Kessler key profiles** — derived from perceptual experiments — matched against a 12-bin chromagram via Pearson correlation. KK profiles handle pop, rock, and electronic music more accurately than Temperley profiles, which are biased toward classical harmonic minor.

All 24 keys (12 major + 12 minor) are evaluated; the highest-correlation match wins, with a confidence score.

### Real-Time Key Segments

Key detection also runs in a **sliding window** mode (10-second window, 4-second step) across the track, producing timestamped key segments. A **hysteresis filter** requires 2+ consecutive windows to agree before emitting a key change, preventing jittery detection on ambiguous passages.

### Beat Grid

Adaptive beat tracking produces a list of beat timestamps that follows tempo changes within the track.

## Configuration

Defaults live in a single JSON file shared between Rust and JavaScript:

```
www/config/player-defaults.json
```

At compile time, `build.rs` reads this file and generates Rust constants (`src/defaults_generated.rs` in `OUT_DIR`), so the WASM module and the browser UIs always agree on defaults with zero runtime cost.

Current defaults:

| Parameter | Default | Description |
|-----------|---------|-------------|
| Gain Compensation | 0.35 (~2.1 dB) | Makeup gain for vocoder level loss |
| M/S Stereo | ON | Stereo width preservation |
| Phase Lock | ON | Identity phase locking |
| Transient Detection | OFF | Spectral flux transient reset |
| Cubic Resampler | OFF | Uses linear interpolation |
| Soft Limiter | OFF | Tanh soft clipping |
| Transient Sensitivity | 0.5 | Medium sensitivity |

## Browser Interfaces

### AutoDJ

Dual-deck player with independent tempo/pitch control per deck and a crossfader.

- **XY Pad** per deck: X axis = tempo (0.5x–1.5x linear), Y axis = pitch (±12 semitones)
- **Equal-power crossfader**: `gain_A = cos(pos × π/2)`, `gain_B = sin(pos × π/2)` — both at −3 dB at center
- **Live analysis display**: BPM, key (updates in real time as the track plays), beat LED
- **Transport**: play/pause toggle, stop, time display

### RtTempoPitchQA

Full quality-analysis interface with individual controls for every DSP feature:

- Tempo slider (0.5x–2.0x exponential)
- Pitch slider (±12 semitones)
- Toggle buttons for phase lock, transient detection, cubic resampler, soft limiter, M/S mode
- Gain compensation slider
- Transient sensitivity slider
- Real-time BPM, key, and beat LED display

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Audio decoding | [symphonia](https://github.com/pdeljanov/Symphonia) 0.5 — pure Rust MP3 decoder |
| FFT | [rustfft](https://github.com/ejmahler/RustFFT) 6 — optimized FFT for the phase vocoder |
| WASM interop | [wasm-bindgen](https://github.com/rustwasm/wasm-bindgen) 0.2 + js-sys + web-sys |
| Serialization | [serde](https://serde.rs/) + serde-wasm-bindgen — structs cross the WASM boundary as JsValue |
| Build | wasm-pack → wasm32-unknown-unknown, opt-level 3, LTO |
| Containerization | Docker multi-stage (rust:1.86 builder → nginx:alpine) |
| Frontend | Vanilla JS + Web Audio API, no frameworks |

## Building

### Docker (recommended)

```bash
docker build -t rusty-player .
docker run -p 8080:80 rusty-player
```

Open `http://localhost:8080` for the landing page.

### Local

Requires Rust toolchain and wasm-pack:

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-pack
wasm-pack build --target web --out-dir www/pkg --release
```

Then serve `www/` with any static file server that sets the correct CORS headers:

```bash
npx serve www
```

> **Note**: Cross-Origin headers (`Cross-Origin-Opener-Policy: same-origin`, `Cross-Origin-Embedder-Policy: require-corp`) are required for `SharedArrayBuffer` support. The included nginx config handles this automatically.

## Testing

```bash
# Run all 72 tests
cargo test

# Via Docker
docker run --rm -v $(pwd):/app -w /app rust:1.86-slim-bookworm cargo test
```

Tests cover:
- Phase vocoder stretch ratios and phase tracking (15 tests)
- Resampler interpolation accuracy (10 tests)
- Processor pipeline, parameter clamping, bypass mode (18 tests)
- BPM detection, key detection, beat grid, key segments (25 tests)
- MP3 decoding and edge cases (4 tests)

## Project Structure

```
├── build.rs                    # Generates Rust constants from config JSON
├── Cargo.toml
├── Dockerfile                  # Multi-stage: build WASM → serve with nginx
├── nginx.conf
├── src/
│   ├── lib.rs                  # WASM entry point, RustyPlayer API
│   ├── processor.rs            # DSP pipeline orchestrator
│   ├── vocoder.rs              # Phase vocoder (STFT, overlap-add)
│   ├── resampler.rs            # Linear + cubic Hermite resampling
│   ├── decoder.rs              # MP3 decoding via symphonia
│   └── analysis.rs             # BPM, key, beat grid, key segments
├── www/
│   ├── index.html              # Landing page
│   ├── config/
│   │   └── player-defaults.json
│   ├── pkg/                    # WASM output (generated)
│   ├── worklet.js              # AudioWorklet processor (shared)
│   ├── AutoDJ/                 # Dual-deck interface
│   │   ├── index.html
│   │   ├── app.js
│   │   └── style.css
│   └── RtTempoPitchQA/         # QA interface with full controls
│       ├── index.html
│       ├── app.js
│       └── style.css
└── docs/
    ├── dependencies/           # Reference docs for key crates
    └── science/                # DSP theory and algorithm notes
```

## License

MIT

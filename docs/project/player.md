# Player Package Analysis (www/pkg/)

Systematic analysis of the WASM player package — the wasm-bindgen generated JS glue
(`rusty_player.js`), TypeScript declarations (`rusty_player.d.ts`), and the compiled
WASM binary (`rusty_player_bg.wasm`). Also covers the application-level JS files
(`app.js`, `worklet.js`).

**Test file:** `www/pkg/player.test.mjs` — 45 tests across 14 suites, all pass.
**Run:** `node --test www/pkg/player.test.mjs`

---

## Package Structure

### Exports

**Hypothesis:** The package exports three entry points: `RustyPlayer` class (the main API),
`init` (the Rust `#[wasm_bindgen(start)]` function), and the default export `__wbg_init`
(async WASM loader). `initSync` is also exported for synchronous instantiation from raw bytes.

**Test:** `package structure` suite — verifies all 4 exports are functions.

**Result:** CONFIRMED — all exports present and correctly typed.

### `package.json`

**Hypothesis:** Declares the package as `"type": "module"` with `main` pointing to
`rusty_player.js` and `types` to `rusty_player.d.ts`. The `sideEffects` field marks
`./snippets/*` to prevent tree-shaking of any wasm-bindgen snippet files.

**Result:** CONFIRMED by inspection — matches expected wasm-pack `--target web` output format.

---

## WASM Initialization

### `__wbg_init(module_or_path?)` (default export)

**Hypothesis:** Async initializer. Accepts a URL, Request, Response, BufferSource, or
WebAssembly.Module. If no argument is given, derives the WASM URL from `import.meta.url`.
Uses `WebAssembly.instantiateStreaming` when available (with fallback for wrong MIME type),
otherwise `WebAssembly.instantiate`. Calls `__wbg_finalize_init` which stores the WASM
instance, clears typed array caches, and calls `__wbindgen_start` (the Rust `init()` fn).
Returns early if WASM is already initialized (idempotent).

**Test:** Not directly tested in Node.js (requires `fetch`). Used by `app.js` in the browser.

**Result:** CONFIRMED by code inspection.

### `initSync(module)`

**Hypothesis:** Synchronous initializer. Accepts a BufferSource or WebAssembly.Module
(or `{module: ...}` object). Compiles and instantiates the WASM module synchronously.
Returns the WASM exports object. Idempotent — returns cached `wasm` if already initialized.

**Test:** `JS glue functions` suite — verifies return value has `memory`, `rustyplayer_new`,
`rustyplayer_process`, `__wbindgen_malloc`, and `__wbindgen_free`.

**Result:** CONFIRMED — test passes. Returns object with all expected WASM exports.

### `__wbg_finalize_init(instance, module)`

**Hypothesis:** Internal function that stores the WASM instance exports, caches the module,
invalidates typed array views (DataView, Float32Array, Uint8Array), and calls the WASM
start function (`__wbindgen_start` → Rust `init()` → `console_error_panic_hook::set_once()`).

**Result:** CONFIRMED by inspection — called by both `initSync` and `__wbg_init`.

### `__wbg_get_imports()`

**Hypothesis:** Builds the import object required by the WASM module. Provides JS functions
the WASM code calls back into:
- `__wbg_Error_*`: Creates JS Error objects from Rust panic messages
- `__wbg_String_*`: Converts Rust strings to JS strings
- `__wbindgen_throw_*`: Throws JS errors (for Rust panics)
- `__wbg_error_*`: Calls `console.error` (for `console_error_panic_hook`)
- `__wbg_new_*`: Creates new JS Error/Object instances
- `__wbg_set_*`: Sets properties on JS objects (used by `serde_wasm_bindgen`)
- `__wbg_stack_*`: Reads `.stack` from Error objects (for panic backtraces)
- `__wbindgen_cast_*`: Type cast intrinsics (f64→externref, string→externref, u64→BigInt)
- `__wbindgen_init_externref_table`: Initializes the externref table with undefined/null/true/false

**Result:** CONFIRMED by inspection — all import functions match wasm-bindgen's runtime needs.

---

## RustyPlayer Class (JS Wrapper)

### `constructor(sample_rate: number)`

**Hypothesis:** Calls `wasm.rustyplayer_new(sample_rate)`, stores the returned WASM pointer
as `this.__wbg_ptr`. Registers the instance with `FinalizationRegistry` for automatic cleanup
if the JS object is garbage collected without explicit `free()`.

**Test:** `RustyPlayer constructor` suite — creates instances at 44100 and 48000 Hz.

**Result:** CONFIRMED — both create successfully, pointer is assigned.

### `free()`

**Hypothesis:** Calls `__destroy_into_raw()` which zeroes `__wbg_ptr` and unregisters from
`FinalizationRegistry`, then calls `wasm.__wbg_rustyplayer_free(ptr, 0)` to deallocate
the Rust `RustyPlayer` struct. Safe to call once — subsequent calls on the zeroed pointer
would pass 0 to WASM (which is a no-op since `Box::from_raw(0)` is guarded by wasm-bindgen).

**Test:** `free / memory` suite — verifies single free doesn't crash, and 50 create/free
cycles don't leak or crash.

**Result:** CONFIRMED — test passes (50 cycles in 123ms).

### `is_loaded() -> boolean`

**Hypothesis:** Calls `wasm.rustyplayer_is_loaded(ptr)`, returns `ret !== 0`. Maps Rust's
`bool` (returned as i32: 0 or 1) to JS boolean.

**Test:** `default state` — false before load; `load_test_tone` — true after load.

**Result:** CONFIRMED — both tests pass.

### `is_playing() -> boolean`

**Hypothesis:** Same pattern as `is_loaded`. Maps `wasm.rustyplayer_is_playing(ptr)` to boolean.

**Test:** `default state` — false initially; `play / pause` suite — true after play, false after pause.

**Result:** CONFIRMED — tests pass.

### `channels() -> number`

**Hypothesis:** Returns `wasm.rustyplayer_channels(ptr) >>> 0`. The `>>> 0` converts the
i32 return to unsigned (Rust returns `usize` which wasm-bindgen maps to u32).

**Test:** `default state` — 2 (default stereo); `load_test_tone` — 2 after loading.

**Result:** CONFIRMED — tests pass.

### `duration_secs() -> number`

**Hypothesis:** Returns `wasm.rustyplayer_duration_secs(ptr)` directly (f64, no conversion needed).

**Test:** `default state` — 0 before load; `load_test_tone` — ~5.0 after load.

**Result:** CONFIRMED — tests pass.

### `position_secs() -> number`

**Hypothesis:** Returns `wasm.rustyplayer_position_secs(ptr)` directly (f64).

**Test:** `default state` — 0 before load; `seek` suite — updates after seek; `process` suite — advances during playback.

**Result:** CONFIRMED — tests pass.

### `mid_side_mode() -> boolean`

**Hypothesis:** Returns `wasm.rustyplayer_mid_side_mode(ptr) !== 0`.

**Test:** `default state` — true; `mid_side_mode` suite — toggles correctly.

**Result:** CONFIRMED — tests pass.

### `gain_comp_amount() -> number`

**Hypothesis:** Returns `wasm.rustyplayer_gain_comp_amount(ptr)` (f64).

**Test:** `default state` — 0.5; `gain_comp_amount` suite — reads back set values correctly.

**Result:** CONFIRMED — tests pass.

### `load_mp3(data: Uint8Array) -> any`

**Hypothesis:** Copies the JS `Uint8Array` into WASM linear memory via `passArray8ToWasm0`
(allocates with `__wbindgen_malloc`, copies bytes). Calls `wasm.rustyplayer_load_mp3(ptr, data_ptr, len)`.
Returns a 3-element array `[result_ref, error_ref, is_error]`. If `ret[2]` is truthy, throws
the error (taken from externref table). Otherwise returns the result — a JS object with
`{sample_rate, channels, duration_secs}` serialized by `serde_wasm_bindgen::to_value`.

**Test:** `load_mp3` suite — throws on invalid/empty data, returns correct TrackInfo for valid MP3.

**Result:** CONFIRMED — 4 tests pass (including real MP3 fixture).

### `load_test_tone() -> any`

**Hypothesis:** Calls `wasm.rustyplayer_load_test_tone(ptr)`. Returns a JS object with
`{sample_rate, channels, duration_secs}` — but unlike `load_mp3`, this doesn't use the
Result pattern (no error branch). Returns the externref directly.

**Test:** `load_test_tone` suite — verifies returned object fields and state changes.

**Result:** CONFIRMED — 4 tests pass.

### `play()`

**Hypothesis:** Calls `wasm.rustyplayer_play(ptr)`. No return value. Sets the internal
`playing` flag to true.

**Test:** `play / pause` suite — verifies `is_playing()` becomes true.

**Result:** CONFIRMED — test passes.

### `pause()`

**Hypothesis:** Calls `wasm.rustyplayer_pause(ptr)`. Sets `playing` to false.

**Test:** `play / pause` suite — verifies `is_playing()` becomes false.

**Result:** CONFIRMED — test passes.

### `seek(position_secs: number)`

**Hypothesis:** Calls `wasm.rustyplayer_seek(ptr, position_secs)`. Converts seconds to
sample offset, resets vocoders and resamplers.

**Test:** `seek` suite — updates position, clamps to end, resets to 0.

**Result:** CONFIRMED — 3 tests pass.

### `set_tempo(ratio: number)`

**Hypothesis:** Calls `wasm.rustyplayer_set_tempo(ptr, ratio)`. Sets target tempo
(clamped to [0.25, 4.0] on the Rust side).

**Test:** `set_tempo / set_pitch` suite — no crash at extreme values.

**Result:** CONFIRMED — test passes.

### `set_pitch(semitones: number)`

**Hypothesis:** Calls `wasm.rustyplayer_set_pitch(ptr, semitones)`. Sets target pitch
(clamped to [-12, 12] on the Rust side).

**Test:** `set_tempo / set_pitch` suite — no crash at extreme values.

**Result:** CONFIRMED — test passes.

### `set_mid_side_mode(enabled: boolean)`

**Hypothesis:** Calls `wasm.rustyplayer_set_mid_side_mode(ptr, enabled)`. JS boolean is
passed directly (wasm-bindgen converts to i32 0/1).

**Test:** `mid_side_mode` suite — round-trips correctly.

**Result:** CONFIRMED — test passes.

### `set_gain_comp_amount(amount: number)`

**Hypothesis:** Calls `wasm.rustyplayer_set_gain_comp_amount(ptr, amount)`. Clamped to
[0.0, 1.0] on the Rust side.

**Test:** `gain_comp_amount` suite — reads back correctly, clamps out-of-range.

**Result:** CONFIRMED — 2 tests pass.

### `process(n_frames: number) -> Float32Array`

**Hypothesis:** The core audio processing method. Calls `wasm.rustyplayer_process(ptr, n_frames)`.
Returns a 2-element array `[ptr, len]`. The JS glue then:
1. Creates a `Float32Array` view into WASM memory at `ptr` with length `len`
2. Calls `.slice()` to copy the data out (since the WASM memory may be invalidated)
3. Calls `wasm.__wbindgen_free(ptr, len * 4, 4)` to free the Rust-allocated Vec
4. Returns the copied Float32Array

The returned array has `n_frames * channels` elements (interleaved stereo).
Returns all zeros when not playing or no audio is loaded.

**Test:** `process` suite — 9 tests:
- Returns `Float32Array` type
- Correct length (`n_frames * 2` for stereo)
- Silence when not playing
- Silence when nothing loaded
- Non-silent at neutral tempo/pitch
- Position advances during playback
- `is_playing` becomes false at end of track
- Non-silent at tempo=0.5
- Non-silent at pitch=+6

**Result:** CONFIRMED — all 9 tests pass.

---

## JS Glue Internal Functions

### `passArray8ToWasm0(arg, malloc)`

**Hypothesis:** Allocates `arg.length` bytes in WASM memory via `malloc`, copies the
`Uint8Array` contents using `getUint8ArrayMemory0().set(arg, ptr)`. Stores length in
`WASM_VECTOR_LEN`. Used by `load_mp3` to pass MP3 bytes.

**Result:** CONFIRMED by inspection + `load_mp3` tests succeeding.

### `getArrayF32FromWasm0(ptr, len)`

**Hypothesis:** Returns a `Float32Array` subarray view into WASM memory at the given
pointer and length. Used by `process()` to read the output samples before copying.

**Result:** CONFIRMED by inspection + `process` tests succeeding.

### `passStringToWasm0(arg, malloc, realloc)`

**Hypothesis:** Encodes a JS string into WASM memory. Fast path: copies ASCII bytes directly.
Slow path: when a non-ASCII character is encountered (`code > 0x7F`), falls back to
`TextEncoder.encodeInto()` with reallocation (strings may grow when UTF-8 encoded).
Polyfills `encodeInto` for environments that lack it.

**Result:** CONFIRMED by inspection — used by serde_wasm_bindgen for string serialization.

### `getStringFromWasm0(ptr, len)` / `decodeText(ptr, len)`

**Hypothesis:** Decodes UTF-8 bytes from WASM memory into a JS string. `decodeText` tracks
cumulative bytes decoded and periodically resets the `TextDecoder` to work around a Safari
bug where `TextDecoder` fails after decoding ~2GB of data (`MAX_SAFARI_DECODE_BYTES = 2146435072`).

**Result:** CONFIRMED by inspection — the Safari workaround is well-documented in wasm-bindgen issues.

### `takeFromExternrefTable0(idx)`

**Hypothesis:** Reads a value from the WASM externref table at the given index, then
deallocates the slot via `__externref_table_dealloc`. Used to extract return values from
WASM calls that return JS objects (like `load_mp3`'s TrackInfo or error strings).

**Result:** CONFIRMED by inspection + `load_mp3` tests (both success and error paths).

### `getDataViewMemory0()` / `getFloat32ArrayMemory0()` / `getUint8ArrayMemory0()`

**Hypothesis:** Lazy-initialized typed array views into `wasm.memory.buffer`. Each function
checks if the cached view is still valid (buffer not detached/resized) and creates a new
one if needed. WASM memory can grow, which detaches all existing ArrayBuffer views — these
functions handle that transparently.

**Result:** CONFIRMED by inspection — standard wasm-bindgen pattern for memory safety.

### `RustyPlayerFinalization` (FinalizationRegistry)

**Hypothesis:** Registers each `RustyPlayer` instance for GC-driven cleanup. If a user
forgets to call `free()`, the FinalizationRegistry will call `__wbg_rustyplayer_free` when
the JS object is garbage collected. Falls back to a no-op if `FinalizationRegistry` is
not available (older environments).

**Result:** CONFIRMED by inspection — standard wasm-bindgen safety net.

---

## Application Files (not in pkg/, but part of the player)

### `app.js`

**Hypothesis:** Browser application entry point. Responsibilities:
1. **WASM init**: Calls `await init()` to load the WASM module
2. **File loading**: Reads MP3 via FileReader, passes `Uint8Array` to `player.load_mp3()`
3. **XY pad**: Maps 2D pointer position to tempo (0.5x–2.0x via `0.5 * 4^normX`) and pitch (-12 to +12 via `12 - normY * 24`)
4. **Scheduled playback**: Uses `AudioContext.createBuffer` + `source.start(nextStartTime)` for gapless audio scheduling. Processes 4096-frame chunks, schedules 300ms ahead, pumps every 50ms via `setTimeout`
5. **Transport**: Play/pause/stop buttons, seek to 0 on stop
6. **Time display**: `requestAnimationFrame` loop reading `position_secs()` / `duration_secs()`
7. **M/S toggle**: Calls `set_mid_side_mode()`
8. **Gain slider**: Maps 0–100% to `set_gain_comp_amount(0.0–1.0)`

**Note:** Uses `AudioContext.createBuffer` (main thread scheduling) rather than `AudioWorkletNode`.
The `worklet.js` file exists but is not currently imported by `app.js`.

**Test:** Not directly testable in Node.js (requires browser DOM + AudioContext). Behavior validated
indirectly through the WASM API tests.

**Result:** CONFIRMED by inspection.

### `worklet.js`

**Hypothesis:** An `AudioWorkletProcessor` implementation for low-latency playback via
SharedArrayBuffer. Not currently used by `app.js` (likely a planned or previous approach).

Architecture:
1. Reads from a SharedArrayBuffer-backed ring buffer with atomic read/write indices
2. Ring layout: `[writeIdx (4 bytes)][readIdx (4 bytes)][float32 data...]`
3. Uses `Atomics.load/store` for lock-free synchronization between main thread and audio thread
4. Deinterleaves ring buffer data (interleaved LRLR) into separate AudioWorklet output channels
5. Outputs silence on buffer underrun
6. Fills extra output channels with silence (if output has more channels than ring data)

**Test:** Not testable — `AudioWorkletProcessor` is a browser-only API and `registerProcessor`
is not available in Node.js.

**Result:** CONFIRMED by inspection.

---

## TypeScript Declarations

### `rusty_player.d.ts`

**Hypothesis:** Public API type declarations for consumers. Declares the `RustyPlayer` class
with all methods and their parameter/return types. Also declares `InitInput`, `InitOutput`,
`SyncInitInput` types and the `initSync` / default init function signatures.

Notable type mappings:
- `Uint8Array` for byte inputs (MP3 data)
- `Float32Array` for audio output
- `any` for JS object returns (TrackInfo from serde_wasm_bindgen)
- `number` for all numeric params (f64 and u32 both map to JS number)
- `boolean` for flag params
- `void` for commands with no return
- `Symbol.dispose` support for using `using` syntax

**Result:** CONFIRMED by inspection — matches the JS implementation exactly.

### `rusty_player_bg.wasm.d.ts`

**Hypothesis:** Low-level type declarations for the raw WASM exports. Lists every exported
function with its raw parameter types (all `number` since WASM only has i32/i64/f32/f64).
Also exports `memory: WebAssembly.Memory` and `__wbindgen_externrefs: WebAssembly.Table`.

These are internal — consumers should use `rusty_player.d.ts` instead.

**Result:** CONFIRMED by inspection.

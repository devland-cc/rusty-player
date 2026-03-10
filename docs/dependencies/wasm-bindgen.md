# wasm-bindgen

**Version in use:** 0.2
**Used in:** `src/lib.rs`

- Guide (official book): https://rustwasm.github.io/docs/wasm-bindgen/
- API docs: https://docs.rs/wasm-bindgen/latest/wasm_bindgen/
- Attributes reference: https://rustwasm.github.io/docs/wasm-bindgen/reference/attributes/index.html
- Type reference: https://rustwasm.github.io/docs/wasm-bindgen/reference/types.html
- Start function: https://rustwasm.github.io/docs/wasm-bindgen/reference/attributes/on-rust-exports/start.html

## Overview

Facilitates high-level interactions between Rust-compiled WASM and JavaScript. Core WASM only supports numeric types -- wasm-bindgen generates JS glue code for richer type conversions. Produces `.wasm` + `.js` glue + `.d.ts` TypeScript definitions.

## `#[wasm_bindgen]` Attributes

### On structs

```rust
#[wasm_bindgen]
pub struct RustyPlayer {
    processor: AudioProcessor,  // private, opaque to JS
}
```

- Becomes a JS class. JS receives an opaque handle (pointer into WASM linear memory).
- Only `pub` fields with `#[wasm_bindgen]` are visible to JS. Private fields are hidden.
- Must be `'static` (no lifetime parameters or non-`'static` generics).
- Freed when JS calls `.free()` or via `FinalizationRegistry` GC (don't rely on GC for deterministic cleanup).

### On impl blocks

```rust
#[wasm_bindgen]
impl RustyPlayer {
    pub fn play(&mut self) { ... }      // JS method, mutable borrow
    pub fn is_playing(&self) -> bool { ... }  // JS method, immutable borrow
}
```

- `&self` methods: immutable borrow
- `&mut self` methods: mutable borrow. **Runtime borrow checker** -- panics with "already borrowed" if re-entrant.
- `self` methods: consume the object (JS handle invalidated)
- Static methods (no `self`): become static methods on the JS class

### `#[wasm_bindgen(constructor)]`

```rust
#[wasm_bindgen(constructor)]
pub fn new(sample_rate: u32) -> Self { ... }
```

Enables `new RustyPlayer(44100)` in JS (instead of `RustyPlayer.new(44100)`). Only one per struct.

### `#[wasm_bindgen(start)]`

```rust
#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}
```

- Called automatically when WASM module is instantiated, before any exports.
- Must take no arguments, return `()` or `Result<(), JsValue>`.
- Only one per crate.
- Ideal for: panic hooks, global state init.

## Type Mappings

### Primitives (zero-cost, passed directly)

| Rust | JS |
|------|-----|
| `u8`..`u32`, `i8`..`i32` | `number` |
| `u64`, `i64` | `BigInt` (since 0.2.88+) |
| `f32`, `f64` | `number` |
| `bool` | `boolean` |
| `usize` | `number` (u32 on wasm32) |

### Heap-allocated (require copying)

| Rust | JS | Notes |
|------|-----|-------|
| `String` | `string` | Copied: Rust allocs in WASM, JS reads + creates string, Rust frees |
| `&str` (param) | `string` | JS string UTF-8 encoded into WASM memory |
| `Vec<f32>` | `Float32Array` | **Copied** into new JS typed array, then Rust Vec freed |
| `Vec<u8>` | `Uint8Array` | Copied similarly |
| `&[u8]` (param) | `Uint8Array` | JS data **copied** into WASM memory for duration of call |
| `JsValue` | any JS value | Opaque handle, index into JS object table |
| `Option<T>` | `T \| undefined` | |

### JsValue

```rust
JsValue::from_str("error message")   // JS string
JsValue::NULL                        // JS null
JsValue::UNDEFINED                   // JS undefined
JsValue::from(42)                    // JS number
JsValue::TRUE / JsValue::FALSE      // JS boolean
```

## Vec<f32> Return -- Copy Semantics

```rust
pub fn process(&mut self, n_frames: u32) -> Vec<f32>
```

1. Rust allocates and fills `Vec<f32>` in WASM linear memory
2. JS glue creates a **new** `Float32Array` by **copying** from WASM memory
3. Rust `Vec<f32>` is freed immediately after copying

Every call = alloc + copy + free. For real-time audio this means allocation churn.

**Zero-copy alternative:** Use `wasm_bindgen::memory()` + `js_sys::Float32Array::new_with_byte_offset_and_length()` to create a JS view directly into a pre-allocated WASM buffer. View becomes invalid if WASM memory grows, so recreate as needed.

## &[u8] Parameter

```rust
pub fn load_mp3(&mut self, data: &[u8]) -> Result<JsValue, JsValue>
```

JS glue allocates WASM memory, **copies** the `Uint8Array` in, passes pointer+length, then frees after return. File exists in both JS and WASM memory simultaneously during the call.

## Error Handling

```rust
Result<JsValue, JsValue>
```

- `Ok(value)` -- returned normally to JS
- `Err(js_val)` -- **thrown** as a JS exception
- JS side uses try/catch:
  ```js
  try { const info = player.load_mp3(bytes); }
  catch (e) { console.error(e); }  // e is the JsValue
  ```

## Memory Management

- **Structs:** Freed on `.free()` call or GC via `FinalizationRegistry`. Without explicit free = potential leak.
- **Vec returns:** Freed immediately after copy to JS. No leak.
- **Slice params:** Temporary WASM allocation freed after function returns.
- **WASM memory never shrinks.** High-water mark persists. Freed space is reusable by allocator but not returned to OS.

## Performance for Audio Workloads

- `Vec<f32>` return: allocates + copies every call. Cost is small per call but adds up. Larger frame counts = less relative overhead.
- Minimize JS-WASM boundary crossings. Batch processing (larger frame counts) is better.
- `opt-level = 3` + `lto = true` critical for DSP code performance.
- WASM memory fragmentation can occur with repeated alloc/free cycles in long sessions.

## Companion Crates

| Crate | Version | Usage |
|-------|---------|-------|
| `console_error_panic_hook` | 0.1 | `set_once()` in start function. Logs panic info to `console.error`. |
| `serde-wasm-bindgen` | 0.6 | Bridge serde Serialize/Deserialize with JsValue. See [serde-wasm-bindgen.md](serde-wasm-bindgen.md). |
| `js-sys` | 0.3 | Bindings to JS built-in objects (Array, Float32Array, etc.) |
| `web-sys` | 0.3 | Bindings to Web APIs. Project enables `features = ["console"]`. |
| `wasm-bindgen-futures` | 0.4 | Bridges Rust Futures with JS Promises. `spawn_local()`, `JsFuture::from()`. |

## Gotchas

1. **Runtime borrow panics.** `&mut self` methods use a runtime borrow checker. Re-entrant calls (e.g., from JS callbacks) panic with "already borrowed: BorrowMutError".

2. **Memory leaks from not calling `.free()`.** Don't rely solely on GC.

3. **WASM memory never shrinks.** Loading a 10MB MP3 permanently raises the high-water mark even after the data is freed.

4. **`usize` is 32-bit on wasm32.** 4GB max addressing.

5. **String encoding overhead.** Rust=UTF-8, JS=UTF-16. Every string crossing = re-encoding. Avoid in hot paths.

6. **Panics without `console_error_panic_hook`** produce opaque "unreachable" errors. The project correctly installs the hook.

7. **Large slices cause memory spikes.** `&[u8]` for a 20MB MP3 = 40MB total during the call (JS + WASM copies).

8. **`crate-type = ["cdylib", "rlib"]`** -- `cdylib` produces `.wasm` for wasm-bindgen. `rlib` needed for unit tests.

---

## Learned Notes

<!-- Add notes here as you learn things about wasm-bindgen through usage, debugging, forum posts, etc. -->

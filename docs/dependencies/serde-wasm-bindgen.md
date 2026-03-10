# serde + serde-wasm-bindgen

**Versions in use:** `serde = "1"` (with `features = ["derive"]`), `serde-wasm-bindgen = "0.6"`
**Used in:** `src/lib.rs`

- serde official docs: https://serde.rs/
- serde derive: https://serde.rs/derive.html
- serde-wasm-bindgen docs: https://docs.rs/serde-wasm-bindgen/latest/serde_wasm_bindgen/
- serde-wasm-bindgen crates.io: https://crates.io/crates/serde-wasm-bindgen

## Overview

`serde` provides `#[derive(Serialize)]` / `#[derive(Deserialize)]` for Rust structs. `serde-wasm-bindgen` bridges serde with `JsValue`, converting Rust structs directly to/from native JS objects without intermediate JSON.

## Core API

### Serialization (Rust -> JS)

```rust
serde_wasm_bindgen::to_value<T: Serialize>(value: &T) -> Result<JsValue, Error>
```

### Deserialization (JS -> Rust)

```rust
serde_wasm_bindgen::from_value<T: DeserializeOwned>(value: JsValue) -> Result<T, Error>
```

Note: `from_value` **consumes** the `JsValue` (takes ownership). Clone first if you need to keep it.

## Type Mappings (Rust -> JS)

| Rust Type | JavaScript Type |
|-----------|----------------|
| `bool` | `boolean` |
| `i8`..`i32`, `u8`..`u32` | `number` |
| `i64`, `u64` | `number` (safe range) or `BigInt` |
| `f32`, `f64` | `number` |
| `char`, `String`, `&str` | `string` |
| `Option::Some(v)` | the value |
| `Option::None` | **`undefined`** (not `null`!) |
| `()`, unit struct | `undefined` |
| `Vec<T>`, slices, arrays | `Array` |
| `HashMap<K, V>` | **`Map`** (not plain Object!) |
| Struct with named fields | Plain `Object` |
| Unit enum variant | `string` (variant name) |
| Newtype variant | `{ "VariantName": inner }` |

### Project's TrackInfo

```rust
#[derive(Serialize)]
struct TrackInfo {
    sample_rate: u32,    // -> JS number
    channels: usize,     // -> JS number (usize is u32 on wasm32)
    duration_secs: f64,  // -> JS number
}
```

Produces: `{ sample_rate: 44100, channels: 2, duration_secs: 180.5 }`

## Why serde-wasm-bindgen Over the Old Approach

The old `JsValue::from_serde()` / `JsValue::into_serde()` (deprecated, removed in wasm-bindgen 0.2.90+) had problems:

1. **JSON round-trip overhead** -- serialized to JSON string, then `JSON.parse()`. Double serialization.
2. **Loss of type fidelity** -- JSON can't represent `undefined`, `BigInt`, `Map`, `Uint8Array`, `NaN`, `Infinity`.
3. **Mandatory `serde_json` dependency** -- increased WASM binary size.
4. **Data corruption** -- `u64` beyond `Number.MAX_SAFE_INTEGER` lost precision in JSON.

`serde-wasm-bindgen` creates native JS values directly via `wasm-bindgen`/`js-sys` APIs. No intermediate strings.

## Performance

- **Small structs (like TrackInfo):** Negligible cost. Creates one JS object, sets a few properties. Microseconds.
- **Large `Vec<f32>`:** **Do NOT serialize audio buffers through serde.** Each element becomes a separate JS `Number` in an `Array`. Orders of magnitude slower than wasm-bindgen's native `Vec<f32>` -> `Float32Array` path.
- The project correctly returns audio data as `Vec<f32>` (native wasm-bindgen) and only uses serde for small metadata structs.

## Error Handling

`serde_wasm_bindgen::Error` implements `Display`, `std::error::Error`, and `Into<JsValue>`.

For simple structs with primitives, `to_value()` **will not fail** in practice. The `Result` exists because serde's `Serializer` trait requires it.

Idiomatic error conversion patterns:
```rust
// Current project style:
serde_wasm_bindgen::to_value(&info)
    .map_err(|e| JsValue::from_str(&format!("Serialize error: {e}")))

// Shorter alternative (Error implements Into<JsValue>):
serde_wasm_bindgen::to_value(&info).map_err(JsValue::from)
```

## Configuration: The Serializer Type

For advanced control:

```rust
use serde::Serialize;
use serde_wasm_bindgen::Serializer;

let serializer = Serializer::new()
    .serialize_maps_as_objects(true)    // HashMap -> plain Object (not Map)
    .serialize_missing_as_null(true)    // None -> null (not undefined)
    .serialize_large_number_types_as_bigints(false);  // i64/u64 -> number

let js_value = info.serialize(&serializer)?;

// Or use the convenience method:
let serializer = Serializer::json_compatible();
// = maps as objects + missing as null + no bigints
```

## Useful serde Attributes for WASM

```rust
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]  // sample_rate -> sampleRate
struct TrackInfo {
    sample_rate: u32,      // -> "sampleRate" in JS
    channels: usize,
    duration_secs: f64,    // -> "durationSecs" in JS

    #[serde(skip)]
    internal_field: u32,   // not serialized

    #[serde(default)]
    optional_field: u32,   // uses Default if missing in deserialization

    #[serde(skip_serializing_if = "Option::is_none")]
    maybe: Option<String>, // omitted entirely if None
}
```

**Consider adding `#[serde(rename_all = "camelCase")]`** to `TrackInfo` for JS-idiomatic field names.

## Gotchas

1. **`HashMap` -> JS `Map`, not `Object`.** `result.someKey` doesn't work on `Map`. Use structs (which become objects) or `serialize_maps_as_objects(true)`.

2. **`Option::None` -> `undefined`, not `null`.** JS `=== null` check misses `undefined`. Use `serialize_missing_as_null(true)` if needed.

3. **Large integers silently become `BigInt`.** `BigInt` can't mix with `Number` in JS arithmetic (`bigint + 1` throws). Not a concern for `u32` fields.

4. **Don't serialize audio data through serde.** `Vec<f32>` via serde = `Array` of `Number`. Use wasm-bindgen native `Vec<f32>` = `Float32Array`.

5. **Binary data.** `&[u8]` through serde = `Array` of numbers. Use `js_sys::Uint8Array` directly, or `#[serde(with = "serde_bytes")]` for `Uint8Array` output.

6. **`from_value()` consumes `JsValue`.** Clone first if you need the original.

7. **`serde_wasm_bindgen::Error` != `serde_json::Error`.** If both crates are present, be explicit about which error type.

---

## Learned Notes

<!-- Add notes here as you learn things about serde/serde-wasm-bindgen through usage, debugging, forum posts, etc. -->

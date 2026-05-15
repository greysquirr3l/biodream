# no\_std Usage

The parser and domain modules compile under `#![no_std]` with `alloc`. This
makes biodream usable in embedded environments, WASM, and other
resource-constrained targets.

## What's available in no\_std

| Module | no\_std | Notes |
|---|---|---|
| `biodream::domain` | ✅ | All typed domain types |
| `biodream::parser` | ✅ | Binary parser (binrw, alloc) |
| `biodream::export::csv` | ❌ | Requires `std` I/O |
| `biodream::export::arrow` | ❌ | Requires `std` |
| `biodream::export::parquet` | ❌ | Requires `std` |
| `biodream::export::hdf5` | ❌ | Requires `std` + system lib |
| `biodream::read_file` | ❌ | Requires `std` filesystem |
| `biodream::read_stream` | ✅ | Works with any `Read + Seek` |

## Cargo.toml setup for no\_std

```toml
[dependencies]
biodream = { version = "0.2", default-features = false }
```

This gives you the parser + domain types only, with no I/O. You supply the
bytes and the seek implementation.

## WASM example

```rust
#![no_std]
extern crate alloc;

use alloc::vec::Vec;
use biodream::read_stream;

// In a WASM target, bytes come from the JS side
pub fn parse_acq(bytes: Vec<u8>) -> Result<alloc::string::String, alloc::string::String> {
    let cursor = core2::io::Cursor::new(&bytes);
    let result = read_stream(cursor).map_err(|e| alloc::format!("{e}"))?;
    let df = result.into_value();
    Ok(alloc::format!(
        "{} channels, {} Hz, {} markers",
        df.channels.len(),
        df.metadata.samples_per_second,
        df.markers.len(),
    ))
}
```

> **Note:** In WASM you need a `Read + Seek` shim. The [`core2`](https://crates.io/crates/core2)
> crate provides `core2::io::Cursor` which works in `no_std + alloc`.

## Allocator requirement

The `alloc` crate is required. You must provide a global allocator in your
final binary or WASM module:

```rust
// Example: wee_alloc for WASM
#[global_allocator]
static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;
```

## Compile verification

To verify the no\_std build locally:

```sh
cargo build --target wasm32-unknown-unknown --no-default-features
```

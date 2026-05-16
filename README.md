# biodream

[![CI](https://github.com/greysquirr3l/biodream/actions/workflows/ci.yml/badge.svg)](https://github.com/greysquirr3l/biodream/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/biodream)](https://crates.io/crates/biodream)
[![docs.rs](https://docs.rs/biodream/badge.svg)](https://docs.rs/biodream)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)

Zero-copy, streaming-capable Rust toolkit for reading and writing BIOPAC
AcqKnowledge (.acq) files across all known format versions (v30 through v84+).

Replaces the Python [bioread](https://github.com/uwmadison-chm/bioread) library,
the C# AckReader, and the Windows-only ACKAPI DLL with a single cross-platform
library that handles compressed and uncompressed files, mixed sampling rates,
event markers, journals, and foreign data sections.

## Feature comparison

| Capability | biodream | bioread |
|---|---|---|
| Read uncompressed .acq | yes | yes |
| Read compressed .acq | yes | yes |
| Mixed sampling rates | yes | yes (fixed in 1.0.0) |
| Typed errors with byte offsets | yes | no (silent swallow) |
| Round-trip write | yes (T13) | no |
| no_std / WASM | yes (core parser) | no |
| CSV export | yes | yes |
| Apache Arrow IPC | yes | no |
| Parquet | yes | no |
| HDF5 | yes (opt-in) | yes |
| Format version | v30-v84+ | v30-v84+ |

## Installation

```toml
[dependencies]
biodream = "0.2.7"
```

With optional features:

```toml
[dependencies]
biodream = { version = "0.2.7", features = ["arrow", "parquet", "write"] }
```

## Feature flags

| Flag | Default | Description |
|------|---------|-------------|
| `read` | yes | Read .acq files. Requires `std`. |
| `csv` | yes | CSV export. |
| `write` | no | Write .acq files (round-trip). |
| `arrow` | no | Apache Arrow IPC export. |
| `parquet` | no | Parquet export (requires `arrow`). |
| `hdf5` | no | HDF5 export (requires libhdf5-dev). |
| `serde` | no | Serde derive for domain types. |

## Quick start

```rust
use biodream::parser::reader::read_file;

let result = read_file("path/to/recording.acq")?;

if !result.is_clean() {
    for w in &result.warnings {
        eprintln!("warning: {w}");
    }
}

let datafile = result.into_value();
println!("{}", datafile.summary());

for channel in &datafile.channels {
    let samples = channel.scaled_samples();
    println!("{}: {} samples at {} Hz", channel.name, samples.len(), channel.samples_per_second);
}
```

## no_std usage

The core parser (`domain` and `error` modules) compiles under `#![no_std]` with
`alloc`. Disable default features:

```toml
[dependencies]
biodream = { version = "0.2.7", default-features = false }
```

I/O adapters (`read`, `write`, export targets) require `std`.

## CLI

```sh
cargo install biodream

# Print recording summary
biopac info recording.acq

# Convert to CSV
biopac convert recording.acq --output recording.csv

# Low-level format inspection
biopac inspect recording.acq
```

## License

Licensed under either of:

- MIT license ([LICENSE-MIT](LICENSE-MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))

at your option.

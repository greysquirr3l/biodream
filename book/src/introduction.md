# biodream

[![CI](https://github.com/greysquirr3l/biodream/actions/workflows/ci.yml/badge.svg)](https://github.com/greysquirr3l/biodream/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/biodream.svg)](https://crates.io/crates/biodream)
[![docs.rs](https://docs.rs/biodream/badge.svg)](https://docs.rs/biodream)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)

**biodream** is a zero-copy, streaming-capable Rust toolkit for reading and
writing [BIOPAC AcqKnowledge](https://www.biopac.com/acqknowledge/) `.acq`
files across all known format versions (v30 through v84+).

It replaces the Python [bioread](https://github.com/uwmadison-chm/bioread)
library, the C# AckReader, and the Windows-only ACKAPI DLL with a single
cross-platform Rust crate that handles compressed and uncompressed files, mixed
sampling rates, event markers, journals, and foreign data sections.

## Why biodream?

| Capability | biodream | bioread |
|---|---|---|
| Read uncompressed .acq | ✅ | ✅ |
| Read compressed .acq | ✅ | ✅ |
| Mixed sampling rates | ✅ | ✅ (fixed in 1.0.0) |
| Typed errors with byte offsets | ✅ | ❌ (silent swallow) |
| Round-trip write | ✅ | ❌ |
| no\_std / WASM | ✅ (core parser) | ❌ |
| CSV export | ✅ | ✅ |
| Apache Arrow IPC | ✅ | ❌ |
| Parquet | ✅ | ❌ |
| HDF5 | ✅ (opt-in) | ✅ |
| Format versions | v30–v84+ | v30–v84+ |

## Design principles

- **Zero-copy streaming** — the chunked reader never buffers more than one
  interleave pattern; large recordings don't blow up memory.
- **Typed errors** — every `BiopacError` variant carries the byte offset and
  expected-vs-actual value so corrupt files can be triaged precisely.
- **Parse, don't validate** — untrusted input is parsed once at the adapter
  boundary into typed domain values; raw bytes never leak into application code.
- **Feature-gated footprint** — `default = ["read", "csv"]`; Arrow, Parquet,
  HDF5, write support, and serde are opt-in.
- **`no_std` core** — the parser and domain modules compile under `#![no_std]`
  with `alloc`; only I/O adapters and the CLI require `std`.

## Repository layout

```
src/
  lib.rs          public API surface
  domain/         typed value objects — no I/O
  parser/         binary layout knowledge (binrw)
  export/         CSV, Arrow, Parquet, HDF5
  cli/            biopac binary entry point
examples/         runnable examples
tests/            integration tests + 14 synthetic fixture files
book/             this documentation
docs/
  adr/            architectural decision records
  dev/            developer notes
```

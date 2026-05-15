# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2025-07-01

### Added

#### Parser & Core (T01–T06)

- Binary parser for BIOPAC AcqKnowledge `.acq` files across all known format
  versions (v30 through v84+) using declarative `binrw`-based header structs.
- Version-dispatched parsing: `FileRevision` determines which header layout is
  read; single code path handles all variants cleanly.
- Support for both uncompressed and zlib-compressed data payloads.
- Mixed sampling-rate support: each channel carries its own
  `samples_per_second` and `frequency_divider`, correctly computed from the
  global rate stored in the graph header.
- Event-marker parsing: `Marker`, `MarkerStyle`, and `Timestamp` domain types
  with full textual label support.
- Journal section parsing: raw journal text exposed as `Journal::as_text()`.
- Foreign-data section detection and graceful skip-forward with a `Warning`.
- `ParseResult<T>` wrapper that accumulates non-fatal `Warning`s alongside the
  value; callers iterate `result.warnings` before calling `result.into_value()`.

#### Domain Model (T02–T03)

- Rich domain types: `Datafile`, `GraphMetadata`, `Channel`, `ChannelData`,
  `Marker`, `MarkerStyle`, `Journal`, `Timestamp`, `FileRevision`, `ByteOrder`.
- `Channel::scaled_samples()` converts raw `i16` integers to `f64` via per-
  channel scale and offset; linear-interpolation upsampling for sub-rate channels.
- `ChannelData` enum: `Scaled { raw, scale, offset }` for the common case;
  `Raw(Vec<i16>)` for unprocessed access.
- Typed error hierarchy via `thiserror`: `BiopacError` with variants carrying
  byte offsets and expected-vs-actual values for triage of corrupt files.

#### Write Support (T07, feature `write`)

- Round-trip write support: `write_file` serialises a `Datafile` back to the
  BIOPAC binary format with bitwise fidelity on read-modify-write cycles.
- `WriteOptions` for controlling output behaviour (byte order, version).
- Feature-gated behind `write` to keep the default dependency footprint minimal.

#### Export (T08–T10)

- **CSV** (`default`): `to_csv` with `CsvOptions` (delimiter, time column,
  `TimeFormat` enum for elapsed seconds vs. sample index).
- **Arrow IPC** (feature `arrow`): `export::arrow::to_arrow_ipc` writes an
  Arrow IPC stream compatible with Polars, R `arrow`, and Julia.
- **Parquet** (feature `parquet`): `export::parquet::to_parquet` writes a
  Parquet file suitable for direct loading in DuckDB, Spark, or Pandas.
- **HDF5** (feature `hdf5`): `export::hdf5::to_hdf5` writes a hierarchical
  HDF5 dataset per channel.

#### CLI (T11)

- `biodream` binary with sub-commands: `info`, `csv`, `arrow`, `parquet`.
- `info`: human-readable summary of file metadata and channel list.
- `csv` / `arrow` / `parquet`: batch conversion with feature-gated availability.
- Colorised output via `owo-colors`; structured error reporting with `anyhow`.

#### Lazy / Streaming Reader (T12)

- `LazyDatafile` / `ReadOptions` for deferred channel loading: reads only the
  channel headers on open, then streams individual channels on demand without
  buffering the entire file.

#### Testing (T13–T14)

- 222-test suite covering: unit tests, integration tests against 14 synthetic
  fixture `.acq` binary files (v30–v84+ with and without compression), write
  round-trip tests, and property-based tests via `proptest`.
- Proptest strategies generate arbitrary valid `Datafile` structures and verify
  `write → read → write` produces bitwise-identical output.
- `cargo test --workspace --all-features` runs the full suite in CI.

#### Documentation & Publishing (T15)

- Full rustdoc coverage (`#![warn(missing_docs)]`); `cargo doc --all-features
  --no-deps` produces zero warnings.
- Four runnable examples: `read_file`, `convert_csv`, `arrow_export`,
  `write_file`.
- `README.md` with feature comparison table, installation instructions, feature
  flag reference, quick-start code, `no_std` usage notes, and CLI examples.
- `Cargo.toml` publish metadata: description, repository, license, keywords,
  and categories.

### Architecture

- `no_std`-compatible core (parser + domain) with `alloc`; `std` required only
  by I/O adapters and the CLI binary.
- Feature gates: `default = ["read", "csv"]`; optional: `write`, `arrow`,
  `parquet`, `hdf5`, `serde`.
- MSRV: Rust 1.95.0 (edition 2024, stable toolchain only).
- Full Clippy `-W pedantic / nursery / cargo / perf` profile with zero warnings.

[Unreleased]: https://github.com/greysquirr3l/biodream/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/greysquirr3l/biodream/releases/tag/v0.1.0

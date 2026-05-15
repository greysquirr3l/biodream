# Installation

## Library

Add biodream to your `Cargo.toml`:

```toml
[dependencies]
biodream = "0.2"
```

With optional features:

```toml
[dependencies]
biodream = { version = "0.2", features = ["arrow", "parquet", "write"] }
```

See [Feature Flags](./feature-flags.md) for the full list.

## CLI

Install the `biopac` binary:

```sh
cargo install biodream
```

Or build from source:

```sh
git clone https://github.com/greysquirr3l/biodream
cd biodream
cargo build --release --features "read,write,csv,arrow,parquet"
# binary is at target/release/biopac
```

## Pre-built binaries

Pre-built binaries for Linux x86-64, macOS ARM, macOS x86, and Windows x86-64
are attached to each [GitHub Release](https://github.com/greysquirr3l/biodream/releases).

## MSRV

Rust **1.95.0** stable. Edition 2024.

## HDF5 system dependency

The `hdf5` feature requires `libhdf5-dev` to be installed on the system:

```sh
# Ubuntu / Debian
sudo apt-get install libhdf5-dev

# macOS
brew install hdf5

# Windows: download from https://www.hdfgroup.org/downloads/hdf5/
```

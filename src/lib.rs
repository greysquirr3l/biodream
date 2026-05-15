//! biodream — zero-copy, streaming-capable toolkit for reading and writing
//! BIOPAC `AcqKnowledge` (.acq) files across all known format versions (v30–v84+).
//!
//! # Feature flags
//!
//! | Feature | Default | Description |
//! |---------|---------|-------------|
//! | `std`   | via `read` | Standard library support |
//! | `read`  | yes | Read .acq files from disk or streams |
//! | `write` | no  | Write .acq files (requires `read`) |
//! | `csv`   | yes | CSV export |
//! | `arrow` | no  | Apache Arrow IPC export |
//! | `parquet` | no | Parquet export (requires `arrow`) |
//! | `hdf5`  | no  | HDF5 export (requires libhdf5-dev) |
//! | `serde` | no  | Serde derive for domain types |
//!
//! # `no_std`
//!
//! The core library (`domain` + `error` modules) is `no_std` compatible with
//! `alloc`. I/O adapters and export targets require the `std` feature (enabled
//! transitively by `read`).
#![no_std]
extern crate alloc;

// Bring std into scope for features that require it.
#[cfg(feature = "std")]
extern crate std;

pub mod domain;
pub mod error;

/// Binary parser for .acq files. Requires the `read` feature for I/O.
pub mod parser;

#[cfg(feature = "write")]
pub mod writer;

#[cfg(any(
    feature = "csv",
    feature = "arrow",
    feature = "parquet",
    feature = "hdf5"
))]
pub mod export;

// Top-level re-exports for the most commonly used types.
pub use domain::{
    ByteOrder, Channel, ChannelData, ChannelMetadata, Datafile, FileRevision, GraphMetadata,
    Journal, Marker, MarkerStyle, Timestamp,
};
pub use error::{BiopacError, ParseResult, Warning};

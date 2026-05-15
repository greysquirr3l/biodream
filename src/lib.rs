//! biodream — zero-copy, streaming-capable toolkit for reading and writing
//! BIOPAC `AcqKnowledge` (.acq) files across all known format versions (v30–v84+).
//!
//! # Quick start
//!
//! ```rust,ignore
//! // Read a file in one call
//! let df = biodream::read_file("recording.acq")?.into_value();
//! println!("{}", df.summary());
//!
//! // Lazy: parse headers without loading sample data
//! let lazy = biodream::open_file("recording.acq")?;
//! println!("{} channels", lazy.channel_count());
//! let ecg = lazy.load_channel(0)?;
//!
//! // Filter to specific channels
//! let df = biodream::ReadOptions::new()
//!     .channels(&[0, 2])
//!     .scaled(true)
//!     .read_file("recording.acq")?
//!     .into_value();
//! ```
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
#![warn(missing_docs)]
extern crate alloc;

// Bring std into scope for features that require it.
#[cfg(feature = "std")]
extern crate std;

pub mod domain;
pub mod error;

/// Binary parser for .acq files. Requires the `read` feature for I/O.
pub mod parser;

/// High-level ergonomic API: `ReadOptions` and `LazyDatafile`.
#[cfg(feature = "read")]
pub mod api;

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

// Re-export the ergonomic API types at crate root.
#[cfg(feature = "read")]
pub use api::{LazyDatafile, ReadOptions};

// ---------------------------------------------------------------------------
// Top-level convenience functions
// ---------------------------------------------------------------------------

/// Read a `.acq` file from the filesystem.
///
/// Both compressed and uncompressed files are handled transparently.
/// Returns a [`ParseResult`] containing the [`Datafile`] and any non-fatal
/// [`Warning`]s.
///
/// For more control over which channels are loaded or whether samples are
/// scaled, use [`ReadOptions`].
#[cfg(feature = "read")]
pub fn read_file(path: impl AsRef<std::path::Path>) -> Result<ParseResult<Datafile>, BiopacError> {
    parser::reader::read_file(path)
}

/// Parse a `.acq` file from an in-memory byte slice.
///
/// Useful for WASM and embedded contexts where filesystem access is
/// unavailable.
#[cfg(feature = "read")]
pub fn read_bytes(bytes: &[u8]) -> Result<ParseResult<Datafile>, BiopacError> {
    parser::reader::read_bytes(bytes)
}

/// Read a `.acq` file from any [`std::io::Read`] + [`std::io::Seek`] source.
#[cfg(feature = "read")]
pub fn read_stream<R: std::io::Read + std::io::Seek>(
    reader: R,
) -> Result<ParseResult<Datafile>, BiopacError> {
    parser::reader::read_stream(reader)
}

/// Open a `.acq` file for lazy channel access.
///
/// Headers and markers are parsed immediately. Channel sample data is only
/// read when first requested via [`LazyDatafile::load_channel`].
///
/// See also: [`read_file`] for loading all data at once.
#[cfg(feature = "read")]
pub fn open_file(path: impl AsRef<std::path::Path>) -> Result<LazyDatafile, BiopacError> {
    api::open_file(path)
}

// ---------------------------------------------------------------------------
// CSV export convenience re-exports
// ---------------------------------------------------------------------------

/// Export a [`Datafile`] to CSV.
///
/// See [`export::csv::to_csv`] for full documentation.
#[cfg(feature = "csv")]
pub use export::csv::to_csv;

/// Options controlling CSV output (delimiter, precision, time format, …).
#[cfg(feature = "csv")]
pub use export::csv::CsvOptions;

/// Time-column format for CSV export.
#[cfg(feature = "csv")]
pub use export::csv::TimeFormat;

// ---------------------------------------------------------------------------
// Arrow / Parquet export convenience re-exports
// ---------------------------------------------------------------------------

/// Export a [`Datafile`] as an Arrow IPC stream.
///
/// See [`export::arrow::to_arrow_ipc`] for full documentation.
#[cfg(any(feature = "arrow", feature = "parquet"))]
pub use export::arrow::to_arrow_ipc;

/// Export a [`Datafile`] as a Parquet file.
///
/// See [`export::parquet::to_parquet`] for full documentation.
#[cfg(feature = "parquet")]
pub use export::parquet::to_parquet;

/// Options for Parquet export (compression level, …).
#[cfg(feature = "parquet")]
pub use export::parquet::ParquetOptions;

// ---------------------------------------------------------------------------
// Inspect API — parse headers only, no sample data loaded
// ---------------------------------------------------------------------------

/// Parse headers from a `.acq` file without loading sample data.
///
/// Returns an [`InspectReport`] with graph metadata, per-channel info,
/// foreign-data length, and the byte offset where sample data begins.
#[cfg(feature = "read")]
pub use api::inspect::inspect_file;

/// Parse headers from an in-memory `.acq` byte slice without loading samples.
#[cfg(feature = "read")]
pub use api::inspect::inspect_bytes;

/// Low-level diagnostic report returned by [`inspect_file`] and [`inspect_bytes`].
#[cfg(feature = "read")]
pub use api::inspect::InspectReport;

/// Per-channel header info in an [`InspectReport`].
#[cfg(feature = "read")]
pub use api::inspect::ChannelInspect;

// ---------------------------------------------------------------------------
// Write convenience re-exports
// ---------------------------------------------------------------------------

/// Write a [`Datafile`] to a `.acq` file using default options (uncompressed, Pre-4).
///
/// See [`writer::write_file`] and [`WriteOptions`] for full documentation.
#[cfg(feature = "write")]
pub use writer::write_file;

/// Write a [`Datafile`] to any [`std::io::Write`] sink using default options.
#[cfg(feature = "write")]
pub use writer::write_stream;

/// Options controlling how a [`Datafile`] is serialised to `.acq` format.
///
/// Use builder methods to customise compression, revision, and byte order, then
/// call [`WriteOptions::write_file`] or [`WriteOptions::write_stream`].
#[cfg(feature = "write")]
pub use writer::WriteOptions;

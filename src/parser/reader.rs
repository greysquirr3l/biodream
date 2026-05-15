//! High-level file-reading API (requires `read` feature = `std`).
//!
//! This module exposes `read_file` and `read_stream` — the public entry points
//! for reading .acq files from disk or arbitrary `Read + Seek` streams.

// TODO(T09): implement read_file, read_stream, and lazy channel loading.

use std::io::{Read, Seek};

use crate::domain::Datafile;
use crate::error::{BiopacError, ParseResult};

/// Read a `.acq` file from any `Read + Seek` source.
///
/// Returns a [`ParseResult`] that bundles the [`Datafile`] with any
/// non-fatal [`Warning`](crate::error::Warning)s encountered during parsing.
pub fn read_stream<R: Read + Seek>(
    _reader: R,
) -> Result<ParseResult<Datafile>, BiopacError> {
    // TODO(T09): wire up header parser → data reader → markers → journal.
    Err(BiopacError::Validation(
        "read_stream is not yet implemented (T09)".into(),
    ))
}

#[cfg(feature = "std")]
/// Read a `.acq` file from the filesystem by path.
pub fn read_file(
    path: impl AsRef<std::path::Path>,
) -> Result<ParseResult<Datafile>, BiopacError> {
    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);
    read_stream(reader)
}

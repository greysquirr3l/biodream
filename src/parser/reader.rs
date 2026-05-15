//! High-level file-reading API (requires `read` feature = `std`).
//!
//! This module exposes `read_file` and `read_stream` — the public entry points
//! for reading .acq files from disk or arbitrary `Read + Seek` streams.

use std::io::{Read, Seek};
use std::vec::Vec;

use super::headers::parse_headers;
use super::interleaved::read_interleaved;
use crate::domain::Datafile;
use crate::error::{BiopacError, ParseResult};

/// Read a `.acq` file from any `Read + Seek` source.
///
/// Returns a [`ParseResult`] that bundles the [`Datafile`] with any
/// non-fatal [`Warning`](crate::error::Warning)s encountered during parsing.
pub fn read_stream<R: Read + Seek>(mut reader: R) -> Result<ParseResult<Datafile>, BiopacError> {
    let headers = parse_headers(&mut reader)?;

    // TODO(T06): parse markers/journal.
    // TODO(T07): handle compressed files.
    let (channels, warnings) = read_interleaved(&mut reader, &headers)?;

    let metadata = headers.graph_metadata;
    let datafile = Datafile {
        metadata,
        channels,
        markers: Vec::new(),
        journal: None,
    };
    Ok(ParseResult {
        value: datafile,
        warnings,
    })
}

#[cfg(feature = "std")]
/// Read a `.acq` file from the filesystem by path.
pub fn read_file(path: impl AsRef<std::path::Path>) -> Result<ParseResult<Datafile>, BiopacError> {
    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);
    read_stream(reader)
}

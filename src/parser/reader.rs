//! High-level file-reading API (requires `read` feature = `std`).
//!
//! This module exposes `read_file`, `read_bytes`, and `read_stream` — the
//! public entry points for reading .acq files from disk, byte slices, or
//! arbitrary `Read + Seek` streams.
//!
//! # File layout
//!
//! Uncompressed: `[Interleaved data] → Markers → Journal`\
//! Compressed:   `Markers → Journal → [Compressed data blobs]`
//!
//! The two orderings require different parse sequences; both are handled here.

use std::io::{Read, Seek, SeekFrom};
use std::vec::Vec;

extern crate alloc;

use super::compressed::read_compressed;
use super::headers::parse_headers;
use super::interleaved::read_interleaved;
use super::markers::parse_markers_and_journal;
use crate::domain::Datafile;
use crate::error::{BiopacError, ParseResult, Warning};

/// Read a `.acq` file from any `Read + Seek` source.
///
/// Returns a [`ParseResult`] that bundles the [`Datafile`] with any
/// non-fatal [`Warning`]s encountered during parsing.
pub fn read_stream<R: Read + Seek>(mut reader: R) -> Result<ParseResult<Datafile>, BiopacError> {
    let headers = parse_headers(&mut reader)?;

    let display_orders: Vec<u16> = headers
        .channel_metadata
        .iter()
        .map(|m| m.display_order)
        .collect();
    let file_revision = headers.graph_metadata.file_revision.0;
    let compressed = headers.graph_metadata.compressed;

    // Accumulate non-header warnings; header warnings are merged at the end.
    let mut extra_warnings: Vec<Warning> = Vec::new();

    // File layout differs by compression:
    //   Uncompressed:  [Interleaved data] → Markers → Journal
    //   Compressed:    Markers → Journal → [Compressed data blobs]
    let (channels, markers, journal) = if compressed {
        // 1. Parse markers (they precede compressed data in the stream).
        let mj = parse_markers_and_journal(&mut reader, file_revision, &display_orders);
        let (markers, journal) = match mj {
            Ok(r) => {
                extra_warnings.extend(r.warnings);
                (r.markers, r.journal)
            }
            Err(e) => {
                extra_warnings.push(Warning::new(alloc::format!(
                    "Marker/journal section unreadable: {e}"
                )));
                (Vec::new(), None)
            }
        };
        // 2. Then decompress channel data.
        let (channels, ch_warnings) = read_compressed(&mut reader, &headers)?;
        extra_warnings.extend(ch_warnings);
        (channels, markers, journal)
    } else {
        // 1. Read interleaved sample data.
        let (channels, ch_warnings) = read_interleaved(&mut reader, &headers)?;
        extra_warnings.extend(ch_warnings);
        // 2. Seek to the start of the marker section.
        //    `read_interleaved` uses a large internal buffer that overshoots
        //    the data boundary; reseeking ensures the marker parser starts at
        //    the right byte regardless of how much the reader overread.
        if let Some(data_bytes) = headers.uncompressed_data_byte_count() {
            let marker_start = headers.data_start_offset.saturating_add(data_bytes);
            reader
                .seek(SeekFrom::Start(marker_start))
                .map_err(BiopacError::Io)?;
        }
        // 3. Then parse markers.
        let mj = parse_markers_and_journal(&mut reader, file_revision, &display_orders);
        let (markers, journal) = match mj {
            Ok(r) => {
                extra_warnings.extend(r.warnings);
                (r.markers, r.journal)
            }
            Err(e) => {
                extra_warnings.push(Warning::new(alloc::format!(
                    "Marker/journal section unreadable: {e}"
                )));
                (Vec::new(), None)
            }
        };
        (channels, markers, journal)
    };

    // `headers` is no longer borrowed here — move its fields.
    let metadata = headers.graph_metadata;
    let mut warnings = headers.warnings;
    warnings.extend(extra_warnings);

    let datafile = Datafile {
        metadata,
        channels,
        markers,
        journal,
    };
    Ok(ParseResult {
        value: datafile,
        warnings,
    })
}

/// Read a `.acq` file from the filesystem by path.
#[cfg(feature = "std")]
pub fn read_file(path: impl AsRef<std::path::Path>) -> Result<ParseResult<Datafile>, BiopacError> {
    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);
    read_stream(reader)
}

/// Parse a `.acq` file from an in-memory byte slice.
///
/// Useful for WASM and embedded contexts where filesystem access is unavailable.
#[cfg(feature = "std")]
pub fn read_bytes(bytes: &[u8]) -> Result<ParseResult<Datafile>, BiopacError> {
    read_stream(std::io::Cursor::new(bytes))
}

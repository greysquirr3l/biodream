//! `biopac inspect` low-level header diagnostics API.
//!
//! Parses headers without loading sample data, returning structured
//! diagnostic information for format debugging.

extern crate alloc;

use alloc::vec::Vec;

use std::fs::File;
use std::io::{BufReader, Cursor, Read, Seek};
use std::path::Path;

use crate::{
    domain::{ChannelMetadata, GraphMetadata},
    error::{BiopacError, Warning},
    parser::headers::{dtype::SampleType, parse_headers},
};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Low-level diagnostic report for a `.acq` file.
///
/// Returned by [`inspect_file`] and [`inspect_bytes`]. Contains parsed header
/// data without loading sample data.
#[derive(Debug, Clone)]
pub struct InspectReport {
    /// Graph-level metadata (revision, rate, channels, …).
    pub graph_metadata: GraphMetadata,
    /// Per-channel header info and sample type.
    pub channels: Vec<ChannelInspect>,
    /// Number of bytes in the foreign data section.
    pub foreign_data_len: usize,
    /// Byte offset where sample data begins in the file.
    pub data_start_offset: u64,
    /// Non-fatal warnings from header parsing.
    pub warnings: Vec<Warning>,
}

/// Per-channel header info for [`InspectReport`].
#[derive(Debug, Clone)]
pub struct ChannelInspect {
    /// Channel metadata from the file header.
    pub metadata: ChannelMetadata,
    /// Sample data type: `"I16"` (16-bit integer) or `"F64"` (64-bit float).
    pub dtype: &'static str,
}

// ---------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------

/// Parse headers from a `.acq` file without loading sample data.
///
/// Useful for format debugging and file inspection tools.
pub fn inspect_file(path: impl AsRef<Path>) -> Result<InspectReport, BiopacError> {
    let file = File::open(path.as_ref()).map_err(BiopacError::Io)?;
    let mut reader = BufReader::new(file);
    build_report(&mut reader)
}

/// Parse headers from an in-memory `.acq` byte slice without loading samples.
pub fn inspect_bytes(bytes: &[u8]) -> Result<InspectReport, BiopacError> {
    let mut cursor = Cursor::new(bytes);
    build_report(&mut cursor)
}

// ---------------------------------------------------------------------------
// Internal
// ---------------------------------------------------------------------------

fn build_report<R: Read + Seek>(reader: &mut R) -> Result<InspectReport, BiopacError> {
    let h = parse_headers(reader)?;

    let channels = h
        .channel_metadata
        .into_iter()
        .zip(h.sample_types)
        .map(|(metadata, st)| ChannelInspect {
            metadata,
            dtype: match st {
                SampleType::I16 => "I16",
                SampleType::F64 => "F64",
            },
        })
        .collect();

    Ok(InspectReport {
        graph_metadata: h.graph_metadata,
        channels,
        foreign_data_len: h.foreign_data.len(),
        data_start_offset: h.data_start_offset,
        warnings: h.warnings,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspect_bytes_returns_report() -> Result<(), BiopacError> {
        // Minimal Pre-4 ACQ blob (borrowed from api_integration test helper logic).
        let bytes = build_minimal_pre4(1);
        let report = inspect_bytes(&bytes)?;
        assert_eq!(report.graph_metadata.channel_count, 1);
        assert_eq!(report.channels.len(), 1);
        assert_eq!(report.channels.first().map(|c| c.dtype), Some("I16"));
        assert!(report.data_start_offset > 0);
        Ok(())
    }

    #[test]
    fn inspect_bytes_foreign_data_len_is_zero_for_minimal_file() -> Result<(), BiopacError> {
        let bytes = build_minimal_pre4(2);
        let report = inspect_bytes(&bytes)?;
        assert_eq!(report.foreign_data_len, 0);
        assert_eq!(report.channels.len(), 2);
        Ok(())
    }

    /// Build a minimal Pre-4 uncompressed ACQ blob with `n_channels` channels
    /// and no sample data (`sample_count` = 0).
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        reason = "test helper: channel count and header length are small known-bounded values"
    )]
    fn build_minimal_pre4(n_channels: usize) -> Vec<u8> {
        let chan_hdr_len: usize = 86;
        let mut buf: Vec<u8> = Vec::new();

        // Graph header (256 bytes)
        let mut gh = [0u8; 256];
        // offset 0-1: unused i16 = 0
        gh[2..6].copy_from_slice(&38i32.to_le_bytes());               // lVersion = 38 at offset 2
        gh[6..10].copy_from_slice(&256i32.to_le_bytes());             // lExtItemHeaderLen = 256 at offset 6
        gh[10..12].copy_from_slice(&(n_channels as i16).to_le_bytes()); // nChannels at offset 10
        // offsets 12-15: horiz/curr = 0
        gh[16..24].copy_from_slice(&1.0f64.to_le_bytes());            // dSampleTime = 1ms at offset 16
        gh[252..254].copy_from_slice(&(chan_hdr_len as i16).to_le_bytes()); // nExtItemHeaderLen
        buf.extend_from_slice(&gh);

        // Per-channel headers
        for i in 0..n_channels {
            let mut ch = [0u8; 86];
            ch[0..4].copy_from_slice(&(chan_hdr_len as i32).to_le_bytes()); // lChanHeaderLen
            ch[4..8].copy_from_slice(&0i32.to_le_bytes()); // lBufLength = 0
            ch[8..16].copy_from_slice(&1.0f64.to_le_bytes()); // dAmplScale
            ch[16..24].copy_from_slice(&0.0f64.to_le_bytes()); // dAmplOffset
            ch[24..26].copy_from_slice(&1i16.to_le_bytes()); // nVarSampleDivider
            let name = alloc::format!("CH{i}");
            let name_bytes = name.as_bytes();
            let len = name_bytes.len().min(39);
            if let (Some(dst), Some(src)) = (ch.get_mut(26..26 + len), name_bytes.get(..len)) {
                dst.copy_from_slice(src);
            }
            buf.extend_from_slice(&ch);
        }

        // Foreign data (4 bytes, length = 0)
        buf.extend_from_slice(&0i32.to_le_bytes());

        // Dtype headers (4 bytes each, nType = 2 = i16)
        for _ in 0..n_channels {
            buf.extend_from_slice(&4u16.to_le_bytes()); // nSize
            buf.extend_from_slice(&2u16.to_le_bytes()); // nType = i16
        }

        buf
    }
}

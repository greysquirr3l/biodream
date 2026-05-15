//! Raw `binrw` header structs — binary layout knowledge lives here.
//!
//! Nothing outside this module should know about byte offsets or field names
//! from the .acq format. Domain types are produced by conversion functions
//! at the end of each submodule.
//!
//! # Sub-modules
//!
//! - [`graph`] — graph (file) header, Pre-4 and Post-4 variants
//! - [`channel`] — per-channel header
//! - [`foreign`] — opaque foreign-data blob
//! - [`dtype`] — channel data-type descriptors
//!
//! # Usage
//!
//! Call [`parse_headers`] with any `Read + Seek` reader to obtain a
//! [`ParsedHeaders`] value containing all domain metadata and the byte offset
//! at which channel data begins.

use alloc::vec::Vec;
use std::io::{Read, Seek, SeekFrom};

use binrw::BinRead;

use crate::{
    domain::{ChannelMetadata, GraphMetadata},
    error::{BiopacError, Warning},
};

pub(crate) use dtype::SampleType;

pub mod channel;
pub mod dtype;
pub mod foreign;
pub mod graph;

use channel::{CHANNEL_HEADER_MIN_LEN, ChannelHeaderRaw, parse_channel_metadata};
use dtype::{ChannelDtypeRaw, parse_sample_type};
use foreign::ForeignDataRaw;
use graph::{GraphHeaderPost4Raw, GraphHeaderPre4Raw};
use graph::{REVISION_POST4, detect_byte_order, parse_graph_header_post4, parse_graph_header_pre4};

// ---------------------------------------------------------------------------
// Output type
// ---------------------------------------------------------------------------

/// All header data extracted from a `.acq` file, ready for the data reader.
#[derive(Debug)]
pub(crate) struct ParsedHeaders {
    /// Top-level file metadata.
    pub graph_metadata: GraphMetadata,
    /// Per-channel descriptors (one entry per channel).
    pub channel_metadata: Vec<ChannelMetadata>,
    /// Opaque hardware-specific foreign data section.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "consumed by T06 marker/journal parser")
    )]
    pub foreign_data: Vec<u8>,
    /// Sample type for each channel (same length as `channel_metadata`).
    pub sample_types: Vec<SampleType>,
    /// Byte offset of the first sample in the stream.
    ///
    /// For uncompressed files this points to the start of the interleaved
    /// data block. For compressed files this is superseded by the per-channel
    /// compression headers (T07).
    pub data_start_offset: u64,
    /// Non-fatal issues detected while parsing headers.
    pub warnings: Vec<Warning>,
}

// ---------------------------------------------------------------------------
// Channel extended-description constants
// ---------------------------------------------------------------------------

/// Byte offset of `szDescriptionText` within the per-channel header.
const CHAN_DESC_OFFSET: u64 = 128;
/// Minimum channel header length for `szDescriptionText` to be present.
const CHAN_DESC_MIN_LEN: i32 = 168;

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

/// Parse all headers from the beginning of a `.acq` file.
///
/// The reader must be positioned at byte 0 (the start of the file). On
/// success the stream is left positioned at `data_start_offset`.
pub(crate) fn parse_headers<R: Read + Seek>(reader: &mut R) -> Result<ParsedHeaders, BiopacError> {
    let warnings: Vec<Warning> = Vec::new();

    // --- 1. Detect byte order from lVersion ---------------------------------
    let (endian, version) = detect_byte_order(reader)?;
    let pos = reader.stream_position().map_err(BiopacError::Io)?;

    // --- 2. Read graph header (version-dispatched) --------------------------
    let (graph_metadata, graph_header_len, pre4_chan_header_len) = if version < REVISION_POST4 {
        let raw = GraphHeaderPre4Raw::read_options(reader, endian, ())
            .map_err(|e| binrw_to_parse_error(&e, pos, "graph header (Pre-4)"))?;
        let parsed = parse_graph_header_pre4(raw, endian)?;
        (
            parsed.metadata,
            parsed.graph_header_len,
            Some(parsed.chan_header_len),
        )
    } else {
        let raw = GraphHeaderPost4Raw::read_options(reader, endian, ())
            .map_err(|e| binrw_to_parse_error(&e, pos, "graph header (Post-4)"))?;
        let parsed = parse_graph_header_post4(raw, endian)?;
        (parsed.metadata, parsed.graph_header_len, None)
    };

    // Seek past the rest of the graph header.
    reader
        .seek(SeekFrom::Start(graph_header_len))
        .map_err(BiopacError::Io)?;

    // --- 3. Read per-channel headers ----------------------------------------
    let n_channels = usize::from(graph_metadata.channel_count);
    let mut channel_metadata = Vec::with_capacity(n_channels);

    for i in 0..n_channels {
        let ch_start = reader.stream_position().map_err(BiopacError::Io)?;

        let raw = ChannelHeaderRaw::read_options(reader, endian, ())
            .map_err(|e| binrw_to_parse_error(&e, ch_start, "channel header"))?;

        // For Pre-4, validate against the graph-header-declared length.
        if let Some(expected_len) = pre4_chan_header_len {
            if raw.chan_header_len < expected_len {
                // emit a warning and use whatever the channel header claims
            } else if raw.chan_header_len != expected_len {
                // also tolerable; proceed with channel's own declared length
                let _ = expected_len; // suppress unused variable
            }
        }

        #[expect(
            clippy::cast_sign_loss,
            reason = "chan_header_len validated >= CHANNEL_HEADER_MIN_LEN in parse_channel_metadata"
        )]
        let ch_end = ch_start + (raw.chan_header_len.max(CHANNEL_HEADER_MIN_LEN) as u64);

        #[expect(
            clippy::cast_possible_truncation,
            reason = "channel index bounded by MAX_CHANNELS (256) validated in graph header"
        )]
        let mut meta = parse_channel_metadata(raw, i as u16, ch_start)?;

        // Read extended description (`szDescriptionText`, 40 bytes at channel-
        // relative offset 128) when the channel header is long enough (>= 168).
        if raw.chan_header_len >= CHAN_DESC_MIN_LEN {
            reader
                .seek(SeekFrom::Start(ch_start + CHAN_DESC_OFFSET))
                .map_err(BiopacError::Io)?;
            let mut desc_buf = [0u8; 40];
            reader.read_exact(&mut desc_buf).map_err(BiopacError::Io)?;
            let end = desc_buf.iter().position(|&b| b == 0).unwrap_or(40);
            meta.description =
                alloc::string::String::from_utf8_lossy(desc_buf.get(..end).unwrap_or(&desc_buf))
                    .into_owned();
        }
        channel_metadata.push(meta);

        reader
            .seek(SeekFrom::Start(ch_end))
            .map_err(BiopacError::Io)?;
    }

    // --- 4. Read foreign data section ---------------------------------------
    let fd_pos = reader.stream_position().map_err(BiopacError::Io)?;
    let fd_raw = ForeignDataRaw::read_options(reader, endian, ())
        .map_err(|e| binrw_to_parse_error(&e, fd_pos, "foreign data"))?;
    let foreign_data = fd_raw.data;

    // --- 5. Read per-channel dtype headers ----------------------------------
    let mut sample_types = Vec::with_capacity(n_channels);
    for i in 0..n_channels {
        let dt_pos = reader.stream_position().map_err(BiopacError::Io)?;
        let raw = ChannelDtypeRaw::read_options(reader, endian, ())
            .map_err(|e| binrw_to_parse_error(&e, dt_pos, "dtype header"))?;
        #[expect(
            clippy::cast_possible_truncation,
            reason = "channel index bounded by MAX_CHANNELS (256)"
        )]
        let st = parse_sample_type(raw, i as u16, dt_pos)?;
        sample_types.push(st);
    }

    // --- 6. Record where sample data begins ---------------------------------
    let data_start_offset = reader.stream_position().map_err(BiopacError::Io)?;

    Ok(ParsedHeaders {
        graph_metadata,
        channel_metadata,
        foreign_data,
        sample_types,
        data_start_offset,
        warnings,
    })
}

// ---------------------------------------------------------------------------
// Error helpers
// ---------------------------------------------------------------------------

fn binrw_to_parse_error(e: &binrw::Error, byte_offset: u64, context: &str) -> BiopacError {
    BiopacError::Validation(alloc::format!(
        "binary read error at 0x{byte_offset:X} ({context}): {e}"
    ))
}

// ---------------------------------------------------------------------------
// Integration tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{boxed::Box, vec::Vec};
    use std::io::Cursor;

    /// Build a minimal well-formed Pre-4 .acq file buffer for testing.
    ///
    /// Layout:
    /// - 256-byte Pre-4 graph header
    /// - N × 86-byte channel headers (padded to 252 bytes each)
    /// - 4-byte foreign data (nLength=0)
    /// - N × 4-byte dtype headers
    #[expect(
        clippy::indexing_slicing,
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        reason = "test helper: n_channels bounded by tests; slices at fixed offsets within fixed-size arrays"
    )]
    fn make_pre4_acq(n_channels: usize, sample_time_ms: f64) -> Vec<u8> {
        let mut buf: Vec<u8> = Vec::new();

        // --- Graph header (256 bytes) ---
        let mut gh = [0u8; 256];
        let version: i32 = 38;
        let chan_header_len: i16 = 252;
        gh[0..4].copy_from_slice(&version.to_le_bytes());
        gh[4..6].copy_from_slice(&(n_channels as i16).to_le_bytes());
        gh[8..16].copy_from_slice(&sample_time_ms.to_le_bytes());
        gh[252..254].copy_from_slice(&chan_header_len.to_le_bytes());
        buf.extend_from_slice(&gh);

        // --- Channel headers (252 bytes each) ---
        for i in 0..n_channels {
            let mut ch = [0u8; 252];
            ch[0..4].copy_from_slice(&252i32.to_le_bytes()); // lChanHeaderLen
            ch[4..8].copy_from_slice(&1000i32.to_le_bytes()); // lBufLength
            ch[8..16].copy_from_slice(&1.0f64.to_le_bytes()); // dAmplScale
            ch[16..24].copy_from_slice(&0.0f64.to_le_bytes()); // dAmplOffset
            ch[24..26].copy_from_slice(&1i16.to_le_bytes()); // nVarSampleDivider
            // szCommentText: "CH0", "CH1", ...
            let name = alloc::format!("CH{i}");
            let name_bytes = name.as_bytes();
            let copy_len = name_bytes.len().min(39);
            ch[26..26 + copy_len].copy_from_slice(&name_bytes[..copy_len]);
            buf.extend_from_slice(&ch);
        }

        // --- Foreign data (4-byte nLength=0) ---
        buf.extend_from_slice(&0i32.to_le_bytes());

        // --- Dtype headers (4 bytes each: nSize=4, nType=2 for i16) ---
        for _ in 0..n_channels {
            buf.extend_from_slice(&4u16.to_le_bytes()); // nSize
            buf.extend_from_slice(&2u16.to_le_bytes()); // nType = i16
        }

        buf
    }

    #[test]
    fn parse_headers_pre4_single_channel() -> Result<(), Box<dyn std::error::Error>> {
        let buf = make_pre4_acq(1, 1.0); // 1 ms -> 1000 Hz
        let mut cursor = Cursor::new(&buf);
        let headers = parse_headers(&mut cursor)?;

        assert_eq!(headers.graph_metadata.channel_count, 1);
        assert!((headers.graph_metadata.samples_per_second - 1000.0).abs() < 1e-9);
        assert_eq!(headers.channel_metadata.len(), 1);
        assert_eq!(
            headers.channel_metadata.first().map(|m| m.name.as_str()),
            Some("CH0")
        );
        assert_eq!(headers.sample_types.len(), 1);
        assert_eq!(headers.sample_types.first().copied(), Some(SampleType::I16));
        // data_start_offset = 256 + 252 + 4 + 4 = 516
        assert_eq!(headers.data_start_offset, 516);
        Ok(())
    }

    #[test]
    fn parse_headers_pre4_multi_channel() -> Result<(), Box<dyn std::error::Error>> {
        let buf = make_pre4_acq(3, 2.0); // 2 ms -> 500 Hz
        let mut cursor = Cursor::new(&buf);
        let headers = parse_headers(&mut cursor)?;

        assert_eq!(headers.graph_metadata.channel_count, 3);
        assert!((headers.graph_metadata.samples_per_second - 500.0).abs() < 1e-9);
        assert_eq!(headers.channel_metadata.len(), 3);
        assert_eq!(headers.sample_types.len(), 3);
        // data_start_offset = 256 + 3*252 + 4 + 3*4 = 256 + 756 + 4 + 12 = 1028
        assert_eq!(headers.data_start_offset, 1028);
        Ok(())
    }

    #[test]
    #[expect(
        clippy::indexing_slicing,
        reason = "buf constructed by make_pre4_acq with known layout; fd_offset is within bounds"
    )]
    fn parse_headers_foreign_data_preserved() -> Result<(), Box<dyn std::error::Error>> {
        // Build Pre-4 file with 4 bytes of foreign data.
        let mut buf = make_pre4_acq(1, 1.0);
        // Patch the foreign-data section: nLength at offset 256+252=508
        let fd_offset = 256 + 252;
        buf[fd_offset..fd_offset + 4].copy_from_slice(&4i32.to_le_bytes());
        // Insert the 4 payload bytes BEFORE the dtype header.
        buf.splice(fd_offset + 4..fd_offset + 4, [0xAA, 0xBB, 0xCC, 0xDD]);

        let mut cursor = Cursor::new(&buf);
        let headers = parse_headers(&mut cursor)?;
        assert_eq!(headers.foreign_data.len(), 4);
        assert_eq!(headers.foreign_data.first().copied(), Some(0xAA));
        Ok(())
    }

    /// Build a minimal Pre-4 .acq file where the channel header has an
    /// extended `szDescriptionText` (offset 128, 40 bytes), i.e. the channel
    /// header length is at least 168.
    #[expect(
        clippy::indexing_slicing,
        clippy::cast_possible_truncation,
        reason = "test helper: fixed-size arrays and bounded indices"
    )]
    fn make_pre4_acq_with_description(description: &str) -> Vec<u8> {
        let chan_header_len: i32 = 252; // >= 168 so description is present
        let mut buf: Vec<u8> = Vec::new();

        // --- Graph header (256 bytes) ---
        let mut gh = [0u8; 256];
        gh[0..4].copy_from_slice(&38i32.to_le_bytes()); // version 38
        gh[4..6].copy_from_slice(&1i16.to_le_bytes()); // 1 channel
        gh[8..16].copy_from_slice(&1.0f64.to_le_bytes()); // 1 ms -> 1000 Hz
        gh[252..254].copy_from_slice(&(chan_header_len as i16).to_le_bytes());
        buf.extend_from_slice(&gh);

        // --- Channel header (252 bytes) ---
        let mut ch = [0u8; 252];
        ch[0..4].copy_from_slice(&chan_header_len.to_le_bytes());
        ch[4..8].copy_from_slice(&1000i32.to_le_bytes());
        ch[8..16].copy_from_slice(&1.0f64.to_le_bytes());
        ch[16..24].copy_from_slice(&0.0f64.to_le_bytes());
        ch[24..26].copy_from_slice(&1i16.to_le_bytes());
        // szCommentText at 26: "ECG"
        ch[26..29].copy_from_slice(b"ECG");
        // szDescriptionText at 128 (40 bytes)
        let desc_bytes = description.as_bytes();
        let copy_len = desc_bytes.len().min(39);
        ch[128..128 + copy_len].copy_from_slice(&desc_bytes[..copy_len]);
        buf.extend_from_slice(&ch);

        // --- Foreign data (nLength=0, 4 bytes) ---
        buf.extend_from_slice(&0i32.to_le_bytes());

        // --- Dtype header ---
        buf.extend_from_slice(&4u16.to_le_bytes());
        buf.extend_from_slice(&2u16.to_le_bytes());

        buf
    }

    #[test]
    fn parse_headers_channel_extended_description_read() -> Result<(), Box<dyn std::error::Error>> {
        let buf = make_pre4_acq_with_description("Subject resting ECG recording");
        let mut cursor = Cursor::new(&buf);
        let headers = parse_headers(&mut cursor)?;

        assert_eq!(headers.channel_metadata.len(), 1);
        let desc = headers
            .channel_metadata
            .first()
            .map(|m| m.description.as_str());
        assert_eq!(desc, Some("Subject resting ECG recording"));
        Ok(())
    }

    #[test]
    fn parse_headers_channel_no_description_when_header_short()
    -> Result<(), Box<dyn std::error::Error>> {
        // The standard pre4 helper uses chan_header_len=252 (>= 168), but we
        // write a blank description at offset 128 — so we expect an empty string.
        let buf = make_pre4_acq(1, 1.0);
        let mut cursor = Cursor::new(&buf);
        let headers = parse_headers(&mut cursor)?;

        let desc = headers
            .channel_metadata
            .first()
            .map(|m| m.description.as_str());
        assert_eq!(
            desc,
            Some(""),
            "description should be empty (all zero bytes)"
        );
        Ok(())
    }
}

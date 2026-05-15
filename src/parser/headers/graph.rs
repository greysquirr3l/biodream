//! Graph (file) header structs — Pre-4 and Post-4 variants.
//!
//! # Format reference
//!
//! - Pre-4 (revision < 68, `AcqKnowledge` < 4.0): fixed 256-byte header.
//! - Post-4 (revision >= 68, `AcqKnowledge` >= 4.0): variable-length header;
//!   `lExtItemHeaderLen` (offset 6) stores the total header size in bytes.
//!
//! Field names follow the BIOPAC App Note 156 convention (`lVersion`,
//! `nChannels`, `dSampleTime`, …). All offsets are from the start of the file.
//!
//! # Byte-order detection
//!
//! Byte order is detected from `lVersion`: if the little-endian interpretation
//! yields a value in `[MIN_REVISION, MAX_REVISION]`, the file is LE; otherwise
//! try BE. If neither interpretation is in range, return
//! [`BiopacError::UnsupportedVersion`].

use std::io::{Read, Seek, SeekFrom};

use binrw::{Endian, binrw};

use crate::{
    domain::{ByteOrder, FileRevision, GraphMetadata},
    error::{BiopacError, HeaderSection, ParseError, UnsupportedVersionError},
};

/// First revision that uses the Post-4 variable-length header.
pub(super) const REVISION_POST4: i32 = 68;

/// Inclusive minimum revision this parser accepts.
const REVISION_MIN: i32 = 30;
/// Inclusive maximum revision this parser accepts.
const REVISION_MAX: i32 = 200;
/// Maximum number of channels considered valid.
const MAX_CHANNELS: i16 = 256;

/// Minimum byte length at which the `bCompressed` flag is present in a
/// Post-4 graph header (offset 1936, `AcqKnowledge` >= 3.8.1).
const COMPRESSED_FLAG_MIN_LEN: i32 = 1937;
/// Byte offset of `bCompressed` within the Post-4 graph header.
const COMPRESSED_FLAG_OFFSET: u64 = 1936;

// ---------------------------------------------------------------------------
// Byte-order detection
// ---------------------------------------------------------------------------

/// Read the first 4 bytes and infer the file's byte order from `lVersion`.
///
/// Returns `(endian, revision)`. The stream is rewound to position 0 before
/// returning so that the caller can re-read the full header.
pub(super) fn detect_byte_order<R: Read + Seek>(
    reader: &mut R,
) -> Result<(Endian, i32), BiopacError> {
    let mut buf = [0u8; 4];
    reader.read_exact(&mut buf).map_err(BiopacError::Io)?;
    reader.seek(SeekFrom::Start(0)).map_err(BiopacError::Io)?;

    let le = i32::from_le_bytes(buf);
    if (REVISION_MIN..=REVISION_MAX).contains(&le) {
        return Ok((Endian::Little, le));
    }

    let be = i32::from_be_bytes(buf);
    if (REVISION_MIN..=REVISION_MAX).contains(&be) {
        return Ok((Endian::Big, be));
    }

    Err(BiopacError::UnsupportedVersion(UnsupportedVersionError {
        revision: le,
        min_supported: REVISION_MIN,
        max_supported: REVISION_MAX,
    }))
}

// ---------------------------------------------------------------------------
// Pre-4 graph header (fixed 256 bytes)
// ---------------------------------------------------------------------------

/// Fixed-layout 256-byte graph header for Pre-4 files (revision < 68).
///
/// Field layout (App Note 156):
/// | Offset | Type | Field              |
/// |-------:|------|--------------------|  
/// |      0 | i32  | `lVersion`         |
/// |      4 | i16  | `nChannels`        |
/// |      6 | i16  | `nPreampTypes` (skipped) |
/// |      8 | f64  | `dSampleTime` (ms) |
/// |  16-251| ---  | (skipped)          |
/// |    252 | i16  | `nExtItemHeaderLen`|
/// |    254 | ---  | (2-byte pad)       |
/// Total: 4+2+2+8+236+2+2 = 256 bytes.
#[binrw]
#[derive(Debug, Copy, Clone)]
pub(super) struct GraphHeaderPre4Raw {
    /// `lVersion`: file format revision.
    pub version: i32,
    /// `nChannels`: number of channel headers that follow.
    pub channels: i16,
    /// Skip `nPreampTypes` (2 bytes at offset 6).
    #[br(pad_before = 2)]
    /// `dSampleTime`: sample period in milliseconds.
    pub sample_time_ms: f64,
    /// Skip offsets 16–251 (236 bytes).
    #[br(pad_before = 236, pad_after = 2)]
    /// `nExtItemHeaderLen`: byte length of each per-channel header.
    pub chan_header_len: i16,
}

/// Parse output for a Pre-4 graph header.
pub(super) struct Pre4Parsed {
    /// Domain metadata extracted from the header.
    pub metadata: GraphMetadata,
    /// Total byte length of the graph header (always 256 for Pre-4).
    pub graph_header_len: u64,
    /// Byte length of each per-channel header (`nExtItemHeaderLen`).
    pub chan_header_len: i32,
}

/// Convert a raw Pre-4 header + detected endian into parsed output.
pub(super) fn parse_graph_header_pre4(
    raw: GraphHeaderPre4Raw,
    endian: Endian,
) -> Result<Pre4Parsed, BiopacError> {
    validate_channels(raw.channels, 0)?;
    validate_sample_time(raw.sample_time_ms, 0)?;

    let metadata = GraphMetadata {
        file_revision: FileRevision::new(raw.version),
        samples_per_second: 1000.0 / raw.sample_time_ms,
        channel_count: u16::try_from(raw.channels).unwrap_or(0),
        byte_order: endian_to_byte_order(endian),
        compressed: false, // Pre-4 files are never compressed
    };

    Ok(Pre4Parsed {
        metadata,
        graph_header_len: 256,
        chan_header_len: i32::from(raw.chan_header_len),
    })
}

// ---------------------------------------------------------------------------
// Post-4 graph header (variable length)
// ---------------------------------------------------------------------------

/// Variable-length graph header for Post-4 files (revision >= 68).
///
/// Field layout:
/// | Offset | Type  | Field                    |
/// |-------:|-------|--------------------------|
/// |      0 | i32   | `lVersion`               |
/// |      4 | i16   | `nChannels`              |
/// |      6 | i32   | `lExtItemHeaderLen`      |
/// |     10 | i16   | `lNumItems` (skipped)    |
/// |     12 | f64   | `dSampleTime` (ms)       |
/// |   1936 | u8    | `bCompressed` (optional) |
///
/// After this struct is read the caller **must** seek to `graph_header_len`
/// (the value of `lExtItemHeaderLen`) to land at the first channel header.
#[binrw]
#[derive(Debug, Copy, Clone)]
pub(super) struct GraphHeaderPost4Raw {
    /// `lVersion`: file format revision.
    pub version: i32,
    /// `nChannels`: number of channel headers that follow.
    pub channels: i16,
    /// `lExtItemHeaderLen`: total byte length of this graph header.
    pub graph_header_len: i32,
    /// Skip `lNumItems` (2 bytes at offset 10).
    #[br(pad_before = 2)]
    /// `dSampleTime`: sample period in milliseconds.
    pub sample_time_ms: f64,
    // Cursor is now at offset 20.  Jump to the compression flag if present.
    /// `bCompressed`: non-zero when channel data is zlib-compressed.
    ///
    /// Only present when `graph_header_len >= 1937`.
    #[br(
        if(graph_header_len >= COMPRESSED_FLAG_MIN_LEN),
        seek_before = SeekFrom::Start(COMPRESSED_FLAG_OFFSET)
    )]
    pub compressed: Option<u8>,
}

/// Parse output for a Post-4 graph header.
pub(super) struct Post4Parsed {
    /// Domain metadata extracted from the header.
    pub metadata: GraphMetadata,
    /// Total byte length of the graph header (`lExtItemHeaderLen`).
    pub graph_header_len: u64,
}

/// Convert a raw Post-4 header + detected endian into parsed output.
pub(super) fn parse_graph_header_post4(
    raw: GraphHeaderPost4Raw,
    endian: Endian,
) -> Result<Post4Parsed, BiopacError> {
    validate_channels(raw.channels, 4)?;
    validate_sample_time(raw.sample_time_ms, 12)?;

    if raw.graph_header_len < 20 {
        return Err(BiopacError::Parse(ParseError {
            byte_offset: 6,
            expected: alloc::string::String::from("lExtItemHeaderLen >= 20"),
            actual: alloc::format!("{}", raw.graph_header_len),
            section: HeaderSection::Graph,
        }));
    }

    let compressed = raw.compressed.is_some_and(|b| b != 0);
    #[expect(
        clippy::cast_sign_loss,
        reason = "validated graph_header_len >= 20 above"
    )]
    let graph_header_len = raw.graph_header_len as u64;

    let metadata = GraphMetadata {
        file_revision: FileRevision::new(raw.version),
        samples_per_second: 1000.0 / raw.sample_time_ms,
        channel_count: u16::try_from(raw.channels).unwrap_or(0),
        byte_order: endian_to_byte_order(endian),
        compressed,
    };

    Ok(Post4Parsed {
        metadata,
        graph_header_len,
    })
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

const fn endian_to_byte_order(endian: Endian) -> ByteOrder {
    match endian {
        Endian::Little => ByteOrder::LittleEndian,
        Endian::Big => ByteOrder::BigEndian,
    }
}

fn validate_channels(channels: i16, byte_offset: u64) -> Result<(), BiopacError> {
    if !(1..=MAX_CHANNELS).contains(&channels) {
        return Err(BiopacError::Parse(ParseError {
            byte_offset,
            expected: alloc::format!("1..={MAX_CHANNELS}"),
            actual: alloc::format!("{channels}"),
            section: HeaderSection::Graph,
        }));
    }
    Ok(())
}

fn validate_sample_time(sample_time_ms: f64, byte_offset: u64) -> Result<(), BiopacError> {
    if sample_time_ms <= 0.0 {
        return Err(BiopacError::Parse(ParseError {
            byte_offset,
            expected: alloc::string::String::from("dSampleTime > 0.0"),
            actual: alloc::format!("{sample_time_ms}"),
            section: HeaderSection::Graph,
        }));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{boxed::Box, vec, vec::Vec};
    use binrw::BinRead;
    use std::io::Cursor;

    // ----- Byte-order detection -----------------------------------------

    #[test]
    fn detect_little_endian_revision_38() -> Result<(), Box<dyn std::error::Error>> {
        // lVersion = 38 as LE i32 = [38, 0, 0, 0] + 252 padding bytes
        let mut bytes = [0u8; 256];
        bytes[0..4].copy_from_slice(&38i32.to_le_bytes());
        let mut cursor = Cursor::new(&bytes[..]);
        let (endian, version) = detect_byte_order(&mut cursor)?;
        assert_eq!(endian, Endian::Little);
        assert_eq!(version, 38);
        // Stream must be rewound to 0.
        assert_eq!(cursor.position(), 0);
        Ok(())
    }

    #[test]
    fn detect_big_endian_revision_68() -> Result<(), Box<dyn std::error::Error>> {
        // lVersion = 68 as BE i32 = [0, 0, 0, 68]
        let mut bytes = [0u8; 256];
        bytes[0..4].copy_from_slice(&68i32.to_be_bytes());
        // Make sure LE interpretation is out of range so BE wins.
        // [0, 0, 0, 68] as LE i32 = 0x44000000 = 1140850688, out of [30, 200].
        let mut cursor = Cursor::new(&bytes[..]);
        let (endian, version) = detect_byte_order(&mut cursor)?;
        assert_eq!(endian, Endian::Big);
        assert_eq!(version, 68);
        Ok(())
    }

    #[test]
    fn detect_invalid_version_returns_error() {
        // Neither LE nor BE interpretation is in [30, 200].
        let bytes: [u8; 4] = [0x00, 0x00, 0x00, 0x00]; // both = 0
        let mut cursor = Cursor::new(&bytes[..]);
        let result = detect_byte_order(&mut cursor);
        assert!(result.is_err());
    }

    // ----- Pre-4 graph header -------------------------------------------

    /// Build a minimal 256-byte Pre-4 graph header.
    fn pre4_bytes(
        version: i32,
        channels: i16,
        sample_time_ms: f64,
        chan_header_len: i16,
    ) -> [u8; 256] {
        let mut b = [0u8; 256];
        b[0..4].copy_from_slice(&version.to_le_bytes());
        b[4..6].copy_from_slice(&channels.to_le_bytes());
        // nPreampTypes at 6 — leave as 0
        b[8..16].copy_from_slice(&sample_time_ms.to_le_bytes());
        // offsets 16–251 — leave as 0
        b[252..254].copy_from_slice(&chan_header_len.to_le_bytes());
        // 254–255 pad — 0
        b
    }

    #[test]
    fn pre4_parses_revision_38() -> Result<(), Box<dyn std::error::Error>> {
        // 1.0 ms per sample => 1000 Hz
        let bytes = pre4_bytes(38, 3, 1.0, 252);
        let mut cursor = Cursor::new(&bytes[..]);
        let raw = GraphHeaderPre4Raw::read_le(&mut cursor)?;
        let parsed = parse_graph_header_pre4(raw, Endian::Little)?;

        assert_eq!(parsed.metadata.file_revision, FileRevision::new(38));
        assert_eq!(parsed.metadata.channel_count, 3);
        assert!(
            (parsed.metadata.samples_per_second - 1000.0).abs() < 1e-9,
            "expected 1000 Hz, got {}",
            parsed.metadata.samples_per_second
        );
        assert_eq!(parsed.metadata.byte_order, ByteOrder::LittleEndian);
        assert!(!parsed.metadata.compressed);
        assert_eq!(parsed.graph_header_len, 256);
        assert_eq!(parsed.chan_header_len, 252);
        Ok(())
    }

    #[test]
    fn pre4_zero_channels_returns_error() -> Result<(), Box<dyn std::error::Error>> {
        let bytes = pre4_bytes(38, 0, 1.0, 252);
        let mut cursor = Cursor::new(&bytes[..]);
        let raw = GraphHeaderPre4Raw::read_le(&mut cursor)?;
        let result = parse_graph_header_pre4(raw, Endian::Little);
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn pre4_too_many_channels_returns_error() -> Result<(), Box<dyn std::error::Error>> {
        let bytes = pre4_bytes(38, 300, 1.0, 252); // 300 > MAX_CHANNELS (256)
        let mut cursor = Cursor::new(&bytes[..]);
        let raw = GraphHeaderPre4Raw::read_le(&mut cursor)?;
        let result = parse_graph_header_pre4(raw, Endian::Little);
        assert!(result.is_err());
        if let Err(e) = result {
            let msg = alloc::format!("{e}");
            assert!(msg.contains("Graph"), "should name Graph section: {msg}");
        }
        Ok(())
    }

    // ----- Post-4 graph header ------------------------------------------

    /// Build a minimal 40-byte Post-4 header (no compressed flag).
    #[expect(
        clippy::indexing_slicing,
        clippy::cast_sign_loss,
        reason = "test helper: slices at fixed offsets within buffer of size header_len.max(40) >= 20"
    )]
    fn post4_bytes_short(
        version: i32,
        channels: i16,
        header_len: i32,
        sample_time_ms: f64,
    ) -> Vec<u8> {
        let mut b = vec![0u8; header_len.max(40) as usize];
        b[0..4].copy_from_slice(&version.to_le_bytes());
        b[4..6].copy_from_slice(&channels.to_le_bytes());
        b[6..10].copy_from_slice(&header_len.to_le_bytes());
        // lNumItems at 10 — leave as 0
        b[12..20].copy_from_slice(&sample_time_ms.to_le_bytes());
        b
    }

    #[test]
    fn post4_parses_revision_68() -> Result<(), Box<dyn std::error::Error>> {
        // 2.0 ms per sample => 500 Hz; header_len = 40 (no compressed flag)
        let bytes = post4_bytes_short(68, 2, 40, 2.0);
        let mut cursor = Cursor::new(&bytes);
        let raw = GraphHeaderPost4Raw::read_le(&mut cursor)?;
        let parsed = parse_graph_header_post4(raw, Endian::Little)?;

        assert_eq!(parsed.metadata.file_revision, FileRevision::new(68));
        assert_eq!(parsed.metadata.channel_count, 2);
        assert!(
            (parsed.metadata.samples_per_second - 500.0).abs() < 1e-9,
            "expected 500 Hz, got {}",
            parsed.metadata.samples_per_second
        );
        assert!(!parsed.metadata.compressed); // short header -> no compressed flag -> false
        assert_eq!(parsed.graph_header_len, 40);
        Ok(())
    }

    #[test]
    #[expect(
        clippy::indexing_slicing,
        clippy::cast_sign_loss,
        reason = "test: slices at known offsets within 1940-byte buffer"
    )]
    fn post4_reads_compressed_flag() -> Result<(), Box<dyn std::error::Error>> {
        // Build a 1940-byte header with bCompressed = 1 at offset 1936.
        let header_len: i32 = 1940;
        let mut bytes = vec![0u8; header_len as usize];
        bytes[0..4].copy_from_slice(&77i32.to_le_bytes()); // revision 77
        bytes[4..6].copy_from_slice(&1i16.to_le_bytes()); // 1 channel
        bytes[6..10].copy_from_slice(&header_len.to_le_bytes());
        // lNumItems at 10 leave 0
        bytes[12..20].copy_from_slice(&1.0f64.to_le_bytes()); // 1 ms -> 1000 Hz
        bytes[1936] = 1; // bCompressed = true

        let mut cursor = Cursor::new(&bytes);
        let raw = GraphHeaderPost4Raw::read_le(&mut cursor)?;
        assert_eq!(raw.compressed, Some(1));

        let parsed = parse_graph_header_post4(raw, Endian::Little)?;
        assert!(parsed.metadata.compressed);
        assert_eq!(parsed.metadata.file_revision, FileRevision::new(77));
        assert_eq!(parsed.graph_header_len, 1940);
        Ok(())
    }

    #[test]
    fn post4_no_compressed_flag_for_short_header() -> Result<(), Box<dyn std::error::Error>> {
        let bytes = post4_bytes_short(68, 1, 100, 1.0); // header_len = 100 < 1937
        let mut cursor = Cursor::new(&bytes);
        let raw = GraphHeaderPost4Raw::read_le(&mut cursor)?;
        assert_eq!(raw.compressed, None);
        let parsed = parse_graph_header_post4(raw, Endian::Little)?;
        assert!(!parsed.metadata.compressed);
        Ok(())
    }
}

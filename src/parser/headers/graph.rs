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
    domain::{AcquisitionDateTime, ByteOrder, FileRevision, GraphMetadata},
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

/// Byte offset of the graph title (`szGraphTitle`) within the Post-4 header.
const GRAPH_TITLE_OFFSET: u64 = 236;
/// Minimum header length to contain the title field (236 + 40 = 276).
const GRAPH_HDR_TITLE_MIN_LEN: i32 = 276;

/// Byte offset of the first date/time field (`lSec`) within the Post-4 header.
const GRAPH_DATETIME_OFFSET: u64 = 276;
/// Minimum header length to contain all six date/time fields (276 + 24 = 300).
const GRAPH_HDR_DATETIME_MIN_LEN: i32 = 300;

/// Byte offset of `lMaxAcqSamplesPerSec` (`AcqKnowledge` ≥ 4.2, revision ≥ 74).
const GRAPH_MAX_RATE_OFFSET: u64 = 1940;
/// Minimum header length to contain the max-rate field (1940 + 4 = 1944).
const GRAPH_HDR_MAX_RATE_MIN_LEN: i32 = 1944;

// ---------------------------------------------------------------------------
// Byte-order detection
// ---------------------------------------------------------------------------

/// Read the first 6 bytes and infer the file's byte order from `lVersion`.
///
/// The BIOPAC format begins with a 2-byte unused field (`nItemHeaderLen`) at
/// offset 0, followed by `lVersion` as a 4-byte integer at offset 2. This
/// function skips the prefix and checks the version bytes in both LE and BE
/// order.
///
/// Returns `(endian, revision)`. The stream is rewound to position 0 before
/// returning so that the caller can re-read the full header.
pub(super) fn detect_byte_order<R: Read + Seek>(
    reader: &mut R,
) -> Result<(Endian, i32), BiopacError> {
    let mut buf = [0u8; 6];
    reader.read_exact(&mut buf).map_err(BiopacError::Io)?;
    reader.seek(SeekFrom::Start(0)).map_err(BiopacError::Io)?;

    // buf[0..2] = unused int16 (nItemHeaderLen) — discarded
    // buf[2..6] = lVersion as int32
    let ver_bytes = [buf[2], buf[3], buf[4], buf[5]];

    let le = i32::from_le_bytes(ver_bytes);
    if (REVISION_MIN..=REVISION_MAX).contains(&le) {
        return Ok((Endian::Little, le));
    }

    let be = i32::from_be_bytes(ver_bytes);
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
/// Field layout (BIOPAC format; offsets from start of file):
/// | Offset | Type | Field                   |
/// |-------:|------|-------------------------|
/// |      0 | i16  | unused (`nItemHeaderLen`)|
/// |      2 | i32  | `lVersion`              |
/// |      6 | i32  | `lExtItemHeaderLen` (=256, skipped) |
/// |     10 | i16  | `nChannels`             |
/// |     12 | i16  | `nHorizAxisType` (skipped) |
/// |     14 | i16  | `nCurrChannel` (skipped)|
/// |     16 | f64  | `dSampleTime` (ms)      |
/// |  24-251| ---  | (skipped)               |
/// |    252 | i16  | `nExtItemHeaderLen` (per-channel) |
/// |    254 | ---  | (2-byte pad)            |
/// Total: 2+4+4+2+4+8+228+2+2 = 256 bytes.
#[binrw]
#[derive(Debug, Copy, Clone)]
pub(super) struct GraphHeaderPre4Raw {
    /// Skip 2-byte unused prefix (`nItemHeaderLen`) at offset 0.
    #[br(pad_before = 2)]
    /// `lVersion`: file format revision (at offset 2).
    pub version: i32,
    /// Skip `lExtItemHeaderLen` (4 bytes at offset 6; always 256 for Pre-4).
    #[br(pad_before = 4)]
    /// `nChannels`: number of channel headers that follow (at offset 10).
    pub channels: i16,
    /// Skip `nHorizAxisType` and `nCurrChannel` (4 bytes at offsets 12–15).
    #[br(pad_before = 4)]
    /// `dSampleTime`: sample period in milliseconds (at offset 16).
    pub sample_time_ms: f64,
    /// Skip offsets 24–251 (228 bytes).
    #[br(pad_before = 228, pad_after = 2)]
    /// `nExtItemHeaderLen`: byte length of each per-channel header (at offset 252).
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
    validate_channels(raw.channels, 10)?;
    validate_sample_time(raw.sample_time_ms, 16)?;

    let metadata = GraphMetadata {
        file_revision: FileRevision::new(raw.version),
        samples_per_second: 1000.0 / raw.sample_time_ms,
        channel_count: u16::try_from(raw.channels).unwrap_or(0),
        byte_order: endian_to_byte_order(endian),
        compressed: false, // Pre-4 files are never compressed
        title: None,
        acquisition_datetime: None,
        max_samples_per_second: None,
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
/// Field layout (BIOPAC format; offsets from start of file):
/// | Offset | Type  | Field                    |
/// |-------:|-------|---------------------------|
/// |      0 | i16   | unused (`nItemHeaderLen`) |
/// |      2 | i32   | `lVersion`               |
/// |      6 | i32   | `lExtItemHeaderLen`      |
/// |     10 | i16   | `nChannels`              |
/// |     12 | i16   | `nHorizAxisType` (skipped)|
/// |     14 | i16   | `nCurrChannel` (skipped) |
/// |     16 | f64   | `dSampleTime` (ms)       |
/// |    236 | [u8;40]| `szGraphTitle` (optional)|
/// |    276 | i32×6 | `lSec..lYear` (optional) |
/// |   1936 | u8    | `bCompressed` (optional) |
/// |   1940 | i32   | `lMaxAcqSamplesPerSec` (optional, rev ≥ 74) |
///
/// After this struct is read the caller **must** seek to `graph_header_len`
/// (the value of `lExtItemHeaderLen`) to land at the first channel header.
#[binrw]
#[derive(Debug, Copy, Clone)]
pub(super) struct GraphHeaderPost4Raw {
    /// Skip 2-byte unused prefix (`nItemHeaderLen`) at offset 0.
    #[br(pad_before = 2)]
    /// `lVersion`: file format revision (at offset 2).
    pub version: i32,
    /// `lExtItemHeaderLen`: total byte length of this graph header (at offset 6).
    pub graph_header_len: i32,
    /// `nChannels`: number of channel headers that follow (at offset 10).
    pub channels: i16,
    /// Skip `nHorizAxisType` and `nCurrChannel` (4 bytes at offsets 12–15).
    #[br(pad_before = 4)]
    /// `dSampleTime`: sample period in milliseconds (at offset 16).
    pub sample_time_ms: f64,
    // Cursor is now at offset 24.
    /// `szGraphTitle`: 40-byte null-terminated ASCII title (offset 236).
    ///
    /// Present only when `lExtItemHeaderLen >= 276`.
    #[br(
        if(graph_header_len >= GRAPH_HDR_TITLE_MIN_LEN),
        seek_before = SeekFrom::Start(GRAPH_TITLE_OFFSET)
    )]
    pub title: Option<[u8; 40]>,

    /// `lSec`: acquisition seconds (offset 276).
    ///
    /// When present, offsets 276–299 hold six consecutive `i32` date/time
    /// fields read sequentially without additional seeks.
    #[br(
        if(graph_header_len >= GRAPH_HDR_DATETIME_MIN_LEN),
        seek_before = SeekFrom::Start(GRAPH_DATETIME_OFFSET)
    )]
    pub acq_sec: Option<i32>,
    /// `lMin`: acquisition minutes (offset 280).
    #[br(if(graph_header_len >= GRAPH_HDR_DATETIME_MIN_LEN))]
    pub acq_min: Option<i32>,
    /// `lHour`: acquisition hours (offset 284).
    #[br(if(graph_header_len >= GRAPH_HDR_DATETIME_MIN_LEN))]
    pub acq_hour: Option<i32>,
    /// `lDay`: acquisition day (offset 288).
    #[br(if(graph_header_len >= GRAPH_HDR_DATETIME_MIN_LEN))]
    pub acq_day: Option<i32>,
    /// `lMonth`: acquisition month (offset 292).
    #[br(if(graph_header_len >= GRAPH_HDR_DATETIME_MIN_LEN))]
    pub acq_month: Option<i32>,
    /// `lYear`: acquisition year, full value e.g. 2023 (offset 296).
    #[br(if(graph_header_len >= GRAPH_HDR_DATETIME_MIN_LEN))]
    pub acq_year: Option<i32>,

    /// `bCompressed`: non-zero when channel data is zlib-compressed (offset 1936).
    ///
    /// Only present when `graph_header_len >= 1937`.
    #[br(
        if(graph_header_len >= COMPRESSED_FLAG_MIN_LEN),
        seek_before = SeekFrom::Start(COMPRESSED_FLAG_OFFSET)
    )]
    pub compressed: Option<u8>,

    /// `lMaxAcqSamplesPerSec`: maximum hardware sample rate in Hz (offset 1940).
    ///
    /// Present only in `AcqKnowledge` ≥ 4.2 (revision ≥ 74) files where
    /// `lExtItemHeaderLen >= 1944`.
    #[br(
        if(graph_header_len >= GRAPH_HDR_MAX_RATE_MIN_LEN),
        seek_before = SeekFrom::Start(GRAPH_MAX_RATE_OFFSET)
    )]
    pub max_acq_samples_per_sec: Option<i32>,
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
    validate_channels(raw.channels, 10)?;
    validate_sample_time(raw.sample_time_ms, 16)?;

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

    let title = raw.title.map(|t| null_term_bytes_to_string(&t));

    let acquisition_datetime = match (
        raw.acq_year,
        raw.acq_month,
        raw.acq_day,
        raw.acq_hour,
        raw.acq_min,
        raw.acq_sec,
    ) {
        (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) => {
            Some(AcquisitionDateTime {
                year,
                month,
                day,
                hour,
                minute,
                second,
            })
        }
        _ => None,
    };

    let max_samples_per_second = raw
        .max_acq_samples_per_sec
        .and_then(|n| u32::try_from(n).ok());

    let metadata = GraphMetadata {
        file_revision: FileRevision::new(raw.version),
        samples_per_second: 1000.0 / raw.sample_time_ms,
        channel_count: u16::try_from(raw.channels).unwrap_or(0),
        byte_order: endian_to_byte_order(endian),
        compressed,
        title,
        acquisition_datetime,
        max_samples_per_second,
    };

    Ok(Post4Parsed {
        metadata,
        graph_header_len,
    })
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Decode a fixed-size null-terminated byte array to a `String`.
///
/// Bytes after the first `\0` are discarded. Non-UTF-8 bytes are replaced
/// with the Unicode replacement character.
fn null_term_bytes_to_string(bytes: &[u8]) -> alloc::string::String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    alloc::string::String::from_utf8_lossy(bytes.get(..end).unwrap_or(bytes)).into_owned()
}

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
        // Unused i16 at offset 0, lVersion = 38 as LE i32 at offsets 2-5.
        let mut bytes = [0u8; 256];
        bytes[2..6].copy_from_slice(&38i32.to_le_bytes());
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
        // Unused i16 at offset 0, lVersion = 68 as BE i32 at offsets 2-5.
        let mut bytes = [0u8; 256];
        bytes[2..6].copy_from_slice(&68i32.to_be_bytes());
        // [0, 0, 0, 68] as LE i32 = 0x44000000, out of [30, 200], so BE wins.
        let mut cursor = Cursor::new(&bytes[..]);
        let (endian, version) = detect_byte_order(&mut cursor)?;
        assert_eq!(endian, Endian::Big);
        assert_eq!(version, 68);
        Ok(())
    }

    #[test]
    fn detect_invalid_version_returns_error() {
        // 6 bytes: unused prefix = 0, version bytes = 0 — neither LE nor BE is in [30, 200].
        let bytes = [0u8; 6];
        let mut cursor = Cursor::new(&bytes[..]);
        let result = detect_byte_order(&mut cursor);
        assert!(result.is_err());
    }

    // ----- Pre-4 graph header -------------------------------------------

    /// Build a minimal 256-byte Pre-4 graph header using the BIOPAC format.
    fn pre4_bytes(
        version: i32,
        channels: i16,
        sample_time_ms: f64,
        chan_header_len: i16,
    ) -> [u8; 256] {
        let mut b = [0u8; 256];
        // offset 0-1: unused i16 = 0
        b[2..6].copy_from_slice(&version.to_le_bytes());          // lVersion at offset 2
        b[6..10].copy_from_slice(&256i32.to_le_bytes());           // lExtItemHeaderLen = 256 at offset 6
        b[10..12].copy_from_slice(&channels.to_le_bytes());        // nChannels at offset 10
        // offsets 12-15: nHorizAxisType, nCurrChannel = 0
        b[16..24].copy_from_slice(&sample_time_ms.to_le_bytes());  // dSampleTime at offset 16
        // offsets 24-251 = 0
        b[252..254].copy_from_slice(&chan_header_len.to_le_bytes()); // nExtItemHeaderLen at offset 252
        // 254-255 pad = 0
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

    /// Build a minimal Post-4 header buffer using the BIOPAC format.
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
        // offset 0-1: unused i16 = 0
        b[2..6].copy_from_slice(&version.to_le_bytes());       // lVersion at offset 2
        b[6..10].copy_from_slice(&header_len.to_le_bytes());   // lExtItemHeaderLen at offset 6
        b[10..12].copy_from_slice(&channels.to_le_bytes());    // nChannels at offset 10
        // offsets 12-15: nHorizAxisType, nCurrChannel = 0
        b[16..24].copy_from_slice(&sample_time_ms.to_le_bytes()); // dSampleTime at offset 16
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
        // offset 0-1: unused i16 = 0
        bytes[2..6].copy_from_slice(&77i32.to_le_bytes());        // revision 77 at offset 2
        bytes[6..10].copy_from_slice(&header_len.to_le_bytes());  // lExtItemHeaderLen at offset 6
        bytes[10..12].copy_from_slice(&1i16.to_le_bytes());       // 1 channel at offset 10
        // offsets 12-15: horiz/curr = 0
        bytes[16..24].copy_from_slice(&1.0f64.to_le_bytes());     // 1 ms -> 1000 Hz at offset 16
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

    // ----- New field extraction tests -----------------------------------

    #[test]
    #[expect(
        clippy::indexing_slicing,
        clippy::cast_sign_loss,
        reason = "test: slices at known offsets within a 300-byte buffer"
    )]
    fn post4_extracts_title_when_header_long_enough() -> Result<(), Box<dyn std::error::Error>> {
        let header_len: i32 = 300;
        let mut bytes = vec![0u8; header_len as usize];
        // offset 0-1: unused i16 = 0
        bytes[2..6].copy_from_slice(&68i32.to_le_bytes());        // revision 68 at offset 2
        bytes[6..10].copy_from_slice(&header_len.to_le_bytes());  // lExtItemHeaderLen at offset 6
        bytes[10..12].copy_from_slice(&1i16.to_le_bytes());       // 1 channel at offset 10
        bytes[16..24].copy_from_slice(&1.0f64.to_le_bytes());     // dSampleTime at offset 16
        // Write "ECG Test" at offset 236.
        let title = b"ECG Test\0";
        bytes[236..236 + title.len()].copy_from_slice(title);

        let mut cursor = Cursor::new(&bytes);
        let raw = GraphHeaderPost4Raw::read_le(&mut cursor)?;
        let parsed = parse_graph_header_post4(raw, Endian::Little)?;
        assert_eq!(parsed.metadata.title.as_deref(), Some("ECG Test"));
        Ok(())
    }

    #[test]
    fn post4_title_is_none_for_short_header() -> Result<(), Box<dyn std::error::Error>> {
        let bytes = post4_bytes_short(68, 1, 40, 1.0); // header_len = 40 < 276
        let mut cursor = Cursor::new(&bytes);
        let raw = GraphHeaderPost4Raw::read_le(&mut cursor)?;
        let parsed = parse_graph_header_post4(raw, Endian::Little)?;
        assert!(parsed.metadata.title.is_none());
        Ok(())
    }

    #[test]
    #[expect(
        clippy::indexing_slicing,
        clippy::cast_sign_loss,
        reason = "test: slices at known offsets within a 300-byte buffer"
    )]
    fn post4_extracts_acquisition_datetime() -> Result<(), Box<dyn std::error::Error>> {
        let header_len: i32 = 300;
        let mut bytes = vec![0u8; header_len as usize];
        bytes[2..6].copy_from_slice(&74i32.to_le_bytes());
        bytes[6..10].copy_from_slice(&header_len.to_le_bytes());
        bytes[10..12].copy_from_slice(&2i16.to_le_bytes());
        bytes[16..24].copy_from_slice(&1.0f64.to_le_bytes());
        // Write sec/min/hour/day/month/year at offset 276.
        let dt_fields: [i32; 6] = [30, 45, 9, 14, 3, 2008]; // lSec, lMin, lHour, lDay, lMonth, lYear
        for (i, &v) in dt_fields.iter().enumerate() {
            let offset = 276 + i * 4;
            bytes[offset..offset + 4].copy_from_slice(&v.to_le_bytes());
        }

        let mut cursor = Cursor::new(&bytes);
        let raw = GraphHeaderPost4Raw::read_le(&mut cursor)?;
        let parsed = parse_graph_header_post4(raw, Endian::Little)?;

        let dt = parsed.metadata.acquisition_datetime;
        assert!(dt.is_some(), "expected datetime to be parsed");
        assert_eq!(dt.map(|d| d.year), Some(2008));
        assert_eq!(dt.map(|d| d.month), Some(3));
        assert_eq!(dt.map(|d| d.day), Some(14));
        assert_eq!(dt.map(|d| d.hour), Some(9));
        assert_eq!(dt.map(|d| d.minute), Some(45));
        assert_eq!(dt.map(|d| d.second), Some(30));
        Ok(())
    }

    #[test]
    fn post4_datetime_is_none_for_short_header() -> Result<(), Box<dyn std::error::Error>> {
        let bytes = post4_bytes_short(68, 1, 280, 1.0); // header_len = 280 < 300
        let mut cursor = Cursor::new(&bytes);
        let raw = GraphHeaderPost4Raw::read_le(&mut cursor)?;
        let parsed = parse_graph_header_post4(raw, Endian::Little)?;
        assert!(parsed.metadata.acquisition_datetime.is_none());
        Ok(())
    }

    #[test]
    #[expect(
        clippy::indexing_slicing,
        clippy::cast_sign_loss,
        reason = "test: slices at known offsets within a 1944-byte buffer"
    )]
    fn post4_extracts_max_samples_per_second() -> Result<(), Box<dyn std::error::Error>> {
        let header_len: i32 = 1944;
        let mut bytes = vec![0u8; header_len as usize];
        bytes[2..6].copy_from_slice(&74i32.to_le_bytes());
        bytes[6..10].copy_from_slice(&header_len.to_le_bytes());
        bytes[10..12].copy_from_slice(&1i16.to_le_bytes());
        bytes[16..24].copy_from_slice(&1.0f64.to_le_bytes());
        let max_rate: i32 = 400_000;
        bytes[1940..1944].copy_from_slice(&max_rate.to_le_bytes());

        let mut cursor = Cursor::new(&bytes);
        let raw = GraphHeaderPost4Raw::read_le(&mut cursor)?;
        let parsed = parse_graph_header_post4(raw, Endian::Little)?;
        assert_eq!(parsed.metadata.max_samples_per_second, Some(400_000));
        Ok(())
    }

    #[test]
    fn post4_max_rate_is_none_for_short_header() -> Result<(), Box<dyn std::error::Error>> {
        // header_len = 1940 < 1944
        let bytes = post4_bytes_short(74, 1, 1940, 1.0);
        let mut cursor = Cursor::new(&bytes);
        let raw = GraphHeaderPost4Raw::read_le(&mut cursor)?;
        let parsed = parse_graph_header_post4(raw, Endian::Little)?;
        assert!(parsed.metadata.max_samples_per_second.is_none());
        Ok(())
    }

    #[test]
    fn null_term_bytes_to_string_strips_at_first_null() {
        let input = *b"Hello\0garbage-bytes";
        assert_eq!(null_term_bytes_to_string(&input), "Hello");
    }

    #[test]
    fn null_term_bytes_to_string_all_null() {
        let input = [0u8; 40];
        assert_eq!(null_term_bytes_to_string(&input), "");
    }

    #[test]
    fn null_term_bytes_to_string_no_null() {
        let input = *b"ABCDE";
        assert_eq!(null_term_bytes_to_string(&input), "ABCDE");
    }
}

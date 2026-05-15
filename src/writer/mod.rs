//! Write support for `.acq` files (requires `write` feature).
//!
//! # Supported formats
//!
//! - **Pre-4 uncompressed** (default, revision 43) — widest compatibility;
//!   all physiological analysis tools can read it.
//! - **Post-4 compressed** (revision 68) — per-channel zlib compression;
//!   typically halves file size for large recordings.
//!
//! # Example
//!
//! ```rust,ignore
//! use biodream::{WriteOptions, write_file};
//!
//! // Write with defaults (uncompressed, Pre-4, little-endian)
//! write_file(&datafile, "output.acq")?;
//!
//! // Write compressed
//! WriteOptions::new().compressed(true).write_file(&datafile, "output.acq")?;
//! ```

use alloc::{format, string::ToString, vec, vec::Vec};
use std::io::Write;
use std::path::Path;

use crate::{
    domain::{ByteOrder, Channel, ChannelData, Datafile, Journal, Marker, MarkerStyle, Timestamp},
    error::BiopacError,
    parser::interleaved::compute_sample_pattern,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Pre-4 format revision used by default (3.7.3.x era).
const DEFAULT_REVISION: i32 = 43;

/// Post-4 revision used for compressed output (`AcqKnowledge` 4.0 era).
const COMPRESSED_REVISION: i32 = 68;

/// Minimum revision with per-marker creation timestamps.
const REVISION_TIMESTAMP: i32 = 77;

/// Byte length of the fixed Pre-4 graph header.
const PRE4_GRAPH_HDR_LEN: usize = 256;

/// Byte length of the channel header written by this library.
const CHAN_HDR_LEN: usize = 86;

/// Byte length of the Post-4 graph header written for compressed output.
///
/// 1944 bytes: covers the `bCompressed` flag (offset 1936) and the optional
/// max-rate field (offset 1940).
const POST4_GRAPH_HDR_LEN: usize = 1944;

/// Byte offset of the `bCompressed` flag within a Post-4 graph header.
const POST4_COMPRESSED_FLAG_OFFSET: usize = 1936;

/// Byte length of the marker section header (lLength + lNumMarkers).
const MARKER_HDR_BYTES: i32 = 8;

/// Fixed per-marker field bytes (lSample + nChannel + szStyle + lMarkerTextLen).
const MARKER_FIXED_BYTES: i32 = 14;

/// Extra bytes per marker when revision >= `REVISION_TIMESTAMP` (i64 timestamp).
const MARKER_TIMESTAMP_BYTES: i32 = 8;

// ---------------------------------------------------------------------------
// Public API: WriteOptions
// ---------------------------------------------------------------------------

/// Options controlling how a [`Datafile`] is serialised to `.acq` format.
///
/// Use the builder methods to override defaults, then call
/// [`WriteOptions::write_file`] or [`WriteOptions::write_stream`].
///
/// # Defaults
///
/// | Field | Default | Notes |
/// |-------|---------|-------|
/// | `revision` | 43 | Pre-4 (3.7.3.x), uncompressed |
/// | `compressed` | `false` | Uncompressed |
/// | `byte_order` | `LittleEndian` | LE is universal |
#[derive(Debug, Clone)]
pub struct WriteOptions {
    /// Format revision written into the graph header.
    ///
    /// Revision 43 = Pre-4 (uncompressed). When `compressed = true` the
    /// revision is overridden to 68 (Post-4) automatically.
    pub revision: i32,
    /// Compress channel data with zlib.
    ///
    /// Enabling compression selects the Post-4 file layout (revision ≥ 68)
    /// and applies per-channel zlib compression — typically halving file size.
    pub compressed: bool,
    /// Byte order used for all multi-byte integers and floats.
    pub byte_order: ByteOrder,
}

impl WriteOptions {
    /// Create default options (uncompressed Pre-4, revision 43, LE).
    pub const fn new() -> Self {
        Self {
            revision: DEFAULT_REVISION,
            compressed: false,
            byte_order: ByteOrder::LittleEndian,
        }
    }

    /// Set `compressed = true`; selects Post-4 format and zlib compression.
    #[must_use]
    pub const fn compressed(mut self, compressed: bool) -> Self {
        self.compressed = compressed;
        self
    }

    /// Override the format revision (default 43 for uncompressed, 68 for compressed).
    #[must_use]
    pub const fn revision(mut self, revision: i32) -> Self {
        self.revision = revision;
        self
    }

    /// Override the byte order (default little-endian).
    #[must_use]
    pub const fn byte_order(mut self, byte_order: ByteOrder) -> Self {
        self.byte_order = byte_order;
        self
    }

    /// Write `df` to a file at `path`, creating or truncating it.
    pub fn write_file(&self, df: &Datafile, path: impl AsRef<Path>) -> Result<(), BiopacError> {
        let file = std::fs::File::create(path).map_err(BiopacError::Io)?;
        let mut w = std::io::BufWriter::new(file);
        self.write_stream(df, &mut w)
    }

    /// Write `df` to any [`Write`] sink.
    pub fn write_stream<W: Write>(&self, df: &Datafile, w: &mut W) -> Result<(), BiopacError> {
        if self.compressed {
            write_compressed(df, w, self)
        } else {
            write_uncompressed(df, w, self)
        }
    }
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Public convenience functions
// ---------------------------------------------------------------------------

/// Write `df` to a file at `path` using default options (uncompressed, Pre-4).
///
/// Both compressed and uncompressed files can be read back with
/// [`crate::read_file`].
pub fn write_file(df: &Datafile, path: impl AsRef<Path>) -> Result<(), BiopacError> {
    WriteOptions::default().write_file(df, path)
}

/// Write `df` to any [`Write`] sink using default options.
pub fn write_stream<W: Write>(df: &Datafile, w: &mut W) -> Result<(), BiopacError> {
    WriteOptions::default().write_stream(df, w)
}

// ---------------------------------------------------------------------------
// Uncompressed writer
// ---------------------------------------------------------------------------

fn write_uncompressed<W: Write>(
    df: &Datafile,
    w: &mut W,
    opts: &WriteOptions,
) -> Result<(), BiopacError> {
    let le = opts.byte_order == ByteOrder::LittleEndian;
    let n_ch = df.channels.len();

    // 1. Graph header (256 bytes, Pre-4).
    write_pre4_graph_header(w, opts.revision, n_ch, df.metadata.samples_per_second, le)?;

    // 2. Per-channel headers.
    for ch in &df.channels {
        write_channel_header(w, ch, le)?;
    }

    // 3. Foreign data section (empty).
    write_foreign_data_section(w, le)?;

    // 4. Dtype headers.
    for ch in &df.channels {
        write_dtype_header(w, &ch.data, le)?;
    }

    // 5. Interleaved channel data.
    write_interleaved_data(w, df, le)?;

    // 6. Marker section.
    write_marker_section(w, &df.markers, opts.revision, le)?;

    // 7. Journal.
    write_journal_section(w, df.journal.as_ref(), le)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Compressed writer (Post-4, revision 68)
// ---------------------------------------------------------------------------

fn write_compressed<W: Write>(
    df: &Datafile,
    w: &mut W,
    _opts: &WriteOptions,
) -> Result<(), BiopacError> {
    // Compressed output is always little-endian.
    let le = true;
    let n_ch = df.channels.len();

    // 1. Post-4 graph header (1944 bytes) with bCompressed = 1.
    write_post4_graph_header(w, COMPRESSED_REVISION, n_ch, df.metadata.samples_per_second)?;

    // 2. Per-channel headers.
    for ch in &df.channels {
        write_channel_header(w, ch, le)?;
    }

    // 3. Foreign data section (empty).
    write_foreign_data_section(w, le)?;

    // 4. Dtype headers.
    for ch in &df.channels {
        write_dtype_header(w, &ch.data, le)?;
    }

    // 5. Compressed layout: markers and journal come BEFORE channel data.
    write_marker_section(w, &df.markers, COMPRESSED_REVISION, le)?;
    write_journal_section(w, df.journal.as_ref(), le)?;

    // 6. Per-channel compressed blobs.
    for ch in &df.channels {
        write_compressed_channel(w, ch)?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Graph headers
// ---------------------------------------------------------------------------

/// Write a 256-byte Pre-4 graph header.
fn write_pre4_graph_header<W: Write>(
    w: &mut W,
    revision: i32,
    n_channels: usize,
    samples_per_second: f64,
    le: bool,
) -> Result<(), BiopacError> {
    let mut buf = [0u8; PRE4_GRAPH_HDR_LEN];

    let n_ch = i16::try_from(n_channels).map_err(|_| {
        BiopacError::Validation(format!("too many channels to write as Pre-4: {n_channels}"))
    })?;
    let sample_time_ms = 1000.0_f64 / samples_per_second;
    let chan_hdr_len = i16::try_from(CHAN_HDR_LEN).unwrap_or(i16::MAX);

    put_i32(&mut buf, 0, revision, le);
    put_i16(&mut buf, 4, n_ch, le);
    // offset 6: nPreampTypes = 0 (already zero)
    put_f64(&mut buf, 8, sample_time_ms, le);
    // offset 16–251: zeros (already zero)
    put_i16(&mut buf, 252, chan_hdr_len, le);
    // offset 254–255: zeros

    w.write_all(&buf).map_err(BiopacError::Io)
}

/// Write a 1944-byte Post-4 graph header with `bCompressed = 1`.
fn write_post4_graph_header<W: Write>(
    w: &mut W,
    revision: i32,
    n_channels: usize,
    samples_per_second: f64,
) -> Result<(), BiopacError> {
    let mut buf = [0u8; POST4_GRAPH_HDR_LEN];

    let n_ch = i16::try_from(n_channels).map_err(|_| {
        BiopacError::Validation(format!(
            "too many channels to write as Post-4: {n_channels}"
        ))
    })?;
    let graph_hdr_len = i32::try_from(POST4_GRAPH_HDR_LEN).unwrap_or(i32::MAX);
    let sample_time_ms = 1000.0_f64 / samples_per_second;

    // Always little-endian for compressed files.
    put_i32(&mut buf, 0, revision, true); // lVersion
    put_i16(&mut buf, 4, n_ch, true); // nChannels
    put_i32(&mut buf, 6, graph_hdr_len, true); // lExtItemHeaderLen
    // offset 10: lNumItems = 0 (already zero)
    put_f64(&mut buf, 12, sample_time_ms, true); // dSampleTime
    // offsets 20–1935: zeros
    if let Some(flag) = buf.get_mut(POST4_COMPRESSED_FLAG_OFFSET) {
        *flag = 1; // bCompressed = 1
    }
    // offsets 1937–1943: zeros (max_rate = 0)

    w.write_all(&buf).map_err(BiopacError::Io)
}

// ---------------------------------------------------------------------------
// Channel header (86 bytes)
// ---------------------------------------------------------------------------

fn write_channel_header<W: Write>(w: &mut W, ch: &Channel, le: bool) -> Result<(), BiopacError> {
    let mut buf = [0u8; CHAN_HDR_LEN];

    let chan_hdr_len = i32::try_from(CHAN_HDR_LEN).unwrap_or(i32::MAX);
    let sample_count = i32::try_from(ch.data.len()).map_err(|_| {
        BiopacError::Validation(format!("channel '{}' has too many samples", ch.name))
    })?;
    let (scale, offset_val) = channel_calibration(ch);
    let var_sample_divider = i16::try_from(ch.frequency_divider).unwrap_or(i16::MAX);

    put_i32(&mut buf, 0, chan_hdr_len, le); // lChanHeaderLen
    put_i32(&mut buf, 4, sample_count, le); // lBufLength
    put_f64(&mut buf, 8, scale, le); // dAmplScale
    put_f64(&mut buf, 16, offset_val, le); // dAmplOffset
    put_i16(&mut buf, 24, var_sample_divider, le); // nVarSampleDivider

    // szCommentText: 40 bytes, null-padded ASCII (offset 26).
    let name_bytes = ch.name.as_bytes();
    let name_len = name_bytes.len().min(39);
    if let (Some(dst), Some(src)) = (buf.get_mut(26..26 + name_len), name_bytes.get(..name_len)) {
        dst.copy_from_slice(src);
    }

    // szUnitsText: 20 bytes, null-padded ASCII (offset 66).
    let units_bytes = ch.units.as_bytes();
    let units_len = units_bytes.len().min(19);
    if let (Some(dst), Some(src)) = (
        buf.get_mut(66..66 + units_len),
        units_bytes.get(..units_len),
    ) {
        dst.copy_from_slice(src);
    }

    w.write_all(&buf).map_err(BiopacError::Io)
}

const fn channel_calibration(ch: &Channel) -> (f64, f64) {
    match &ch.data {
        ChannelData::Scaled { scale, offset, .. } => (*scale, *offset),
        _ => (1.0, 0.0),
    }
}

// ---------------------------------------------------------------------------
// Foreign data section
// ---------------------------------------------------------------------------

/// Write an empty foreign data section (nLength = 0; 4 bytes total).
fn write_foreign_data_section<W: Write>(w: &mut W, le: bool) -> Result<(), BiopacError> {
    let bytes = if le {
        0i32.to_le_bytes()
    } else {
        0i32.to_be_bytes()
    };
    w.write_all(&bytes).map_err(BiopacError::Io)
}

// ---------------------------------------------------------------------------
// Dtype header (4 bytes per channel)
// ---------------------------------------------------------------------------

fn write_dtype_header<W: Write>(
    w: &mut W,
    data: &ChannelData,
    le: bool,
) -> Result<(), BiopacError> {
    // n_size field in the file (always 4, the size of the dtype record itself).
    let n_size: u16 = 4;
    // n_type: 1 = f64 samples, 2 = i16 samples.
    let n_type: u16 = match data {
        ChannelData::Float(_) => 1,
        ChannelData::Raw(_) | ChannelData::Scaled { .. } => 2,
    };

    let mut buf = [0u8; 4];
    put_u16(&mut buf, 0, n_size, le);
    put_u16(&mut buf, 2, n_type, le);
    w.write_all(&buf).map_err(BiopacError::Io)
}

// ---------------------------------------------------------------------------
// Interleaved channel data
// ---------------------------------------------------------------------------

fn write_interleaved_data<W: Write>(w: &mut W, df: &Datafile, le: bool) -> Result<(), BiopacError> {
    let dividers: Vec<u16> = df.channels.iter().map(|ch| ch.frequency_divider).collect();
    let pattern = compute_sample_pattern(&dividers);

    if pattern.is_empty() {
        return Ok(());
    }

    let n_ch = df.channels.len();
    let mut indices = vec![0usize; n_ch];
    let totals: Vec<usize> = df.channels.iter().map(|ch| ch.data.len()).collect();

    loop {
        let mut any_written = false;

        for &ch_idx in &pattern {
            let Some(&cur) = indices.get(ch_idx) else {
                continue;
            };
            let Some(&total) = totals.get(ch_idx) else {
                continue;
            };

            if cur < total {
                let Some(ch) = df.channels.get(ch_idx) else {
                    continue;
                };
                write_one_sample(w, &ch.data, cur, le)?;
                if let Some(idx) = indices.get_mut(ch_idx) {
                    *idx += 1;
                }
                any_written = true;
            }
        }

        if !any_written || indices.iter().zip(totals.iter()).all(|(i, t)| i >= t) {
            break;
        }
    }

    Ok(())
}

fn write_one_sample<W: Write>(
    w: &mut W,
    data: &ChannelData,
    idx: usize,
    le: bool,
) -> Result<(), BiopacError> {
    match data {
        ChannelData::Raw(v) | ChannelData::Scaled { raw: v, .. } => {
            let sample = v.get(idx).copied().unwrap_or(0);
            let bytes = if le {
                sample.to_le_bytes()
            } else {
                sample.to_be_bytes()
            };
            w.write_all(&bytes).map_err(BiopacError::Io)
        }
        ChannelData::Float(v) => {
            let sample = v.get(idx).copied().unwrap_or(0.0);
            let bytes = if le {
                sample.to_le_bytes()
            } else {
                sample.to_be_bytes()
            };
            w.write_all(&bytes).map_err(BiopacError::Io)
        }
    }
}

// ---------------------------------------------------------------------------
// Marker section
// ---------------------------------------------------------------------------

fn write_marker_section<W: Write>(
    w: &mut W,
    markers: &[Marker],
    revision: i32,
    le: bool,
) -> Result<(), BiopacError> {
    let has_timestamp = revision >= REVISION_TIMESTAMP;

    // Compute the total section byte count (including the 8-byte header).
    let total_bytes = compute_marker_section_bytes(markers, has_timestamp)?;

    let num_markers = i32::try_from(markers.len())
        .map_err(|_| BiopacError::Validation("too many markers to write".to_string()))?;

    write_i32(w, total_bytes, le)?; // lLength (total section size)
    write_i32(w, num_markers, le)?; // lNumMarkers

    for m in markers {
        write_marker_record(w, m, has_timestamp, le)?;
    }

    Ok(())
}

/// Compute total section bytes for the marker section header `lLength` field.
///
/// Includes the 8-byte section header itself.
fn compute_marker_section_bytes(
    markers: &[Marker],
    has_timestamp: bool,
) -> Result<i32, BiopacError> {
    let mut total: i32 = MARKER_HDR_BYTES;

    for m in markers {
        let text_len = i32::try_from(m.label.len()).map_err(|_| {
            BiopacError::Validation(format!(
                "marker label '{}' is too long to serialise",
                m.label
            ))
        })?;

        let fixed = MARKER_FIXED_BYTES;
        let ts = if has_timestamp {
            MARKER_TIMESTAMP_BYTES
        } else {
            0
        };

        total = total
            .checked_add(fixed)
            .and_then(|v| v.checked_add(text_len))
            .and_then(|v| v.checked_add(ts))
            .ok_or_else(|| BiopacError::Validation("marker section too large".to_string()))?;
    }

    Ok(total)
}

fn write_marker_record<W: Write>(
    w: &mut W,
    m: &Marker,
    has_timestamp: bool,
    le: bool,
) -> Result<(), BiopacError> {
    let sample = i32::try_from(m.global_sample_index).map_err(|_| {
        BiopacError::Validation(format!(
            "marker sample index {} overflows i32",
            m.global_sample_index
        ))
    })?;

    let n_channel: i16 = match m.channel {
        None => -1i16,
        Some(ch) => i16::try_from(ch).map_err(|_| {
            BiopacError::Validation(format!("marker channel index {ch} overflows i16"))
        })?,
    };

    let text_bytes = m.label.as_bytes();
    let text_len = i32::try_from(text_bytes.len())
        .map_err(|_| BiopacError::Validation("marker label too long".to_string()))?;

    write_i32(w, sample, le)?;
    write_i16(w, n_channel, le)?;
    w.write_all(&marker_style_code(&m.style))
        .map_err(BiopacError::Io)?;
    write_i32(w, text_len, le)?;
    w.write_all(text_bytes).map_err(BiopacError::Io)?;

    if has_timestamp {
        let ts = m.created_at.map_or(0i64, Timestamp::as_secs);
        write_i64(w, ts, le)?;
    }

    Ok(())
}

fn marker_style_code(style: &MarkerStyle) -> [u8; 4] {
    match style {
        MarkerStyle::Append => *b"apnd",
        MarkerStyle::UserEvent => *b"usr1",
        MarkerStyle::Waveform => *b"wave",
        MarkerStyle::GlobalEvent => *b"glbl",
        MarkerStyle::Unknown(s) => {
            let mut code = [0u8; 4];
            for (i, b) in s.bytes().take(4).enumerate() {
                if let Some(c) = code.get_mut(i) {
                    *c = b;
                }
            }
            code
        }
    }
}

// ---------------------------------------------------------------------------
// Journal section
// ---------------------------------------------------------------------------

fn write_journal_section<W: Write>(
    w: &mut W,
    journal: Option<&Journal>,
    le: bool,
) -> Result<(), BiopacError> {
    match journal {
        None => write_i32(w, 0, le),
        Some(j) => {
            let text = j.as_text();
            let len = i32::try_from(text.len()).map_err(|_| {
                BiopacError::Validation("journal text too large to write".to_string())
            })?;
            write_i32(w, len, le)?;
            w.write_all(text.as_bytes()).map_err(BiopacError::Io)
        }
    }
}

// ---------------------------------------------------------------------------
// Compressed channel blobs (Post-4 layout)
// ---------------------------------------------------------------------------

fn write_compressed_channel<W: Write>(w: &mut W, ch: &Channel) -> Result<(), BiopacError> {
    use flate2::{Compression, write::ZlibEncoder};

    // Serialise samples to little-endian bytes (compressed channels are always LE).
    let raw_bytes = channel_samples_to_le_bytes(ch);

    let uncompressed_len = i32::try_from(raw_bytes.len()).map_err(|_| {
        BiopacError::Validation(format!("channel '{}' data too large to compress", ch.name))
    })?;

    // Compress with zlib (default level).
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&raw_bytes).map_err(BiopacError::Io)?;
    let compressed = encoder.finish().map_err(BiopacError::Io)?;

    let compressed_len = i32::try_from(compressed.len()).map_err(|_| {
        BiopacError::Validation(format!("channel '{}' compressed data too large", ch.name))
    })?;

    // Post-4 compression header: uncompressed_len, compressed_len, lOffset (= 0).
    write_i32(w, uncompressed_len, true)?;
    write_i32(w, compressed_len, true)?;
    write_i32(w, 0, true)?; // lOffset (not needed; reader seeks by index)

    w.write_all(&compressed).map_err(BiopacError::Io)
}

/// Collect all channel samples as little-endian bytes.
fn channel_samples_to_le_bytes(ch: &Channel) -> Vec<u8> {
    match &ch.data {
        ChannelData::Raw(v) | ChannelData::Scaled { raw: v, .. } => {
            let mut bytes = Vec::with_capacity(v.len() * 2);
            for &s in v {
                bytes.extend_from_slice(&s.to_le_bytes());
            }
            bytes
        }
        ChannelData::Float(v) => {
            let mut bytes = Vec::with_capacity(v.len() * 8);
            for &s in v {
                bytes.extend_from_slice(&s.to_le_bytes());
            }
            bytes
        }
    }
}

// ---------------------------------------------------------------------------
// Byte-writing helpers
// ---------------------------------------------------------------------------

fn write_i16<W: Write>(w: &mut W, v: i16, le: bool) -> Result<(), BiopacError> {
    let bytes = if le { v.to_le_bytes() } else { v.to_be_bytes() };
    w.write_all(&bytes).map_err(BiopacError::Io)
}

fn write_i32<W: Write>(w: &mut W, v: i32, le: bool) -> Result<(), BiopacError> {
    let bytes = if le { v.to_le_bytes() } else { v.to_be_bytes() };
    w.write_all(&bytes).map_err(BiopacError::Io)
}

fn write_i64<W: Write>(w: &mut W, v: i64, le: bool) -> Result<(), BiopacError> {
    let bytes = if le { v.to_le_bytes() } else { v.to_be_bytes() };
    w.write_all(&bytes).map_err(BiopacError::Io)
}

fn put_i16(buf: &mut [u8], offset: usize, v: i16, le: bool) {
    let bytes = if le { v.to_le_bytes() } else { v.to_be_bytes() };
    if let Some(dst) = buf.get_mut(offset..offset + 2) {
        dst.copy_from_slice(&bytes);
    }
}

fn put_i32(buf: &mut [u8], offset: usize, v: i32, le: bool) {
    let bytes = if le { v.to_le_bytes() } else { v.to_be_bytes() };
    if let Some(dst) = buf.get_mut(offset..offset + 4) {
        dst.copy_from_slice(&bytes);
    }
}

fn put_f64(buf: &mut [u8], offset: usize, v: f64, le: bool) {
    let bytes = if le { v.to_le_bytes() } else { v.to_be_bytes() };
    if let Some(dst) = buf.get_mut(offset..offset + 8) {
        dst.copy_from_slice(&bytes);
    }
}

fn put_u16(buf: &mut [u8], offset: usize, v: u16, le: bool) {
    let bytes = if le { v.to_le_bytes() } else { v.to_be_bytes() };
    if let Some(dst) = buf.get_mut(offset..offset + 2) {
        dst.copy_from_slice(&bytes);
    }
}

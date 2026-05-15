//! Per-channel header structs.
//!
//! Each channel has `lChanHeaderLen` bytes of header data. The first 86 bytes
//! have a fixed layout shared between Pre-4 and Post-4 files; the remaining
//! bytes are skipped by the caller after this struct is read.
//!
//! Key fields:
//! - `lChanHeaderLen` — total byte length of this channel header
//! - `lBufLength` — number of samples stored
//! - `dAmplScale`, `dAmplOffset` — calibration coefficients
//! - `nVarSampleDivider` — sampling rate divider (1 = base rate)
//! - `szCommentText` (40 bytes) — channel name, null-terminated ASCII
//! - `szUnitsText` (20 bytes) — unit label, null-terminated ASCII

use binrw::binrw;

use crate::{
    domain::ChannelMetadata,
    error::{BiopacError, HeaderSection, ParseError},
};

/// Minimum byte count this struct reads from a channel header.
///
/// 4 + 4 + 8 + 8 + 2 + 40 + 20 = 86 bytes.
pub(super) const CHANNEL_HEADER_MIN_LEN: i32 = 86;

/// Raw per-channel header — first 86 bytes only.
///
/// After binrw reads this struct the stream is positioned 86 bytes past the
/// start of the channel header.  The caller **must** seek to
/// `channel_start + chan_header_len` before reading the next channel.
#[binrw]
#[derive(Debug, Copy, Clone)]
pub(super) struct ChannelHeaderRaw {
    /// Total length of this channel header in bytes (`lChanHeaderLen`).
    pub chan_header_len: i32, // offset 0
    /// Number of samples stored for this channel (`lBufLength`).
    pub buf_length: i32, // offset 4
    /// Amplitude scale factor (`dAmplScale`). Physical = raw * scale + offset.
    pub ampl_scale: f64, // offset 8
    /// Amplitude offset (`dAmplOffset`).
    pub ampl_offset: f64, // offset 16
    /// Sampling rate divider relative to the base rate (`nVarSampleDivider`).
    pub var_sample_divider: i16, // offset 24
    /// Null-terminated ASCII channel name (`szCommentText`, 40 bytes).
    pub comment_text: [u8; 40], // offset 26
    /// Null-terminated ASCII unit label (`szUnitsText`, 20 bytes).
    pub units_text: [u8; 20], // offset 66
                              // Bytes consumed: 4+4+8+8+2+40+20 = 86
                              // Caller must seek to channel_start + chan_header_len.
}

/// Convert a raw channel header into domain [`ChannelMetadata`].
///
/// `channel_index` and `byte_offset` are used to produce a precise
/// [`ParseError`] when the header is structurally invalid.
pub(super) fn parse_channel_metadata(
    raw: ChannelHeaderRaw,
    channel_index: u16,
    byte_offset: u64,
) -> Result<ChannelMetadata, BiopacError> {
    if raw.chan_header_len < CHANNEL_HEADER_MIN_LEN {
        return Err(BiopacError::Parse(ParseError {
            byte_offset,
            expected: alloc::format!("lChanHeaderLen >= {CHANNEL_HEADER_MIN_LEN}"),
            actual: alloc::format!("{}", raw.chan_header_len),
            section: HeaderSection::Channel(channel_index),
        }));
    }

    let name = null_terminated_ascii(&raw.comment_text);
    let units = null_terminated_ascii(&raw.units_text);

    // nVarSampleDivider <= 0 means "not set" — treat as base rate (divider = 1).
    let frequency_divider = u16::try_from(raw.var_sample_divider).unwrap_or(1).max(1);

    Ok(ChannelMetadata {
        name,
        units,
        description: alloc::string::String::new(),
        frequency_divider,
        amplitude_scale: raw.ampl_scale,
        amplitude_offset: raw.ampl_offset,
        display_order: channel_index,
    })
}

/// Extract a null-terminated ASCII string from a fixed-size byte slice.
///
/// Stops at the first NUL byte; invalid UTF-8 is replaced with U+FFFD.
fn null_terminated_ascii(bytes: &[u8]) -> alloc::string::String {
    let len = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    alloc::string::String::from_utf8_lossy(bytes.get(..len).unwrap_or(bytes)).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::boxed::Box;
    use binrw::BinRead;
    use std::io::Cursor;

    /// Build a minimal 86-byte channel header with specified key values.
    fn make_chan_header_bytes(
        chan_header_len: i32,
        buf_length: i32,
        ampl_scale: f64,
        ampl_offset: f64,
        var_sample_divider: i16,
        name: &[u8; 40],
        units: &[u8; 20],
    ) -> [u8; 86] {
        let mut bytes = [0u8; 86];
        bytes[0..4].copy_from_slice(&chan_header_len.to_le_bytes());
        bytes[4..8].copy_from_slice(&buf_length.to_le_bytes());
        bytes[8..16].copy_from_slice(&ampl_scale.to_le_bytes());
        bytes[16..24].copy_from_slice(&ampl_offset.to_le_bytes());
        bytes[24..26].copy_from_slice(&var_sample_divider.to_le_bytes());
        bytes[26..66].copy_from_slice(name);
        bytes[66..86].copy_from_slice(units);
        bytes
    }

    #[test]
    fn channel_header_frequency_divider_2() -> Result<(), Box<dyn std::error::Error>> {
        let mut name = [0u8; 40];
        name[..3].copy_from_slice(b"ECG");
        let mut units = [0u8; 20];
        units[..2].copy_from_slice(b"mV");

        let raw_bytes = make_chan_header_bytes(252, 1000, 1.0, 0.0, 2, &name, &units);
        let mut reader = Cursor::new(&raw_bytes[..]);
        let raw = ChannelHeaderRaw::read_le(&mut reader)?;
        assert_eq!(raw.var_sample_divider, 2);

        let meta = parse_channel_metadata(raw, 0, 0)?;
        assert_eq!(meta.frequency_divider, 2);
        assert_eq!(meta.name, "ECG");
        assert_eq!(meta.units, "mV");
        assert!((meta.amplitude_scale - 1.0_f64).abs() < f64::EPSILON);
        assert!(meta.amplitude_offset.abs() < f64::EPSILON);
        Ok(())
    }

    #[test]
    fn channel_header_non_positive_divider_becomes_1() -> Result<(), Box<dyn std::error::Error>> {
        // nVarSampleDivider = 0 -> frequency_divider = 1 (base rate)
        let name = [0u8; 40];
        let units = [0u8; 20];
        let raw_bytes = make_chan_header_bytes(252, 500, 2.0, -1.0, 0, &name, &units);
        let mut reader = Cursor::new(&raw_bytes[..]);
        let raw = ChannelHeaderRaw::read_le(&mut reader)?;
        let meta = parse_channel_metadata(raw, 1, 0)?;
        assert_eq!(meta.frequency_divider, 1);
        Ok(())
    }

    #[test]
    fn channel_header_too_short_returns_error() -> Result<(), Box<dyn std::error::Error>> {
        // chan_header_len = 80 < CHANNEL_HEADER_MIN_LEN (86)
        let name = [0u8; 40];
        let units = [0u8; 20];
        let raw_bytes = make_chan_header_bytes(80, 0, 1.0, 0.0, 1, &name, &units);
        let mut reader = Cursor::new(&raw_bytes[..]);
        let raw = ChannelHeaderRaw::read_le(&mut reader)?;
        let result = parse_channel_metadata(raw, 0, 0x1A3C);
        assert!(result.is_err(), "too-short header should fail");
        if let Err(e) = result {
            let msg = alloc::format!("{e}");
            assert!(msg.contains("0x1A3C"), "should include byte offset: {msg}");
            assert!(msg.contains("Channel"), "should name section: {msg}");
        }
        Ok(())
    }

    #[test]
    fn null_terminated_ascii_helper() {
        let mut buf = [0u8; 20];
        buf[..4].copy_from_slice(b"temp");
        let s = null_terminated_ascii(&buf);
        assert_eq!(s, "temp");
    }
}

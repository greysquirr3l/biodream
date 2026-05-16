//! Per-channel header structs.
//!
//! Each channel has `lChanHeaderLen` bytes of header data.  The first 112 bytes
//! have the same field layout across all BIOPAC versions (Pre-4 and Post-4):
//! the `V_20a` base layout.  `nVarSampleDivider` lives at a version-dependent
//! offset outside this struct and is passed separately to
//! `parse_channel_metadata`.
//!
//! Key fields:
//! - `lChanHeaderLen` — total byte length of this channel header
//! - `szCommentText` (40 bytes, offset 6) — channel name, null-terminated
//! - `szUnitsText` (20 bytes, offset 68) — unit label, null-terminated
//! - `lBufLength` (offset 88) — number of samples stored
//! - `dAmplScale` (offset 92), `dAmplOffset` (offset 100) — calibration
//! - `nVarSampleDivider` — sampling rate divider (read by caller)

use binrw::binrw;

use crate::{
    domain::ChannelMetadata,
    error::{BiopacError, HeaderSection, ParseError},
};

/// Minimum byte count this struct reads from a channel header.
///
/// The first 112 bytes have the same layout across all BIOPAC versions
/// (`V_20a` base): `lChanHeaderLen`, `nNum`, `szCommentText`, `notColor`,
/// `nDispChan`, `dVoltOffset`, `dVoltScale`, `szUnitsText`, `lBufLength`,
/// `dAmplScale`, `dAmplOffset`, `nChanOrder`, `nDispSize`.
///
/// 4 + 2 + 40 + 4 + 2 + 8 + 8 + 20 + 4 + 8 + 8 + 2 + 2 = 112 bytes.
pub(super) const CHANNEL_HEADER_MIN_LEN: i32 = 112;

/// Raw per-channel header — first 112 bytes (`V_20a` base layout).
///
/// This layout is shared between Pre-4 and Post-4 BIOPAC files.
/// `nVarSampleDivider` lives at a version-dependent offset beyond this struct
/// and is read separately by the caller, then passed to
/// `parse_channel_metadata`.
///
/// After binrw reads this struct the stream is positioned 112 bytes past the
/// start of the channel header.  The caller **must** seek to
/// `channel_start + chan_header_len` before reading the next channel.
#[binrw]
#[derive(Debug, Copy, Clone)]
pub(super) struct ChannelHeaderRaw {
    /// Total length of this channel header in bytes (`lChanHeaderLen`).
    pub chan_header_len: i32, // offset 0
    /// Channel sequence number (`nNum`).
    pub num: i16, // offset 4
    /// Null-terminated channel name (`szCommentText`, 40 bytes).
    pub comment_text: [u8; 40], // offset 6
    /// Display colour bytes, not used by this parser (`notColor`/`rgbColor`).
    pub not_color: [u8; 4], // offset 46
    /// Display channel index (`nDispChan`).
    pub disp_chan: i16, // offset 50
    /// Voltage offset in raw units (`dVoltOffset`).
    pub volt_offset: f64, // offset 52
    /// Voltage scale in raw units (`dVoltScale`).
    pub volt_scale: f64, // offset 60
    /// Null-terminated unit label (`szUnitsText`, 20 bytes).
    pub units_text: [u8; 20], // offset 68
    /// Number of samples stored for this channel (`lBufLength`).
    pub buf_length: i32, // offset 88
    /// Amplitude scale factor (`dAmplScale`).  Physical = raw × scale + offset.
    pub ampl_scale: f64, // offset 92
    /// Amplitude offset (`dAmplOffset`).
    pub ampl_offset: f64, // offset 100
    /// Display order index (`nChanOrder`).
    pub chan_order: i16, // offset 108
    /// Display size hint (`nDispSize`).
    pub disp_size: i16, // offset 110
                        // 4+2+40+4+2+8+8+20+4+8+8+2+2 = 112 bytes consumed.
                        // nVarSampleDivider is at version-dependent offset; read by caller.
                        // Caller must seek to channel_start + chan_header_len before next channel.
}

/// Convert a raw channel header into domain [`ChannelMetadata`].
///
/// `var_sample_divider` is the `nVarSampleDivider` value read by the caller
/// from its version-dependent offset in the file (Post-4: offset 152;
/// Pre-4 `V_30r+`: offset 250; otherwise pass `1`).
///
/// `channel_index` and `byte_offset` are used to produce a precise
/// [`ParseError`] when the header is structurally invalid.
pub(super) fn parse_channel_metadata(
    raw: ChannelHeaderRaw,
    var_sample_divider: i16,
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
    let frequency_divider = u16::try_from(var_sample_divider).unwrap_or(1).max(1);

    // lBufLength is signed; negative values mean "not set" — treat as 0.
    #[expect(
        clippy::cast_sign_loss,
        reason = "lBufLength clamped to >=0 before cast"
    )]
    let sample_count = raw.buf_length.max(0) as u32;

    Ok(ChannelMetadata {
        name,
        units,
        description: alloc::string::String::new(),
        frequency_divider,
        amplitude_scale: raw.ampl_scale,
        amplitude_offset: raw.ampl_offset,
        display_order: channel_index,
        sample_count,
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

    /// Build a minimal 112-byte channel header (`V_20a` base layout) with
    /// specified key values.  `nVarSampleDivider` is intentionally absent —
    /// it lives at version-dependent offsets beyond this struct and is passed
    /// directly to `parse_channel_metadata` in tests.
    fn make_chan_header_bytes(
        chan_header_len: i32,
        buf_length: i32,
        ampl_scale: f64,
        ampl_offset: f64,
        name: &[u8; 40],
        units: &[u8; 20],
    ) -> [u8; 112] {
        let mut bytes = [0u8; 112];
        bytes[0..4].copy_from_slice(&chan_header_len.to_le_bytes()); // lChanHeaderLen
        // offset 4-5: nNum = 0
        bytes[6..46].copy_from_slice(name); // szCommentText
        // offset 46-49: notColor = 0
        // offset 50-51: nDispChan = 0
        // offset 52-59: dVoltOffset = 0.0
        // offset 60-67: dVoltScale = 0.0
        bytes[68..88].copy_from_slice(units); // szUnitsText
        bytes[88..92].copy_from_slice(&buf_length.to_le_bytes()); // lBufLength
        bytes[92..100].copy_from_slice(&ampl_scale.to_le_bytes()); // dAmplScale
        bytes[100..108].copy_from_slice(&ampl_offset.to_le_bytes()); // dAmplOffset
        // offset 108-109: nChanOrder = 0
        // offset 110-111: nDispSize = 0
        bytes
    }

    #[test]
    fn channel_header_frequency_divider_2() -> Result<(), Box<dyn std::error::Error>> {
        let mut name = [0u8; 40];
        name[..3].copy_from_slice(b"ECG");
        let mut units = [0u8; 20];
        units[..2].copy_from_slice(b"mV");

        let raw_bytes = make_chan_header_bytes(252, 1000, 1.0, 0.0, &name, &units);
        let mut reader = Cursor::new(&raw_bytes[..]);
        let raw = ChannelHeaderRaw::read_le(&mut reader)?;
        assert_eq!(raw.buf_length, 1000);
        assert!((raw.ampl_scale - 1.0_f64).abs() < f64::EPSILON);

        // var_sample_divider=2 is passed explicitly; in a real file it would
        // be read by the caller from offset 152 (Post-4) or 250 (Pre-4).
        let meta = parse_channel_metadata(raw, 2, 0, 0)?;
        assert_eq!(meta.frequency_divider, 2);
        assert_eq!(meta.name, "ECG");
        assert_eq!(meta.units, "mV");
        assert!((meta.amplitude_scale - 1.0_f64).abs() < f64::EPSILON);
        assert!(meta.amplitude_offset.abs() < f64::EPSILON);
        Ok(())
    }

    #[test]
    fn channel_header_non_positive_divider_becomes_1() -> Result<(), Box<dyn std::error::Error>> {
        // var_sample_divider = 0 -> frequency_divider = 1 (base rate)
        let name = [0u8; 40];
        let units = [0u8; 20];
        let raw_bytes = make_chan_header_bytes(252, 500, 2.0, -1.0, &name, &units);
        let mut reader = Cursor::new(&raw_bytes[..]);
        let raw = ChannelHeaderRaw::read_le(&mut reader)?;
        let meta = parse_channel_metadata(raw, 0, 1, 0)?;
        assert_eq!(meta.frequency_divider, 1);
        Ok(())
    }

    #[test]
    fn channel_header_too_short_returns_error() -> Result<(), Box<dyn std::error::Error>> {
        // chan_header_len = 80 < CHANNEL_HEADER_MIN_LEN (112)
        let name = [0u8; 40];
        let units = [0u8; 20];
        let raw_bytes = make_chan_header_bytes(80, 0, 1.0, 0.0, &name, &units);
        let mut reader = Cursor::new(&raw_bytes[..]);
        let raw = ChannelHeaderRaw::read_le(&mut reader)?;
        let result = parse_channel_metadata(raw, 1, 0, 0x1A3C);
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

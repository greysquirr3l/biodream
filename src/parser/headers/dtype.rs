//! Channel data-type descriptor structs.
//!
//! Each channel has a 4-byte dtype header: `nSize` (u16) + `nType` (u16).
//!
//! Known `nType` values:
//! - `1` → `f64` (8 bytes per sample)
//! - `2` → `i16` (2 bytes per sample)

use binrw::binrw;

use crate::error::{BiopacError, HeaderSection, ParseError};

/// Sample data type for a channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleType {
    /// 64-bit IEEE 754 float (`nType = 1`), 8 bytes per sample.
    F64,
    /// 16-bit signed integer (`nType = 2`), 2 bytes per sample.
    I16,
}

impl SampleType {
    /// Size in bytes of one sample for this type.
    pub const fn byte_size(self) -> usize {
        match self {
            Self::F64 => 8,
            Self::I16 => 2,
        }
    }
}

/// Raw 4-byte channel dtype header — binary layout of the dtype record.
#[binrw]
#[derive(Debug, Copy, Clone)]
pub(super) struct ChannelDtypeRaw {
    /// Record size (unused; typically 4).
    pub n_size: u16,
    /// Type code: 1 = f64, 2 = i16.
    pub n_type: u16,
}

/// Convert a raw dtype record into a [`SampleType`], providing context for errors.
pub(super) fn parse_sample_type(
    raw: ChannelDtypeRaw,
    channel_index: u16,
    byte_offset: u64,
) -> Result<SampleType, BiopacError> {
    match raw.n_type {
        1 => Ok(SampleType::F64),
        2 => Ok(SampleType::I16),
        other => Err(BiopacError::Parse(ParseError {
            byte_offset,
            expected: alloc::string::String::from("1 (f64) or 2 (i16)"),
            actual: alloc::format!("{other}"),
            section: HeaderSection::ChannelDtype(channel_index),
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::boxed::Box;
    use binrw::BinRead;
    use std::io::Cursor;

    #[test]
    fn dtype_n_type_1_is_f64() -> Result<(), Box<dyn std::error::Error>> {
        // nSize=4 (LE u16), nType=1 (LE u16)
        let bytes: [u8; 4] = [4, 0, 1, 0];
        let mut reader = Cursor::new(&bytes[..]);
        let raw = ChannelDtypeRaw::read_le(&mut reader)?;
        assert_eq!(raw.n_type, 1);
        let st = parse_sample_type(raw, 0, 0)?;
        assert_eq!(st, SampleType::F64);
        assert_eq!(st.byte_size(), 8);
        Ok(())
    }

    #[test]
    fn dtype_n_type_2_is_i16() -> Result<(), Box<dyn std::error::Error>> {
        let bytes: [u8; 4] = [4, 0, 2, 0];
        let mut reader = Cursor::new(&bytes[..]);
        let raw = ChannelDtypeRaw::read_le(&mut reader)?;
        assert_eq!(raw.n_type, 2);
        let st = parse_sample_type(raw, 0, 0)?;
        assert_eq!(st, SampleType::I16);
        assert_eq!(st.byte_size(), 2);
        Ok(())
    }

    #[test]
    fn dtype_unknown_type_returns_parse_error() {
        let bytes: [u8; 4] = [4, 0, 99, 0]; // nType=99 — unknown
        let mut reader = Cursor::new(&bytes[..]);
        let raw = ChannelDtypeRaw::read_le(&mut reader);
        assert!(raw.is_ok(), "dtype struct should parse: {raw:?}");
        if let Ok(r) = raw {
            let result = parse_sample_type(r, 2, 0x100);
            assert!(result.is_err(), "unknown nType should be an error");
            if let Err(e) = result {
                let msg = alloc::format!("{e}");
                assert!(
                    msg.contains("ChannelDtype"),
                    "error should name section: {msg}"
                );
            }
        }
    }

    #[test]
    fn sample_type_byte_sizes() {
        assert_eq!(SampleType::F64.byte_size(), 8);
        assert_eq!(SampleType::I16.byte_size(), 2);
    }
}

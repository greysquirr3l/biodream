//! Foreign data section — opaque hardware-specific blob.
//!
//! The foreign data section follows the channel headers. Its length is given
//! by the `nLength` field (int32). Contents are acquisition-hardware-specific
//! and are stored as an opaque `Vec<u8>`.

use alloc::vec::Vec;

use binrw::binrw;

/// Raw foreign data section: a 4-byte length followed by that many bytes.
#[binrw]
#[derive(Debug)]
pub(super) struct ForeignDataRaw {
    /// Total byte count of the opaque payload (may be 0).
    pub n_length: i32,
    /// Opaque hardware-specific payload bytes.
    #[br(count = usize::try_from(n_length.max(0)).unwrap_or(0))]
    pub data: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::boxed::Box;
    use binrw::BinRead;
    use std::io::Cursor;

    #[test]
    fn foreign_data_empty() -> Result<(), Box<dyn std::error::Error>> {
        // nLength = 0 (LE i32)
        let bytes: [u8; 4] = [0, 0, 0, 0];
        let mut reader = Cursor::new(&bytes[..]);
        let raw = ForeignDataRaw::read_le(&mut reader)?;
        assert_eq!(raw.n_length, 0);
        assert!(raw.data.is_empty());
        Ok(())
    }

    #[test]
    fn foreign_data_with_payload() -> Result<(), Box<dyn std::error::Error>> {
        // nLength = 4 (LE), then 4 bytes of payload [0xDE, 0xAD, 0xBE, 0xEF]
        let bytes: [u8; 8] = [4, 0, 0, 0, 0xDE, 0xAD, 0xBE, 0xEF];
        let mut reader = Cursor::new(&bytes[..]);
        let raw = ForeignDataRaw::read_le(&mut reader)?;
        assert_eq!(raw.n_length, 4);
        assert_eq!(raw.data.len(), 4);
        assert_eq!(raw.data.first().copied(), Some(0xDE));
        Ok(())
    }

    #[test]
    fn foreign_data_negative_length_yields_empty() -> Result<(), Box<dyn std::error::Error>> {
        // nLength = -1 (LE i32) = [0xFF, 0xFF, 0xFF, 0xFF]
        let bytes: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF];
        let mut reader = Cursor::new(&bytes[..]);
        let raw = ForeignDataRaw::read_le(&mut reader)?;
        assert_eq!(raw.n_length, -1);
        // negative length -> max(0) = 0, so data is empty
        assert!(raw.data.is_empty());
        Ok(())
    }
}

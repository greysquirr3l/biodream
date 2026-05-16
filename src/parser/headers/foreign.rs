//! Foreign data section — opaque hardware-specific blob.
//!
//! The foreign data section follows the channel headers. `nLength` / `lLength`
//! is the **total** byte count of the section, including the 4-byte length
//! field itself. The opaque payload is therefore `max(nLength - 4, 0)` bytes.
//! Contents are acquisition-hardware-specific and not interpreted here.

use alloc::vec::Vec;

use binrw::binrw;

/// Raw foreign data section.
///
/// `n_length` is the **total** byte count of the section (including this
/// 4-byte field). The opaque payload is `max(n_length - 4, 0)` bytes.
#[binrw]
#[derive(Debug)]
pub(super) struct ForeignDataRaw {
    /// Total byte count of the section, including this 4-byte field.
    pub n_length: i32,
    /// Opaque hardware-specific payload (`n_length - 4` bytes; clamped to 0).
    #[br(count = usize::try_from((n_length - 4).max(0)).unwrap_or(0))]
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
        // nLength = 0 → payload = max(0 - 4, 0) = 0
        let bytes: [u8; 4] = [0, 0, 0, 0];
        let mut reader = Cursor::new(&bytes[..]);
        let raw = ForeignDataRaw::read_le(&mut reader)?;
        assert_eq!(raw.n_length, 0);
        assert!(raw.data.is_empty());
        Ok(())
    }

    #[test]
    fn foreign_data_header_only() -> Result<(), Box<dyn std::error::Error>> {
        // nLength = 4 (total = just the length field, no payload)
        let bytes: [u8; 4] = [4, 0, 0, 0];
        let mut reader = Cursor::new(&bytes[..]);
        let raw = ForeignDataRaw::read_le(&mut reader)?;
        assert_eq!(raw.n_length, 4);
        assert!(raw.data.is_empty());
        Ok(())
    }

    #[test]
    fn foreign_data_with_payload() -> Result<(), Box<dyn std::error::Error>> {
        // nLength = 8 (total: 4-byte length + 4-byte payload)
        let bytes: [u8; 8] = [8, 0, 0, 0, 0xDE, 0xAD, 0xBE, 0xEF];
        let mut reader = Cursor::new(&bytes[..]);
        let raw = ForeignDataRaw::read_le(&mut reader)?;
        assert_eq!(raw.n_length, 8);
        assert_eq!(raw.data.len(), 4);
        assert_eq!(raw.data.first().copied(), Some(0xDE));
        Ok(())
    }

    #[test]
    fn foreign_data_big_endian_post4_typical() -> Result<(), Box<dyn std::error::Error>> {
        // Post-4 big-endian: lLength = 8 → [0x00, 0x00, 0x00, 0x08] + 4 payload bytes
        let bytes: [u8; 8] = [0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00];
        let mut reader = Cursor::new(&bytes[..]);
        let raw = ForeignDataRaw::read_be(&mut reader)?;
        assert_eq!(raw.n_length, 8);
        assert_eq!(raw.data.len(), 4);
        Ok(())
    }

    #[test]
    fn foreign_data_negative_length_yields_empty() -> Result<(), Box<dyn std::error::Error>> {
        // nLength = -1 → max(-1 - 4, 0) = 0, so data is empty
        let bytes: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF];
        let mut reader = Cursor::new(&bytes[..]);
        let raw = ForeignDataRaw::read_le(&mut reader)?;
        assert_eq!(raw.n_length, -1);
        assert!(raw.data.is_empty());
        Ok(())
    }
}

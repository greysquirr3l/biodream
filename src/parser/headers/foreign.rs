//! Foreign data section — opaque hardware-specific blob.
//!
//! The foreign data section follows the channel headers. Its length is given
//! by `nLength` in the preceding foreign data header. Contents are
//! acquisition-hardware-specific and are stored as an opaque `Vec<u8>`.

// TODO(T04): implement binrw ForeignDataRaw.

//! Channel data-type descriptor structs.
//!
//! Each channel has a 4-byte dtype header: `nSize` (u16) + `nType` (u16).
//!
//! Known `nType` values:
//! - `1` → `f64` (8 bytes per sample)
//! - `2` → `i16` (2 bytes per sample)

// TODO(T04): implement binrw ChannelDtypeRaw and From → SampleType enum.

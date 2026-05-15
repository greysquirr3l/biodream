//! Binary parser for BIOPAC AcqKnowledge (.acq) files.
//!
//! # Module layout
//!
//! - [`headers`] — raw `binrw` structs mirroring the on-disk layout (T04)
//! - [`interleaved`] — streaming reader for uncompressed channel data (T05)
//! - [`markers`] — marker and journal section parser (T06)
//! - [`compressed`] — per-channel zlib decompression (T07)
//! - [`reader`] — high-level `read_file` / `read_stream` API (T09, requires `std`)

#[cfg(feature = "read")]
pub mod headers;
#[cfg(feature = "read")]
pub mod interleaved;

#[cfg(feature = "read")]
pub mod markers;

// Per-channel zlib decompression (T07) — requires `read` (= `std` + flate2)
#[cfg(feature = "read")]
pub mod compressed;

// High-level I/O API — only available with the `read` (= `std`) feature.
#[cfg(feature = "read")]
pub mod reader;

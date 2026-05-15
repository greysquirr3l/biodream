//! Binary parser for BIOPAC AcqKnowledge (.acq) files.
//!
//! # Module layout
//!
//! - `headers` — raw `binrw` structs mirroring the on-disk layout
//! - `interleaved` — streaming reader for uncompressed channel data
//! - `markers` — marker and journal section parser
//! - `compressed` — per-channel zlib decompression
//! - `reader` — high-level `read_file` / `read_stream` API (requires `read` feature)

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

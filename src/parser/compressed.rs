//! Compressed channel reader — per-channel zlib decompression.
//!
//! Compressed .acq file layout:
//! ```text
//! Graph Header → Channel Headers → Foreign Data → Channel Dtypes
//! → Marker Header → Markers → Journal
//! → [Compressed Channel 0] → … → [Compressed Channel N-1]
//! ```
//!
//! This is the reverse of uncompressed files where data precedes markers.
//!
//! Each compressed channel blob starts with a compression header (Pre4 and
//! Post4 variants differ), then the raw zlib-compressed samples.
//!
//! Decompressed data is always little-endian regardless of the file's byte
//! order flag. Each channel is decompressed independently using [`flate2`].

// TODO(T07): implement CompressedChannelReader.

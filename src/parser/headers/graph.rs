//! Graph (file) header structs — Pre4 and Post4 variants.
//!
//! # Format reference
//!
//! - Pre4 (revision < 68, `AcqKnowledge` < 4.0): fixed 256-byte header.
//! - Post4 (revision >= 68, `AcqKnowledge` >= 4.0): variable-length header;
//!   `lExtItemHeaderLen` stores the total header size.
//!
//! Field names follow the BIOPAC App Note 156 convention (lVersion, nChannels,
//! dSampleTime, …). All offsets are from the start of the file.
//!
//! Byte order is detected from `lVersion`: if the little-endian interpretation
//! yields a value in [30, 200], the file is LE; otherwise try BE.

// TODO(T04): implement binrw GraphHeaderPre4Raw and GraphHeaderPost4Raw structs
// with proper padding / seek_before attributes, and TryFrom → GraphMetadata.

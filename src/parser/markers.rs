//! Marker header and journal section parser.
//!
//! Layout (follows the channel data section in uncompressed files, and follows
//! channel dtype headers in compressed files):
//!
//! 1. Marker header: `lLength` (total marker section bytes), `lNumMarkers` (count).
//! 2. Per-marker record: `lSample`, `nChannel`, `szStyle[4]`, `lMarkerTextLen`,
//!    then `lMarkerTextLen` bytes of label text (may contain embedded NULLs).
//! 3. Journal: variable-length text or HTML following the marker section.
//!
//! The journal parser is fault-tolerant: corruption produces a [`Warning`](crate::error::Warning),
//! not an error, and the [`Datafile`](crate::domain::Datafile) is still returned.

// TODO(T06): implement MarkerSectionParser and JournalParser.

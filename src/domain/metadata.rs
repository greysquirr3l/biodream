//! File-level metadata types.

use core::fmt;

/// Byte order of a .acq file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ByteOrder {
    /// Intel byte order (little-endian).
    LittleEndian,
    /// Motorola byte order (big-endian).
    BigEndian,
}

/// BIOPAC file format revision number.
///
/// The revision number appears as `lVersion` in the graph header. Revisions
/// below 68 are "Pre-4" (Acq < 4.0); revisions >= 68 are "Post-4".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FileRevision(pub i32);

impl FileRevision {
    /// Construct a `FileRevision` from its raw integer value.
    #[inline]
    pub const fn new(revision: i32) -> Self {
        Self(revision)
    }

    /// Returns `true` if this file was written by `AcqKnowledge` < 4.0.
    ///
    /// Pre-4 files (revision < 68) use a fixed 256-byte graph header and lack
    /// per-channel compression support.
    #[inline]
    pub const fn is_pre_v4(self) -> bool {
        self.0 < 68
    }

    /// Returns `true` if this revision supports per-channel compression.
    ///
    /// Compression was introduced in `AcqKnowledge` 4.0 (revision 68).
    #[inline]
    pub const fn is_compressed_capable(self) -> bool {
        self.0 >= 68
    }

    /// Returns a human-readable version string for the given revision.
    ///
    /// Uses exclusive range patterns (stable since 1.80) for clean dispatch.
    pub const fn display_version(self) -> &'static str {
        match self.0 {
            ..30 => "unknown (<3.0)",
            30..35 => "3.0.x",
            35..38 => "3.5.x",
            38..41 => "3.7.x",
            41..45 => "3.7.3.x",
            45..60 => "3.x",
            60..62 => "3.8.x",
            62..68 => "3.9.x",
            68..70 => "4.0",
            70..73 => "4.1.x",
            73 => "4.1",
            74 => "4.2",
            75 => "4.3",
            76 => "4.3.1",
            77 => "4.4",
            78 => "4.4.1",
            79..84 => "4.4.x",
            84.. => "4.4.2+",
        }
    }
}

impl fmt::Display for FileRevision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "rev{} ({})", self.0, self.display_version())
    }
}

/// Top-level metadata extracted from the graph header.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GraphMetadata {
    /// Format revision from `lVersion`.
    pub file_revision: FileRevision,
    /// Samples per second at the base (highest) rate.
    pub samples_per_second: f64,
    /// Number of channels declared in the header.
    pub channel_count: u16,
    /// Byte order of the file.
    pub byte_order: ByteOrder,
    /// Whether the channel data is zlib-compressed.
    pub compressed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_revision_display_version_v84() {
        let rev = FileRevision::new(84);
        // Any non-empty string is "sensible" per the task spec.
        assert!(!rev.display_version().is_empty());
        assert!(rev.display_version().contains("4.4"));
    }

    #[test]
    fn file_revision_pre_v4_boundary() {
        assert!(FileRevision::new(38).is_pre_v4());
        // boundary: 67 < 68, still pre_v4; 68 is the first post_v4 revision
        assert!(FileRevision::new(67).is_pre_v4());
        assert!(!FileRevision::new(68).is_pre_v4());
    }
}

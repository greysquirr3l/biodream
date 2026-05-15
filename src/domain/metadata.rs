//! File-level metadata types.

use alloc::string::String;
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
            79..83 => "4.4.x",
            83 => "4.4.2",
            84.. => "5.x",
        }
    }
}

impl fmt::Display for FileRevision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "rev{} ({})", self.0, self.display_version())
    }
}

/// Recording date and time extracted from the Post-4 graph header.
///
/// Corresponds to the six `lSec`/`lMin`/`lHour`/`lDay`/`lMonth`/`lYear`
/// fields at offsets 276–299. Present only when `lExtItemHeaderLen >= 300`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AcquisitionDateTime {
    /// Full year (e.g. 2023).
    pub year: i32,
    /// Month, 1–12.
    pub month: i32,
    /// Day of month, 1–31.
    pub day: i32,
    /// Hour, 0–23.
    pub hour: i32,
    /// Minute, 0–59.
    pub minute: i32,
    /// Second, 0–59.
    pub second: i32,
}

impl fmt::Display for AcquisitionDateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
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
    /// Graph title (`szGraphTitle`, offset 236 in Post-4 header).
    ///
    /// `None` when the header is too short to contain the field (pre-4 files
    /// and early Post-4 files with `lExtItemHeaderLen < 276`).
    pub title: Option<String>,
    /// Recording date and time (`lSec`/`lMin`/`lHour`/`lDay`/`lMonth`/`lYear`).
    ///
    /// `None` for Pre-4 files or Post-4 files with `lExtItemHeaderLen < 300`.
    pub acquisition_datetime: Option<AcquisitionDateTime>,
    /// Maximum hardware sample rate in Hz (`lMaxAcqSamplesPerSec`, offset 1940).
    ///
    /// Present only in `AcqKnowledge` ≥ 4.2 (revision ≥ 74) files where
    /// `lExtItemHeaderLen ≥ 1944`. This is the hardware capability ceiling, not
    /// the actual recording rate (which comes from `dSampleTime`).
    pub max_samples_per_second: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_revision_display_version_v84() {
        let rev = FileRevision::new(84);
        assert!(!rev.display_version().is_empty());
        assert!(rev.display_version().contains('5'));
    }

    #[test]
    fn file_revision_display_version_v83_is_442() {
        assert_eq!(FileRevision::new(83).display_version(), "4.4.2");
    }

    #[test]
    fn file_revision_pre_v4_boundary() {
        assert!(FileRevision::new(38).is_pre_v4());
        assert!(FileRevision::new(67).is_pre_v4());
        assert!(!FileRevision::new(68).is_pre_v4());
    }

    #[test]
    fn acquisition_datetime_display() {
        let dt = AcquisitionDateTime {
            year: 2023,
            month: 6,
            day: 15,
            hour: 14,
            minute: 30,
            second: 5,
        };
        assert_eq!(alloc::format!("{dt}"), "2023-06-15 14:30:05");
    }

    #[test]
    fn graph_metadata_optional_fields_default_none() {
        let meta = GraphMetadata {
            file_revision: FileRevision::new(38),
            samples_per_second: 1000.0,
            channel_count: 1,
            byte_order: ByteOrder::LittleEndian,
            compressed: false,
            title: None,
            acquisition_datetime: None,
            max_samples_per_second: None,
        };
        assert!(meta.title.is_none());
        assert!(meta.acquisition_datetime.is_none());
        assert!(meta.max_samples_per_second.is_none());
    }

    #[test]
    fn graph_metadata_with_title_and_datetime() {
        let dt = AcquisitionDateTime {
            year: 2008,
            month: 3,
            day: 12,
            hour: 9,
            minute: 0,
            second: 0,
        };
        let meta = GraphMetadata {
            file_revision: FileRevision::new(74),
            samples_per_second: 2000.0,
            channel_count: 4,
            byte_order: ByteOrder::LittleEndian,
            compressed: false,
            title: Some(alloc::string::String::from("Stress Test")),
            acquisition_datetime: Some(dt),
            max_samples_per_second: Some(400_000),
        };
        assert_eq!(meta.title.as_deref(), Some("Stress Test"));
        assert_eq!(meta.acquisition_datetime.map(|dt| dt.year), Some(2008));
        assert_eq!(meta.max_samples_per_second, Some(400_000));
    }
}

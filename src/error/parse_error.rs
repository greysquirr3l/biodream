//! [`ParseError`] and [`HeaderSection`] — structural parse failures.

use alloc::string::String;
use core::fmt;

use thiserror::Error;

/// Which section of the .acq file was being parsed when an error occurred.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum HeaderSection {
    /// Top-level graph (file) header.
    Graph,
    /// Channel header for the given channel index.
    Channel(u16),
    /// Foreign data / hardware-specific blob.
    Foreign,
    /// Channel data-type descriptor for the given channel index.
    ChannelDtype(u16),
    /// Marker header or individual marker record.
    Marker,
    /// Journal section.
    Journal,
    /// Raw sample data section.
    Data,
    /// Compressed data blob for the given channel index.
    Compressed(u16),
}

impl fmt::Display for HeaderSection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Graph => write!(f, "Graph"),
            Self::Channel(n) => write!(f, "Channel({n})"),
            Self::Foreign => write!(f, "Foreign"),
            Self::ChannelDtype(n) => write!(f, "ChannelDtype({n})"),
            Self::Marker => write!(f, "Marker"),
            Self::Journal => write!(f, "Journal"),
            Self::Data => write!(f, "Data"),
            Self::Compressed(n) => write!(f, "Compressed({n})"),
        }
    }
}

/// A structural parse failure with byte offset and diagnostic context.
///
/// Error messages are grep-friendly:
/// ```text
/// parse error at byte 0x1A3C in Channel(2) header:
///     expected nChanHeaderLen >= 252, got 180
/// ```
#[derive(Debug, Error)]
#[error(
    "parse error at byte 0x{byte_offset:X} in {section} header: \
     expected {expected}, got {actual}"
)]
pub struct ParseError {
    /// Byte offset in the file where the error was detected.
    pub byte_offset: u64,
    /// What the parser expected to find.
    pub expected: String,
    /// What was actually found.
    pub actual: String,
    /// Which header section was being parsed.
    pub section: HeaderSection,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_error_display_includes_hex_offset() {
        let e = ParseError {
            byte_offset: 0x1A3C,
            expected: String::from("nChanHeaderLen >= 252"),
            actual: String::from("180"),
            section: HeaderSection::Channel(2),
        };
        let s = alloc::format!("{e}");
        assert!(s.contains("0x1A3C"), "should contain hex offset: {s}");
        assert!(s.contains("Channel(2)"), "should name the section: {s}");
        assert!(s.contains("252"), "should mention expected: {s}");
        assert!(s.contains("180"), "should mention actual: {s}");
    }

    #[test]
    fn all_section_variants_display() {
        let sections = [
            HeaderSection::Graph,
            HeaderSection::Channel(0),
            HeaderSection::Foreign,
            HeaderSection::ChannelDtype(3),
            HeaderSection::Marker,
            HeaderSection::Journal,
            HeaderSection::Data,
            HeaderSection::Compressed(1),
        ];
        for s in &sections {
            // Must not panic.
            let _ = alloc::format!("{s}");
        }
    }
}

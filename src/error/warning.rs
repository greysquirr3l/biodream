//! Non-fatal [`Warning`] type for parse anomalies.

use alloc::string::String;
use core::fmt;

use crate::error::HeaderSection;

/// A non-fatal issue encountered during parsing.
///
/// Warnings are collected alongside a successful [`crate::error::ParseResult`]
/// rather than causing the parse to fail. They indicate data quality issues or
/// forward-compatibility unknowns that the caller may want to act on.
///
/// Examples:
/// - An unknown header field was skipped.
/// - The journal section was truncated or malformed (data was still returned).
/// - A marker label contained unexpected encoding.
/// - Extra trailing bytes were found after the expected end of section.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Warning {
    /// Human-readable description.
    pub message: String,
    /// The section in which the warning was generated, if known.
    pub section: Option<HeaderSection>,
    /// File byte offset at which the warning was generated, if known.
    pub byte_offset: Option<u64>,
}

impl Warning {
    /// Construct a warning with a message and no location context.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            section: None,
            byte_offset: None,
        }
    }

    /// Attach section context to this warning.
    #[must_use]
    pub const fn in_section(mut self, section: HeaderSection) -> Self {
        self.section = Some(section);
        self
    }

    /// Attach a byte offset to this warning.
    #[must_use]
    pub const fn at_offset(mut self, offset: u64) -> Self {
        self.byte_offset = Some(offset);
        self
    }
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "warning")?;
        if let Some(ref sec) = self.section {
            write!(f, " in {sec}")?;
        }
        if let Some(off) = self.byte_offset {
            write!(f, " at 0x{off:X}")?;
        }
        write!(f, ": {}", self.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warning_display_minimal() {
        let w = Warning::new("unknown field skipped");
        let s = alloc::format!("{w}");
        assert!(s.contains("unknown field skipped"));
    }

    #[test]
    fn warning_display_with_section_and_offset() {
        let w = Warning::new("truncated journal")
            .in_section(HeaderSection::Journal)
            .at_offset(0xFF00);
        let s = alloc::format!("{w}");
        assert!(s.contains("Journal"), "section in display: {s}");
        assert!(s.contains("0xFF00"), "offset in display: {s}");
    }
}

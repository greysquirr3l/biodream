//! Marker types — event annotations on physiological recordings.

use alloc::string::String;
use core::fmt;

/// Unix timestamp (seconds since 1970-01-01 00:00:00 UTC).
///
/// BIOPAC uses 1970-01-01 as its epoch reference (`REF_DATE` in bioread).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Timestamp(pub i64);

impl Timestamp {
    /// Construct a timestamp from seconds since the Unix epoch.
    #[inline]
    pub const fn from_secs(secs: i64) -> Self {
        Self(secs)
    }

    /// Return the number of seconds since the Unix epoch.
    #[inline]
    pub const fn as_secs(self) -> i64 {
        self.0
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}s (unix)", self.0)
    }
}

/// Marker style, derived from the 4-character style code in the file.
///
/// Known codes from bioread:
/// - `"apnd"` → [`Append`](MarkerStyle::Append)
/// - `"usr1"`–`"usr8"` → [`UserEvent`](MarkerStyle::UserEvent)
/// - `"wave"` → [`Waveform`](MarkerStyle::Waveform)
/// - `"glbl"` → [`GlobalEvent`](MarkerStyle::GlobalEvent)
/// - anything else → [`Unknown`](MarkerStyle::Unknown)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum MarkerStyle {
    /// Segment append / splice point (`"apnd"`).
    Append,
    /// User-defined event (`"usr1"`–`"usr8"`).
    UserEvent,
    /// Waveform marker (`"wave"`).
    Waveform,
    /// Global event marker (`"glbl"`).
    GlobalEvent,
    /// Style code not recognised by this library.
    Unknown(String),
}

impl MarkerStyle {
    /// Parse a 4-character style code from the marker header.
    pub fn from_code(code: &str) -> Self {
        match code.trim_end_matches('\0') {
            "apnd" => Self::Append,
            s if s.starts_with("usr") => Self::UserEvent,
            "wave" => Self::Waveform,
            "glbl" => Self::GlobalEvent,
            other => Self::Unknown(other.into()),
        }
    }
}

impl fmt::Display for MarkerStyle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Append => write!(f, "Append"),
            Self::UserEvent => write!(f, "UserEvent"),
            Self::Waveform => write!(f, "Waveform"),
            Self::GlobalEvent => write!(f, "GlobalEvent"),
            Self::Unknown(s) => write!(f, "Unknown({s})"),
        }
    }
}

/// A single event marker on the timeline of a recording.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Marker {
    /// Human-readable label (from marker text).
    pub label: String,
    /// Global sample index at which this marker occurs.
    pub global_sample_index: usize,
    /// Which channel this marker is attached to, or `None` for a global marker.
    ///
    /// Corresponds to `nChannel == -1` in the file (global) or the channel's
    /// display-order index.
    pub channel: Option<usize>,
    /// Visual style of the marker.
    pub style: MarkerStyle,
    /// Timestamp when the marker was created (v4.4.0+ files only).
    pub created_at: Option<Timestamp>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_style_apnd() {
        assert_eq!(MarkerStyle::from_code("apnd"), MarkerStyle::Append);
    }

    #[test]
    fn marker_style_user_events() {
        assert_eq!(MarkerStyle::from_code("usr1"), MarkerStyle::UserEvent);
        assert_eq!(MarkerStyle::from_code("usr8"), MarkerStyle::UserEvent);
    }

    #[test]
    fn marker_style_unknown() {
        let s = MarkerStyle::from_code("xyzw");
        assert!(matches!(s, MarkerStyle::Unknown(_)));
    }

    #[test]
    fn marker_style_unknown_with_null_padding() {
        // Style codes from the file may be null-padded.
        let s = MarkerStyle::from_code("apnd\0");
        assert_eq!(s, MarkerStyle::Append);
    }

    #[test]
    fn timestamp_round_trip() {
        let ts = Timestamp::from_secs(1_700_000_000);
        assert_eq!(ts.as_secs(), 1_700_000_000);
    }
}

//! Journal type — free-text or HTML annotation attached to a recording.

use alloc::string::String;
use core::fmt;

/// The journal section of a .acq file.
///
/// In `AcqKnowledge` < 4.2, the journal is plain text. From 4.2 onwards it is an
/// HTML document. Callers can distinguish the two with [`Journal::as_html`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Journal {
    /// Plain-text journal (pre-v4.2 files).
    Plain(String),
    /// HTML journal (v4.2+ files).
    Html(String),
}

impl Journal {
    /// Return the journal content as a string slice regardless of encoding.
    pub const fn as_text(&self) -> &str {
        match self {
            Self::Plain(s) | Self::Html(s) => s.as_str(),
        }
    }

    /// Return `Some(&str)` if this journal is HTML, `None` if plain text.
    pub const fn as_html(&self) -> Option<&str> {
        match self {
            Self::Html(s) => Some(s.as_str()),
            Self::Plain(_) => None,
        }
    }

    /// Returns `true` if the journal is stored as HTML.
    pub const fn is_html(&self) -> bool {
        matches!(self, Self::Html(_))
    }
}

impl fmt::Display for Journal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Plain(s) => write!(f, "Journal::Plain({} chars)", s.len()),
            Self::Html(s) => write!(f, "Journal::Html({} chars)", s.len()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_journal_as_text() {
        let j = Journal::Plain(String::from("experiment notes"));
        assert_eq!(j.as_text(), "experiment notes");
        assert!(j.as_html().is_none());
        assert!(!j.is_html());
    }

    #[test]
    fn html_journal_as_html() {
        let j = Journal::Html(String::from("<html><body>notes</body></html>"));
        assert!(j.as_html().is_some());
        assert!(j.is_html());
        assert_eq!(j.as_text(), "<html><body>notes</body></html>");
    }
}

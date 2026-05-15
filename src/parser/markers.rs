//! Marker header and journal section parser.
//!
//! Layout (follows the channel data section in uncompressed files, and follows
//! channel dtype headers in compressed files):
//!
//! 1. Marker header: `lLength` (total marker section bytes), `lNumMarkers` (count).
//! 2. Per-marker record: `lSample`, `nChannel`, `szStyle[4]`, `lMarkerTextLen`,
//!    then `lMarkerTextLen` bytes of label text (may contain embedded NULLs).
//!    In v4.4.0+ (revision ≥ 77) an additional 8-byte i64 timestamp follows.
//! 3. Journal: `lJournalLen` (i32) followed by that many bytes of text.
//!
//! The journal parser is fault-tolerant: corruption produces a
//! [`Warning`](crate::error::Warning), not an error, and the
//! [`Datafile`](crate::domain::Datafile) is still returned.

use alloc::{string::String, vec, vec::Vec};
use std::io::{Read, Seek};

use crate::{
    domain::{Journal, Marker, MarkerStyle, Timestamp},
    error::{BiopacError, Warning},
};

/// Minimum revision that stores a per-marker creation timestamp.
const REVISION_TIMESTAMP: i32 = 77;

/// Size of the fixed marker section header (lLength + lNumMarkers).
const MARKER_HDR_BYTES: i32 = 8;

/// Size of the fixed per-marker record fields before the variable text.
/// lSample(4) + nChannel(2) + szStyle(4) + lMarkerTextLen(4) = 14
const MARKER_FIXED_BYTES: usize = 14;

/// Extra bytes added per-marker in v4.4.0+ for the creation timestamp.
const MARKER_TIMESTAMP_BYTES: usize = 8;

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn read_i16_le<R: Read>(r: &mut R) -> Result<i16, BiopacError> {
    let mut buf = [0u8; 2];
    r.read_exact(&mut buf).map_err(BiopacError::Io)?;
    Ok(i16::from_le_bytes(buf))
}

fn read_i32_le<R: Read>(r: &mut R) -> Result<i32, BiopacError> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf).map_err(BiopacError::Io)?;
    Ok(i32::from_le_bytes(buf))
}

fn read_i64_le<R: Read>(r: &mut R) -> Result<i64, BiopacError> {
    let mut buf = [0u8; 8];
    r.read_exact(&mut buf).map_err(BiopacError::Io)?;
    Ok(i64::from_le_bytes(buf))
}

fn read_bytes<R: Read>(r: &mut R, n: usize) -> Result<Vec<u8>, BiopacError> {
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf).map_err(BiopacError::Io)?;
    Ok(buf)
}

/// Decode raw bytes to a String: try UTF-8, fall back to Latin-1.
/// Text is terminated at the first NUL byte if present.
fn decode_text(bytes: &[u8]) -> String {
    // Trim at first NUL.
    let trimmed = bytes
        .iter()
        .position(|&b| b == 0)
        .map_or(bytes, |pos| &bytes[..pos]);

    if let Ok(s) = core::str::from_utf8(trimmed) {
        s.into()
    } else {
        // Latin-1 fallback: each byte maps directly to the Unicode code point.
        trimmed.iter().map(|&b| char::from(b)).collect()
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Result of parsing the marker + journal sections.
pub(crate) struct MarkersAndJournal {
    pub markers: Vec<Marker>,
    pub journal: Option<Journal>,
    pub warnings: Vec<Warning>,
}

/// Parse the marker section and optional journal from the current reader position.
///
/// `file_revision` controls whether per-marker timestamps are read.
/// `channel_display_orders` maps channel data-index → display order; the
/// marker's `nChannel` value is a display-order index, so this slice is used
/// to find the correct channel position.
///
/// If the journal section is corrupt or missing, a warning is emitted and
/// `journal` is `None`.
pub(crate) fn parse_markers_and_journal<R: Read + Seek>(
    reader: &mut R,
    file_revision: i32,
    channel_display_orders: &[u16],
) -> Result<MarkersAndJournal, BiopacError> {
    let mut warnings: Vec<Warning> = Vec::new();

    // --- Marker section header ---------------------------------------------
    let total_length = read_i32_le(reader)?; // includes the 8-byte header itself
    let num_markers = read_i32_le(reader)?;

    if num_markers < 0 {
        return Err(BiopacError::Validation(alloc::format!(
            "invalid lNumMarkers: {num_markers}"
        )));
    }

    #[expect(
        clippy::cast_sign_loss,
        reason = "num_markers checked >= 0 above"
    )]
    let num_markers = num_markers as usize;

    // Compute the byte offset at which the journal begins.
    // total_length includes the 8-byte header. We've read 8 bytes so far.
    let has_timestamp = file_revision >= REVISION_TIMESTAMP;

    // --- Parse individual markers ------------------------------------------
    let mut markers = Vec::with_capacity(num_markers);
    let mut bytes_consumed: i32 = MARKER_HDR_BYTES;

    for _ in 0..num_markers {
        let sample = read_i32_le(reader)?;
        let n_channel = read_i16_le(reader)?;
        let style_bytes = read_bytes(reader, 4)?;
        let text_len = read_i32_le(reader)?;

        if text_len < 0 {
            return Err(BiopacError::Validation(alloc::format!(
                "invalid lMarkerTextLen: {text_len}"
            )));
        }
        #[expect(
            clippy::cast_sign_loss,
            reason = "text_len checked >= 0 above"
        )]
        let text_len = text_len as usize;

        let text_bytes = read_bytes(reader, text_len)?;

        let created_at = if has_timestamp {
            let ts = read_i64_le(reader)?;
            Some(Timestamp::from_secs(ts))
        } else {
            None
        };

        // style_bytes is always ASCII; decode as Latin-1 for safety.
        let style_code: String = style_bytes.iter().map(|&b| char::from(b)).collect();
        let style = MarkerStyle::from_code(&style_code);
        let label = decode_text(&text_bytes);

        // Map nChannel: -1 = global (None), otherwise find the channel with
        // the matching display_order.
        let channel = if n_channel < 0 {
            None
        } else {
            #[expect(
                clippy::cast_sign_loss,
                reason = "n_channel >= 0 checked by the if branch"
            )]
            let display_order = n_channel as u16;
            channel_display_orders
                .iter()
                .position(|&d| d == display_order)
        };

        #[expect(
            clippy::cast_sign_loss,
            reason = "sample is a byte index/sample index; negative values map to 0"
        )]
        let global_sample_index = sample.max(0) as usize;

        markers.push(Marker {
            label,
            global_sample_index,
            channel,
            style,
            created_at,
        });

        // Track bytes consumed to know where the journal starts.
        #[expect(
            clippy::cast_possible_truncation,
            reason = "text_len fits in i32 in practice (marker text is tiny)"
        )]
        {
            bytes_consumed +=
                MARKER_FIXED_BYTES as i32 + text_len as i32;
            if has_timestamp {
                bytes_consumed += MARKER_TIMESTAMP_BYTES as i32;
            }
        }
    }

    // Consume any padding between end-of-markers and journal.
    // total_length is measured from the start of the marker section header.
    let remaining = total_length - bytes_consumed;
    if remaining > 0 {
        #[expect(
            clippy::cast_sign_loss,
            reason = "remaining checked > 0 above"
        )]
        let skip = remaining as u64;
        if let Err(e) = std::io::copy(&mut reader.by_ref().take(skip), &mut std::io::sink()) {
            warnings.push(Warning::new(alloc::format!(
                "Failed to skip {skip} padding bytes after markers: {e}"
            )));
        }
    } else if remaining < 0 {
        warnings.push(Warning::new(alloc::format!(
            "Marker section overrun by {} bytes; journal position may be wrong",
            -remaining
        )));
    }

    // --- Journal -----------------------------------------------------------
    let journal = match parse_journal(reader) {
        Ok(j) => j,
        Err(e) => {
            warnings.push(Warning::new(alloc::format!(
                "Journal parse failed (recording data still intact): {e}"
            )));
            None
        }
    };

    Ok(MarkersAndJournal {
        markers,
        journal,
        warnings,
    })
}

fn parse_journal<R: Read>(reader: &mut R) -> Result<Option<Journal>, BiopacError> {
    // Journal length prefix
    let len = match read_i32_le(reader) {
        Ok(n) => n,
        Err(_) => return Ok(None), // EOF before journal — fine
    };

    if len <= 0 {
        return Ok(None);
    }

    #[expect(
        clippy::cast_sign_loss,
        reason = "len checked > 0 above"
    )]
    let len = len as usize;

    let bytes = read_bytes(reader, len)?;
    let text = decode_text(&bytes);

    // Detect HTML by presence of a DOCTYPE or opening <html tag.
    let lower_head: String = text
        .chars()
        .take(100)
        .flat_map(char::to_lowercase)
        .collect();
    let is_html = lower_head.contains("<!doctype") || lower_head.contains("<html");

    if is_html {
        Ok(Some(Journal::Html(text)))
    } else {
        Ok(Some(Journal::Plain(text)))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{boxed::Box, vec::Vec};
    use std::io::Cursor;

    fn write_i16_le(v: i16) -> [u8; 2] {
        v.to_le_bytes()
    }
    fn write_i32_le(v: i32) -> [u8; 4] {
        v.to_le_bytes()
    }
    fn write_i64_le(v: i64) -> [u8; 8] {
        v.to_le_bytes()
    }

    /// Build a complete marker section for testing.
    ///
    /// `markers`: list of (sample, channel, style, text).
    /// `file_revision`: controls whether timestamps are written.
    fn build_marker_section(
        markers: &[(i32, i16, &str, &str, Option<i64>)],
        file_revision: i32,
    ) -> Vec<u8> {
        let has_ts = file_revision >= REVISION_TIMESTAMP;
        let mut body = Vec::<u8>::new();
        for &(sample, channel, style, text, ts) in markers {
            body.extend_from_slice(&write_i32_le(sample));
            body.extend_from_slice(&write_i16_le(channel));
            // style is 4 bytes, padded with NUL
            let mut style_bytes = [0u8; 4];
            for (i, b) in style.bytes().take(4).enumerate() {
                style_bytes[i] = b;
            }
            body.extend_from_slice(&style_bytes);
            #[expect(
                clippy::cast_possible_truncation,
                reason = "test text is tiny"
            )]
            body.extend_from_slice(&write_i32_le(text.len() as i32));
            body.extend_from_slice(text.as_bytes());
            if has_ts {
                body.extend_from_slice(&write_i64_le(ts.unwrap_or(0)));
            }
        }
        let total = MARKER_HDR_BYTES + body.len() as i32;
        let mut out = Vec::<u8>::new();
        out.extend_from_slice(&write_i32_le(total));
        #[expect(
            clippy::cast_possible_truncation,
            reason = "test marker count is tiny"
        )]
        out.extend_from_slice(&write_i32_le(markers.len() as i32));
        out.extend_from_slice(&body);
        out
    }

    fn build_journal_section(text: &str) -> Vec<u8> {
        let mut out = Vec::<u8>::new();
        #[expect(
            clippy::cast_possible_truncation,
            reason = "test journal text is small"
        )]
        out.extend_from_slice(&write_i32_le(text.len() as i32));
        out.extend_from_slice(text.as_bytes());
        out
    }

    // -----------------------------------------------------------------------
    // Marker parsing tests
    // -----------------------------------------------------------------------

    #[test]
    fn three_markers_global_and_channel() -> Result<(), Box<dyn std::error::Error>> {
        // 1 global marker + 2 channel-specific
        let items = [
            (100i32, -1i16, "apnd", "start", None),
            (200i32, 0i16, "usr1", "event A", None),
            (300i32, 1i16, "glbl", "end", None),
        ];
        let rev = 73; // pre-4.4, no timestamps
        let mut bytes = build_marker_section(&items, rev);
        bytes.extend_from_slice(&build_journal_section(""));

        let display_orders = [0u16, 1u16];
        let mut cur = Cursor::new(&bytes[..]);
        let result = parse_markers_and_journal(&mut cur, rev, &display_orders)?;

        assert_eq!(result.markers.len(), 3);
        let m0 = result.markers.first().ok_or("missing marker 0")?;
        let m1 = result.markers.get(1).ok_or("missing marker 1")?;
        let m2 = result.markers.get(2).ok_or("missing marker 2")?;

        assert_eq!(m0.channel, None);
        assert_eq!(m0.global_sample_index, 100);
        assert_eq!(m0.style, MarkerStyle::Append);
        assert_eq!(m0.label, "start");

        assert_eq!(m1.channel, Some(0));
        assert_eq!(m1.global_sample_index, 200);
        assert_eq!(m1.style, MarkerStyle::UserEvent);

        assert_eq!(m2.channel, Some(1));
        assert_eq!(m2.global_sample_index, 300);
        assert_eq!(m2.style, MarkerStyle::GlobalEvent);
        Ok(())
    }

    #[test]
    fn global_marker_maps_to_none() -> Result<(), Box<dyn std::error::Error>> {
        let items = [(50i32, -1i16, "apnd", "seg", None)];
        let mut bytes = build_marker_section(&items, 73);
        bytes.extend_from_slice(&build_journal_section(""));

        let mut cur = Cursor::new(&bytes[..]);
        let result = parse_markers_and_journal(&mut cur, 73, &[0u16])?;
        let m = result.markers.first().ok_or("missing marker")?;
        assert_eq!(m.channel, None);
        Ok(())
    }

    #[test]
    fn marker_text_with_embedded_null() -> Result<(), Box<dyn std::error::Error>> {
        // Text "hello\0world" — should be trimmed to "hello"
        let text = "hello\0world";
        let items = [(0i32, -1i16, "apnd", text, None)];
        let mut bytes = build_marker_section(&items, 73);
        bytes.extend_from_slice(&build_journal_section(""));

        let mut cur = Cursor::new(&bytes[..]);
        let result = parse_markers_and_journal(&mut cur, 73, &[])?;
        let m = result.markers.first().ok_or("missing marker")?;
        assert_eq!(m.label, "hello");
        Ok(())
    }

    #[test]
    fn marker_created_at_timestamp_v44() -> Result<(), Box<dyn std::error::Error>> {
        let epoch_secs = 1_700_000_000i64;
        let items = [(0i32, -1i16, "apnd", "ts", Some(epoch_secs))];
        let mut bytes = build_marker_section(&items, 77); // rev 77 = v4.4
        bytes.extend_from_slice(&build_journal_section(""));

        let mut cur = Cursor::new(&bytes[..]);
        let result = parse_markers_and_journal(&mut cur, 77, &[])?;
        let m = result.markers.first().ok_or("missing marker")?;
        assert_eq!(m.created_at, Some(Timestamp::from_secs(epoch_secs)));
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Journal tests
    // -----------------------------------------------------------------------

    #[test]
    fn plain_text_journal() -> Result<(), Box<dyn std::error::Error>> {
        let mut bytes = build_marker_section(&[], 73);
        bytes.extend_from_slice(&build_journal_section("notes about session"));

        let mut cur = Cursor::new(&bytes[..]);
        let result = parse_markers_and_journal(&mut cur, 73, &[])?;
        match result.journal.ok_or("no journal")? {
            Journal::Plain(s) => assert_eq!(s, "notes about session"),
            Journal::Html(_) => return Err("expected plain".into()),
        }
        Ok(())
    }

    #[test]
    fn html_journal_detected() -> Result<(), Box<dyn std::error::Error>> {
        let html = "<!DOCTYPE html><html><body>notes</body></html>";
        let mut bytes = build_marker_section(&[], 74);
        bytes.extend_from_slice(&build_journal_section(html));

        let mut cur = Cursor::new(&bytes[..]);
        let result = parse_markers_and_journal(&mut cur, 74, &[])?;
        assert!(result.journal.ok_or("no journal")?.is_html());
        Ok(())
    }

    #[test]
    fn corrupted_journal_produces_warning() -> Result<(), Box<dyn std::error::Error>> {
        // marker section is fine, but journal length claims more bytes than available
        let mut bytes = build_marker_section(&[], 73);
        // Write journal length of 1000 but provide 0 bytes
        bytes.extend_from_slice(&write_i32_le(1000));

        let mut cur = Cursor::new(&bytes[..]);
        let result = parse_markers_and_journal(&mut cur, 73, &[])?;
        assert!(result.journal.is_none());
        assert!(
            !result.warnings.is_empty(),
            "expected a warning for corrupt journal"
        );
        Ok(())
    }

    #[test]
    fn empty_marker_section_no_journal() -> Result<(), Box<dyn std::error::Error>> {
        // Build an 8-byte marker header with 0 markers and no journal data.
        let mut bytes = Vec::<u8>::new();
        bytes.extend_from_slice(&write_i32_le(8)); // total length = just the header
        bytes.extend_from_slice(&write_i32_le(0)); // 0 markers
        // No journal — EOF

        let mut cur = Cursor::new(&bytes[..]);
        let result = parse_markers_and_journal(&mut cur, 73, &[])?;
        assert!(result.markers.is_empty());
        assert!(result.journal.is_none());
        Ok(())
    }
}


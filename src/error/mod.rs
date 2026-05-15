//! Typed error hierarchy for biodream.
//!
//! All errors carry enough context to diagnose corrupt or malformed `.acq`
//! files: byte offsets, expected-vs-actual values, and which header section
//! failed. This is the primary improvement over bioread, which silently swallows
//! many format errors.
//!
//! # `no_std`
//!
//! All error types compile under `#![no_std]` + `alloc`. When the `std`
//! feature is enabled, `BiopacError` also carries a variant for `std::io::Error`.

use alloc::{string::String, vec::Vec};

use thiserror::Error;

// Re-export the main types at crate root.
pub use parse_error::{HeaderSection, ParseError};
pub use warning::Warning;

mod parse_error;
mod warning;

/// Top-level library error.
///
/// All public API functions that can fail return `Result<T, BiopacError>`.
/// [`ParseError`] and [`CompressionError`] variants implement `From` so that
/// the `?` operator can be used throughout the parser.
///
/// The `Send + Sync` bound is verified by a compile-time assertion below.
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum BiopacError {
    /// A structural parse failure with a byte offset and diagnostic context.
    #[error("{0}")]
    Parse(#[from] ParseError),

    /// A standard I/O error (only available with the `std` feature).
    #[cfg(feature = "std")]
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// zlib decompression failure on a compressed channel.
    #[error("{0}")]
    Compression(#[from] CompressionError),

    /// The file's format revision is outside the supported range.
    #[error("{0}")]
    UnsupportedVersion(#[from] UnsupportedVersionError),

    /// A referenced channel index is out of bounds.
    #[error("invalid channel: {0}")]
    InvalidChannel(String),

    /// A general validation failure (e.g. inconsistent header counts).
    #[error("validation error: {0}")]
    Validation(String),

    /// An Apache Arrow error during schema construction or IPC serialisation.
    ///
    /// Only available with the `arrow` or `parquet` feature.
    #[cfg(any(feature = "arrow", feature = "parquet"))]
    #[error("arrow error: {0}")]
    Arrow(#[from] arrow_schema::ArrowError),

    /// A Parquet serialisation error.
    ///
    /// Only available with the `parquet` feature.
    #[cfg(feature = "parquet")]
    #[error("parquet error: {0}")]
    Parquet(#[from] parquet::errors::ParquetError),
}

// Compile-time assertion: BiopacError is Send + Sync so it can flow across
// async task boundaries and thread channels.
const _: fn() = || {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<BiopacError>();
};

/// Compression failure on a specific channel.
#[derive(Debug, Error)]
#[error("compression error in channel {channel_index}: {message}")]
pub struct CompressionError {
    /// Zero-based channel index.
    pub channel_index: u16,
    /// Human-readable error description.
    pub message: String,
}

/// The file's revision is outside the range this library supports.
#[derive(Debug, Error)]
#[error(
    "unsupported file revision {revision} \
     (supported range: {min_supported}..={max_supported})"
)]
pub struct UnsupportedVersionError {
    /// Actual revision found in the file.
    pub revision: i32,
    /// Lowest revision this build supports.
    pub min_supported: i32,
    /// Highest revision this build supports.
    pub max_supported: i32,
}

/// A successful parse result bundled with any non-fatal warnings.
///
/// Warnings represent issues that did not prevent a result from being produced:
/// unknown header fields, truncated journals, out-of-range values that were
/// clamped, and so on. Callers that care about data integrity should inspect
/// the `warnings` list.
#[derive(Debug)]
pub struct ParseResult<T> {
    /// The successfully parsed value.
    pub value: T,
    /// Non-fatal issues encountered during parsing.
    pub warnings: Vec<Warning>,
}

impl<T> ParseResult<T> {
    /// Wrap `value` with an empty warning list.
    pub const fn ok(value: T) -> Self {
        Self {
            value,
            warnings: Vec::new(),
        }
    }

    /// Returns `true` if no warnings were emitted.
    pub const fn is_clean(&self) -> bool {
        self.warnings.is_empty()
    }

    /// Discard warnings and return just the value.
    pub fn into_value(self) -> T {
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compression_error_display() {
        let e = CompressionError {
            channel_index: 2,
            message: String::from("unexpected end of stream"),
        };
        let s = alloc::format!("{e}");
        assert!(s.contains("channel 2"));
        assert!(s.contains("unexpected end of stream"));
    }

    #[test]
    fn unsupported_version_display() {
        let e = UnsupportedVersionError {
            revision: 25,
            min_supported: 30,
            max_supported: 84,
        };
        let s = alloc::format!("{e}");
        assert!(s.contains("25"));
        assert!(s.contains("30"));
        assert!(s.contains("84"));
    }

    #[test]
    fn all_variants_are_constructable() {
        let _p = BiopacError::Parse(ParseError {
            byte_offset: 0x1A3C,
            expected: String::from("nChanHeaderLen >= 252"),
            actual: String::from("180"),
            section: HeaderSection::Channel(2),
        });
        let _c = BiopacError::Compression(CompressionError {
            channel_index: 0,
            message: String::from("test"),
        });
        drop(BiopacError::UnsupportedVersion(UnsupportedVersionError {
            revision: 20,
            min_supported: 30,
            max_supported: 84,
        }));
        let _i = BiopacError::InvalidChannel(String::from("out of range"));
        let _v = BiopacError::Validation(String::from("inconsistent channel count"));
    }

    #[test]
    fn parse_result_ok_is_clean() {
        let r: ParseResult<u32> = ParseResult::ok(42);
        assert!(r.is_clean());
        assert_eq!(r.into_value(), 42);
    }
}

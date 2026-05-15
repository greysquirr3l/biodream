//! Parquet export (requires `parquet` feature, builds on `arrow`).
//!
//! Writes a single row-group Parquet file from a [`crate::domain::Datafile`].
//! The Parquet schema mirrors the Arrow schema produced by the Arrow export
//! module; file-level and per-channel
//! metadata are preserved via schema metadata key–value pairs.
//!
//! ZSTD compression is applied at the configurable level (default: 3).

use std::io::Write;

use parquet::{
    arrow::arrow_writer::ArrowWriter,
    basic::{Compression, ZstdLevel},
    file::properties::WriterProperties,
};

use crate::domain::Datafile;
use crate::error::BiopacError;
use crate::export::arrow::build_record_batch;

/// Options for Parquet export.
#[derive(Debug, Clone, Copy)]
pub struct ParquetOptions {
    /// ZSTD compression level (1–22; default: 3).
    zstd_level: i32,
}

impl Default for ParquetOptions {
    fn default() -> Self {
        Self { zstd_level: 3 }
    }
}

impl ParquetOptions {
    /// Create a new `ParquetOptions` with default settings (ZSTD level 3).
    #[must_use]
    pub const fn new() -> Self {
        Self { zstd_level: 3 }
    }

    /// Set the ZSTD compression level (1–22).
    ///
    /// Returns a new `ParquetOptions` with the updated level.
    #[must_use]
    pub const fn zstd_level(mut self, level: i32) -> Self {
        self.zstd_level = level;
        self
    }
}

/// Write `datafile` as a Parquet file to `writer`.
///
/// The output is a single row-group file compressed with ZSTD at the level
/// specified in `options`.  Column layout mirrors the Arrow schema: a
/// `time_seconds` Float64 column followed by one Float64 column per channel,
/// all upsampled to the base rate.
///
/// # Errors
///
/// Returns [`BiopacError::Arrow`] if the Arrow batch cannot be constructed, or
/// [`BiopacError::Parquet`] if Parquet serialisation fails.
pub fn to_parquet<W: Write + Send>(
    datafile: &Datafile,
    writer: W,
    options: &ParquetOptions,
) -> Result<(), BiopacError> {
    let batch = build_record_batch(datafile)?;
    let zstd = ZstdLevel::try_new(options.zstd_level)?;
    let props = WriterProperties::builder()
        .set_compression(Compression::ZSTD(zstd))
        .build();
    let mut arrow_writer = ArrowWriter::try_new(writer, batch.schema(), Some(props))?;
    arrow_writer.write(&batch)?;
    arrow_writer.close()?;
    Ok(())
}

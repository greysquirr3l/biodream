//! Apache Arrow IPC export (requires `arrow` feature).
//!
//! Produces a single `RecordBatch` with a `time_seconds` column (Float64)
//! followed by one Float64 column per channel.  All channels are upsampled to
//! the base rate using linear interpolation so that every column has the same
//! length.
//!
//! Schema metadata:
//! - `"file_revision"` — the `.acq` format revision number as a string
//! - `"base_samples_per_second"` — the recording base rate
//!
//! Per-channel field metadata:
//! - `"units"` — physical unit label (e.g. `"mV"`)
//! - `"samples_per_second"` — the channel's effective sample rate
//! - `"channel_index"` — zero-based ordinal position in the file

use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;

use alloc::string::ToString;
use alloc::vec::Vec;

use arrow_array::{Float64Array, RecordBatch};
use arrow_schema::{DataType, Field, Schema};

use crate::domain::Datafile;
use crate::error::BiopacError;

/// Write `datafile` as an Arrow IPC stream to `writer`.
///
/// All channels are upsampled to the base rate (linear interpolation).
/// The stream contains a single [`RecordBatch`].
///
/// # Errors
///
/// Returns [`BiopacError::Arrow`] if schema construction or IPC serialisation
/// fails.
pub fn to_arrow_ipc<W: Write>(datafile: &Datafile, writer: W) -> Result<(), BiopacError> {
    let batch = build_record_batch(datafile)?;
    let mut stream_writer = arrow_ipc::writer::StreamWriter::try_new(writer, &batch.schema())?;
    stream_writer.write(&batch)?;
    stream_writer.finish()?;
    Ok(())
}

/// Build an Arrow [`RecordBatch`] from a [`Datafile`].
///
/// Called by both [`to_arrow_ipc`] and the Parquet exporter.  The batch
/// schema carries file-level and per-channel metadata (see module docs).
///
/// # Errors
///
/// Returns [`BiopacError::Arrow`] if the batch cannot be constructed (e.g. a
/// column length mismatch — should not occur in practice).
pub(crate) fn build_record_batch(datafile: &Datafile) -> Result<RecordBatch, BiopacError> {
    let base_rate = datafile.metadata.samples_per_second;
    let n_rows = compute_total_rows(datafile);
    let schema = build_schema(datafile, base_rate);

    // Time column: 0.0, 1/rate, 2/rate, …
    let time_col: Float64Array = (0..n_rows)
        .map(|i| {
            #[expect(
                clippy::cast_precision_loss,
                reason = "row index; precision loss negligible for physiological sample counts"
            )]
            let idx = i as f64;
            if base_rate > 0.0 {
                idx / base_rate
            } else {
                idx
            }
        })
        .collect();

    let mut columns: Vec<Arc<dyn arrow_array::Array>> =
        Vec::with_capacity(1 + datafile.channels.len());
    columns.push(Arc::new(time_col));

    for ch in &datafile.channels {
        let mut samples = ch.upsampled(base_rate);
        match samples.len().cmp(&n_rows) {
            std::cmp::Ordering::Less => samples.resize(n_rows, 0.0),
            std::cmp::Ordering::Greater => samples.truncate(n_rows),
            std::cmp::Ordering::Equal => {}
        }
        columns.push(Arc::new(Float64Array::from(samples)));
    }

    Ok(RecordBatch::try_new(schema, columns)?)
}

/// Construct the Arrow schema with file-level and per-channel metadata.
fn build_schema(datafile: &Datafile, base_rate: f64) -> Arc<Schema> {
    let schema_meta = HashMap::from([
        (
            "file_revision".to_string(),
            datafile.metadata.file_revision.to_string(),
        ),
        ("base_samples_per_second".to_string(), base_rate.to_string()),
    ]);

    let time_field = Field::new("time_seconds", DataType::Float64, false);

    let channel_fields: Vec<Field> = datafile
        .channels
        .iter()
        .enumerate()
        .map(|(idx, ch)| {
            let meta = HashMap::from([
                ("units".to_string(), ch.units.clone()),
                (
                    "samples_per_second".to_string(),
                    ch.samples_per_second.to_string(),
                ),
                ("channel_index".to_string(), idx.to_string()),
            ]);
            Field::new(ch.name.as_str(), DataType::Float64, false).with_metadata(meta)
        })
        .collect();

    let mut fields = Vec::with_capacity(1 + channel_fields.len());
    fields.push(time_field);
    fields.extend(channel_fields);

    Arc::new(Schema::new_with_metadata(fields, schema_meta))
}

/// Compute the total number of base-rate rows.
///
/// Each channel contributes `point_count × upsample_factor` rows; the result
/// is the maximum across all channels.
fn compute_total_rows(datafile: &Datafile) -> usize {
    let base = datafile.metadata.samples_per_second;
    datafile
        .channels
        .iter()
        .map(|ch| {
            if ch.samples_per_second <= 0.0 || base <= 0.0 {
                return ch.point_count;
            }
            let factor_f = base / ch.samples_per_second;
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "factor_f = base/rate; always a small positive integer for valid .acq data"
            )]
            let factor = (factor_f + 0.5) as usize;
            ch.point_count.saturating_mul(factor.max(1))
        })
        .max()
        .unwrap_or(0)
}

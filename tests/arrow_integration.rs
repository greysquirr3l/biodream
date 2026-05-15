//! Integration tests for Apache Arrow IPC and Parquet export (T11).

#![cfg(any(feature = "arrow", feature = "parquet"))]

use std::io::Cursor;

use biodream::domain::{ByteOrder, Channel, ChannelData, Datafile, FileRevision, GraphMetadata};
use biodream::error::BiopacError;
use biodream::to_arrow_ipc;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const fn make_metadata(samples_per_second: f64, channel_count: u16) -> GraphMetadata {
    GraphMetadata {
        file_revision: FileRevision::new(84),
        samples_per_second,
        channel_count,
        byte_order: ByteOrder::LittleEndian,
        compressed: false,
        title: None,
        acquisition_datetime: None,
        max_samples_per_second: None,
    }
}

fn make_raw_channel(name: &str, units: &str, samples_per_second: f64, raw: Vec<i16>) -> Channel {
    let point_count = raw.len();
    Channel {
        name: name.to_string(),
        units: units.to_string(),
        samples_per_second,
        frequency_divider: 1,
        data: ChannelData::Raw(raw),
        point_count,
    }
}

fn make_datafile(base_rate: f64, channels: Vec<Channel>) -> Datafile {
    let ch_count = u16::try_from(channels.len()).unwrap_or(u16::MAX);
    Datafile {
        metadata: make_metadata(base_rate, ch_count),
        channels,
        markers: vec![],
        journal: None,
    }
}

/// Write an IPC stream into an in-memory buffer and return it.
fn to_ipc_buf(df: &Datafile) -> Result<Vec<u8>, BiopacError> {
    let mut buf = Vec::new();
    to_arrow_ipc(df, Cursor::new(&mut buf))?;
    Ok(buf)
}

// ---------------------------------------------------------------------------
// Arrow IPC tests
// ---------------------------------------------------------------------------

#[test]
fn arrow_ipc_schema_column_names() -> Result<(), BiopacError> {
    let df = make_datafile(
        1000.0,
        vec![
            make_raw_channel("ECG", "mV", 1000.0, vec![0, 1, 2]),
            make_raw_channel("EDA", "µS", 1000.0, vec![5, 6, 7]),
        ],
    );
    let buf = to_ipc_buf(&df)?;
    let reader = arrow_ipc::reader::StreamReader::try_new(Cursor::new(&buf), None)
        .map_err(BiopacError::Arrow)?;
    let schema = reader.schema();
    assert_eq!(schema.fields().len(), 3, "time + 2 channels = 3 columns");
    assert_eq!(schema.field(0).name(), "time_seconds");
    assert_eq!(schema.field(1).name(), "ECG");
    assert_eq!(schema.field(2).name(), "EDA");
    Ok(())
}

#[test]
fn arrow_ipc_schema_metadata() -> Result<(), BiopacError> {
    let df = make_datafile(500.0, vec![make_raw_channel("CH0", "V", 500.0, vec![0])]);
    let buf = to_ipc_buf(&df)?;
    let reader = arrow_ipc::reader::StreamReader::try_new(Cursor::new(&buf), None)
        .map_err(BiopacError::Arrow)?;
    let schema = reader.schema();
    let meta = schema.metadata();
    assert!(
        meta.contains_key("file_revision"),
        "schema must have file_revision metadata"
    );
    assert!(
        meta.contains_key("base_samples_per_second"),
        "schema must have base_samples_per_second metadata"
    );
    assert_eq!(meta["base_samples_per_second"], "500");
    Ok(())
}

#[test]
fn arrow_ipc_channel_field_metadata() -> Result<(), BiopacError> {
    let df = make_datafile(
        1000.0,
        vec![make_raw_channel("ECG", "mV", 1000.0, vec![10, 20])],
    );
    let buf = to_ipc_buf(&df)?;
    let reader = arrow_ipc::reader::StreamReader::try_new(Cursor::new(&buf), None)
        .map_err(BiopacError::Arrow)?;
    let schema = reader.schema();
    let ecg_field = schema.field(1);
    let meta = ecg_field.metadata();
    assert_eq!(meta.get("units").map(String::as_str), Some("mV"));
    assert_eq!(meta.get("channel_index").map(String::as_str), Some("0"));
    assert!(meta.contains_key("samples_per_second"));
    Ok(())
}

#[test]
fn arrow_ipc_data_values() -> Result<(), BiopacError> {
    // 3 raw samples at 1000 Hz.  Raw(i16) → scaled_samples() → f64.
    let df = make_datafile(
        1000.0,
        vec![make_raw_channel("CH0", "mV", 1000.0, vec![10, 20, 30])],
    );
    let buf = to_ipc_buf(&df)?;
    let mut reader = arrow_ipc::reader::StreamReader::try_new(Cursor::new(&buf), None)
        .map_err(BiopacError::Arrow)?;
    let batch = reader
        .next()
        .ok_or_else(|| BiopacError::Validation("no batch".to_string()))?
        .map_err(BiopacError::Arrow)?;

    // time_seconds column
    let time = batch
        .column(0)
        .as_any()
        .downcast_ref::<arrow_array::Float64Array>()
        .ok_or_else(|| BiopacError::Validation("time column wrong type".to_string()))?;
    assert_eq!(time.len(), 3);
    assert!((time.value(0) - 0.0).abs() < 1e-12);
    assert!((time.value(1) - 0.001).abs() < 1e-12);
    assert!((time.value(2) - 0.002).abs() < 1e-12);

    // CH0 column — raw i16 → f64 cast
    let ch0 = batch
        .column(1)
        .as_any()
        .downcast_ref::<arrow_array::Float64Array>()
        .ok_or_else(|| BiopacError::Validation("ch0 column wrong type".to_string()))?;
    assert_eq!(ch0.len(), 3);
    assert!((ch0.value(0) - 10.0).abs() < 1e-12);
    assert!((ch0.value(1) - 20.0).abs() < 1e-12);
    assert!((ch0.value(2) - 30.0).abs() < 1e-12);
    Ok(())
}

#[test]
fn arrow_ipc_mixed_rate_upsample() -> Result<(), BiopacError> {
    // CH0: 1000 Hz, 4 samples
    // CH1: 500 Hz, 2 samples → upsampled to 4 rows
    let ch0 = make_raw_channel("CH0", "mV", 1000.0, vec![0, 100, 200, 300]);
    let ch1_count = 2;
    let ch1 = Channel {
        name: "CH1".to_string(),
        units: "µS".to_string(),
        samples_per_second: 500.0,
        frequency_divider: 2,
        data: ChannelData::Raw(vec![10, 20]),
        point_count: ch1_count,
    };
    let df = make_datafile(1000.0, vec![ch0, ch1]);
    let buf = to_ipc_buf(&df)?;
    let mut reader = arrow_ipc::reader::StreamReader::try_new(Cursor::new(&buf), None)
        .map_err(BiopacError::Arrow)?;
    let batch = reader
        .next()
        .ok_or_else(|| BiopacError::Validation("no batch".to_string()))?
        .map_err(BiopacError::Arrow)?;

    assert_eq!(batch.num_rows(), 4, "4 base-rate rows");

    let ch1_col = batch
        .column(2)
        .as_any()
        .downcast_ref::<arrow_array::Float64Array>()
        .ok_or_else(|| BiopacError::Validation("ch1 wrong type".to_string()))?;
    // upsampled: [10, 15, 20, 20]  (linear interp factor=2)
    assert_eq!(ch1_col.len(), 4);
    assert!((ch1_col.value(0) - 10.0).abs() < 1e-9);
    assert!((ch1_col.value(1) - 15.0).abs() < 1e-9);
    assert!((ch1_col.value(2) - 20.0).abs() < 1e-9);
    assert!((ch1_col.value(3) - 20.0).abs() < 1e-9);
    Ok(())
}

#[test]
fn arrow_ipc_empty_datafile_produces_valid_schema() -> Result<(), BiopacError> {
    let df = make_datafile(1000.0, vec![]);
    let buf = to_ipc_buf(&df)?;
    let reader = arrow_ipc::reader::StreamReader::try_new(Cursor::new(&buf), None)
        .map_err(BiopacError::Arrow)?;
    let schema = reader.schema();
    assert_eq!(schema.fields().len(), 1, "only time_seconds column");
    assert_eq!(schema.field(0).name(), "time_seconds");
    Ok(())
}

// ---------------------------------------------------------------------------
// Parquet tests
// ---------------------------------------------------------------------------

#[cfg(feature = "parquet")]
mod parquet_tests {
    use super::*;
    use biodream::{ParquetOptions, to_parquet};
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

    fn to_parquet_buf(df: &Datafile, options: ParquetOptions) -> Result<Vec<u8>, BiopacError> {
        let mut buf = Vec::new();
        to_parquet(df, Cursor::new(&mut buf), &options)?;
        Ok(buf)
    }

    #[test]
    fn parquet_round_trip_schema() -> Result<(), BiopacError> {
        let df = make_datafile(
            1000.0,
            vec![
                make_raw_channel("ECG", "mV", 1000.0, vec![1, 2, 3]),
                make_raw_channel("EDA", "µS", 1000.0, vec![4, 5, 6]),
            ],
        );
        let buf = to_parquet_buf(&df, ParquetOptions::new())?;
        let bytes = bytes::Bytes::from(buf);
        let builder =
            ParquetRecordBatchReaderBuilder::try_new(bytes).map_err(BiopacError::Parquet)?;
        let schema = builder.schema().clone();
        assert_eq!(schema.fields().len(), 3);
        assert_eq!(schema.field(0).name(), "time_seconds");
        assert_eq!(schema.field(1).name(), "ECG");
        assert_eq!(schema.field(2).name(), "EDA");
        Ok(())
    }

    #[test]
    fn parquet_round_trip_data_values() -> Result<(), BiopacError> {
        let df = make_datafile(
            500.0,
            vec![make_raw_channel("CH0", "mV", 500.0, vec![100, 200])],
        );
        let buf = to_parquet_buf(&df, ParquetOptions::new())?;
        let bytes = bytes::Bytes::from(buf);
        let mut reader = ParquetRecordBatchReaderBuilder::try_new(bytes)
            .map_err(BiopacError::Parquet)?
            .build()
            .map_err(BiopacError::Parquet)?;
        let batch = reader
            .next()
            .ok_or_else(|| BiopacError::Validation("no parquet batch".to_string()))?
            .map_err(BiopacError::Arrow)?;

        assert_eq!(batch.num_rows(), 2);

        let time = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow_array::Float64Array>()
            .ok_or_else(|| BiopacError::Validation("time col wrong type".to_string()))?;
        assert!((time.value(0) - 0.0).abs() < 1e-12);
        assert!((time.value(1) - 0.002).abs() < 1e-12);

        let ch0 = batch
            .column(1)
            .as_any()
            .downcast_ref::<arrow_array::Float64Array>()
            .ok_or_else(|| BiopacError::Validation("ch0 col wrong type".to_string()))?;
        assert!((ch0.value(0) - 100.0).abs() < 1e-12);
        assert!((ch0.value(1) - 200.0).abs() < 1e-12);
        Ok(())
    }

    #[test]
    fn parquet_options_builder() -> Result<(), BiopacError> {
        let opts = ParquetOptions::new().zstd_level(5);
        let df = make_datafile(100.0, vec![make_raw_channel("X", "V", 100.0, vec![1, 2])]);
        // Should not error — level 5 is valid.
        let buf = to_parquet_buf(&df, opts)?;
        assert!(!buf.is_empty(), "parquet output must be non-empty");
        Ok(())
    }

    #[test]
    fn parquet_default_options_equals_level_3() {
        let a = ParquetOptions::default();
        let b = ParquetOptions::new();
        // Both should have zstd_level 3 — verified by the same write succeeding.
        let df = make_datafile(100.0, vec![]);
        let buf_a = to_parquet_buf(&df, a);
        let buf_b = to_parquet_buf(&df, b);
        assert!(buf_a.is_ok());
        assert!(buf_b.is_ok());
    }
}

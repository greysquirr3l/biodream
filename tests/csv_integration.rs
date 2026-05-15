//! Integration tests for CSV export (T10).
//!
//! These tests exercise `biodream::to_csv` end-to-end: they build a `Datafile`
//! from synthetic bytes via the public read API, export to CSV, then parse the
//! CSV output to verify correctness. This ensures the full chain from parser
//! output through the export layer is wired correctly.

use biodream::{
    BiopacError, Channel, ChannelData, CsvOptions, Datafile, GraphMetadata, TimeFormat, to_csv,
};
use biodream::{ByteOrder, FileRevision};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const fn make_metadata(rate: f64, n_channels: u16) -> GraphMetadata {
    GraphMetadata {
        file_revision: FileRevision::new(38),
        samples_per_second: rate,
        channel_count: n_channels,
        byte_order: ByteOrder::LittleEndian,
        compressed: false,
        title: None,
        acquisition_datetime: None,
        max_samples_per_second: None,
    }
}

fn make_channel(name: &str, units: &str, rate: f64, divider: u16, samples: Vec<i16>) -> Channel {
    let n = samples.len();
    Channel {
        name: name.to_string(),
        units: units.to_string(),
        samples_per_second: rate / f64::from(divider),
        frequency_divider: divider,
        data: ChannelData::Scaled {
            raw: samples,
            scale: 1.0,
            offset: 0.0,
        },
        point_count: n,
    }
}

/// Export `df` to a `String` using `opts`.
fn export_to_string(df: &Datafile, opts: &CsvOptions) -> Result<String, BiopacError> {
    let mut buf: Vec<u8> = Vec::new();
    to_csv(df, &mut buf, opts)?;
    String::from_utf8(buf).map_err(|e| BiopacError::Validation(e.to_string()))
}

/// Parse CSV text into a `Vec<Vec<String>>` (rows × columns).
fn parse_csv(text: &str) -> Vec<Vec<String>> {
    text.lines()
        .map(|line| line.split(',').map(str::to_string).collect())
        .collect()
}

// ---------------------------------------------------------------------------
// Integration: end-to-end with synthetic parser output
// ---------------------------------------------------------------------------

#[test]
fn csv_full_pipeline_via_read_bytes() -> Result<(), BiopacError> {
    // Build a 2-channel uncompressed Pre-4 file in memory, parse it with the
    // public API, then export to CSV and verify.
    let n = 4usize;
    let channels = 2usize;
    let chan_hdr_len: i32 = 252;
    let chan_hdr_usize = usize::try_from(chan_hdr_len).unwrap_or(252);

    let mut blob: Vec<u8> = Vec::new();

    // Graph header (256 bytes)
    blob.extend_from_slice(&38i32.to_le_bytes()); // lVersion
    blob.extend_from_slice(&i16::try_from(channels).unwrap_or(0).to_le_bytes());
    blob.extend_from_slice(&0i16.to_le_bytes()); // nPreampTypes
    blob.extend_from_slice(&1.0f64.to_le_bytes()); // dSampleTime = 1ms → 1000 Hz
    blob.extend(std::iter::repeat_n(0u8, 236));
    blob.extend_from_slice(&i16::try_from(chan_hdr_len).unwrap_or(252).to_le_bytes());
    blob.extend_from_slice(&[0u8; 2]);
    assert_eq!(blob.len(), 256);

    // Channel headers
    for ch in 0..channels {
        let start = blob.len();
        blob.extend_from_slice(&chan_hdr_len.to_le_bytes());
        blob.extend_from_slice(&i32::try_from(n).unwrap_or(0).to_le_bytes());
        blob.extend_from_slice(&1.0f64.to_le_bytes()); // scale
        blob.extend_from_slice(&0.0f64.to_le_bytes()); // offset
        blob.extend_from_slice(&1i16.to_le_bytes()); // divider

        let name = format!("CH{ch}");
        let mut name_bytes = [0u8; 40];
        let src = name.as_bytes();
        let len = src.len().min(39);
        if let (Some(dst), Some(s)) = (name_bytes.get_mut(..len), src.get(..len)) {
            dst.copy_from_slice(s);
        }
        blob.extend_from_slice(&name_bytes);

        let mut units_bytes = [0u8; 20];
        if let Some(dst) = units_bytes.get_mut(..2) {
            dst.copy_from_slice(b"mV");
        }
        blob.extend_from_slice(&units_bytes);

        let consumed = blob.len() - start;
        let pad = chan_hdr_usize.saturating_sub(consumed);
        blob.extend(std::iter::repeat_n(0u8, pad));
        assert_eq!(blob.len() - start, chan_hdr_usize);
    }

    // Foreign data section
    blob.extend_from_slice(&0i32.to_le_bytes());

    // Dtype headers (i16 type = code 4, byte size = 2)
    for _ in 0..channels {
        blob.extend_from_slice(&4u16.to_le_bytes());
        blob.extend_from_slice(&2u16.to_le_bytes());
    }

    // Interleaved data: CH0 values 0,1,2,3 — CH1 values 10,11,12,13
    for s in 0..n {
        for ch in 0..channels {
            blob.extend_from_slice(
                &i16::try_from(ch * 10 + s).unwrap_or(0).to_le_bytes(),
            );
        }
    }

    // Marker section (empty)
    blob.extend_from_slice(&8i32.to_le_bytes());
    blob.extend_from_slice(&0i32.to_le_bytes());

    // Journal (empty)
    blob.extend_from_slice(&0i32.to_le_bytes());

    let df = biodream::read_bytes(&blob)?.value;
    let csv = export_to_string(&df, &CsvOptions::new().precision(3))?;
    let rows = parse_csv(&csv);

    // Header
    let header = rows
        .first()
        .ok_or_else(|| BiopacError::Validation("missing header row".into()))?;
    assert_eq!(
        header.iter().map(String::as_str).collect::<Vec<_>>(),
        vec!["time_s", "CH0", "CH1"]
    );

    // Row at t=0: time=0.000, CH0=0.000, CH1=10.000
    let r0 = rows
        .get(1)
        .ok_or_else(|| BiopacError::Validation("missing data row 0".into()))?;
    assert_eq!(r0.first().map(String::as_str), Some("0.000"), "time at t=0");
    assert_eq!(r0.get(1).map(String::as_str), Some("0.000"), "CH0 at t=0");
    assert_eq!(r0.get(2).map(String::as_str), Some("10.000"), "CH1 at t=0");

    // Row at t=3: time=0.003, CH0=3.000, CH1=13.000
    let r3 = rows
        .get(4)
        .ok_or_else(|| BiopacError::Validation("missing data row 3".into()))?;
    assert_eq!(r3.first().map(String::as_str), Some("0.003"), "time at t=3");
    assert_eq!(r3.get(1).map(String::as_str), Some("3.000"), "CH0 at t=3");
    assert_eq!(r3.get(2).map(String::as_str), Some("13.000"), "CH1 at t=3");

    // 1 header + 4 data rows
    assert_eq!(rows.len(), 5);

    Ok(())
}

#[test]
fn csv_channel_filter_integration() -> Result<(), BiopacError> {
    let rate = 100.0;
    let df = Datafile {
        metadata: make_metadata(rate, 3),
        channels: vec![
            make_channel("A", "mV", rate, 1, vec![1, 2, 3]),
            make_channel("B", "mV", rate, 1, vec![10, 20, 30]),
            make_channel("C", "mV", rate, 1, vec![100, 200, 300]),
        ],
        markers: vec![],
        journal: None,
    };

    // Only export channel 2 (C)
    let csv = export_to_string(&df, &CsvOptions::new().channels(&[2]).precision(0))?;
    let rows = parse_csv(&csv);

    let header = rows
        .first()
        .ok_or_else(|| BiopacError::Validation("missing header".into()))?;
    assert_eq!(header.iter().map(String::as_str).collect::<Vec<_>>(), ["time_s", "C"]);

    let r1 = rows
        .get(1)
        .ok_or_else(|| BiopacError::Validation("missing row 1".into()))?;
    assert_eq!(r1.get(1).map(String::as_str), Some("100"), "C[0]");

    Ok(())
}

#[test]
fn csv_mixed_rate_integration() -> Result<(), BiopacError> {
    // BASE channel at 100 Hz (divider=1, 6 samples)
    // HALF channel at 50 Hz (divider=2, 3 samples)
    let rate = 100.0;
    let df = Datafile {
        metadata: make_metadata(rate, 2),
        channels: vec![
            Channel {
                name: "BASE".to_string(),
                units: "mV".to_string(),
                samples_per_second: rate,
                frequency_divider: 1,
                data: ChannelData::Raw(vec![1, 2, 3, 4, 5, 6]),
                point_count: 6,
            },
            Channel {
                name: "HALF".to_string(),
                units: "mV".to_string(),
                samples_per_second: rate / 2.0,
                frequency_divider: 2,
                data: ChannelData::Raw(vec![10, 20, 30]),
                point_count: 3,
            },
        ],
        markers: vec![],
        journal: None,
    };

    let csv = export_to_string(&df, &CsvOptions::new().precision(0))?;
    let rows = parse_csv(&csv);

    // 1 header + 6 data rows
    assert_eq!(rows.len(), 7);

    // t=0: BASE=1, HALF=10
    let r0 = rows
        .get(1)
        .ok_or_else(|| BiopacError::Validation("missing r0".into()))?;
    assert_eq!(r0.get(1).map(String::as_str), Some("1"));
    assert_eq!(r0.get(2).map(String::as_str), Some("10"));

    // t=1: BASE=2, HALF=empty
    let r1 = rows
        .get(2)
        .ok_or_else(|| BiopacError::Validation("missing r1".into()))?;
    assert_eq!(r1.get(1).map(String::as_str), Some("2"));
    assert_eq!(r1.get(2).map(String::as_str), Some(""));

    // t=2: BASE=3, HALF=20
    let r2 = rows
        .get(3)
        .ok_or_else(|| BiopacError::Validation("missing r2".into()))?;
    assert_eq!(r2.get(1).map(String::as_str), Some("3"));
    assert_eq!(r2.get(2).map(String::as_str), Some("20"));

    Ok(())
}

#[test]
fn csv_time_seconds_precision() -> Result<(), BiopacError> {
    let rate = 1000.0;
    let df = Datafile {
        metadata: make_metadata(rate, 1),
        channels: vec![make_channel("X", "V", rate, 1, vec![0, 1])],
        markers: vec![],
        journal: None,
    };

    let csv = export_to_string(&df, &CsvOptions::new().precision(6))?;
    let rows = parse_csv(&csv);

    // t=1 → 0.001000 s
    let r1 = rows
        .get(2)
        .ok_or_else(|| BiopacError::Validation("missing row at t=1".into()))?;
    assert_eq!(r1.first().map(String::as_str), Some("0.001000"));

    Ok(())
}

#[test]
fn csv_time_milliseconds() -> Result<(), BiopacError> {
    let rate = 1000.0;
    let df = Datafile {
        metadata: make_metadata(rate, 1),
        channels: vec![make_channel("X", "V", rate, 1, vec![0, 1])],
        markers: vec![],
        journal: None,
    };

    let csv = export_to_string(
        &df,
        &CsvOptions::new()
            .time_format(TimeFormat::Milliseconds)
            .precision(3),
    )?;
    let header = csv.lines().next().unwrap_or("");
    assert!(header.starts_with("time_ms"), "expected time_ms column");

    let rows = parse_csv(&csv);
    // t=1 → 1.000 ms
    let r1 = rows
        .get(2)
        .ok_or_else(|| BiopacError::Validation("missing row at t=1".into()))?;
    assert_eq!(r1.first().map(String::as_str), Some("1.000"));

    Ok(())
}

#[test]
fn csv_time_hms() -> Result<(), BiopacError> {
    let rate = 1.0; // 1 Hz for easy HMS verification
    let df = Datafile {
        metadata: make_metadata(rate, 1),
        channels: vec![make_channel("X", "V", rate, 1, vec![0, 1, 2])],
        markers: vec![],
        journal: None,
    };

    let csv = export_to_string(&df, &CsvOptions::new().time_format(TimeFormat::Hms))?;
    let rows = parse_csv(&csv);

    let r0 = rows
        .get(1)
        .ok_or_else(|| BiopacError::Validation("missing row at t=0s".into()))?;
    assert_eq!(r0.first().map(String::as_str), Some("00:00:00.000000"));

    let r1 = rows
        .get(2)
        .ok_or_else(|| BiopacError::Validation("missing row at t=1s".into()))?;
    assert_eq!(r1.first().map(String::as_str), Some("00:00:01.000000"));

    Ok(())
}

#[test]
fn csv_to_file_and_back() -> Result<(), BiopacError> {
    use std::path::PathBuf;

    let rate = 500.0;
    let df = Datafile {
        metadata: make_metadata(rate, 2),
        channels: vec![
            make_channel("ECG", "mV", rate, 1, vec![100, 200, 300]),
            make_channel("RESP", "mmHg", rate, 1, vec![-10, -20, -30]),
        ],
        markers: vec![],
        journal: None,
    };

    let path: PathBuf = std::env::temp_dir().join("biodream_test_csv_roundtrip.csv");
    {
        let f = std::fs::File::create(&path).map_err(BiopacError::Io)?;
        to_csv(&df, f, &CsvOptions::new().precision(2))?;
    }

    let text = std::fs::read_to_string(&path).map_err(BiopacError::Io)?;
    let rows = parse_csv(&text);

    assert_eq!(rows.len(), 4, "1 header + 3 data rows");

    let header = rows
        .first()
        .ok_or_else(|| BiopacError::Validation("missing header".into()))?;
    assert_eq!(
        header.iter().map(String::as_str).collect::<Vec<_>>(),
        ["time_s", "ECG", "RESP"]
    );

    let r0 = rows
        .get(1)
        .ok_or_else(|| BiopacError::Validation("missing row 0".into()))?;
    assert_eq!(r0.get(1).map(String::as_str), Some("100.00"), "ECG[0]");
    assert_eq!(r0.get(2).map(String::as_str), Some("-10.00"), "RESP[0]");

    let _ = std::fs::remove_file(&path);
    Ok(())
}

#[test]
fn csv_invalid_channel_index() {
    let rate = 1000.0;
    let df = Datafile {
        metadata: make_metadata(rate, 1),
        channels: vec![make_channel("X", "V", rate, 1, vec![1, 2])],
        markers: vec![],
        journal: None,
    };

    let mut buf: Vec<u8> = Vec::new();
    let result = to_csv(&df, &mut buf, &CsvOptions::new().channels(&[5]));
    assert!(
        matches!(result, Err(BiopacError::InvalidChannel(_))),
        "out-of-range channel index should return InvalidChannel"
    );
}

#[test]
fn csv_include_raw_columns() -> Result<(), BiopacError> {
    let rate = 10.0;
    let df = Datafile {
        metadata: make_metadata(rate, 1),
        channels: vec![Channel {
            name: "SIG".to_string(),
            units: "mV".to_string(),
            samples_per_second: rate,
            frequency_divider: 1,
            data: ChannelData::Scaled {
                raw: vec![100, 200],
                scale: 0.5,
                offset: 1.0,
            },
            point_count: 2,
        }],
        markers: vec![],
        journal: None,
    };

    let csv = export_to_string(&df, &CsvOptions::new().include_raw(true).precision(1))?;
    let rows = parse_csv(&csv);

    let header = rows
        .first()
        .ok_or_else(|| BiopacError::Validation("missing header".into()))?;
    assert!(
        header.iter().any(|c| c == "SIG"),
        "SIG column should exist"
    );
    assert!(
        header.iter().any(|c| c == "SIG_raw"),
        "SIG_raw column should exist"
    );

    // SIG[0] scaled = 100 * 0.5 + 1.0 = 51.0; raw = 100
    let r0 = rows
        .get(1)
        .ok_or_else(|| BiopacError::Validation("missing row 0".into()))?;
    // Find column indices dynamically
    let sig_idx = header
        .iter()
        .position(|c| c == "SIG")
        .ok_or_else(|| BiopacError::Validation("SIG col missing".into()))?;
    let raw_idx = header
        .iter()
        .position(|c| c == "SIG_raw")
        .ok_or_else(|| BiopacError::Validation("SIG_raw col missing".into()))?;
    assert_eq!(
        r0.get(sig_idx).map(String::as_str),
        Some("51.0"),
        "scaled value"
    );
    assert_eq!(
        r0.get(raw_idx).map(String::as_str),
        Some("100"),
        "raw value"
    );

    Ok(())
}

#[test]
fn csv_tsv_delimiter() -> Result<(), BiopacError> {
    let rate = 10.0;
    let df = Datafile {
        metadata: make_metadata(rate, 2),
        channels: vec![
            make_channel("A", "mV", rate, 1, vec![1]),
            make_channel("B", "mV", rate, 1, vec![2]),
        ],
        markers: vec![],
        journal: None,
    };

    let mut buf: Vec<u8> = Vec::new();
    to_csv(&df, &mut buf, &CsvOptions::new().delimiter(b'\t'))?;
    let text = String::from_utf8(buf).map_err(|e| BiopacError::Validation(e.to_string()))?;

    let header = text.lines().next().unwrap_or("");
    assert!(header.contains('\t'), "TSV header should use tabs");
    assert!(!header.contains(','), "TSV header should not use commas");

    Ok(())
}

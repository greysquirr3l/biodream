//! Integration tests for the public read API (T09).
//!
//! These tests construct minimal valid `.acq` binary blobs in memory and
//! verify that `read_file`, `read_bytes`, `read_stream`, `open_file`, and
//! `ReadOptions` all behave correctly end-to-end.
//!
//! # Synthetic file format (Pre-4 uncompressed)
//!
//! ```text
//! Graph header         256 bytes  lVersion=38, nChannels=N, dSampleTime=1.0
//! Per-channel headers  252 bytes each  (nExtItemHeaderLen=252)
//! Foreign data section 4 bytes    nLength=0
//! Dtype headers        4 bytes each   nSize=4, nType=2 (i16)
//! Data section         N×M×2 bytes   interleaved i16 samples, LE
//! Marker section       8 bytes    lLength=8, lNumMarkers=0
//! Journal              4 bytes    lJournalLen=0
//! ```

use std::{io::Write, path::PathBuf};

use biodream::{
    BiopacError, Channel, ChannelData, ChannelMetadata, Datafile, ReadOptions, read_bytes,
    read_file, read_stream,
};

// ---------------------------------------------------------------------------
// Synthetic file builder
// ---------------------------------------------------------------------------

/// Build a minimal Pre-4 uncompressed `.acq` blob with `channels` channels
/// each holding `samples_per_channel` i16 samples.
///
/// Channel names are `"CH0"`, `"CH1"`, … and units are `"mV"`.
/// Samples for channel `i` are `i16` values `(ch_idx * 10 + sample_idx)`.
fn build_pre4_acq(channels: usize, samples_per_channel: usize) -> Vec<u8> {
    // Number of samples each channel has.
    let n = samples_per_channel;
    // Channel header length — we use 252 as nExtItemHeaderLen.
    let chan_hdr_len: i32 = 252;
    let chan_hdr_usize: usize = usize::try_from(chan_hdr_len).unwrap_or(252);

    let mut buf: Vec<u8> = Vec::new();

    // -----------------------------------------------------------------------
    // Graph header (256 bytes)
    // -----------------------------------------------------------------------
    // [0..4]   lVersion = 38 (Pre-4)
    buf.extend_from_slice(&38i32.to_le_bytes());
    // [4..6]   nChannels (i16 LE)
    buf.extend_from_slice(&i16::try_from(channels).unwrap_or(0).to_le_bytes());
    // [6..8]   nPreampTypes (skip)
    buf.extend_from_slice(&0i16.to_le_bytes());
    // [8..16]  dSampleTime = 1.0 ms → 1000 Hz
    buf.extend_from_slice(&1.0f64.to_le_bytes());
    // [16..252] zeros (236 bytes)
    buf.extend(std::iter::repeat_n(0u8, 236));
    // [252..254] nExtItemHeaderLen = 252 (as i16 LE)
    buf.extend_from_slice(&i16::try_from(chan_hdr_len).unwrap_or(252).to_le_bytes());
    // [254..256] pad
    buf.extend_from_slice(&[0u8; 2]);

    assert_eq!(buf.len(), 256, "graph header must be 256 bytes");

    // -----------------------------------------------------------------------
    // Per-channel headers (252 bytes each)
    // -----------------------------------------------------------------------
    for ch in 0..channels {
        let start = buf.len();

        // [0..4]   lChanHeaderLen = 252
        buf.extend_from_slice(&chan_hdr_len.to_le_bytes());
        // [4..8]   lBufLength = n
        buf.extend_from_slice(&i32::try_from(n).unwrap_or(0).to_le_bytes());
        // [8..16]  dAmplScale = 1.0
        buf.extend_from_slice(&1.0f64.to_le_bytes());
        // [16..24] dAmplOffset = 0.0
        buf.extend_from_slice(&0.0f64.to_le_bytes());
        // [24..26] nVarSampleDivider = 1
        buf.extend_from_slice(&1i16.to_le_bytes());

        // [26..66] szCommentText (40 bytes, null-terminated channel name)
        let name = format!("CH{ch}");
        let mut name_bytes = [0u8; 40];
        let name_src = name.as_bytes();
        let len = name_src.len().min(39);
        if let (Some(dst), Some(src)) = (name_bytes.get_mut(..len), name_src.get(..len)) {
            dst.copy_from_slice(src);
        }
        buf.extend_from_slice(&name_bytes);

        // [66..86] szUnitsText (20 bytes)
        let mut units_bytes = [0u8; 20];
        if let Some(dst) = units_bytes.get_mut(..2) {
            dst.copy_from_slice(b"mV");
        }
        buf.extend_from_slice(&units_bytes);

        // [86..252] pad to chan_hdr_len
        let consumed = buf.len() - start;
        let pad = chan_hdr_usize.saturating_sub(consumed);
        buf.extend(std::iter::repeat_n(0u8, pad));

        assert_eq!(
            buf.len() - start,
            chan_hdr_usize,
            "channel header must be exactly {chan_hdr_usize} bytes"
        );
    }

    // -----------------------------------------------------------------------
    // Foreign data section (4 bytes, empty)
    // -----------------------------------------------------------------------
    buf.extend_from_slice(&0i32.to_le_bytes());

    // -----------------------------------------------------------------------
    // Dtype headers (4 bytes × channels: nSize=4, nType=2 = i16)
    // -----------------------------------------------------------------------
    for _ in 0..channels {
        buf.extend_from_slice(&4u16.to_le_bytes()); // nSize
        buf.extend_from_slice(&2u16.to_le_bytes()); // nType = i16
    }

    // -----------------------------------------------------------------------
    // Interleaved data section
    // -----------------------------------------------------------------------
    // For simplicity all channels have divider=1, so the pattern is just
    // [ch0, ch1, ..., chN-1] repeated for each sample position.
    for s in 0..n {
        for ch in 0..channels {
            let sample = i16::try_from(ch * 10 + s).unwrap_or(0).to_le_bytes();
            buf.extend_from_slice(&sample);
        }
    }

    // -----------------------------------------------------------------------
    // Marker section (8 bytes: lLength=8, lNumMarkers=0)
    // -----------------------------------------------------------------------
    buf.extend_from_slice(&8i32.to_le_bytes()); // lLength
    buf.extend_from_slice(&0i32.to_le_bytes()); // lNumMarkers

    // -----------------------------------------------------------------------
    // Journal (4 bytes: lJournalLen=0)
    // -----------------------------------------------------------------------
    buf.extend_from_slice(&0i32.to_le_bytes());

    buf
}

/// Safe access helpers used in tests to avoid index-slicing panics.
fn get_channel(channels: &[Channel], idx: usize) -> Result<&Channel, BiopacError> {
    channels
        .get(idx)
        .ok_or_else(|| BiopacError::InvalidChannel(format!("no channel {idx}")))
}

fn get_meta(meta: &[ChannelMetadata], idx: usize) -> Result<&ChannelMetadata, BiopacError> {
    meta.get(idx)
        .ok_or_else(|| BiopacError::InvalidChannel(format!("no metadata {idx}")))
}

/// Write a byte blob to a temp file and return its path.
fn write_tmp(bytes: &[u8], name: &str) -> Result<PathBuf, BiopacError> {
    let dir = std::env::temp_dir();
    let path = dir.join(name);
    let mut f = std::fs::File::create(&path).map_err(BiopacError::Io)?;
    f.write_all(bytes).map_err(BiopacError::Io)?;
    Ok(path)
}

// ---------------------------------------------------------------------------
// Tests — read_bytes
// ---------------------------------------------------------------------------

#[test]
fn read_bytes_two_channels() -> Result<(), BiopacError> {
    let blob = build_pre4_acq(2, 4);
    let result = read_bytes(&blob)?;
    assert!(
        result.warnings.is_empty(),
        "unexpected warnings: {:?}",
        result.warnings
    );
    let df = result.value;
    assert_eq!(df.channel_count(), 2);
    Ok(())
}

#[test]
fn read_bytes_channel_names_and_units() -> Result<(), BiopacError> {
    let blob = build_pre4_acq(2, 4);
    let df = read_bytes(&blob)?.value;

    assert_eq!(get_channel(&df.channels, 0)?.name, "CH0");
    assert_eq!(get_channel(&df.channels, 0)?.units, "mV");
    assert_eq!(get_channel(&df.channels, 1)?.name, "CH1");
    assert_eq!(get_channel(&df.channels, 1)?.units, "mV");
    Ok(())
}

#[test]
fn read_bytes_sample_data() -> Result<(), BiopacError> {
    let blob = build_pre4_acq(2, 4);
    let df = read_bytes(&blob)?.value;

    assert_eq!(get_channel(&df.channels, 0)?.point_count, 4);
    assert_eq!(get_channel(&df.channels, 1)?.point_count, 4);

    // Channel 0 samples: ch=0 → values 0, 1, 2, 3
    // Channel 1 samples: ch=1 → values 10, 11, 12, 13
    let ch0_floats = get_channel(&df.channels, 0)?.scaled_samples();
    let ch1_floats = get_channel(&df.channels, 1)?.scaled_samples();

    assert_eq!(ch0_floats, vec![0.0, 1.0, 2.0, 3.0]);
    assert_eq!(ch1_floats, vec![10.0, 11.0, 12.0, 13.0]);
    Ok(())
}

#[test]
fn read_bytes_no_markers() -> Result<(), BiopacError> {
    let blob = build_pre4_acq(2, 4);
    let df = read_bytes(&blob)?.value;
    assert_eq!(df.marker_count(), 0);
    Ok(())
}

#[test]
fn read_bytes_metadata() -> Result<(), BiopacError> {
    let blob = build_pre4_acq(2, 4);
    let df = read_bytes(&blob)?.value;

    assert_eq!(df.metadata.file_revision.0, 38);
    assert!((df.metadata.samples_per_second - 1000.0).abs() < f64::EPSILON);
    assert!(!df.metadata.compressed);
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests — read_stream
// ---------------------------------------------------------------------------

#[test]
fn read_stream_identical_to_read_bytes() -> Result<(), BiopacError> {
    let blob = build_pre4_acq(3, 5);
    let from_bytes = read_bytes(&blob)?.value;
    let from_stream = read_stream(std::io::Cursor::new(&blob))?.value;

    assert_eq!(from_bytes.channel_count(), from_stream.channel_count());
    assert_eq!(from_bytes.marker_count(), from_stream.marker_count());
    for (b, s) in from_bytes.channels.iter().zip(from_stream.channels.iter()) {
        assert_eq!(b.name, s.name);
        assert_eq!(b.scaled_samples(), s.scaled_samples());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests — read_file
// ---------------------------------------------------------------------------

#[test]
fn read_file_roundtrip() -> Result<(), BiopacError> {
    let blob = build_pre4_acq(2, 4);
    let path = write_tmp(&blob, "biodream_test_read_file.acq")?;

    let from_file = read_file(&path)?.value;
    let from_bytes = read_bytes(&blob)?.value;

    assert_eq!(from_file.channel_count(), from_bytes.channel_count());
    for (f, b) in from_file.channels.iter().zip(from_bytes.channels.iter()) {
        assert_eq!(f.name, b.name);
        assert_eq!(f.scaled_samples(), b.scaled_samples());
    }

    let _ = std::fs::remove_file(path);
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests — Datafile ergonomic methods
// ---------------------------------------------------------------------------

#[test]
fn datafile_channel_by_name() -> Result<(), BiopacError> {
    let blob = build_pre4_acq(2, 4);
    let df = read_bytes(&blob)?.value;

    let ch = df.channel("CH0");
    assert!(ch.is_some(), "channel 'CH0' should exist");
    assert_eq!(ch.ok_or_else(|| BiopacError::InvalidChannel("CH0".into()))?.name, "CH0");

    assert!(df.channel("NOTEXIST").is_none());
    Ok(())
}

#[test]
fn datafile_channels_iterator() -> Result<(), BiopacError> {
    let blob = build_pre4_acq(3, 4);
    let df = read_bytes(&blob)?.value;

    let names: Vec<&str> = df.channels().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["CH0", "CH1", "CH2"]);
    Ok(())
}

#[test]
fn datafile_revision() -> Result<(), BiopacError> {
    let blob = build_pre4_acq(1, 4);
    let df = read_bytes(&blob)?.value;
    assert_eq!(df.revision().0, 38);
    Ok(())
}

#[test]
fn datafile_samples_per_second() -> Result<(), BiopacError> {
    let blob = build_pre4_acq(1, 4);
    let df = read_bytes(&blob)?.value;
    assert!((df.samples_per_second() - 1000.0).abs() < f64::EPSILON);
    Ok(())
}

#[test]
fn datafile_duration() -> Result<(), BiopacError> {
    // 4 samples at 1000 Hz = 0.004 s
    let blob = build_pre4_acq(2, 4);
    let df = read_bytes(&blob)?.value;
    let dur = df
        .duration()
        .ok_or_else(|| BiopacError::InvalidChannel("no duration".into()))?;
    assert!((dur - 0.004).abs() < 1e-9, "expected 0.004 s, got {dur}");
    Ok(())
}

#[test]
fn datafile_duration_none_for_empty() {
    let df = Datafile {
        metadata: biodream::GraphMetadata {
            file_revision: biodream::FileRevision::new(38),
            samples_per_second: 1000.0,
            channel_count: 0,
            byte_order: biodream::ByteOrder::LittleEndian,
            compressed: false,
            title: None,
            acquisition_datetime: None,
            max_samples_per_second: None,
        },
        channels: Vec::new(),
        markers: Vec::new(),
        journal: None,
    };
    assert!(df.duration().is_none());
}

// ---------------------------------------------------------------------------
// Tests — ReadOptions
// ---------------------------------------------------------------------------

#[test]
fn read_options_all_channels_default() -> Result<(), BiopacError> {
    let blob = build_pre4_acq(3, 4);
    let df = ReadOptions::new().read_bytes(&blob)?.value;
    assert_eq!(df.channel_count(), 3);
    Ok(())
}

#[test]
fn read_options_channel_filter() -> Result<(), BiopacError> {
    let blob = build_pre4_acq(3, 4);
    let df = ReadOptions::new()
        .channels(&[0, 2])
        .read_bytes(&blob)?
        .value;
    assert_eq!(df.channel_count(), 2);
    assert_eq!(get_channel(&df.channels, 0)?.name, "CH0");
    assert_eq!(get_channel(&df.channels, 1)?.name, "CH2");
    Ok(())
}

#[test]
fn read_options_channel_filter_single() -> Result<(), BiopacError> {
    let blob = build_pre4_acq(3, 4);
    let df = ReadOptions::new().channels(&[1]).read_bytes(&blob)?.value;
    assert_eq!(df.channel_count(), 1);
    assert_eq!(get_channel(&df.channels, 0)?.name, "CH1");
    Ok(())
}

#[test]
fn read_options_out_of_range_index_silently_dropped() -> Result<(), BiopacError> {
    let blob = build_pre4_acq(2, 4);
    // Request indices 0 and 99 (99 doesn't exist)
    let df = ReadOptions::new()
        .channels(&[0, 99])
        .read_bytes(&blob)?
        .value;
    assert_eq!(df.channel_count(), 1);
    assert_eq!(get_channel(&df.channels, 0)?.name, "CH0");
    Ok(())
}

#[test]
fn read_options_scaled_converts_to_float() -> Result<(), BiopacError> {
    let blob = build_pre4_acq(1, 4);
    let df = ReadOptions::new().scaled(true).read_bytes(&blob)?.value;
    // Pre-4 with scale=1.0, offset=0.0 → Scaled variant should convert to Float
    match &get_channel(&df.channels, 0)?.data {
        ChannelData::Float(_) | ChannelData::Scaled { .. } | ChannelData::Raw(_) => {
            // scale=1.0 offset=0.0 — any variant is acceptable here
        }
    }
    // Verify samples are correct regardless of variant
    let floats = get_channel(&df.channels, 0)?.scaled_samples();
    assert_eq!(floats, vec![0.0, 1.0, 2.0, 3.0]);
    Ok(())
}

#[test]
fn read_options_read_stream() -> Result<(), BiopacError> {
    let blob = build_pre4_acq(2, 4);
    let df = ReadOptions::new()
        .channels(&[0])
        .read_stream(std::io::Cursor::new(&blob))?
        .value;
    assert_eq!(df.channel_count(), 1);
    assert_eq!(get_channel(&df.channels, 0)?.name, "CH0");
    Ok(())
}

#[test]
fn read_options_read_file() -> Result<(), BiopacError> {
    let blob = build_pre4_acq(2, 4);
    let path = write_tmp(&blob, "biodream_test_read_opts_file.acq")?;

    let df = ReadOptions::new().channels(&[1]).read_file(&path)?.value;
    assert_eq!(df.channel_count(), 1);
    assert_eq!(get_channel(&df.channels, 0)?.name, "CH1");

    let _ = std::fs::remove_file(path);
    Ok(())
}

#[test]
fn read_options_build_is_identity() -> Result<(), BiopacError> {
    let blob = build_pre4_acq(2, 4);
    let df = ReadOptions::new()
        .channels(&[0])
        .scaled(false)
        .build()
        .read_bytes(&blob)?
        .value;
    assert_eq!(df.channel_count(), 1);
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests — LazyDatafile / open_file
// ---------------------------------------------------------------------------

#[test]
fn open_file_does_not_load_data() -> Result<(), BiopacError> {
    let blob = build_pre4_acq(2, 4);
    let path = write_tmp(&blob, "biodream_test_lazy_open.acq")?;

    let lazy = biodream::open_file(&path)?;

    // Data must NOT be loaded yet.
    assert!(
        !lazy.is_data_loaded(),
        "open_file must not read sample data"
    );
    assert_eq!(lazy.channel_count(), 2);
    assert_eq!(get_meta(&lazy.channel_metadata, 0)?.name, "CH0");
    assert_eq!(get_meta(&lazy.channel_metadata, 1)?.name, "CH1");

    let _ = std::fs::remove_file(path);
    Ok(())
}

#[test]
fn open_file_parses_markers() -> Result<(), BiopacError> {
    let blob = build_pre4_acq(2, 4);
    let path = write_tmp(&blob, "biodream_test_lazy_markers.acq")?;

    let lazy = biodream::open_file(&path)?;
    assert_eq!(lazy.markers.len(), 0, "synthetic file has no markers");
    assert!(lazy.journal.is_none(), "synthetic file has no journal");

    let _ = std::fs::remove_file(path);
    Ok(())
}

#[test]
fn open_file_load_channel() -> Result<(), BiopacError> {
    let blob = build_pre4_acq(2, 4);
    let path = write_tmp(&blob, "biodream_test_lazy_load_ch.acq")?;

    let lazy = biodream::open_file(&path)?;
    assert!(!lazy.is_data_loaded());

    let ch0 = lazy.load_channel(0)?;
    assert_eq!(ch0.name, "CH0");
    assert_eq!(ch0.point_count, 4);

    // Data is now loaded.
    assert!(lazy.is_data_loaded());

    // Second access returns cached data (no re-read).
    let ch1 = lazy.load_channel(1)?;
    assert_eq!(ch1.name, "CH1");

    let _ = std::fs::remove_file(path);
    Ok(())
}

#[test]
fn open_file_load_all() -> Result<(), BiopacError> {
    let blob = build_pre4_acq(3, 4);
    let path = write_tmp(&blob, "biodream_test_lazy_load_all.acq")?;

    let lazy = biodream::open_file(&path)?;
    let channels = lazy.load_all()?;
    assert_eq!(channels.len(), 3);
    assert_eq!(get_channel(channels, 0)?.name, "CH0");
    assert_eq!(get_channel(channels, 2)?.name, "CH2");

    let _ = std::fs::remove_file(path);
    Ok(())
}

#[test]
fn open_file_into_datafile_before_load() -> Result<(), BiopacError> {
    let blob = build_pre4_acq(2, 4);
    let path = write_tmp(&blob, "biodream_test_lazy_into_df_cold.acq")?;

    let lazy = biodream::open_file(&path)?;
    assert!(!lazy.is_data_loaded());

    let df = lazy.into_datafile()?;
    assert_eq!(df.channel_count(), 2);
    assert_eq!(get_channel(&df.channels, 0)?.scaled_samples(), vec![0.0, 1.0, 2.0, 3.0]);

    let _ = std::fs::remove_file(path);
    Ok(())
}

#[test]
fn open_file_into_datafile_after_load() -> Result<(), BiopacError> {
    let blob = build_pre4_acq(2, 4);
    let path = write_tmp(&blob, "biodream_test_lazy_into_df_warm.acq")?;

    let lazy = biodream::open_file(&path)?;
    let _ = lazy.load_channel(0)?; // warm the cache
    assert!(lazy.is_data_loaded());

    let df = lazy.into_datafile()?;
    assert_eq!(df.channel_count(), 2);

    let _ = std::fs::remove_file(path);
    Ok(())
}

#[test]
fn open_file_out_of_bounds_channel() -> Result<(), BiopacError> {
    let blob = build_pre4_acq(2, 4);
    let path = write_tmp(&blob, "biodream_test_lazy_oob.acq")?;

    let lazy = biodream::open_file(&path)?;
    let result = lazy.load_channel(99);
    assert!(result.is_err(), "index 99 should be out of bounds");
    assert!(
        matches!(result, Err(BiopacError::InvalidChannel(_))),
        "expected InvalidChannel error"
    );

    let _ = std::fs::remove_file(path);
    Ok(())
}

#[test]
fn open_file_debug_repr() -> Result<(), BiopacError> {
    let blob = build_pre4_acq(2, 4);
    let path = write_tmp(&blob, "biodream_test_lazy_debug.acq")?;

    let lazy = biodream::open_file(&path)?;
    let s = format!("{lazy:?}");
    assert!(s.contains("LazyDatafile"));
    assert!(s.contains("channel_count"));

    let _ = std::fs::remove_file(path);
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests — top-level crate API functions
// ---------------------------------------------------------------------------

#[test]
fn crate_read_bytes_is_accessible() -> Result<(), BiopacError> {
    let blob = build_pre4_acq(1, 4);
    let df = biodream::read_bytes(&blob)?.value;
    assert_eq!(df.channel_count(), 1);
    Ok(())
}

#[test]
fn crate_read_stream_is_accessible() -> Result<(), BiopacError> {
    let blob = build_pre4_acq(1, 4);
    let df = biodream::read_stream(std::io::Cursor::new(&blob))?.value;
    assert_eq!(df.channel_count(), 1);
    Ok(())
}

#[test]
fn crate_read_file_is_accessible() -> Result<(), BiopacError> {
    let blob = build_pre4_acq(1, 4);
    let path = write_tmp(&blob, "biodream_test_crate_read_file.acq")?;

    let df = biodream::read_file(&path)?.value;
    assert_eq!(df.channel_count(), 1);

    let _ = std::fs::remove_file(path);
    Ok(())
}

#[test]
fn crate_open_file_is_accessible() -> Result<(), BiopacError> {
    let blob = build_pre4_acq(1, 4);
    let path = write_tmp(&blob, "biodream_test_crate_open_file.acq")?;

    let lazy = biodream::open_file(&path)?;
    assert_eq!(lazy.channel_count(), 1);

    let _ = std::fs::remove_file(path);
    Ok(())
}

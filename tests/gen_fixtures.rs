//! Fixture generator for the biodream test suite (T14).
//!
//! This file contains a single `#[ignore]`d test that, when run explicitly,
//! writes synthetic `.acq` files and their JSON companion sidecars into
//! `tests/fixtures/`.  It must be run with `--all-features` so that both the
//! `write` and `read` features are active.
//!
//! # Regenerating fixtures
//!
//! ```sh
//! cargo test --all-features --test gen_fixtures gen_fixtures -- --ignored
//! ```
//!
//! Commit the generated `tests/fixtures/*.acq` and `tests/fixtures/*.json`
//! files after re-running the generator.

#![cfg(all(feature = "write", feature = "read"))]

use std::{
    fs,
    path::{Path, PathBuf},
};

use biodream::{
    ByteOrder, Channel, ChannelData, Datafile, FileRevision, GraphMetadata, Journal, Marker,
    MarkerStyle, ReadOptions, WriteOptions,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

/// Build a linear-ramp `Vec<i16>` with `n` samples: `[0, 1, 2, ..., n-1]`.
///
/// Values cycle 0–255 so each sample fits in a `u8`-sized range and
/// the expected first/last values are trivially computable.
fn ramp(n: usize) -> Vec<i16> {
    (0u8..=255).cycle().take(n).map(i16::from).collect()
}

/// Minimal `GraphMetadata` with the given revision.
const fn meta(revision: i32, channels: u16, sps: f64) -> GraphMetadata {
    GraphMetadata {
        file_revision: FileRevision::new(revision),
        samples_per_second: sps,
        channel_count: channels,
        byte_order: ByteOrder::LittleEndian,
        compressed: false,
        title: None,
        acquisition_datetime: None,
        max_samples_per_second: None,
    }
}

/// Write `df` with `opts` into `<fixtures_dir>/<name>.acq`, then read it back
/// and write a JSON sidecar at `<fixtures_dir>/<name>.json`.
fn write_fixture(
    df: &Datafile,
    opts: &WriteOptions,
    name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let dir = fixtures_dir();
    fs::create_dir_all(&dir)?;

    // Write the .acq file.
    let acq_path = dir.join(format!("{name}.acq"));
    opts.write_file(df, &acq_path)?;

    // Read it back so the sidecar reflects what the parser actually returns.
    let raw = fs::read(&acq_path)?;
    let parsed = ReadOptions::new().read_bytes(&raw)?.into_value();

    // Build the channels array for the sidecar.
    let channels_json: Vec<serde_json::Value> = parsed
        .channels
        .iter()
        .map(|ch| {
            serde_json::json!({
                "name": ch.name,
                "units": ch.units,
                "frequency_divider": ch.frequency_divider,
                "samples_per_second": ch.samples_per_second,
                "point_count": ch.point_count,
            })
        })
        .collect();

    // Build the markers array for the sidecar.
    let markers_json: Vec<serde_json::Value> = parsed
        .markers
        .iter()
        .map(|m| {
            serde_json::json!({
                "label": m.label,
                "global_sample_index": m.global_sample_index,
                "channel": m.channel,
            })
        })
        .collect();

    let sidecar = serde_json::json!({
        "revision": parsed.metadata.file_revision.0,
        "compressed": parsed.metadata.compressed,
        "samples_per_second": parsed.metadata.samples_per_second,
        "channel_count": parsed.channels.len(),
        "channels": channels_json,
        "markers": markers_json,
        "journal": parsed.journal.as_ref().map(biodream::Journal::as_text),
    });

    let json_path = dir.join(format!("{name}.json"));
    fs::write(json_path, serde_json::to_string_pretty(&sidecar)?)?;

    println!("  wrote {name}.acq  +  {name}.json");
    Ok(())
}

// ---------------------------------------------------------------------------
// Fixture builders
// ---------------------------------------------------------------------------

fn basic_v38() -> (Datafile, WriteOptions) {
    let df = Datafile {
        metadata: meta(38, 1, 1000.0),
        channels: vec![Channel {
            name: String::from("ECG"),
            units: String::from("mV"),
            samples_per_second: 1000.0,
            frequency_divider: 1,
            data: ChannelData::Raw(ramp(100)),
            point_count: 100,
        }],
        markers: Vec::new(),
        journal: None,
    };
    let opts = WriteOptions::new().revision(38);
    (df, opts)
}

fn basic_v43() -> (Datafile, WriteOptions) {
    let df = Datafile {
        metadata: meta(43, 1, 1000.0),
        channels: vec![Channel {
            name: String::from("ECG"),
            units: String::from("mV"),
            samples_per_second: 1000.0,
            frequency_divider: 1,
            data: ChannelData::Raw(ramp(100)),
            point_count: 100,
        }],
        markers: Vec::new(),
        journal: None,
    };
    (df, WriteOptions::new())
}

fn multichannel_v43() -> (Datafile, WriteOptions) {
    let df = Datafile {
        metadata: meta(43, 2, 1000.0),
        channels: vec![
            Channel {
                name: String::from("ECG"),
                units: String::from("mV"),
                samples_per_second: 1000.0,
                frequency_divider: 1,
                data: ChannelData::Raw(ramp(100)),
                point_count: 100,
            },
            Channel {
                name: String::from("RESP"),
                units: String::from("au"),
                samples_per_second: 1000.0,
                frequency_divider: 1,
                data: ChannelData::Raw(ramp(100)),
                point_count: 100,
            },
        ],
        markers: Vec::new(),
        journal: None,
    };
    (df, WriteOptions::new())
}

fn mixed_rate_v43() -> (Datafile, WriteOptions) {
    let df = Datafile {
        metadata: meta(43, 2, 1000.0),
        channels: vec![
            Channel {
                name: String::from("ECG"),
                units: String::from("mV"),
                samples_per_second: 1000.0,
                frequency_divider: 1,
                data: ChannelData::Raw(ramp(100)),
                point_count: 100,
            },
            Channel {
                name: String::from("RESP"),
                units: String::from("au"),
                samples_per_second: 500.0,
                frequency_divider: 2,
                data: ChannelData::Raw(ramp(50)),
                point_count: 50,
            },
        ],
        markers: Vec::new(),
        journal: None,
    };
    (df, WriteOptions::new())
}

fn compressed_v68() -> (Datafile, WriteOptions) {
    let df = Datafile {
        metadata: GraphMetadata {
            file_revision: FileRevision::new(68),
            samples_per_second: 1000.0,
            channel_count: 2,
            byte_order: ByteOrder::LittleEndian,
            compressed: true,
            title: None,
            acquisition_datetime: None,
            max_samples_per_second: None,
        },
        channels: vec![
            Channel {
                name: String::from("ECG"),
                units: String::from("mV"),
                samples_per_second: 1000.0,
                frequency_divider: 1,
                data: ChannelData::Raw(ramp(100)),
                point_count: 100,
            },
            Channel {
                name: String::from("RESP"),
                units: String::from("au"),
                samples_per_second: 1000.0,
                frequency_divider: 1,
                data: ChannelData::Raw(ramp(100)),
                point_count: 100,
            },
        ],
        markers: Vec::new(),
        journal: None,
    };
    let opts = WriteOptions::new().compressed(true);
    (df, opts)
}

fn markers_v43() -> (Datafile, WriteOptions) {
    let df = Datafile {
        metadata: meta(43, 1, 1000.0),
        channels: vec![Channel {
            name: String::from("ECG"),
            units: String::from("mV"),
            samples_per_second: 1000.0,
            frequency_divider: 1,
            data: ChannelData::Raw(ramp(500)),
            point_count: 500,
        }],
        markers: vec![
            Marker {
                label: String::from("Baseline"),
                global_sample_index: 0,
                channel: None,
                style: MarkerStyle::GlobalEvent,
                created_at: None,
            },
            Marker {
                label: String::from("Stimulus"),
                global_sample_index: 200,
                channel: None,
                style: MarkerStyle::UserEvent,
                created_at: None,
            },
            Marker {
                label: String::from("Response"),
                global_sample_index: 400,
                channel: None,
                style: MarkerStyle::UserEvent,
                created_at: None,
            },
        ],
        journal: None,
    };
    (df, WriteOptions::new())
}

fn journal_v43() -> (Datafile, WriteOptions) {
    let df = Datafile {
        metadata: meta(43, 1, 1000.0),
        channels: vec![Channel {
            name: String::from("ECG"),
            units: String::from("mV"),
            samples_per_second: 1000.0,
            frequency_divider: 1,
            data: ChannelData::Raw(ramp(100)),
            point_count: 100,
        }],
        markers: Vec::new(),
        journal: Some(Journal::Plain(String::from(
            "Subject: S01\nCondition: Rest\n",
        ))),
    };
    (df, WriteOptions::new())
}

// ---------------------------------------------------------------------------
// Generator entry point
// ---------------------------------------------------------------------------

/// Generate all fixture files.
///
/// Run with:
/// ```sh
/// cargo test --all-features --test gen_fixtures gen_fixtures -- --ignored
/// ```
#[test]
#[ignore = "generator: run manually to produce fixture files in tests/fixtures/"]
fn gen_fixtures() -> Result<(), Box<dyn std::error::Error>> {
    println!("\nGenerating fixtures in {}...", fixtures_dir().display());

    let (df, opts) = basic_v38();
    write_fixture(&df, &opts, "basic_v38")?;

    let (df, opts) = basic_v43();
    write_fixture(&df, &opts, "basic_v43")?;

    let (df, opts) = multichannel_v43();
    write_fixture(&df, &opts, "multichannel_v43")?;

    let (df, opts) = mixed_rate_v43();
    write_fixture(&df, &opts, "mixed_rate_v43")?;

    let (df, opts) = compressed_v68();
    write_fixture(&df, &opts, "compressed_v68")?;

    let (df, opts) = markers_v43();
    write_fixture(&df, &opts, "markers_v43")?;

    let (df, opts) = journal_v43();
    write_fixture(&df, &opts, "journal_v43")?;

    println!("Done. Commit tests/fixtures/ to the repository.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Sanity tests that do NOT require committed fixture files
// (run every time, not just when regenerating)
// ---------------------------------------------------------------------------

/// Verify that each fixture builder produces a Datafile that round-trips
/// cleanly through the writer + parser without committed files.
#[test]
fn gen_roundtrip_basic_v38() -> Result<(), Box<dyn std::error::Error>> {
    let (df, opts) = basic_v38();
    let mut buf = Vec::new();
    opts.write_stream(&df, &mut buf)?;
    let parsed = ReadOptions::new().read_bytes(&buf)?.into_value();
    assert_eq!(parsed.channels.len(), 1);
    assert_eq!(parsed.channels.first().ok_or("no channel")?.name, "ECG");
    Ok(())
}

#[test]
fn gen_roundtrip_basic_v43() -> Result<(), Box<dyn std::error::Error>> {
    let (df, opts) = basic_v43();
    let mut buf = Vec::new();
    opts.write_stream(&df, &mut buf)?;
    let parsed = ReadOptions::new().read_bytes(&buf)?.into_value();
    assert_eq!(parsed.channels.len(), 1);
    Ok(())
}

#[test]
fn gen_roundtrip_multichannel_v43() -> Result<(), Box<dyn std::error::Error>> {
    let (df, opts) = multichannel_v43();
    let mut buf = Vec::new();
    opts.write_stream(&df, &mut buf)?;
    let parsed = ReadOptions::new().read_bytes(&buf)?.into_value();
    assert_eq!(parsed.channels.len(), 2);
    assert_eq!(parsed.channels.first().ok_or("no ch0")?.name, "ECG");
    assert_eq!(parsed.channels.get(1).ok_or("no ch1")?.name, "RESP");
    Ok(())
}

#[test]
fn gen_roundtrip_mixed_rate_v43() -> Result<(), Box<dyn std::error::Error>> {
    let (df, opts) = mixed_rate_v43();
    let mut buf = Vec::new();
    opts.write_stream(&df, &mut buf)?;
    let parsed = ReadOptions::new().read_bytes(&buf)?.into_value();
    assert_eq!(parsed.channels.len(), 2);
    let ch1 = parsed.channels.get(1).ok_or("no ch1")?;
    assert_eq!(ch1.frequency_divider, 2);
    assert_eq!(ch1.point_count, 50);
    Ok(())
}

#[test]
fn gen_roundtrip_compressed_v68() -> Result<(), Box<dyn std::error::Error>> {
    let (df, opts) = compressed_v68();
    let mut buf = Vec::new();
    opts.write_stream(&df, &mut buf)?;
    let parsed = ReadOptions::new().read_bytes(&buf)?.into_value();
    assert!(parsed.metadata.compressed);
    assert_eq!(parsed.channels.len(), 2);
    Ok(())
}

#[test]
fn gen_roundtrip_markers_v43() -> Result<(), Box<dyn std::error::Error>> {
    let (df, opts) = markers_v43();
    let mut buf = Vec::new();
    opts.write_stream(&df, &mut buf)?;
    let parsed = ReadOptions::new().read_bytes(&buf)?.into_value();
    assert_eq!(parsed.markers.len(), 3);
    assert_eq!(parsed.markers.first().ok_or("no m0")?.label, "Baseline");
    assert_eq!(
        parsed.markers.get(1).ok_or("no m1")?.global_sample_index,
        200
    );
    Ok(())
}

#[test]
fn gen_roundtrip_journal_v43() -> Result<(), Box<dyn std::error::Error>> {
    let (df, opts) = journal_v43();
    let mut buf = Vec::new();
    opts.write_stream(&df, &mut buf)?;
    let parsed = ReadOptions::new().read_bytes(&buf)?.into_value();
    let journal = parsed.journal.ok_or("journal should be present")?;
    assert!(journal.as_text().contains("Subject: S01"));
    Ok(())
}

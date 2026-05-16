//! Integration tests that validate committed fixture files against their JSON
//! companion sidecars (T14).
//!
//! Each test reads a pre-generated `.acq` file from `tests/fixtures/`, parses
//! it with the public `read_file` API, and asserts that every field documented
//! in the companion `.json` sidecar matches the parsed value.
//!
//! Fixture files are generated (and regenerated) by running:
//!
//! ```sh
//! cargo test --all-features --test gen_fixtures gen_fixtures -- --ignored
//! ```

#![cfg(all(feature = "write", feature = "read"))]

use std::path::{Path, PathBuf};

use biodream::{ChannelData, ReadOptions};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

/// Load the `.acq` file and its `.json` sidecar from `tests/fixtures/<name>`.
fn load_fixture(
    name: &str,
) -> Result<(biodream::Datafile, serde_json::Value), Box<dyn std::error::Error>> {
    let dir = fixtures_dir();

    let acq_path = dir.join(format!("{name}.acq"));
    let json_path = dir.join(format!("{name}.json"));

    let df = ReadOptions::new().read_file(&acq_path)?.into_value();
    let json_bytes = std::fs::read(&json_path)?;
    let sidecar: serde_json::Value = serde_json::from_slice(&json_bytes)?;

    Ok((df, sidecar))
}

/// Assert that basic scalar metadata matches the sidecar.
fn assert_metadata(
    df: &biodream::Datafile,
    s: &serde_json::Value,
) -> Result<(), Box<dyn std::error::Error>> {
    let expected_revision = s["revision"].as_i64().ok_or("revision missing")?;
    assert_eq!(
        i64::from(df.metadata.file_revision.0),
        expected_revision,
        "file_revision"
    );

    let expected_compressed = s["compressed"].as_bool().ok_or("compressed missing")?;
    assert_eq!(df.metadata.compressed, expected_compressed, "compressed");

    let expected_sps = s["samples_per_second"].as_f64().ok_or("sps missing")?;
    assert!(
        (df.metadata.samples_per_second - expected_sps).abs() < 1e-6,
        "samples_per_second mismatch: {} vs {}",
        df.metadata.samples_per_second,
        expected_sps
    );

    let expected_ch_count = s["channel_count"].as_u64().ok_or("channel_count missing")?;
    assert_eq!(df.channels.len() as u64, expected_ch_count, "channel_count");

    Ok(())
}

/// Assert per-channel metadata from the sidecar.
fn assert_channels(
    df: &biodream::Datafile,
    s: &serde_json::Value,
) -> Result<(), Box<dyn std::error::Error>> {
    let expected_channels = s["channels"].as_array().ok_or("channels missing")?;
    assert_eq!(df.channels.len(), expected_channels.len());

    for (i, (ch, exp)) in df.channels.iter().zip(expected_channels).enumerate() {
        let exp_name = exp["name"].as_str().ok_or("name missing")?;
        assert_eq!(ch.name, exp_name, "ch[{i}].name");

        let exp_units = exp["units"].as_str().ok_or("units missing")?;
        assert_eq!(ch.units, exp_units, "ch[{i}].units");

        let exp_div = exp["frequency_divider"]
            .as_u64()
            .ok_or("freq_div missing")?;
        assert_eq!(
            u64::from(ch.frequency_divider),
            exp_div,
            "ch[{i}].frequency_divider"
        );

        let exp_count = exp["point_count"].as_u64().ok_or("point_count missing")?;
        assert_eq!(ch.point_count as u64, exp_count, "ch[{i}].point_count");
    }

    Ok(())
}

/// Assert marker list from the sidecar.
fn assert_markers(
    df: &biodream::Datafile,
    s: &serde_json::Value,
) -> Result<(), Box<dyn std::error::Error>> {
    let expected_markers = s["markers"].as_array().ok_or("markers missing")?;
    assert_eq!(df.markers.len(), expected_markers.len(), "marker count");

    for (i, (m, exp)) in df.markers.iter().zip(expected_markers).enumerate() {
        let exp_label = exp["label"].as_str().ok_or("label missing")?;
        assert_eq!(m.label, exp_label, "marker[{i}].label");

        let exp_idx = exp["global_sample_index"]
            .as_u64()
            .ok_or("global_sample_index missing")?;
        assert_eq!(
            m.global_sample_index as u64, exp_idx,
            "marker[{i}].global_sample_index"
        );
    }

    Ok(())
}

/// Assert journal text from the sidecar.
fn assert_journal(
    df: &biodream::Datafile,
    s: &serde_json::Value,
) -> Result<(), Box<dyn std::error::Error>> {
    match s["journal"].as_str() {
        Some(expected_text) => {
            let journal = df.journal.as_ref().ok_or("expected journal but got None")?;
            assert_eq!(journal.as_text(), expected_text, "journal text");
        }
        None => {
            assert!(df.journal.is_none(), "expected no journal but got Some");
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Per-fixture tests
// ---------------------------------------------------------------------------

#[test]
fn fixture_basic_v38() -> Result<(), Box<dyn std::error::Error>> {
    let (df, sidecar) = load_fixture("basic_v38")?;
    assert_metadata(&df, &sidecar)?;
    assert_channels(&df, &sidecar)?;
    assert_markers(&df, &sidecar)?;
    assert_journal(&df, &sidecar)?;

    // Spot-check: first 5 raw samples should be 0, 1, 2, 3, 4.
    let ch = df.channels.first().ok_or("no channel")?;
    let ChannelData::Raw(ref raw) = ch.data else {
        return Err("expected Raw data".into());
    };
    assert_eq!(raw.first().copied().ok_or("no sample 0")?, 0i16);
    assert_eq!(raw.get(4).copied().ok_or("no sample 4")?, 4i16);

    Ok(())
}

#[test]
fn fixture_basic_v43() -> Result<(), Box<dyn std::error::Error>> {
    let (df, sidecar) = load_fixture("basic_v43")?;
    assert_metadata(&df, &sidecar)?;
    assert_channels(&df, &sidecar)?;
    assert_markers(&df, &sidecar)?;
    assert_journal(&df, &sidecar)?;

    let ch = df.channels.first().ok_or("no channel")?;
    let ChannelData::Raw(ref raw) = ch.data else {
        return Err("expected Raw data".into());
    };
    assert_eq!(raw.get(99).copied().ok_or("no sample 99")?, 99i16);

    Ok(())
}

#[test]
fn fixture_multichannel_v43() -> Result<(), Box<dyn std::error::Error>> {
    let (df, sidecar) = load_fixture("multichannel_v43")?;
    assert_metadata(&df, &sidecar)?;
    assert_channels(&df, &sidecar)?;
    assert_markers(&df, &sidecar)?;
    assert_journal(&df, &sidecar)?;

    assert_eq!(df.channels.len(), 2);
    assert_eq!(df.channels.first().ok_or("no ch0")?.name, "ECG");
    assert_eq!(df.channels.get(1).ok_or("no ch1")?.name, "RESP");

    Ok(())
}

#[test]
fn fixture_mixed_rate_v44() -> Result<(), Box<dyn std::error::Error>> {
    let (df, sidecar) = load_fixture("mixed_rate_v44")?;
    assert_metadata(&df, &sidecar)?;
    assert_channels(&df, &sidecar)?;
    assert_markers(&df, &sidecar)?;
    assert_journal(&df, &sidecar)?;

    let ch1 = df.channels.get(1).ok_or("no ch1")?;
    assert_eq!(ch1.frequency_divider, 2);
    assert_eq!(ch1.point_count, 50);
    assert!(
        (ch1.samples_per_second - 500.0).abs() < 1.0,
        "expected ~500 sps for RESP, got {}",
        ch1.samples_per_second
    );

    Ok(())
}

#[test]
fn fixture_compressed_v68() -> Result<(), Box<dyn std::error::Error>> {
    let (df, sidecar) = load_fixture("compressed_v68")?;
    assert_metadata(&df, &sidecar)?;
    assert_channels(&df, &sidecar)?;
    assert_markers(&df, &sidecar)?;
    assert_journal(&df, &sidecar)?;

    assert!(df.metadata.compressed);
    assert_eq!(df.channels.len(), 2);

    // Both channels should have 100 samples with correct ramp data.
    // Compressed channels parse as Scaled; use scaled_samples() for uniform access.
    for ch in &df.channels {
        assert_eq!(ch.point_count, 100, "expected 100 samples in {}", ch.name);
        let samples = ch.scaled_samples();
        assert!(
            (samples.first().copied().ok_or("no sample 0")? - 0.0).abs() < 1e-9,
            "first sample should be 0 in {}",
            ch.name
        );
        assert!(
            (samples.get(9).copied().ok_or("no sample 9")? - 9.0).abs() < 1e-9,
            "sample[9] should be 9 in {}",
            ch.name
        );
    }

    Ok(())
}

#[test]
fn fixture_markers_v43() -> Result<(), Box<dyn std::error::Error>> {
    let (df, sidecar) = load_fixture("markers_v43")?;
    assert_metadata(&df, &sidecar)?;
    assert_channels(&df, &sidecar)?;
    assert_markers(&df, &sidecar)?;
    assert_journal(&df, &sidecar)?;

    assert_eq!(df.markers.len(), 3);
    assert_eq!(df.markers.first().ok_or("no m0")?.label, "Baseline");
    assert_eq!(df.markers.first().ok_or("no m0")?.global_sample_index, 0);
    assert_eq!(df.markers.get(1).ok_or("no m1")?.global_sample_index, 200);
    assert_eq!(df.markers.get(2).ok_or("no m2")?.global_sample_index, 400);

    Ok(())
}

#[test]
fn fixture_journal_v43() -> Result<(), Box<dyn std::error::Error>> {
    let (df, sidecar) = load_fixture("journal_v43")?;
    assert_metadata(&df, &sidecar)?;
    assert_channels(&df, &sidecar)?;
    assert_markers(&df, &sidecar)?;
    assert_journal(&df, &sidecar)?;

    let journal = df.journal.as_ref().ok_or("expected journal")?;
    assert!(journal.as_text().contains("Subject: S01"));
    assert!(journal.as_text().contains("Condition: Rest"));

    Ok(())
}

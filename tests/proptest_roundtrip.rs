//! Property-based roundtrip tests for the `.acq` writer + parser (T14).
//!
//! Uses [`proptest`] to generate arbitrary small `Datafile` values, write them
//! to in-memory bytes with [`WriteOptions`], parse them back with
//! [`ReadOptions`], and assert that all essential fields are preserved.
//!
//! This covers the write → read contract for a wide range of channel counts,
//! sample lengths, and channel names without enumerating cases manually.

#![cfg(all(feature = "write", feature = "read"))]

use biodream::{
    ByteOrder, Channel, ChannelData, Datafile, FileRevision, GraphMetadata, ReadOptions,
    WriteOptions,
};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

/// Generate a valid channel name: 1–8 ASCII uppercase letters.
fn arb_channel_name() -> impl Strategy<Value = String> {
    "[A-Z]{1,8}".prop_map(String::from)
}

/// Generate a valid unit string: 1–4 lowercase ASCII letters.
fn arb_units() -> impl Strategy<Value = String> {
    "[a-z]{1,4}".prop_map(String::from)
}

/// Generate a `Channel` at `frequency_divider` = 1 with `n_samples` raw `i16` samples.
///
/// All channels within a file must share the same sample count (real .acq files
/// record equal-duration channels), so the sample count is passed in from the
/// enclosing datafile strategy rather than being generated independently.
fn arb_channel(n_samples: usize) -> impl Strategy<Value = Channel> {
    (
        arb_channel_name(),
        arb_units(),
        prop::collection::vec(any::<i16>(), n_samples..=n_samples),
    )
        .prop_map(move |(name, units, samples)| Channel {
            name,
            units,
            samples_per_second: 1000.0,
            frequency_divider: 1,
            data: ChannelData::Raw(samples),
            point_count: n_samples,
        })
}

/// Generate a `Datafile` with 1–3 channels and no markers/journal.
///
/// All channels share the same sample count (2–40) to match .acq file semantics:
/// real recordings always have equal-duration channels.
fn arb_datafile() -> impl Strategy<Value = Datafile> {
    (1..=3usize, 2..=40usize).prop_flat_map(|(n_ch, n_samples)| {
        prop::collection::vec(arb_channel(n_samples), n_ch..=n_ch).prop_map(move |channels| {
            Datafile {
                metadata: GraphMetadata {
                    file_revision: FileRevision::new(43),
                    samples_per_second: 1000.0,
                    #[expect(
                        clippy::cast_possible_truncation,
                        reason = "n_ch bounded 1..=3, always fits u16"
                    )]
                    channel_count: n_ch as u16,
                    byte_order: ByteOrder::LittleEndian,
                    compressed: false,
                    title: None,
                    acquisition_datetime: None,
                    max_samples_per_second: None,
                },
                channels,
                markers: Vec::new(),
                journal: None,
            }
        })
    })
}

// ---------------------------------------------------------------------------
// Roundtrip helper
// ---------------------------------------------------------------------------

/// Write `df` to bytes, parse it back, and assert key fields are preserved.
///
/// Returns an error string on any write, parse, or assertion failure so that
/// the proptest runner can report the failing case.
fn roundtrip(df: &Datafile) -> Result<(), String> {
    // Write to in-memory buffer.
    let mut buf: Vec<u8> = Vec::new();
    WriteOptions::new()
        .write_stream(df, &mut buf)
        .map_err(|e| format!("write failed: {e}"))?;

    // Parse back.
    let parsed = ReadOptions::new()
        .read_bytes(&buf)
        .map_err(|e| format!("read failed: {e}"))?
        .into_value();

    // Assert channel count matches.
    if df.channels.len() != parsed.channels.len() {
        return Err(format!(
            "channel count: expected {}, got {}",
            df.channels.len(),
            parsed.channels.len()
        ));
    }

    // Assert per-channel fields.
    for (i, (orig, got)) in df.channels.iter().zip(parsed.channels.iter()).enumerate() {
        if orig.name != got.name {
            return Err(format!(
                "ch[{i}].name: expected {:?}, got {:?}",
                orig.name, got.name
            ));
        }
        if orig.units != got.units {
            return Err(format!(
                "ch[{i}].units: expected {:?}, got {:?}",
                orig.units, got.units
            ));
        }
        if orig.frequency_divider != got.frequency_divider {
            return Err(format!(
                "ch[{i}].frequency_divider: expected {}, got {}",
                orig.frequency_divider, got.frequency_divider
            ));
        }
        if orig.point_count != got.point_count {
            return Err(format!(
                "ch[{i}].point_count: expected {}, got {}",
                orig.point_count, got.point_count
            ));
        }

        // Assert raw sample data is bit-identical.
        let ChannelData::Raw(ref orig_raw) = orig.data else {
            return Err(format!("ch[{i}]: test data must be Raw"));
        };
        let ChannelData::Raw(ref got_raw) = got.data else {
            return Err(format!("ch[{i}]: parsed data must be Raw"));
        };
        if orig_raw != got_raw {
            return Err(format!(
                "ch[{i}].data: raw samples differ ({} orig vs {} got, first orig={:?} first got={:?})",
                orig_raw.len(),
                got_raw.len(),
                orig_raw.first(),
                got_raw.first(),
            ));
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Proptest cases
// ---------------------------------------------------------------------------

proptest! {
    /// Write an arbitrary `Datafile`, parse it back, assert all fields equal.
    #[test]
    fn prop_roundtrip_uncompressed(df in arb_datafile()) {
        if let Err(e) = roundtrip(&df) {
            return Err(proptest::test_runner::TestCaseError::Fail(e.into()));
        }
    }

    /// Vary channel count specifically to exercise interleave serialization.
    #[test]
    fn prop_roundtrip_channel_count(
        n_channels in 1usize..=3,
        n_samples in 2usize..=30,
    ) {
        let channels: Vec<Channel> = (0..n_channels)
            .map(|i| {
                let label = format!("CH{i}");
                let samples: Vec<i16> = vec![0i16; n_samples];
                Channel {
                    name: label,
                    units: String::from("au"),
                    samples_per_second: 1000.0,
                    frequency_divider: 1,
                    point_count: n_samples,
                    data: ChannelData::Raw(samples),
                }
            })
            .collect();

        let df = Datafile {
            metadata: GraphMetadata {
                file_revision: FileRevision::new(43),
                samples_per_second: 1000.0,
                #[expect(clippy::cast_possible_truncation, reason = "n_channels bounded 1..=3, always fits u16")]
                channel_count: n_channels as u16,
                byte_order: ByteOrder::LittleEndian,
                compressed: false,
                title: None,
                acquisition_datetime: None,
                max_samples_per_second: None,
            },
            channels,
            markers: Vec::new(),
            journal: None,
        };

        if let Err(e) = roundtrip(&df) {
            return Err(proptest::test_runner::TestCaseError::Fail(e.into()));
        }
    }
}

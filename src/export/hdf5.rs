//! HDF5 export (requires `hdf5` feature and the `libhdf5-dev` system library).
//!
//! The file layout is intentionally straightforward:
//! - `/metadata/*` scalar datasets (revision, base rate, compressed, channel count)
//! - `/channels/<idx>/scaled` float64 samples per channel
//! - `/channels/<idx>/raw` int16 samples when available and enabled
//! - `/markers/*` columnar marker fields (sample index, channel, created_at)

use std::path::Path;

use alloc::format;
use alloc::vec::Vec;

use crate::domain::{ChannelData, Datafile};
use crate::error::BiopacError;

/// Options for HDF5 export.
#[derive(Debug, Clone, Copy)]
pub struct Hdf5Options {
    /// Emit raw integer channel samples (where available) as `/channels/<idx>/raw`.
    pub include_raw: bool,
}

impl Default for Hdf5Options {
    fn default() -> Self {
        Self { include_raw: true }
    }
}

impl Hdf5Options {
    /// Create a new `Hdf5Options` with defaults.
    #[must_use]
    pub const fn new() -> Self {
        Self { include_raw: true }
    }

    /// Toggle writing raw integer datasets.
    #[must_use]
    pub const fn include_raw(mut self, yes: bool) -> Self {
        self.include_raw = yes;
        self
    }
}

/// Write `datafile` as an HDF5 file at `path`.
pub fn to_hdf5(
    datafile: &Datafile,
    path: impl AsRef<Path>,
    options: &Hdf5Options,
) -> Result<(), BiopacError> {
    let file = hdf5::File::create(path)?;

    let meta = file.create_group("metadata")?;
    meta.new_dataset_builder()
        .with_data(&[datafile.metadata.file_revision.0])
        .create("revision")?;
    meta.new_dataset_builder()
        .with_data(&[datafile.metadata.samples_per_second])
        .create("samples_per_second")?;
    let compressed: u8 = if datafile.metadata.compressed { 1 } else { 0 };
    meta.new_dataset_builder()
        .with_data(&[compressed])
        .create("compressed")?;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "channel counts are bounded by on-disk u16 headers"
    )]
    let channel_count = datafile.channels.len() as u32;
    meta.new_dataset_builder()
        .with_data(&[channel_count])
        .create("channel_count")?;

    let channels = file.create_group("channels")?;
    for (idx, ch) in datafile.channels.iter().enumerate() {
        let group = channels.create_group(&format!("{idx:04}"))?;

        let scaled = ch.scaled_samples();
        group
            .new_dataset_builder()
            .with_data(scaled.as_slice())
            .create("scaled")?;

        group
            .new_dataset_builder()
            .with_data(&[ch.samples_per_second])
            .create("samples_per_second")?;
        group
            .new_dataset_builder()
            .with_data(&[u32::from(ch.frequency_divider)])
            .create("frequency_divider")?;
        #[expect(
            clippy::cast_possible_truncation,
            reason = "point_count originates from parsed file lengths and is practical-size"
        )]
        let point_count = ch.point_count as u64;
        group
            .new_dataset_builder()
            .with_data(&[point_count])
            .create("point_count")?;

        if options.include_raw {
            match &ch.data {
                ChannelData::Raw(raw) | ChannelData::Scaled { raw, .. } => {
                    group
                        .new_dataset_builder()
                        .with_data(raw.as_slice())
                        .create("raw")?;
                }
                ChannelData::Float(_) => {}
            }
        }
    }

    let markers = file.create_group("markers")?;
    let marker_sample: Vec<u64> = datafile
        .markers
        .iter()
        .map(|m| {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "sample index is bounded by recording size"
            )]
            {
                m.global_sample_index as u64
            }
        })
        .collect();
    let marker_channel: Vec<i64> = datafile
        .markers
        .iter()
        .map(|m| {
            m.channel.map_or(-1_i64, |c| {
                #[expect(
                    clippy::cast_possible_wrap,
                    clippy::cast_possible_truncation,
                    reason = "channel indices are small positive ordinals"
                )]
                {
                    c as i64
                }
            })
        })
        .collect();
    let marker_created_at: Vec<i64> = datafile
        .markers
        .iter()
        .map(|m| m.created_at.map_or(i64::MIN, |ts| ts.as_secs()))
        .collect();

    markers
        .new_dataset_builder()
        .with_data(marker_sample.as_slice())
        .create("sample_index")?;
    markers
        .new_dataset_builder()
        .with_data(marker_channel.as_slice())
        .create("channel")?;
    markers
        .new_dataset_builder()
        .with_data(marker_created_at.as_slice())
        .create("created_at")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;

    use crate::domain::{ByteOrder, Channel, FileRevision, GraphMetadata, Marker, MarkerStyle};

    fn sample_datafile() -> Datafile {
        Datafile {
            metadata: GraphMetadata {
                file_revision: FileRevision::new(73),
                samples_per_second: 1000.0,
                channel_count: 1,
                byte_order: ByteOrder::LittleEndian,
                compressed: false,
                title: None,
                acquisition_datetime: None,
                max_samples_per_second: None,
            },
            channels: alloc::vec![Channel {
                name: String::from("ECG"),
                units: String::from("mV"),
                samples_per_second: 1000.0,
                frequency_divider: 1,
                data: ChannelData::Scaled {
                    raw: alloc::vec![100, 200, 300],
                    scale: 0.01,
                    offset: 0.0,
                },
                point_count: 3,
            }],
            markers: alloc::vec![Marker {
                label: String::from("start"),
                global_sample_index: 1,
                channel: None,
                style: MarkerStyle::GlobalEvent,
                created_at: None,
            }],
            journal: None,
        }
    }

    #[test]
    fn writes_hdf5_file_with_expected_groups() -> Result<(), BiopacError> {
        let path =
            std::env::temp_dir().join(format!("biodream_hdf5_test_{}.h5", std::process::id()));
        to_hdf5(&sample_datafile(), &path, &Hdf5Options::new())?;

        let file = hdf5::File::open(&path)?;
        assert!(file.group("metadata").is_ok());
        assert!(file.group("channels").is_ok());
        assert!(file.group("markers").is_ok());

        let _ = std::fs::remove_file(path);
        Ok(())
    }
}

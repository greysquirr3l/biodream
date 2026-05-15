//! Top-level container for a parsed .acq recording.

use alloc::vec::Vec;
use core::fmt;

use crate::domain::{Channel, FileRevision, GraphMetadata, Journal, Marker};

/// A fully parsed BIOPAC `AcqKnowledge` recording.
///
/// This is the primary output of [`crate::parser`]. It holds the graph
/// metadata, all channels (with their samples), all markers, and the optional
/// journal section.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Datafile {
    /// Graph-level metadata (revision, sample rate, byte order, …).
    pub metadata: GraphMetadata,
    /// All channels in file order.
    pub channels: Vec<Channel>,
    /// All event markers in file order.
    pub markers: Vec<Marker>,
    /// Optional journal annotation.
    pub journal: Option<Journal>,
}

impl Datafile {
    /// Look up a channel by name.
    ///
    /// Returns the first channel whose name matches `name` exactly.
    pub fn channel_by_name(&self, name: &str) -> Option<&Channel> {
        self.channels.iter().find(|c| c.name == name)
    }

    /// Returns the base (highest) sample rate across all channels.
    ///
    /// This is the `samples_per_second` field from the graph metadata.
    pub const fn base_sample_rate(&self) -> f64 {
        self.metadata.samples_per_second
    }

    /// Total number of channels.
    pub const fn channel_count(&self) -> usize {
        self.channels.len()
    }

    /// Total number of markers.
    pub const fn marker_count(&self) -> usize {
        self.markers.len()
    }

    /// Look up a channel by name (ergonomic alias for [`Datafile::channel_by_name`]).
    pub fn channel(&self, name: &str) -> Option<&Channel> {
        self.channel_by_name(name)
    }

    /// Iterate over all channels in file order.
    pub fn channels(&self) -> impl Iterator<Item = &Channel> {
        self.channels.iter()
    }

    /// The file's revision number.
    pub const fn revision(&self) -> FileRevision {
        self.metadata.file_revision
    }

    /// Base sampling rate as defined in the graph header (samples per second).
    pub const fn samples_per_second(&self) -> f64 {
        self.metadata.samples_per_second
    }

    /// Duration of the recording in seconds, computed from the longest channel.
    ///
    /// Returns `None` if there are no channels or all channels have zero
    /// samples.
    pub fn duration(&self) -> Option<f64> {
        self.channels
            .iter()
            .filter(|ch| ch.samples_per_second > 0.0 && ch.point_count > 0)
            .map(|ch| {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "point_count is a sample index; precision loss is negligible for physiological recordings"
                )]
                let count = ch.point_count as f64;
                count / ch.samples_per_second
            })
            .reduce(f64::max)
    }

    /// Returns a summary suitable for display or logging.
    pub fn summary(&self) -> impl fmt::Display + '_ {
        DatafileSummary(self)
    }
}

impl fmt::Display for Datafile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Datafile({} channels, {} markers, {})",
            self.channels.len(),
            self.markers.len(),
            self.metadata.file_revision,
        )
    }
}

struct DatafileSummary<'a>(&'a Datafile);

impl fmt::Display for DatafileSummary<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let d = self.0;
        writeln!(
            f,
            "AcqKnowledge file  revision={} ({}) compressed={} rate={} Hz",
            d.metadata.file_revision.0,
            d.metadata.file_revision.display_version(),
            d.metadata.compressed,
            d.metadata.samples_per_second,
        )?;
        for (i, ch) in d.channels.iter().enumerate() {
            writeln!(f, "  [{i}] {ch}")?;
        }
        if d.markers.is_empty() {
            writeln!(f, "  (no markers)")?;
        } else {
            writeln!(f, "  {} marker(s)", d.markers.len())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ByteOrder, ChannelData, FileRevision};

    fn make_datafile() -> Datafile {
        Datafile {
            metadata: GraphMetadata {
                file_revision: FileRevision::new(73),
                samples_per_second: 1000.0,
                channel_count: 2,
                byte_order: ByteOrder::LittleEndian,
                compressed: false,
                title: None,
                acquisition_datetime: None,
                max_samples_per_second: None,
            },
            channels: alloc::vec![
                Channel {
                    name: String::from("ECG"),
                    units: String::from("mV"),
                    samples_per_second: 1000.0,
                    frequency_divider: 1,
                    data: ChannelData::Raw(alloc::vec![0, 1, 2]),
                    point_count: 3,
                },
                Channel {
                    name: String::from("EEG"),
                    units: String::from("μV"),
                    samples_per_second: 500.0,
                    frequency_divider: 2,
                    data: ChannelData::Raw(alloc::vec![10, 20]),
                    point_count: 2,
                },
            ],
            markers: Vec::new(),
            journal: None,
        }
    }

    use alloc::string::String;

    #[test]
    fn channel_by_name_found() {
        let df = make_datafile();
        assert!(df.channel_by_name("ECG").is_some());
        assert_eq!(
            df.channel_by_name("ECG").map(|c| &c.units),
            Some(&String::from("mV"))
        );
    }

    #[test]
    fn channel_by_name_missing() {
        let df = make_datafile();
        assert!(df.channel_by_name("nonexistent").is_none());
    }

    #[test]
    fn display_format_is_not_empty() {
        let df = make_datafile();
        let s = alloc::format!("{df}");
        assert!(s.contains("Datafile("));
        assert!(s.contains("2 channels"));
    }
}

//! Channel types — the primary data carrier.

use alloc::{string::String, vec::Vec};
use core::fmt;

/// Sample data for a single channel.
///
/// Stores raw integer samples, float samples, or lazily-scaleable integer
/// samples paired with their calibration coefficients. Scale/offset is applied
/// on demand via [`Channel::scaled_samples`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ChannelData {
    /// Raw 16-bit signed samples (no scale/offset applied).
    Raw(Vec<i16>),
    /// Native 64-bit floating-point samples.
    Float(Vec<f64>),
    /// Raw samples paired with calibration coefficients.
    ///
    /// Physical value = `raw * scale + offset`.
    Scaled {
        /// Raw 16-bit samples from the file.
        raw: Vec<i16>,
        /// Amplitude scale factor (`dAmplScale`).
        scale: f64,
        /// Amplitude offset (`dAmplOffset`).
        offset: f64,
    },
}

impl ChannelData {
    /// Number of samples stored.
    pub const fn len(&self) -> usize {
        match self {
            Self::Raw(v) => v.len(),
            Self::Float(v) => v.len(),
            Self::Scaled { raw, .. } => raw.len(),
        }
    }

    /// Returns `true` if there are no samples.
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Per-channel metadata extracted from the channel header.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ChannelMetadata {
    /// Channel name (from `szCommentText`).
    pub name: String,
    /// Unit string (from `szUnitsText`).
    pub units: String,
    /// Extended description (if present).
    pub description: String,
    /// Sampling rate divider relative to the base rate.
    ///
    /// A value of 1 means this channel runs at the base sample rate. A value
    /// of 2 means it runs at half the base rate.
    pub frequency_divider: u16,
    /// Amplitude scale coefficient (`dAmplScale`).
    pub amplitude_scale: f64,
    /// Amplitude offset (`dAmplOffset`).
    pub amplitude_offset: f64,
    /// Display order index.
    pub display_order: u16,
    /// Expected number of samples for this channel (`lBufLength`).
    ///
    /// A value of 0 means the count was not set in the file header; the reader
    /// will fall back to reading until EOF.
    pub sample_count: u32,
}

/// A single physiological signal channel with its sample data.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Channel {
    /// Channel name.
    pub name: String,
    /// Physical unit label (e.g. `"mV"`, `"°C"`).
    pub units: String,
    /// Effective sampling rate for this channel in samples/second.
    pub samples_per_second: f64,
    /// Frequency divider relative to the base rate.
    pub frequency_divider: u16,
    /// Sample data — raw, float, or scaleable.
    pub data: ChannelData,
    /// Number of samples stored in `data`.
    pub point_count: usize,
}

impl Channel {
    /// Returns all samples as scaled `f64` values.
    ///
    /// For `ChannelData::Scaled`, applies `raw * scale + offset`.
    /// For `ChannelData::Raw`, casts each `i16` to `f64` with no scaling.
    /// For `ChannelData::Float`, clones the underlying vector.
    pub fn scaled_samples(&self) -> Vec<f64> {
        match &self.data {
            ChannelData::Raw(raw) => raw.iter().map(|&v| f64::from(v)).collect(),
            ChannelData::Float(f) => f.clone(),
            ChannelData::Scaled { raw, scale, offset } => {
                raw.iter().map(|&v| f64::from(v) * scale + offset).collect()
            }
        }
    }

    /// Interpolates this channel's data to a higher `base_rate` by repeating
    /// each sample `factor` times (nearest-neighbour upsampling).
    ///
    /// If `self.samples_per_second` already equals `base_rate`, the scaled
    /// samples are returned unchanged (no copy of raw data is made until then).
    ///
    /// Uses a sliding-window linear-interpolation pass internally.
    pub fn upsampled(&self, base_rate: f64) -> Vec<f64> {
        let src = self.scaled_samples();

        let factor_f = base_rate / self.samples_per_second;
        // Nothing to do when rates already match.
        // Use direct difference check instead of abs() which is not available
        // in all no_std configurations.
        let diff = factor_f - 1.0_f64;
        if diff > -f64::EPSILON && diff < f64::EPSILON {
            return src;
        }

        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "factor_f = base_rate / samples_per_second; always positive for valid .acq files"
        )]
        let factor = (factor_f + 0.5) as usize;
        if factor == 0 {
            return src;
        }

        let mut out = Vec::with_capacity(src.len() * factor);

        // Linear interpolation between consecutive samples.
        for [a, b] in src.array_windows::<2>() {
            for i in 0..factor {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "sample indices are small frame counts; precision loss is irrelevant for resampling"
                )]
                out.push(a + (b - a) * (i as f64 / factor as f64));
            }
        }

        // Repeat the last sample to fill the final block.
        if let Some(&last) = src.last() {
            for _ in 0..factor {
                out.push(last);
            }
        }

        out
    }
}

impl fmt::Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Channel({:?}, {} samples, {} Hz)",
            self.name, self.point_count, self.samples_per_second
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_channel(divider: u16, base_rate: f64, samples: Vec<i16>) -> Channel {
        let count = samples.len();
        Channel {
            name: String::from("test"),
            units: String::from("mV"),
            samples_per_second: base_rate / f64::from(divider),
            frequency_divider: divider,
            data: ChannelData::Raw(samples),
            point_count: count,
        }
    }

    #[test]
    fn channel_frequency_divider_halves_rate() {
        let ch = make_channel(2, 1000.0, alloc::vec![0, 1, 2]);
        assert!((ch.samples_per_second - 500.0).abs() < f64::EPSILON);
    }

    #[test]
    fn scaled_samples_raw_cast() {
        let ch = make_channel(1, 1000.0, alloc::vec![1, 2, 3]);
        let scaled = ch.scaled_samples();
        assert_eq!(scaled, alloc::vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn scaled_samples_with_coefficients() {
        let ch = Channel {
            name: String::from("ecg"),
            units: String::from("mV"),
            samples_per_second: 1000.0,
            frequency_divider: 1,
            data: ChannelData::Scaled {
                raw: alloc::vec![0, 100],
                scale: 0.01,
                offset: -0.5,
            },
            point_count: 2,
        };
        let scaled = ch.scaled_samples();
        assert!((scaled.first().copied().unwrap_or(f64::NAN) - (-0.5)).abs() < 1e-10);
        assert!((scaled.get(1).copied().unwrap_or(f64::NAN) - 0.5).abs() < 1e-10);
    }

    #[test]
    fn upsampled_identity_at_same_rate() {
        let ch = make_channel(1, 1000.0, alloc::vec![10, 20, 30]);
        let up = ch.upsampled(1000.0);
        assert_eq!(up, alloc::vec![10.0, 20.0, 30.0]);
    }

    #[test]
    fn upsampled_doubles_samples_at_2x_rate() {
        let ch = make_channel(2, 1000.0, alloc::vec![0, 10]);
        // rate = 500 Hz, upsampling to 1000 Hz → factor = 2
        let up = ch.upsampled(1000.0);
        // window [0, 10]: i=0 → 0.0, i=1 → 5.0
        // last sample repeated: 10.0, 10.0
        assert_eq!(up.len(), 4);
        assert!((up.first().copied().unwrap_or(f64::NAN) - 0.0).abs() < 1e-10);
        assert!((up.get(1).copied().unwrap_or(f64::NAN) - 5.0).abs() < 1e-10);
        assert!((up.get(2).copied().unwrap_or(f64::NAN) - 10.0).abs() < 1e-10);
        assert!((up.get(3).copied().unwrap_or(f64::NAN) - 10.0).abs() < 1e-10);
    }
}

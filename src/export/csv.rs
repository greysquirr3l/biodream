//! CSV export for biodream recordings.
//!
//! Produces one row per base-rate time step, with a leading time column and one
//! column per channel. Channels that run slower than the base rate receive empty
//! cells in the rows where they hold no sample — matching the behaviour of
//! `bioread`'s `acq2txt`.
//!
//! # Example
//!
//! ```rust,ignore
//! use biodream::{read_file, CsvOptions, TimeFormat, to_csv};
//! use std::io::BufWriter;
//!
//! let df = read_file("recording.acq")?.into_value();
//! let stdout = BufWriter::new(std::io::stdout());
//! to_csv(&df, stdout, &CsvOptions::new())?;
//! ```

use std::io::Write;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::domain::{ChannelData, Datafile};
use crate::error::BiopacError;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// How to express the leading time-index column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeFormat {
    /// Fractional seconds since recording start (e.g. `0.001000`). Default.
    #[default]
    Seconds,
    /// Fractional milliseconds since recording start (e.g. `1.000000`).
    Milliseconds,
    /// `HH:MM:SS.ffffff` wall-clock string.
    Hms,
}

/// Options that control CSV output layout and formatting.
///
/// All setter methods return `Self` for chaining:
///
/// ```rust
/// # use biodream::CsvOptions;
/// let opts = CsvOptions::new().precision(4).delimiter(b'\t');
/// ```
#[derive(Debug, Clone)]
pub struct CsvOptions {
    /// Field separator byte. Default: `b','`.
    pub delimiter: u8,
    /// Decimal places for floating-point values. Default: `6`.
    pub precision: usize,
    /// Time column representation. Default: [`TimeFormat::Seconds`].
    pub time_format: TimeFormat,
    /// If set, only export channels at these zero-based indices.
    pub channel_indices: Option<Vec<usize>>,
    /// Value written for absent samples. Default: `""` (matches bioread).
    pub fill_value: String,
    /// Emit `<name>_raw` integer columns alongside the scaled columns.
    pub include_raw: bool,
}

impl Default for CsvOptions {
    fn default() -> Self {
        Self {
            delimiter: b',',
            precision: 6,
            time_format: TimeFormat::Seconds,
            channel_indices: None,
            fill_value: String::new(),
            include_raw: false,
        }
    }
}

impl CsvOptions {
    /// Create a `CsvOptions` with defaults (comma delimiter, 6 decimal places,
    /// seconds time column, all channels).
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the field separator. Use `b'\t'` for TSV.
    #[must_use]
    pub const fn delimiter(mut self, d: u8) -> Self {
        self.delimiter = d;
        self
    }

    /// Set the number of decimal places for floating-point values.
    #[must_use]
    pub const fn precision(mut self, p: usize) -> Self {
        self.precision = p;
        self
    }

    /// Set the time column format.
    #[must_use]
    pub const fn time_format(mut self, f: TimeFormat) -> Self {
        self.time_format = f;
        self
    }

    /// Restrict output to these channel indices (zero-based).
    ///
    /// Any out-of-range index causes [`to_csv`] to return
    /// [`BiopacError::InvalidChannel`].
    #[must_use]
    pub fn channels(mut self, indices: &[usize]) -> Self {
        self.channel_indices = Some(indices.to_vec());
        self
    }

    /// Override the fill string for cells where a sub-rate channel has no
    /// sample. The default `""` matches bioread's `acq2txt` output.
    #[must_use]
    pub fn fill_value(mut self, v: impl Into<String>) -> Self {
        self.fill_value = v.into();
        self
    }

    /// Emit a `<name>_raw` integer column immediately after each scaled column.
    /// Channels with native float data emit the fill value for the raw column.
    #[must_use]
    pub const fn include_raw(mut self, yes: bool) -> Self {
        self.include_raw = yes;
        self
    }
}

// ---------------------------------------------------------------------------
// Export entry point
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Private helpers for to_csv
// ---------------------------------------------------------------------------

/// Per-channel data buffer, pre-computed once before the row loop.
struct ChannelBuf {
    scaled: Vec<f64>,
    /// `None` for Float channels even when `include_raw` is set — there are
    /// no raw integer samples to report for those.
    raw: Option<Vec<i16>>,
    divider: usize,
}

/// Write `datafile` as CSV to `writer`.
///
/// Rows correspond to base-rate time steps. Channels with a lower sampling rate
/// (divider > 1) have empty cells for time steps where they hold no sample.
///
/// # Errors
///
/// - [`BiopacError::Io`] — underlying write failure.
/// - [`BiopacError::InvalidChannel`] — `options.channel_indices` contains an
///   out-of-range index.
#[expect(
    clippy::too_many_lines,
    reason = "single coherent export routine; splitting would obscure the data-flow"
)]
pub fn to_csv<W: Write>(
    datafile: &Datafile,
    writer: W,
    options: &CsvOptions,
) -> Result<(), BiopacError> {
    let base_rate = datafile.metadata.samples_per_second;

    // --- Channel selection ------------------------------------------------
    // Validate all requested indices before touching the writer.
    let selected: Vec<(usize, &crate::domain::Channel)> = match &options.channel_indices {
        None => datafile.channels.iter().enumerate().collect(),
        Some(indices) => {
            let mut out = Vec::with_capacity(indices.len());
            for &idx in indices {
                let ch = datafile.channels.get(idx).ok_or_else(|| {
                    BiopacError::InvalidChannel(format!(
                        "channel index {idx} out of range (file has {} channels)",
                        datafile.channels.len()
                    ))
                })?;
                out.push((idx, ch));
            }
            out
        }
    };

    // --- Pre-compute sample arrays ----------------------------------------
    // Compute scaled floats (and optionally raw integers) for each channel
    // once, rather than recomputing per-row.
    let mut bufs: Vec<ChannelBuf> = Vec::with_capacity(selected.len());
    for (_, ch) in &selected {
        let scaled = ch.scaled_samples();
        let raw = if options.include_raw {
            match &ch.data {
                ChannelData::Raw(v) | ChannelData::Scaled { raw: v, .. } => Some(v.clone()),
                ChannelData::Float(_) => None,
            }
        } else {
            None
        };
        let divider = usize::from(ch.frequency_divider).max(1);
        bufs.push(ChannelBuf {
            scaled,
            raw,
            divider,
        });
    }

    // --- Row count --------------------------------------------------------
    // Total rows = max(point_count × divider) across all selected channels.
    // When the selection is empty this is 0 and we write only the header.
    let total_rows = selected
        .iter()
        .map(|(_, ch)| {
            ch.point_count
                .saturating_mul(usize::from(ch.frequency_divider).max(1))
        })
        .max()
        .unwrap_or(0);

    // --- CSV writer -------------------------------------------------------
    let mut wtr = csv::WriterBuilder::new()
        .delimiter(options.delimiter)
        .from_writer(writer);

    // Header row.
    let time_col = match options.time_format {
        TimeFormat::Seconds => "time_s",
        TimeFormat::Milliseconds => "time_ms",
        TimeFormat::Hms => "time_hms",
    };
    let col_cap = 1 + selected.len() * (if options.include_raw { 2 } else { 1 });
    let mut header: Vec<String> = Vec::with_capacity(col_cap);
    header.push(time_col.to_string());
    for (_, ch) in &selected {
        header.push(ch.name.clone());
        if options.include_raw {
            header.push(format!("{}_raw", ch.name));
        }
    }
    wtr.write_record(&header).map_err(wrap_csv_err)?;

    // Data rows — reuse the row buffer across iterations to minimise allocation.
    let mut row: Vec<String> = Vec::with_capacity(col_cap);

    for t in 0..total_rows {
        #[expect(
            clippy::cast_precision_loss,
            reason = "row index; for physiological recordings the precision loss is negligible"
        )]
        let t_f = t as f64;

        let time_str = if base_rate > 0.0 {
            let t_secs = t_f / base_rate;
            match options.time_format {
                TimeFormat::Seconds => {
                    format!("{t_secs:.prec$}", prec = options.precision)
                }
                TimeFormat::Milliseconds => {
                    format!("{:.prec$}", t_secs * 1_000.0, prec = options.precision)
                }
                TimeFormat::Hms => format_hms(t_secs),
            }
        } else {
            "0".to_string()
        };

        row.clear();
        row.push(time_str);

        for buf in &bufs {
            if t % buf.divider == 0 {
                let s_idx = t / buf.divider;

                match buf.scaled.get(s_idx) {
                    Some(&v) => row.push(format!("{v:.prec$}", prec = options.precision)),
                    None => row.push(options.fill_value.clone()),
                }

                if options.include_raw {
                    match buf.raw.as_deref().and_then(|r| r.get(s_idx)) {
                        Some(&v) => row.push(v.to_string()),
                        None => row.push(options.fill_value.clone()),
                    }
                }
            } else {
                row.push(options.fill_value.clone());
                if options.include_raw {
                    row.push(options.fill_value.clone());
                }
            }
        }

        wtr.write_record(&row).map_err(wrap_csv_err)?;
    }

    wtr.flush().map_err(BiopacError::Io)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Format `secs` as `HH:MM:SS.ffffff`.
fn format_hms(secs: f64) -> String {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "secs is non-negative from a valid .acq file; truncation to u64 is intentional"
    )]
    let whole = secs as u64;
    let h = whole / 3_600;
    let m = (whole % 3_600) / 60;
    let s = secs % 60.0;
    format!("{h:02}:{m:02}:{s:09.6}")
}

/// Wrap a `csv::Error` into a `BiopacError::Io`.
fn wrap_csv_err(e: csv::Error) -> BiopacError {
    BiopacError::Io(std::io::Error::other(e))
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use alloc::string::ToString;
    use alloc::vec::Vec;

    use super::*;

    fn two_channel_datafile(rate: f64, n: usize) -> Datafile {
        use crate::domain::{ByteOrder, Channel, ChannelData, FileRevision, GraphMetadata};

        Datafile {
            metadata: GraphMetadata {
                file_revision: FileRevision::new(38),
                samples_per_second: rate,
                channel_count: 2,
                byte_order: ByteOrder::LittleEndian,
                compressed: false,
                title: None,
                acquisition_datetime: None,
                max_samples_per_second: None,
            },
            channels: alloc::vec![
                Channel {
                    name: "ECG".to_string(),
                    units: "mV".to_string(),
                    samples_per_second: rate,
                    frequency_divider: 1,
                    data: ChannelData::Scaled {
                        raw: (0..n).map(|i| i16::try_from(i).unwrap_or(0)).collect(),
                        scale: 1.0,
                        offset: 0.0,
                    },
                    point_count: n,
                },
                Channel {
                    name: "GSR".to_string(),
                    units: "\u{b5}S".to_string(),
                    samples_per_second: rate,
                    frequency_divider: 1,
                    data: ChannelData::Float(
                        (0..n)
                            .map(|i| f64::from(i16::try_from(i).unwrap_or(0)) * 2.0)
                            .collect(),
                    ),
                    point_count: n,
                },
            ],
            markers: alloc::vec![],
            journal: None,
        }
    }

    fn csv_string(df: &Datafile, opts: &CsvOptions) -> Result<String, BiopacError> {
        let mut buf: Vec<u8> = Vec::new();
        to_csv(df, &mut buf, opts)?;
        String::from_utf8(buf).map_err(|e| BiopacError::Validation(e.to_string()))
    }

    fn parse_rows(csv: &str) -> alloc::vec::Vec<alloc::vec::Vec<alloc::string::String>> {
        csv.lines()
            .map(|l| l.split(',').map(ToString::to_string).collect())
            .collect()
    }

    #[test]
    fn header_matches_channel_names() -> Result<(), BiopacError> {
        let df = two_channel_datafile(1000.0, 4);
        let csv = csv_string(&df, &CsvOptions::new())?;
        let first_line = csv.lines().next().unwrap_or("");
        assert_eq!(first_line, "time_s,ECG,GSR");
        Ok(())
    }

    #[test]
    fn row_count_matches_sample_count() -> Result<(), BiopacError> {
        let n = 8;
        let df = two_channel_datafile(1000.0, n);
        let csv = csv_string(&df, &CsvOptions::new())?;
        // 1 header + n data rows
        assert_eq!(csv.lines().count(), n + 1);
        Ok(())
    }

    #[test]
    fn values_roundtrip_correctly() -> Result<(), BiopacError> {
        let df = two_channel_datafile(1000.0, 3);
        let csv = csv_string(&df, &CsvOptions::new().precision(1))?;
        let rows = parse_rows(&csv);

        // Row index 2 → t=1 ms: ECG=1.0, GSR=2.0
        let data = rows
            .get(2)
            .ok_or_else(|| BiopacError::Validation("missing row at t=1ms".into()))?;
        assert_eq!(data.get(1).map(String::as_str), Some("1.0"), "ECG at t=1ms");
        assert_eq!(data.get(2).map(String::as_str), Some("2.0"), "GSR at t=1ms");
        Ok(())
    }

    #[test]
    fn precision_controls_decimal_places() -> Result<(), BiopacError> {
        let df = two_channel_datafile(1000.0, 2);
        let csv2 = csv_string(&df, &CsvOptions::new().precision(2))?;
        let csv4 = csv_string(&df, &CsvOptions::new().precision(4))?;

        // first data row: time = 0.0 s
        let row2: alloc::vec::Vec<&str> = csv2.lines().nth(1).unwrap_or("").split(',').collect();
        let row4: alloc::vec::Vec<&str> = csv4.lines().nth(1).unwrap_or("").split(',').collect();
        assert_eq!(row2.first().copied(), Some("0.00"), "2dp time");
        assert_eq!(row4.first().copied(), Some("0.0000"), "4dp time");
        Ok(())
    }

    #[test]
    fn milliseconds_time_format() -> Result<(), BiopacError> {
        let df = two_channel_datafile(1000.0, 2);
        let csv = csv_string(
            &df,
            &CsvOptions::new()
                .time_format(TimeFormat::Milliseconds)
                .precision(3),
        )?;
        let header = csv.lines().next().unwrap_or("");
        assert_eq!(header.split(',').next(), Some("time_ms"));

        // second data row: t=1 → 1.000 ms
        let row: Vec<&str> = csv.lines().nth(2).unwrap_or("").split(',').collect();
        assert_eq!(row.first().copied(), Some("1.000"));
        Ok(())
    }

    #[test]
    fn hms_time_format() -> Result<(), BiopacError> {
        let df = two_channel_datafile(1.0, 2); // 1 Hz → t=0 s and t=1 s
        let csv = csv_string(&df, &CsvOptions::new().time_format(TimeFormat::Hms))?;
        let row0: Vec<&str> = csv.lines().nth(1).unwrap_or("").split(',').collect();
        assert_eq!(row0.first().copied(), Some("00:00:00.000000"));
        let row1: Vec<&str> = csv.lines().nth(2).unwrap_or("").split(',').collect();
        assert_eq!(row1.first().copied(), Some("00:00:01.000000"));
        Ok(())
    }

    #[test]
    fn tab_delimiter() -> Result<(), BiopacError> {
        let df = two_channel_datafile(1000.0, 1);
        let csv = csv_string(&df, &CsvOptions::new().delimiter(b'\t'))?;
        let header = csv.lines().next().unwrap_or("");
        assert!(header.contains('\t'), "header should be tab-separated");
        assert!(!header.contains(','), "header should not contain commas");
        Ok(())
    }

    #[test]
    fn channel_filter_selects_subset() -> Result<(), BiopacError> {
        let df = two_channel_datafile(1000.0, 4);
        let csv = csv_string(&df, &CsvOptions::new().channels(&[1]))?;
        let header = csv.lines().next().unwrap_or("");
        assert_eq!(header, "time_s,GSR");
        Ok(())
    }

    #[test]
    fn channel_filter_invalid_index_returns_error() {
        let df = two_channel_datafile(1000.0, 4);
        let mut buf: Vec<u8> = Vec::new();
        let result = to_csv(&df, &mut buf, &CsvOptions::new().channels(&[99]));
        assert!(
            matches!(result, Err(BiopacError::InvalidChannel(_))),
            "out-of-range index should return InvalidChannel"
        );
    }

    #[test]
    fn mixed_rate_produces_empty_cells() -> Result<(), BiopacError> {
        use crate::domain::{ByteOrder, Channel, ChannelData, FileRevision, GraphMetadata};

        // FAST at 1000 Hz (divider=1), SLOW at 500 Hz (divider=2).
        let rate = 1000.0;
        let df = Datafile {
            metadata: GraphMetadata {
                file_revision: FileRevision::new(38),
                samples_per_second: rate,
                channel_count: 2,
                byte_order: ByteOrder::LittleEndian,
                compressed: false,
                title: None,
                acquisition_datetime: None,
                max_samples_per_second: None,
            },
            channels: alloc::vec![
                Channel {
                    name: "FAST".to_string(),
                    units: "mV".to_string(),
                    samples_per_second: rate,
                    frequency_divider: 1,
                    data: ChannelData::Raw(alloc::vec![10, 20, 30, 40]),
                    point_count: 4,
                },
                Channel {
                    name: "SLOW".to_string(),
                    units: "µS".to_string(),
                    samples_per_second: rate / 2.0,
                    frequency_divider: 2,
                    data: ChannelData::Raw(alloc::vec![100, 200]),
                    point_count: 2,
                },
            ],
            markers: alloc::vec![],
            journal: None,
        };

        let csv = csv_string(&df, &CsvOptions::new().precision(0))?;
        let rows = parse_rows(&csv);

        // 1 header + 4 data rows (FAST drives the row count)
        assert_eq!(rows.len(), 5, "should have 5 rows total");

        // t=0: both have samples
        let r0 = rows
            .get(1)
            .ok_or_else(|| BiopacError::Validation("missing row 0".into()))?;
        assert_eq!(r0.get(1).map(String::as_str), Some("10"), "FAST[0]");
        assert_eq!(r0.get(2).map(String::as_str), Some("100"), "SLOW[0]");

        // t=1: FAST has sample, SLOW is empty
        let r1 = rows
            .get(2)
            .ok_or_else(|| BiopacError::Validation("missing row 1".into()))?;
        assert_eq!(r1.get(1).map(String::as_str), Some("20"), "FAST[1]");
        assert_eq!(r1.get(2).map(String::as_str), Some(""), "SLOW empty at t=1");

        // t=2: both have samples
        let r2 = rows
            .get(3)
            .ok_or_else(|| BiopacError::Validation("missing row 2".into()))?;
        assert_eq!(r2.get(1).map(String::as_str), Some("30"), "FAST[2]");
        assert_eq!(r2.get(2).map(String::as_str), Some("200"), "SLOW[1]");

        Ok(())
    }

    #[test]
    fn custom_fill_value() -> Result<(), BiopacError> {
        use crate::domain::{ByteOrder, Channel, ChannelData, FileRevision, GraphMetadata};

        let rate = 1000.0;
        let df = Datafile {
            metadata: GraphMetadata {
                file_revision: FileRevision::new(38),
                samples_per_second: rate,
                channel_count: 1,
                byte_order: ByteOrder::LittleEndian,
                compressed: false,
                title: None,
                acquisition_datetime: None,
                max_samples_per_second: None,
            },
            channels: alloc::vec![Channel {
                name: "SLOW".to_string(),
                units: "mV".to_string(),
                samples_per_second: rate / 2.0,
                frequency_divider: 2,
                data: ChannelData::Raw(alloc::vec![1, 2]),
                point_count: 2,
            }],
            markers: alloc::vec![],
            journal: None,
        };

        let csv = csv_string(&df, &CsvOptions::new().fill_value("N/A").precision(0))?;
        // Row at t=1 (odd index → no SLOW sample)
        let row1: Vec<&str> = csv.lines().nth(2).unwrap_or("").split(',').collect();
        assert_eq!(row1.get(1).copied(), Some("N/A"));
        Ok(())
    }

    #[test]
    fn include_raw_adds_raw_columns() -> Result<(), BiopacError> {
        let df = two_channel_datafile(1000.0, 2);
        let csv = csv_string(&df, &CsvOptions::new().include_raw(true))?;
        let header = csv.lines().next().unwrap_or("");
        assert!(header.contains("ECG_raw"), "header should contain ECG_raw");
        assert!(header.contains("GSR_raw"), "header should contain GSR_raw");
        Ok(())
    }

    #[test]
    fn empty_datafile_writes_only_header() -> Result<(), BiopacError> {
        use crate::domain::{ByteOrder, FileRevision, GraphMetadata};

        let df = Datafile {
            metadata: GraphMetadata {
                file_revision: FileRevision::new(38),
                samples_per_second: 1000.0,
                channel_count: 0,
                byte_order: ByteOrder::LittleEndian,
                compressed: false,
                title: None,
                acquisition_datetime: None,
                max_samples_per_second: None,
            },
            channels: alloc::vec![],
            markers: alloc::vec![],
            journal: None,
        };

        let csv = csv_string(&df, &CsvOptions::new())?;
        assert_eq!(
            csv.lines().count(),
            1,
            "empty datafile should produce only the header row"
        );
        Ok(())
    }

    #[test]
    fn format_hms_zero() {
        assert_eq!(format_hms(0.0), "00:00:00.000000");
    }

    #[test]
    fn format_hms_one_hour() {
        assert_eq!(format_hms(3_600.0), "01:00:00.000000");
    }

    #[test]
    fn format_hms_fractional_seconds() {
        assert_eq!(format_hms(0.001), "00:00:00.001000");
    }
}

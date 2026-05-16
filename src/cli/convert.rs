//! `biopac convert` — export a .acq file to CSV, Arrow IPC, Parquet, or HDF5.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::Args;

use biodream::{CsvOptions, ReadOptions, TimeFormat};

// ---------------------------------------------------------------------------
// Time format for CSV
// ---------------------------------------------------------------------------

/// Time-column format for CSV output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum CliTimeFormat {
    /// Fractional seconds since recording start (default).
    #[default]
    Seconds,
    /// Fractional milliseconds since recording start.
    Milliseconds,
    /// `HH:MM:SS.ffffff` wall-clock string.
    Hms,
}

impl From<CliTimeFormat> for TimeFormat {
    fn from(f: CliTimeFormat) -> Self {
        match f {
            CliTimeFormat::Seconds => Self::Seconds,
            CliTimeFormat::Milliseconds => Self::Milliseconds,
            CliTimeFormat::Hms => Self::Hms,
        }
    }
}

// ---------------------------------------------------------------------------
// CLI args
// ---------------------------------------------------------------------------

/// Output format for `biopac convert`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    /// Comma-separated values.
    Csv,
    /// Apache Arrow IPC streaming format.
    Arrow,
    /// Apache Parquet columnar format.
    Parquet,
    /// HDF5 container.
    Hdf5,
    /// MATLAB v7.3-compatible HDF5 container.
    Mat,
}

/// Arguments for the `convert` subcommand.
#[derive(Debug, Args)]
pub struct ConvertArgs {
    /// Path to the .acq file, or `-` to read from stdin.
    #[arg(value_name = "FILE")]
    pub path: PathBuf,

    /// Output file path. Format is inferred from the extension when not set.
    ///
    /// Default: derive from input filename (e.g. `data.acq` → `data.csv`).
    #[arg(short, long, value_name = "OUTPUT")]
    pub output: Option<PathBuf>,

    /// Output format: csv, arrow, parquet, hdf5, or mat. Inferred from the output
    /// file extension when not specified.
    #[arg(short = 'F', long, value_name = "FORMAT")]
    pub format: Option<OutputFormat>,

    /// Channel indices to include (comma-separated, e.g. `0,2`).
    /// Default: all channels. Conflicts with --channel-name and --channel-contains.
    #[arg(long, value_delimiter = ',', value_name = "INDEX", conflicts_with_all = ["channel_name", "channel_contains"])]
    pub channels: Option<Vec<usize>>,

    /// Select channels by exact name. May be specified multiple times.
    /// Conflicts with --channels and --channel-contains.
    #[arg(long = "channel-name", value_name = "NAME", conflicts_with_all = ["channels", "channel_contains"])]
    pub channel_name: Option<Vec<String>>,

    /// Select the first channel whose name contains this substring (case-insensitive).
    /// Conflicts with --channels and --channel-name.
    #[arg(long = "channel-contains", value_name = "NEEDLE", conflicts_with_all = ["channels", "channel_name"])]
    pub channel_contains: Option<String>,

    /// Convert raw integer samples to scaled float values.
    #[arg(long)]
    pub scaled: bool,

    // --- CSV-only options -------------------------------------------------
    /// Time-column format for CSV output: seconds (default), milliseconds, or hms.
    #[arg(long, value_enum, default_value_t = CliTimeFormat::Seconds, value_name = "FMT")]
    pub time_format: CliTimeFormat,

    /// Decimal places for floating-point values in CSV output. Default: 6.
    #[arg(long, default_value_t = 6, value_name = "N")]
    pub precision: usize,

    /// Field separator for CSV output. Use a single character or `tab`. Default: `,`.
    #[arg(long, default_value = ",", value_name = "CHAR")]
    pub delimiter: String,

    /// Emit a `<name>_raw` integer column alongside each scaled column in CSV output.
    #[arg(long)]
    pub include_raw: bool,

    /// Value written for absent samples in CSV output. Default: empty string.
    #[arg(long, default_value = "", value_name = "STR")]
    pub fill_value: String,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn run(args: ConvertArgs) -> anyhow::Result<()> {
    let format = resolve_format(args.format, &args.path, args.output.as_deref())?;

    // Build CSV options before args fields are partially moved below.
    let csv_opts = build_csv_options(&args)?;

    let output_path = resolve_output_path(args.output, &args.path, format)?;

    // Resolve channel indices from whichever selection flag was given.
    let channel_indices = resolve_channel_indices(
        args.channels.as_deref(),
        args.channel_name.as_deref(),
        args.channel_contains.as_deref(),
        &args.path,
    )?;

    let result = read_with_options(&args.path, channel_indices.as_deref(), args.scaled)?;
    let df = result.value;

    write_output(format, &df, &output_path, &csv_opts)
}

// ---------------------------------------------------------------------------
// Channel resolution
// ---------------------------------------------------------------------------

/// Resolve the final channel index list from whichever selection flag was given.
///
/// Returns `None` when no filter was requested (= all channels).
fn resolve_channel_indices(
    by_index: Option<&[usize]>,
    by_name: Option<&[String]>,
    by_contains: Option<&str>,
    path: &Path,
) -> anyhow::Result<Option<Vec<usize>>> {
    if let Some(indices) = by_index {
        return Ok(Some(indices.to_vec()));
    }
    if by_name.is_none() && by_contains.is_none() {
        return Ok(None);
    }
    // Name-based selection requires a seekable file (stdin not supported).
    if path == Path::new("-") {
        anyhow::bail!(
            "--channel-name and --channel-contains require a file path; \
             cannot resolve channel names from stdin"
        );
    }
    let lazy =
        biodream::open_file(path).with_context(|| format!("failed to open {}", path.display()))?;

    if let Some(names) = by_name {
        let mut indices = Vec::with_capacity(names.len());
        for name in names {
            let idx = lazy.find_channel_by_name(name).ok_or_else(|| {
                let available = lazy
                    .channel_metadata
                    .iter()
                    .map(|m| m.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                anyhow::anyhow!("no channel named {name:?} (available: {available})")
            })?;
            indices.push(idx);
        }
        return Ok(Some(indices));
    }

    if let Some(needle) = by_contains {
        let idx = lazy.find_channel_containing(needle).ok_or_else(|| {
            let available = lazy
                .channel_metadata
                .iter()
                .map(|m| m.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            anyhow::anyhow!("no channel containing {needle:?} (available: {available})")
        })?;
        return Ok(Some(vec![idx]));
    }

    Ok(None)
}

// ---------------------------------------------------------------------------
// CSV options
// ---------------------------------------------------------------------------

/// Parse a single-character ASCII delimiter from a CLI string.
///
/// Accepts `"tab"` or `"\t"` as aliases for the tab character.
fn parse_delimiter(s: &str) -> anyhow::Result<u8> {
    if s == "tab" || s == "\\t" || s == "\t" {
        return Ok(b'\t');
    }
    let mut chars = s.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii() => Ok(c as u8),
        _ => anyhow::bail!(
            "--delimiter must be a single ASCII character or 'tab'; got {:?}",
            s
        ),
    }
}

/// Build `CsvOptions` from the CLI args.
fn build_csv_options(args: &ConvertArgs) -> anyhow::Result<CsvOptions> {
    let delimiter = parse_delimiter(&args.delimiter)?;
    Ok(CsvOptions::new()
        .delimiter(delimiter)
        .precision(args.precision)
        .time_format(TimeFormat::from(args.time_format))
        .include_raw(args.include_raw)
        .fill_value(args.fill_value.clone()))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn read_with_options(
    path: &Path,
    channel_indices: Option<&[usize]>,
    scaled: bool,
) -> anyhow::Result<biodream::ParseResult<biodream::Datafile>> {
    let mut opts = ReadOptions::new().scaled(scaled);
    if let Some(indices) = channel_indices {
        opts = opts.channels(indices);
    }

    if path == Path::new("-") {
        use std::io::Read;
        let mut bytes = Vec::new();
        std::io::stdin()
            .read_to_end(&mut bytes)
            .context("failed to read from stdin")?;
        opts.read_bytes(&bytes)
            .context("failed to parse .acq from stdin")
    } else {
        opts.read_file(path)
            .with_context(|| format!("failed to read {}", path.display()))
    }
}

/// Resolve the output format from the explicit flag, then the output extension,
/// then the input extension.
pub fn resolve_format(
    explicit: Option<OutputFormat>,
    input: &Path,
    output: Option<&Path>,
) -> anyhow::Result<OutputFormat> {
    if let Some(f) = explicit {
        return Ok(f);
    }
    // Try output extension first, then fall back to a csv default when the
    // path is unambiguous.
    let ext_path = output.unwrap_or(input);
    let ext = ext_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    match ext.as_str() {
        "csv" | "txt" | "tsv" => Ok(OutputFormat::Csv),
        "arrow" | "ipc" => Ok(OutputFormat::Arrow),
        "parquet" | "pq" => Ok(OutputFormat::Parquet),
        "h5" | "hdf5" => Ok(OutputFormat::Hdf5),
        "mat" => Ok(OutputFormat::Mat),
        // Default to CSV when the extension is the .acq input itself.
        _ if output.is_none() => Ok(OutputFormat::Csv),
        other => anyhow::bail!(
            "cannot infer output format from extension '.{other}'; \
             use --format csv|arrow|parquet|hdf5|mat"
        ),
    }
}

fn resolve_output_path(
    explicit: Option<PathBuf>,
    input: &Path,
    format: OutputFormat,
) -> anyhow::Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p);
    }

    if input == Path::new("-") {
        anyhow::bail!("--output is required when reading from stdin");
    }

    let ext = match format {
        OutputFormat::Csv => "csv",
        OutputFormat::Arrow => "arrow",
        OutputFormat::Parquet => "parquet",
        OutputFormat::Hdf5 => "h5",
        OutputFormat::Mat => "mat",
    };

    // file_prefix() (stable 1.91) returns the stem before the first dot.
    let stem = input
        .file_prefix()
        .unwrap_or_else(|| std::ffi::OsStr::new("output"));
    Ok(PathBuf::from(stem).with_extension(ext))
}

fn write_output(
    format: OutputFormat,
    df: &biodream::Datafile,
    output_path: &Path,
    csv_opts: &CsvOptions,
) -> anyhow::Result<()> {
    match format {
        OutputFormat::Csv => write_csv(df, output_path, csv_opts),
        OutputFormat::Arrow => write_arrow(df, output_path),
        OutputFormat::Parquet => write_parquet(df, output_path),
        OutputFormat::Hdf5 => write_hdf5(df, output_path),
        OutputFormat::Mat => write_mat(df, output_path),
    }
}

fn write_csv(
    df: &biodream::Datafile,
    output_path: &Path,
    csv_opts: &CsvOptions,
) -> anyhow::Result<()> {
    let file = File::create(output_path)
        .with_context(|| format!("failed to create {}", output_path.display()))?;
    let mut w = BufWriter::new(file);
    biodream::to_csv(df, &mut w, csv_opts).context("CSV export failed")?;
    w.flush().context("flush failed")
}

#[cfg(any(feature = "arrow", feature = "parquet"))]
fn write_arrow(df: &biodream::Datafile, output_path: &Path) -> anyhow::Result<()> {
    let file = File::create(output_path)
        .with_context(|| format!("failed to create {}", output_path.display()))?;
    let mut writer = BufWriter::new(file);
    biodream::to_arrow_ipc(df, &mut writer).context("Arrow IPC export failed")?;
    writer.flush().context("flush failed")
}

#[cfg(not(any(feature = "arrow", feature = "parquet")))]
fn write_arrow(_df: &biodream::Datafile, _output_path: &Path) -> anyhow::Result<()> {
    anyhow::bail!(
        "Arrow IPC export requires the 'arrow' feature; \
         recompile with: cargo build --features arrow"
    )
}

#[cfg(feature = "parquet")]
fn write_parquet(df: &biodream::Datafile, output_path: &Path) -> anyhow::Result<()> {
    let file = File::create(output_path)
        .with_context(|| format!("failed to create {}", output_path.display()))?;
    let writer = BufWriter::new(file);
    biodream::to_parquet(df, writer, &biodream::ParquetOptions::default())
        .context("Parquet export failed")
}

#[cfg(not(feature = "parquet"))]
fn write_parquet(_df: &biodream::Datafile, _output_path: &Path) -> anyhow::Result<()> {
    anyhow::bail!(
        "Parquet export requires the 'parquet' feature; \
         recompile with: cargo build --features parquet"
    )
}

#[cfg(feature = "hdf5")]
fn write_hdf5(df: &biodream::Datafile, output_path: &Path) -> anyhow::Result<()> {
    biodream::to_hdf5(df, output_path, &biodream::Hdf5Options::default())
        .with_context(|| format!("HDF5 export failed: {}", output_path.display()))
}

#[cfg(not(feature = "hdf5"))]
fn write_hdf5(_df: &biodream::Datafile, _output_path: &Path) -> anyhow::Result<()> {
    anyhow::bail!(
        "HDF5 export requires the 'hdf5' feature; \
         recompile with: cargo build --features hdf5"
    )
}

fn write_mat(df: &biodream::Datafile, output_path: &Path) -> anyhow::Result<()> {
    // MATLAB v7.3 files are HDF5 containers. We emit the same layout as `hdf5`
    // and let the `.mat` extension drive downstream tooling.
    write_hdf5(df, output_path)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn resolve_format_explicit_csv() -> anyhow::Result<()> {
        let f = resolve_format(Some(OutputFormat::Csv), Path::new("a.acq"), None)?;
        assert_eq!(f, OutputFormat::Csv);
        Ok(())
    }

    #[test]
    fn resolve_format_from_output_extension() -> anyhow::Result<()> {
        let f = resolve_format(None, Path::new("a.acq"), Some(Path::new("out.parquet")))?;
        assert_eq!(f, OutputFormat::Parquet);
        Ok(())
    }

    #[test]
    fn resolve_format_hdf5_extensions() -> anyhow::Result<()> {
        let a = resolve_format(None, Path::new("a.acq"), Some(Path::new("out.h5")))?;
        let b = resolve_format(None, Path::new("a.acq"), Some(Path::new("out.hdf5")))?;
        assert_eq!(a, OutputFormat::Hdf5);
        assert_eq!(b, OutputFormat::Hdf5);
        Ok(())
    }

    #[test]
    fn resolve_format_mat_extension() -> anyhow::Result<()> {
        let f = resolve_format(None, Path::new("a.acq"), Some(Path::new("out.mat")))?;
        assert_eq!(f, OutputFormat::Mat);
        Ok(())
    }

    #[test]
    fn resolve_format_arrow_from_ipc_extension() -> anyhow::Result<()> {
        let f = resolve_format(None, Path::new("a.acq"), Some(Path::new("out.ipc")))?;
        assert_eq!(f, OutputFormat::Arrow);
        Ok(())
    }

    #[test]
    fn resolve_format_defaults_to_csv_when_no_output() -> anyhow::Result<()> {
        let f = resolve_format(None, Path::new("data.acq"), None)?;
        assert_eq!(f, OutputFormat::Csv);
        Ok(())
    }

    #[test]
    fn resolve_format_unknown_extension_returns_error() {
        let r = resolve_format(None, Path::new("a.acq"), Some(Path::new("out.xyz")));
        assert!(r.is_err());
    }

    #[test]
    fn resolve_output_path_derives_csv_from_input() -> anyhow::Result<()> {
        let p = resolve_output_path(None, Path::new("data.acq"), OutputFormat::Csv)?;
        assert_eq!(p, PathBuf::from("data.csv"));
        Ok(())
    }

    #[test]
    fn resolve_output_path_derives_arrow_from_input() -> anyhow::Result<()> {
        let p = resolve_output_path(None, Path::new("my.data.acq"), OutputFormat::Arrow)?;
        // file_prefix("my.data.acq") = "my"
        assert_eq!(p, PathBuf::from("my.arrow"));
        Ok(())
    }

    #[test]
    fn resolve_output_path_stdin_without_explicit_errors() {
        let r = resolve_output_path(None, Path::new("-"), OutputFormat::Csv);
        assert!(r.is_err());
    }

    #[test]
    fn resolve_output_path_derives_hdf5_from_input() -> anyhow::Result<()> {
        let p = resolve_output_path(None, Path::new("data.acq"), OutputFormat::Hdf5)?;
        assert_eq!(p, PathBuf::from("data.h5"));
        Ok(())
    }

    #[test]
    fn resolve_output_path_derives_mat_from_input() -> anyhow::Result<()> {
        let p = resolve_output_path(None, Path::new("data.acq"), OutputFormat::Mat)?;
        assert_eq!(p, PathBuf::from("data.mat"));
        Ok(())
    }
}

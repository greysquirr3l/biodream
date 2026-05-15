//! `biopac convert` — export a .acq file to CSV, Arrow IPC, or Parquet.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::Args;

use biodream::{CsvOptions, ReadOptions};

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

    /// Output format: csv, arrow, or parquet. Inferred from the output
    /// file extension when not specified.
    #[arg(short = 'F', long, value_name = "FORMAT")]
    pub format: Option<OutputFormat>,

    /// Channel indices to include (comma-separated, e.g. `0,2`).
    /// Default: all channels.
    #[arg(long, value_delimiter = ',', value_name = "INDEX")]
    pub channels: Option<Vec<usize>>,

    /// Convert raw integer samples to scaled float values.
    #[arg(long)]
    pub scaled: bool,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn run(args: ConvertArgs) -> anyhow::Result<()> {
    let format = resolve_format(args.format, &args.path, args.output.as_deref())?;
    let output_path = resolve_output_path(args.output, &args.path, format)?;

    // Read the .acq file with optional channel filter and scaling.
    let result = read_with_options(&args.path, args.channels.as_deref(), args.scaled)?;
    let df = result.value;

    // Write to the resolved output path.
    let file = File::create(&output_path)
        .with_context(|| format!("failed to create {}", output_path.display()))?;

    write_output(format, &df, file)
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
        "csv" => Ok(OutputFormat::Csv),
        "arrow" | "ipc" => Ok(OutputFormat::Arrow),
        "parquet" | "pq" => Ok(OutputFormat::Parquet),
        // Default to CSV when the extension is the .acq input itself.
        _ if output.is_none() => Ok(OutputFormat::Csv),
        other => anyhow::bail!(
            "cannot infer output format from extension '.{other}'; \
             use --format csv|arrow|parquet"
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
    };

    // file_prefix() (stable 1.91) returns the stem before the first dot.
    let stem = input
        .file_prefix()
        .unwrap_or_else(|| std::ffi::OsStr::new("output"));
    Ok(PathBuf::from(stem).with_extension(ext))
}

fn write_output(format: OutputFormat, df: &biodream::Datafile, file: File) -> anyhow::Result<()> {
    match format {
        OutputFormat::Csv => {
            let mut w = BufWriter::new(file);
            biodream::to_csv(df, &mut w, &CsvOptions::default()).context("CSV export failed")?;
            w.flush().context("flush failed")
        }
        OutputFormat::Arrow => write_arrow(df, BufWriter::new(file)),
        OutputFormat::Parquet => write_parquet(df, BufWriter::new(file)),
    }
}

#[cfg(any(feature = "arrow", feature = "parquet"))]
fn write_arrow<W: Write>(df: &biodream::Datafile, mut writer: W) -> anyhow::Result<()> {
    biodream::to_arrow_ipc(df, &mut writer).context("Arrow IPC export failed")?;
    writer.flush().context("flush failed")
}

#[cfg(not(any(feature = "arrow", feature = "parquet")))]
fn write_arrow<W: Write>(_df: &biodream::Datafile, _writer: W) -> anyhow::Result<()> {
    anyhow::bail!(
        "Arrow IPC export requires the 'arrow' feature; \
         recompile with: cargo build --features arrow"
    )
}

#[cfg(feature = "parquet")]
fn write_parquet<W: Write + Send>(df: &biodream::Datafile, writer: W) -> anyhow::Result<()> {
    biodream::to_parquet(df, writer, &biodream::ParquetOptions::default())
        .context("Parquet export failed")
}

#[cfg(not(feature = "parquet"))]
fn write_parquet<W: Write + Send>(_df: &biodream::Datafile, _writer: W) -> anyhow::Result<()> {
    anyhow::bail!(
        "Parquet export requires the 'parquet' feature; \
         recompile with: cargo build --features parquet"
    )
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
}

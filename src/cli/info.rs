//! `biopac info` — print a recording summary.

use std::io::{self, Read, Write};
use std::path::Path;

use anyhow::Context;
use clap::Args;

use biodream::{Datafile, LazyDatafile, ParseResult};

// ---------------------------------------------------------------------------
// CLI args
// ---------------------------------------------------------------------------

/// Arguments for the `info` subcommand.
#[derive(Debug, Args)]
pub struct InfoArgs {
    /// Path to the .acq file, or `-` to read from stdin.
    #[arg(value_name = "FILE")]
    pub path: std::path::PathBuf,

    /// Output structured JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

pub fn run(args: &InfoArgs) -> anyhow::Result<()> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    run_inner(&args.path, args.json, &mut out)
}
pub fn run_inner(path: &Path, json: bool, writer: &mut impl Write) -> anyhow::Result<()> {
    if path == Path::new("-") {
        // Stdin: must buffer all bytes first; no lazy option.
        let result = read_acq(path)?;
        print_info(&result, json, writer)
    } else {
        // File path: use LazyDatafile — headers and markers only, no sample data read.
        let lazy = biodream::open_file(path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        print_info_lazy(&lazy, json, writer)
    }
}

// ---------------------------------------------------------------------------
// Shared helpers (used by markers and inspect modules)
// ---------------------------------------------------------------------------

/// Load a `.acq` file from `path`, or from stdin when `path` is `"-"`.
pub(super) fn read_acq(path: &Path) -> anyhow::Result<ParseResult<Datafile>> {
    read_acq_with_stdin(path, &mut io::stdin())
}

/// Testable variant that accepts an explicit stdin reader.
pub(super) fn read_acq_with_stdin<R: Read>(
    path: &Path,
    stdin: &mut R,
) -> anyhow::Result<ParseResult<Datafile>> {
    if path == Path::new("-") {
        let mut bytes = Vec::new();
        stdin
            .read_to_end(&mut bytes)
            .context("failed to read from stdin")?;
        biodream::read_bytes(&bytes).context("failed to parse .acq from stdin")
    } else {
        biodream::read_file(path).with_context(|| format!("failed to read {}", path.display()))
    }
}

// ---------------------------------------------------------------------------
// Formatting — lazy path (file on disk, headers only)
// ---------------------------------------------------------------------------

fn print_info_lazy(lazy: &LazyDatafile, json: bool, writer: &mut impl Write) -> anyhow::Result<()> {
    for w in &lazy.warnings {
        eprintln!("warning: {w}");
    }
    if json {
        print_json_lazy(lazy, writer)
    } else {
        print_summary_lazy(lazy, writer)
    }
}

fn print_json_lazy(lazy: &LazyDatafile, writer: &mut impl Write) -> anyhow::Result<()> {
    let base_rate = lazy.metadata.samples_per_second;
    let channels: Vec<serde_json::Value> = lazy
        .channel_metadata
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let sps = if m.frequency_divider == 0 {
                base_rate
            } else {
                base_rate / f64::from(m.frequency_divider)
            };
            serde_json::json!({
                "index": i,
                "name": m.name,
                "units": m.units,
                "samples_per_second": sps,
                "samples": m.sample_count,
            })
        })
        .collect();

    let byte_order = match lazy.metadata.byte_order {
        biodream::ByteOrder::LittleEndian => "LittleEndian",
        biodream::ByteOrder::BigEndian => "BigEndian",
    };
    let duration = lazy
        .channel_metadata
        .iter()
        .filter(|m| m.frequency_divider > 0 && m.sample_count > 0)
        .map(|m| {
            let sps = base_rate / f64::from(m.frequency_divider);
            f64::from(m.sample_count) / sps
        })
        .reduce(f64::max);

    let obj = serde_json::json!({
        "revision": lazy.metadata.file_revision.0,
        "version": lazy.metadata.file_revision.display_version(),
        "compressed": lazy.metadata.compressed,
        "byte_order": byte_order,
        "samples_per_second": base_rate,
        "duration_seconds": duration,
        "channel_count": lazy.channel_metadata.len(),
        "marker_count": lazy.markers.len(),
        "title": lazy.metadata.title.as_deref(),
        "acquisition_datetime": lazy.metadata.acquisition_datetime.as_ref().map(std::string::ToString::to_string),
        "channels": channels,
    });
    serde_json::to_writer_pretty(&mut *writer, &obj).context("JSON serialisation failed")?;
    writeln!(writer).context("write failed")
}

fn print_summary_lazy(lazy: &LazyDatafile, writer: &mut impl Write) -> anyhow::Result<()> {
    let base_rate = lazy.metadata.samples_per_second;
    writeln!(
        writer,
        "AcqKnowledge file  revision={} ({}) compressed={} rate={} Hz",
        lazy.metadata.file_revision.0,
        lazy.metadata.file_revision.display_version(),
        lazy.metadata.compressed,
        base_rate,
    )
    .context("write failed")?;
    for (i, m) in lazy.channel_metadata.iter().enumerate() {
        let sps = if m.frequency_divider == 0 {
            base_rate
        } else {
            base_rate / f64::from(m.frequency_divider)
        };
        writeln!(
            writer,
            "  [{i}] Channel({:?}, {} samples, {} Hz)",
            m.name, m.sample_count, sps
        )
        .context("write failed")?;
    }
    if lazy.markers.is_empty() {
        writeln!(writer, "  (no markers)").context("write failed")?;
    } else {
        writeln!(writer, "  {} marker(s)", lazy.markers.len()).context("write failed")?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Formatting — eager path (stdin or pre-loaded Datafile)
// ---------------------------------------------------------------------------

/// Print recording info to `writer`.
pub fn print_info(
    result: &ParseResult<Datafile>,
    json: bool,
    writer: &mut impl Write,
) -> anyhow::Result<()> {
    let df = &result.value;
    if json {
        print_json(df, writer)
    } else {
        write!(writer, "{}", df.summary()).context("write failed")?;
        Ok(())
    }
}

fn print_json(df: &Datafile, writer: &mut impl Write) -> anyhow::Result<()> {
    let channels: Vec<serde_json::Value> = df
        .channels()
        .enumerate()
        .map(|(i, ch)| {
            serde_json::json!({
                "index": i,
                "name": ch.name,
                "units": ch.units,
                "samples_per_second": ch.samples_per_second,
                "samples": ch.point_count,
            })
        })
        .collect();

    let byte_order = match df.metadata.byte_order {
        biodream::ByteOrder::LittleEndian => "LittleEndian",
        biodream::ByteOrder::BigEndian => "BigEndian",
    };

    let obj = serde_json::json!({
        "revision": df.metadata.file_revision.0,
        "version": df.metadata.file_revision.display_version(),
        "compressed": df.metadata.compressed,
        "byte_order": byte_order,
        "samples_per_second": df.metadata.samples_per_second,
        "duration_seconds": df.duration(),
        "channel_count": df.channel_count(),
        "marker_count": df.marker_count(),
        "title": df.metadata.title.as_deref(),
        "acquisition_datetime": df.metadata.acquisition_datetime.as_ref().map(std::string::ToString::to_string),
        "channels": channels,
    });

    serde_json::to_writer_pretty(&mut *writer, &obj).context("JSON serialisation failed")?;
    writeln!(writer).context("write failed")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use biodream::{
        ChannelData, FileRevision,
        domain::{ByteOrder, Channel, GraphMetadata, Journal},
        error::ParseResult,
    };

    fn make_datafile() -> ParseResult<Datafile> {
        ParseResult::ok(Datafile {
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
            channels: vec![Channel {
                name: String::from("ECG"),
                units: String::from("mV"),
                samples_per_second: 1000.0,
                frequency_divider: 1,
                data: ChannelData::Raw(vec![0i16, 1, 2]),
                point_count: 3,
            }],
            markers: vec![],
            journal: None::<Journal>,
        })
    }

    #[test]
    fn info_human_contains_acqknowledge() {
        let result = make_datafile();
        let mut out = Vec::new();
        print_info(&result, false, &mut out).ok();
        let s = String::from_utf8(out).unwrap_or_default();
        assert!(
            s.contains("AcqKnowledge"),
            "expected 'AcqKnowledge' in: {s}"
        );
    }

    #[test]
    fn info_json_is_valid_json() {
        let result = make_datafile();
        let mut out = Vec::new();
        print_info(&result, true, &mut out).ok();
        let s = String::from_utf8(out).unwrap_or_default();
        let v: serde_json::Value = serde_json::from_str(&s).unwrap_or(serde_json::Value::Null);
        assert_eq!(v.get("revision"), Some(&serde_json::json!(73)));
        assert_eq!(v.get("channel_count"), Some(&serde_json::json!(1)));
    }

    #[test]
    fn info_json_channels_array() {
        let result = make_datafile();
        let mut out = Vec::new();
        print_info(&result, true, &mut out).ok();
        let s = String::from_utf8(out).unwrap_or_default();
        let v: serde_json::Value = serde_json::from_str(&s).unwrap_or(serde_json::Value::Null);
        let empty = vec![];
        let channels = v
            .get("channels")
            .and_then(|c| c.as_array())
            .unwrap_or(&empty);
        assert_eq!(channels.len(), 1);
        assert_eq!(
            channels.first().and_then(|ch| ch.get("name")),
            Some(&serde_json::json!("ECG"))
        );
    }
}

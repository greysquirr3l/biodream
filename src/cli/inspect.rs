//! `biopac inspect` — low-level binary layout diagnostics.

use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::Context;
use clap::Args;

use biodream::InspectReport;

// ---------------------------------------------------------------------------
// CLI args
// ---------------------------------------------------------------------------

/// Arguments for the `inspect` subcommand.
#[derive(Debug, Args)]
pub struct InspectArgs {
    /// Path to the .acq file, or `-` to read from stdin.
    #[arg(value_name = "FILE")]
    pub path: PathBuf,

    /// Display byte offsets in hexadecimal rather than decimal.
    #[arg(long)]
    pub hex: bool,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn run(args: &InspectArgs) -> anyhow::Result<()> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    run_inner(&args.path, args.hex, &mut out)
}
pub fn run_inner(path: &std::path::Path, hex: bool, writer: &mut impl Write) -> anyhow::Result<()> {
    let report = load_report(path)?;
    print_report(&report, hex, writer)
}

fn load_report(path: &std::path::Path) -> anyhow::Result<InspectReport> {
    if path == std::path::Path::new("-") {
        use std::io::Read;
        let mut bytes = Vec::new();
        std::io::stdin()
            .read_to_end(&mut bytes)
            .context("failed to read from stdin")?;
        biodream::inspect_bytes(&bytes).context("failed to inspect .acq from stdin")
    } else {
        biodream::inspect_file(path)
            .with_context(|| format!("failed to inspect {}", path.display()))
    }
}

fn print_report(report: &InspectReport, hex: bool, writer: &mut impl Write) -> anyhow::Result<()> {
    let m = &report.graph_metadata;

    writeln!(writer, "--- AcqKnowledge File Diagnostics ---").context("write failed")?;
    writeln!(writer, "Revision       : {}", m.file_revision.0).context("write failed")?;
    writeln!(
        writer,
        "Version        : {}",
        m.file_revision.display_version()
    )
    .context("write failed")?;
    writeln!(writer, "Channels       : {}", m.channel_count).context("write failed")?;
    writeln!(writer, "Compressed     : {}", m.compressed).context("write failed")?;
    writeln!(writer, "Samples/sec    : {}", m.samples_per_second).context("write failed")?;

    if hex {
        writeln!(
            writer,
            "Data offset    : {:#010x}",
            report.data_start_offset
        )
        .context("write failed")?;
    } else {
        writeln!(
            writer,
            "Data offset    : {} bytes",
            report.data_start_offset
        )
        .context("write failed")?;
    }

    writeln!(writer, "Foreign data   : {} bytes", report.foreign_data_len)
        .context("write failed")?;

    if !report.warnings.is_empty() {
        writeln!(writer, "Warnings       : {}", report.warnings.len()).context("write failed")?;
        for w in &report.warnings {
            writeln!(writer, "  - {}", w.message).context("write failed")?;
        }
    }

    writeln!(writer).context("write failed")?;
    writeln!(writer, "--- Channels ---").context("write failed")?;
    writeln!(
        writer,
        "{:<4} {:<24} {:<12} {:<6} Samples",
        "Idx", "Name", "Units", "DType"
    )
    .context("write failed")?;
    for (i, ch) in report.channels.iter().enumerate() {
        writeln!(
            writer,
            "{:<4} {:<24} {:<12} {:<6} {}",
            i, ch.metadata.name, ch.metadata.units, ch.dtype, ch.metadata.sample_count,
        )
        .context("write failed")?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use biodream::{
        ChannelInspect, ChannelMetadata, FileRevision, InspectReport, domain::GraphMetadata,
    };

    fn make_report() -> InspectReport {
        InspectReport {
            graph_metadata: GraphMetadata {
                file_revision: FileRevision::new(73),
                samples_per_second: 1000.0,
                channel_count: 1,
                byte_order: biodream::domain::ByteOrder::LittleEndian,
                compressed: false,
                title: None,
                acquisition_datetime: None,
                max_samples_per_second: None,
            },
            channels: vec![ChannelInspect {
                metadata: ChannelMetadata {
                    name: String::from("ECG"),
                    units: String::from("mV"),
                    description: String::new(),
                    frequency_divider: 1,
                    amplitude_scale: 1.0,
                    amplitude_offset: 0.0,
                    display_order: 0,
                    sample_count: 1024,
                },
                dtype: "I16",
            }],
            foreign_data_len: 0,
            data_start_offset: 512,
            warnings: vec![],
        }
    }

    #[test]
    fn inspect_decimal_offset() {
        let report = make_report();
        let mut out = Vec::new();
        print_report(&report, false, &mut out).ok();
        let s = String::from_utf8(out).unwrap_or_default();
        assert!(s.contains("512 bytes"), "expected decimal offset in:\n{s}");
        assert!(!s.contains("0x"), "unexpected hex prefix in:\n{s}");
    }

    #[test]
    fn inspect_hex_offset() {
        let report = make_report();
        let mut out = Vec::new();
        print_report(&report, true, &mut out).ok();
        let s = String::from_utf8(out).unwrap_or_default();
        assert!(s.contains("0x"), "expected 0x prefix in:\n{s}");
    }

    #[test]
    fn inspect_lists_channel_name() {
        let report = make_report();
        let mut out = Vec::new();
        print_report(&report, false, &mut out).ok();
        let s = String::from_utf8(out).unwrap_or_default();
        assert!(s.contains("ECG"), "expected channel name in:\n{s}");
        assert!(s.contains("I16"), "expected dtype in:\n{s}");
    }
}

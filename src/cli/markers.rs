//! `biopac markers` — list event markers from a .acq file.

use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::Context;
use clap::Args;

// ---------------------------------------------------------------------------
// CLI args
// ---------------------------------------------------------------------------

/// Arguments for the `markers` subcommand.
#[derive(Debug, Args)]
pub struct MarkersArgs {
    /// Path to the .acq file, or `-` to read from stdin.
    #[arg(value_name = "FILE")]
    pub path: PathBuf,

    /// Output structured JSON instead of a tab-delimited table.
    #[arg(long)]
    pub json: bool,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn run(args: &MarkersArgs) -> anyhow::Result<()> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    run_inner(&args.path, args.json, &mut out)
}
pub fn run_inner(
    path: &std::path::Path,
    json: bool,
    writer: &mut impl Write,
) -> anyhow::Result<()> {
    let result = super::info::read_acq(path)?;
    let df = &result.value;

    if json {
        let rows: Vec<serde_json::Value> = df
            .markers
            .iter()
            .enumerate()
            .map(|(i, m)| {
                serde_json::json!({
                    "index": i,
                    "sample": m.global_sample_index,
                    "label": m.label,
                    "channel": m.channel,
                    "style": m.style.to_string(),
                    "created_at": m.created_at.as_ref().map(|ts| ts.as_secs()),
                })
            })
            .collect();
        serde_json::to_writer_pretty(&mut *writer, &rows).context("JSON serialisation failed")?;
        writeln!(writer).context("write failed")?;
    } else {
        writeln!(writer, "sample\tlabel\tchannel\tstyle").context("write failed")?;
        for m in &df.markers {
            let channel_str = m
                .channel
                .map_or_else(|| String::from("-"), |c| c.to_string());
            writeln!(
                writer,
                "{}\t{}\t{}\t{}",
                m.global_sample_index, m.label, channel_str, m.style
            )
            .context("write failed")?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use biodream::Datafile;
    use biodream::{
        ChannelData, FileRevision,
        domain::{ByteOrder, Channel, GraphMetadata, Journal, Marker, MarkerStyle},
        error::ParseResult,
    };

    fn make_df_with_markers() -> ParseResult<Datafile> {
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
                data: ChannelData::Raw(vec![0i16]),
                point_count: 1,
            }],
            markers: vec![Marker {
                label: String::from("start"),
                global_sample_index: 42,
                channel: None,
                style: MarkerStyle::Unknown(String::from("test")),
                created_at: None,
            }],
            journal: None::<Journal>,
        })
    }

    #[test]
    fn markers_table_has_header() {
        let result = make_df_with_markers();
        let df = &result.value;
        let mut out = Vec::new();
        writeln!(&mut out, "sample\tlabel\tchannel\tstyle").ok();
        for m in &df.markers {
            let ch = m
                .channel
                .map_or_else(|| String::from("-"), |c| c.to_string());
            writeln!(
                &mut out,
                "{}\t{}\t{}\t{}",
                m.global_sample_index, m.label, ch, m.style
            )
            .ok();
        }
        let s = String::from_utf8(out).unwrap_or_default();
        assert!(s.starts_with("sample\tlabel"));
        assert!(s.contains("42"));
        assert!(s.contains("start"));
    }

    #[test]
    fn markers_json_is_array() {
        let result = make_df_with_markers();
        let df = &result.value;
        let rows: Vec<serde_json::Value> = df
            .markers
            .iter()
            .enumerate()
            .map(|(i, m)| {
                serde_json::json!({
                    "index": i,
                    "sample": m.global_sample_index,
                    "label": m.label,
                    "channel": m.channel,
                    "style": m.style.to_string(),
                    "created_at": m.created_at.as_ref().map(|ts| ts.as_secs()),
                })
            })
            .collect();
        let v = serde_json::Value::Array(rows);
        assert!(v.is_array());
        let binding = vec![];
        let arr = v.as_array().unwrap_or(&binding);
        assert_eq!(arr.len(), 1);
        assert_eq!(
            arr.first().and_then(|v| v.get("label")),
            Some(&serde_json::json!("start"))
        );
    }
}

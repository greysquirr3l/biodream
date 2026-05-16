//! `biopac signals` — physiological signal processing subcommands.
//!
//! Requires the `physio` feature.

use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::Context;
use clap::{Args, Subcommand};

// ---------------------------------------------------------------------------
// Top-level args
// ---------------------------------------------------------------------------

/// Arguments for the `signals` subcommand group.
#[derive(Debug, Args)]
pub struct SignalsArgs {
    #[command(subcommand)]
    command: SignalsCommand,
}

#[derive(Debug, Subcommand)]
enum SignalsCommand {
    /// Detect R-peaks in an ECG channel using Pan-Tompkins QRS detection.
    DetectPeaks(DetectPeaksArgs),

    /// Compute beat-by-beat pulse-transit time (PTT) between ECG and PPG channels.
    Ptt(PttArgs),
}

// ---------------------------------------------------------------------------
// detect-peaks args
// ---------------------------------------------------------------------------

/// Arguments for `biopac signals detect-peaks`.
#[derive(Debug, Args)]
struct DetectPeaksArgs {
    /// Path to the .acq file.
    #[arg(value_name = "FILE")]
    path: PathBuf,

    /// Zero-based index of the channel containing the ECG signal. Default: 0.
    #[arg(long, default_value_t = 0, value_name = "INDEX")]
    channel: usize,

    /// Output sample indices as JSON instead of a plain tab-delimited table.
    #[arg(long)]
    json: bool,
}

// ---------------------------------------------------------------------------
// ptt args
// ---------------------------------------------------------------------------

/// Arguments for `biopac signals ptt`.
#[derive(Debug, Args)]
struct PttArgs {
    /// Path to the .acq file.
    #[arg(value_name = "FILE")]
    path: PathBuf,

    /// Zero-based index of the channel containing the ECG signal. Default: 0.
    #[arg(long, default_value_t = 0, value_name = "INDEX")]
    ecg: usize,

    /// Zero-based index of the channel containing the PPG signal. Default: 1.
    #[arg(long, default_value_t = 1, value_name = "INDEX")]
    ppg: usize,

    /// Minimum valid PTT in milliseconds. Default: 30.
    #[arg(long, default_value_t = 30.0, value_name = "MS")]
    min_ptt: f64,

    /// Maximum valid PTT in milliseconds. Default: 380.
    #[arg(long, default_value_t = 380.0, value_name = "MS")]
    max_ptt: f64,

    /// Output as JSON instead of a plain table.
    #[arg(long)]
    json: bool,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Dispatch the signals subcommand.
pub fn run(args: &SignalsArgs) -> anyhow::Result<()> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    match &args.command {
        SignalsCommand::DetectPeaks(a) => run_detect_peaks(a, &mut out),
        SignalsCommand::Ptt(a) => run_ptt(a, &mut out),
    }
}

// ---------------------------------------------------------------------------
// detect-peaks
// ---------------------------------------------------------------------------

fn run_detect_peaks(args: &DetectPeaksArgs, writer: &mut impl Write) -> anyhow::Result<()> {
    let result = biodream::read_file(&args.path)
        .with_context(|| format!("failed to read {}", args.path.display()))?;
    let df = result.value;

    let ch = df.channels.get(args.channel).ok_or_else(|| {
        anyhow::anyhow!(
            "channel {} out of bounds (file has {} channel(s))",
            args.channel,
            df.channels.len()
        )
    })?;

    let fs = ch.samples_per_second;
    let samples = ch.scaled_samples();
    let peaks = biodream::signals::detect_r_peaks(&samples, fs);

    if args.json {
        let obj = serde_json::json!({
            "channel": args.channel,
            "channel_name": ch.name,
            "fs": fs,
            "peak_count": peaks.len(),
            "peaks": peaks,
        });
        serde_json::to_writer_pretty(&mut *writer, &obj).context("JSON serialisation failed")?;
        writeln!(writer).context("write failed")
    } else {
        writeln!(
            writer,
            "# R-peaks in channel {} ({})  fs={} Hz",
            args.channel, ch.name, fs
        )
        .context("write failed")?;
        writeln!(writer, "# {} peak(s) detected", peaks.len()).context("write failed")?;
        writeln!(writer, "sample\ttime_s").context("write failed")?;
        for &s in &peaks {
            #[expect(
                clippy::cast_precision_loss,
                reason = "sample indices are small frame counts; f64 is sufficient for any physiological recording"
            )]
            let t = s as f64 / fs;
            writeln!(writer, "{s}\t{t:.6}").context("write failed")?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// ptt
// ---------------------------------------------------------------------------

fn run_ptt(args: &PttArgs, writer: &mut impl Write) -> anyhow::Result<()> {
    let result = biodream::read_file(&args.path)
        .with_context(|| format!("failed to read {}", args.path.display()))?;
    let df = result.value;
    let n = df.channels.len();

    let ecg_ch = df.channels.get(args.ecg).ok_or_else(|| {
        anyhow::anyhow!(
            "ECG channel {} out of bounds (file has {} channel(s))",
            args.ecg,
            n
        )
    })?;
    let ppg_ch = df.channels.get(args.ppg).ok_or_else(|| {
        anyhow::anyhow!(
            "PPG channel {} out of bounds (file has {} channel(s))",
            args.ppg,
            n
        )
    })?;

    let ecg_fs = ecg_ch.samples_per_second;
    let ppg_fs = ppg_ch.samples_per_second;
    if (ecg_fs - ppg_fs).abs() > f64::EPSILON {
        anyhow::bail!(
            "ECG (ch{} {} Hz) and PPG (ch{} {} Hz) must have the same sampling rate",
            args.ecg,
            ecg_fs,
            args.ppg,
            ppg_fs
        );
    }

    let ecg = ecg_ch.scaled_samples();
    let ppg = ppg_ch.scaled_samples();
    let fs = ecg_fs;

    let r_peaks = biodream::signals::detect_r_peaks(&ecg, fs);
    let ppg_feet = biodream::signals::detect_ppg_feet(&ppg, fs);
    let ptts = biodream::signals::beat_ptt(&r_peaks, &ppg_feet, fs, args.min_ptt, args.max_ptt);
    let median = biodream::signals::median_ptt(&r_peaks, &ppg_feet, fs);
    let hr = biodream::signals::heart_rate_bpm(&r_peaks, fs);

    if args.json {
        let obj = serde_json::json!({
            "ecg_channel": args.ecg,
            "ppg_channel": args.ppg,
            "fs": fs,
            "r_peak_count": r_peaks.len(),
            "beat_count": ptts.len(),
            "ptt_ms": ptts,
            "median_ptt_ms": median,
            "heart_rate_bpm": hr,
        });
        serde_json::to_writer_pretty(&mut *writer, &obj).context("JSON serialisation failed")?;
        writeln!(writer).context("write failed")
    } else {
        writeln!(
            writer,
            "# PTT — ECG ch{} ({}) vs PPG ch{} ({})  fs={} Hz",
            args.ecg, ecg_ch.name, args.ppg, ppg_ch.name, fs
        )
        .context("write failed")?;
        writeln!(
            writer,
            "# {} beat(s)  median PTT: {:.1} ms  heart rate: {:.1} BPM",
            ptts.len(),
            median.unwrap_or(f64::NAN),
            hr.unwrap_or(f64::NAN),
        )
        .context("write failed")?;
        writeln!(writer, "beat\tptt_ms").context("write failed")?;
        for (i, ptt) in ptts.iter().enumerate() {
            writeln!(writer, "{i}\t{ptt:.3}").context("write failed")?;
        }
        Ok(())
    }
}

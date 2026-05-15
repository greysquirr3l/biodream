//! `biopac plot` — render .acq channel waveforms as a PNG or SVG image.

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use clap::Args;
use plotters::prelude::*;

use biodream::{Channel, Marker};

use super::info::read_acq;

// ─── Colour palette (Matplotlib tab10) ─────────────────────────────────────

const PALETTE: &[RGBColor] = &[
    RGBColor(31, 119, 180),
    RGBColor(255, 127, 14),
    RGBColor(44, 160, 44),
    RGBColor(214, 39, 40),
    RGBColor(148, 103, 189),
    RGBColor(140, 86, 75),
    RGBColor(227, 119, 194),
    RGBColor(127, 127, 127),
    RGBColor(188, 189, 34),
    RGBColor(23, 190, 207),
];

fn ch_color(idx: usize) -> RGBColor {
    PALETTE
        .get(idx % PALETTE.len())
        .copied()
        .unwrap_or(RGBColor(127, 127, 127))
}

// ─── Output format ──────────────────────────────────────────────────────────

/// Output image format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ImageFormat {
    /// PNG raster image.
    Png,
    /// Scalable Vector Graphics.
    Svg,
}

// ─── CLI args ────────────────────────────────────────────────────────────────

/// Arguments for the `plot` subcommand.
#[derive(Debug, Args)]
pub struct PlotArgs {
    /// Path to the .acq file, or `-` to read from stdin.
    #[arg(value_name = "FILE")]
    pub path: PathBuf,

    /// Output image path.
    ///
    /// Defaults to `<input-stem>.png` (or `.svg` with `--format svg`).
    #[arg(short, long, value_name = "PATH")]
    pub output: Option<PathBuf>,

    /// Force output format (overrides extension inference).
    #[arg(long, value_enum)]
    pub format: Option<ImageFormat>,

    /// Image width in pixels (PNG) or SVG coordinate units.
    #[arg(long, default_value_t = 1200)]
    pub width: u32,

    /// Height per channel subplot in pixels / SVG units.
    #[arg(long, default_value_t = 200, value_name = "HEIGHT")]
    pub height_per_channel: u32,

    /// Channels to include: names or 0-based indices, comma-separated.
    ///
    /// Example: `--channels ECG,RESP` or `--channels 0,2`.
    /// Default: all channels.
    #[arg(long, value_delimiter = ',', value_name = "CH")]
    pub channels: Option<Vec<String>>,

    /// Start time in seconds (default: beginning of recording).
    #[arg(long, value_name = "SECS")]
    pub start: Option<f64>,

    /// End time in seconds (default: end of recording).
    #[arg(long, value_name = "SECS")]
    pub end: Option<f64>,
}

// ─── Entry point ─────────────────────────────────────────────────────────────

pub fn run(args: &PlotArgs) -> Result<()> {
    let result = read_acq(&args.path)?;
    let df = result.value;

    // -- Channel selection -----------------------------------------------
    let selected: Vec<(usize, &Channel)> = match &args.channels {
        None => df.channels.iter().enumerate().collect(),
        Some(specs) => {
            let mut out = Vec::new();
            for spec in specs {
                if let Ok(idx) = spec.parse::<usize>() {
                    let ch = df.channels.get(idx).ok_or_else(|| {
                        anyhow!(
                            "channel index {idx} is out of range \
                             (file has {} channels)",
                            df.channels.len()
                        )
                    })?;
                    out.push((idx, ch));
                } else {
                    let (idx, ch) = df
                        .channels
                        .iter()
                        .enumerate()
                        .find(|(_, c)| c.name == *spec)
                        .ok_or_else(|| anyhow!("channel '{spec}' not found"))?;
                    out.push((idx, ch));
                }
            }
            out
        }
    };

    if selected.is_empty() {
        return Err(anyhow!("no channels to plot"));
    }

    // -- Output path and format ------------------------------------------
    let out_path = resolve_output(&args.path, args.output.as_deref(), args.format);
    let fmt = infer_format(&out_path, args.format);

    // -- Layout ----------------------------------------------------------
    let n_panels =
        u32::try_from(selected.len()).map_err(|_| anyhow!("too many channels to plot"))?;
    let total_height = n_panels
        .checked_mul(args.height_per_channel)
        .ok_or_else(|| anyhow!("image dimensions overflow u32"))?;

    let base_rate = df.metadata.samples_per_second;

    // -- Render ----------------------------------------------------------
    match fmt {
        ImageFormat::Png => {
            let root =
                BitMapBackend::new(&out_path, (args.width, total_height)).into_drawing_area();
            render(
                &root,
                &selected,
                &df.markers,
                base_rate,
                args.start,
                args.end,
            )
            .map_err(|e| anyhow!("PNG render error: {e:?}"))?;
            root.present()
                .map_err(|e| anyhow!("PNG write error: {e:?}"))?;
        }
        ImageFormat::Svg => {
            let root = SVGBackend::new(&out_path, (args.width, total_height)).into_drawing_area();
            render(
                &root,
                &selected,
                &df.markers,
                base_rate,
                args.start,
                args.end,
            )
            .map_err(|e| anyhow!("SVG render error: {e:?}"))?;
            root.present()
                .map_err(|e| anyhow!("SVG write error: {e:?}"))?;
        }
    }

    eprintln!("wrote {}", out_path.display());
    Ok(())
}

// ─── Path / format helpers ───────────────────────────────────────────────────

fn resolve_output(input: &Path, output: Option<&Path>, format: Option<ImageFormat>) -> PathBuf {
    if let Some(p) = output {
        return p.to_owned();
    }
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    let ext = match format {
        Some(ImageFormat::Svg) => "svg",
        _ => "png",
    };
    PathBuf::from(format!("{stem}.{ext}"))
}

fn infer_format(path: &Path, forced: Option<ImageFormat>) -> ImageFormat {
    if let Some(f) = forced {
        return f;
    }
    match path.extension().and_then(|e| e.to_str()) {
        Some("svg") => ImageFormat::Svg,
        _ => ImageFormat::Png,
    }
}

// ─── Index / time conversion helpers ────────────────────────────────────────

/// Convert a time in seconds to a sample index, clamped to `[0, max]`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "t_sec * rate is non-negative and bounded by the recording duration"
)]
fn sec_to_idx(t_sec: f64, rate: f64, max: usize) -> usize {
    ((t_sec * rate) as usize).min(max)
}

/// Convert a sample index to time in seconds.
#[expect(
    clippy::cast_precision_loss,
    reason = "sample indices are bounded by point_count; f64 precision is sufficient for time-axis display"
)]
fn idx_to_sec(idx: usize, rate: f64) -> f64 {
    idx as f64 / rate
}

// ─── Drawing ─────────────────────────────────────────────────────────────────

/// Maximum data points drawn per channel.
///
/// Long recordings at high sample rates produce millions of samples. We
/// subsample by stepping every `ceil(n / MAX_PLOT_POINTS)` samples, keeping
/// rendering fast while preserving the visual shape of the waveform.
const MAX_PLOT_POINTS: usize = 8_000;

fn render<DB: DrawingBackend>(
    root: &DrawingArea<DB, plotters::coord::Shift>,
    channels: &[(usize, &Channel)],
    markers: &[Marker],
    base_rate: f64,
    start_sec: Option<f64>,
    end_sec: Option<f64>,
) -> Result<(), DrawingAreaErrorKind<DB::ErrorType>> {
    root.fill(&WHITE)?;

    let panels = root.split_evenly((channels.len(), 1));
    let last_idx = channels.len().saturating_sub(1);

    for (panel_idx, (panel, &(palette_idx, ch))) in panels.iter().zip(channels.iter()).enumerate() {
        let rate = ch.samples_per_second;
        let n_total = ch.point_count;

        // Time window.
        let t_start = start_sec.unwrap_or(0.0_f64).max(0.0_f64);
        let t_end_max = idx_to_sec(n_total, rate);
        let t_end = end_sec.unwrap_or(t_end_max).min(t_end_max);
        // Ensure a strictly positive window to avoid degenerate chart ranges.
        let t_end = if t_end <= t_start {
            t_start + 0.001
        } else {
            t_end
        };

        let i_start = sec_to_idx(t_start, rate, n_total);
        let i_end = (sec_to_idx(t_end, rate, n_total) + 1).min(n_total);

        let samples = ch.scaled_samples();
        let slice = samples.get(i_start..i_end).unwrap_or(&[]);
        let (y_min, y_max) = y_range(slice);

        let is_last = panel_idx == last_idx;
        let caption = format!("{} [{}]", ch.name, ch.units);

        let mut chart = ChartBuilder::on(panel)
            .caption(caption, ("sans-serif", 14).into_font())
            .margin(4_u32)
            .x_label_area_size(if is_last { 30_u32 } else { 0_u32 })
            .y_label_area_size(65_u32)
            .build_cartesian_2d(t_start..t_end, y_min..y_max)?;

        chart
            .configure_mesh()
            .x_labels(if is_last { 5 } else { 0 })
            .y_labels(4)
            .x_desc(if is_last { "Time (s)" } else { "" })
            .draw()?;

        // Subsample for rendering speed on long recordings.
        let stride = (slice.len() / MAX_PLOT_POINTS).max(1);
        let color = ch_color(palette_idx);

        chart.draw_series(LineSeries::new(
            slice
                .iter()
                .enumerate()
                .step_by(stride)
                .map(|(i, &v)| (idx_to_sec(i_start + i, rate), v)),
            color,
        ))?;

        // Overlay event markers as vertical red lines.
        for m in markers {
            let t_m = idx_to_sec(m.global_sample_index, base_rate);
            if t_m < t_start || t_m > t_end {
                continue;
            }
            chart.draw_series(std::iter::once(PathElement::new(
                vec![(t_m, y_min), (t_m, y_max)],
                RED,
            )))?;
        }
    }

    Ok(())
}

/// Compute Y-axis (min, max) with 5 % padding.
///
/// Handles empty slices, flat signals, and non-finite (NaN / Inf) samples.
fn y_range(samples: &[f64]) -> (f64, f64) {
    if samples.is_empty() {
        return (-1.0, 1.0);
    }

    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for &v in samples {
        if !v.is_finite() {
            continue;
        }
        if v < lo {
            lo = v;
        }
        if v > hi {
            hi = v;
        }
    }

    if !lo.is_finite() {
        // All samples were non-finite.
        return (-1.0, 1.0);
    }

    if (hi - lo).abs() < f64::EPSILON {
        // Flat signal: give ±1 around the constant value.
        return (lo - 1.0, hi + 1.0);
    }

    let pad = (hi - lo) * 0.05;
    (lo - pad, hi + pad)
}

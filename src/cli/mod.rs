//! CLI subcommands for the `biopac` binary.
//!
//! Each subcommand lives in its own module and is dispatched from [`run`].
//! All subcommands require the `read` feature (= `std`).

mod convert;
mod info;
mod inspect;
mod markers;
#[cfg(feature = "plot")]
mod plot;
#[cfg(feature = "physio")]
mod signals;

use clap::{Parser, Subcommand};

use convert::ConvertArgs;
use info::InfoArgs;
use inspect::InspectArgs;
use markers::MarkersArgs;
#[cfg(feature = "plot")]
use plot::PlotArgs;
#[cfg(feature = "physio")]
use signals::SignalsArgs;

// ---------------------------------------------------------------------------
// Top-level CLI struct
// ---------------------------------------------------------------------------

const LONG_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (git:",
    env!("BIODREAM_GIT_SHA"),
    " ",
    env!("BIODREAM_GIT_DATE"),
    ")",
);

/// biodream CLI — read, inspect, and convert BIOPAC `AcqKnowledge` (.acq) files.
#[derive(Debug, Parser)]
#[command(
    name = "biopac",
    version,
    long_version = LONG_VERSION,
    about = "Read, inspect, and convert BIOPAC AcqKnowledge (.acq) files",
    long_about = None,
    arg_required_else_help = true,
)]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print channel metadata and recording summary.
    Info(InfoArgs),

    /// Convert a .acq file to another format (csv, arrow, parquet).
    Convert(ConvertArgs),

    /// List event markers.
    Markers(MarkersArgs),

    /// Show low-level binary layout diagnostics.
    Inspect(InspectArgs),

    /// Detect physiological signals: R-peaks, PTT, heart rate. Requires the `physio` feature.
    #[cfg(feature = "physio")]
    Signals(SignalsArgs),

    /// Render channel waveforms as a PNG or SVG image.
    #[cfg(feature = "plot")]
    Plot(PlotArgs),
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Parse the command-line arguments and run the selected subcommand.
pub fn run() -> anyhow::Result<()> {
    let cli = Cli::try_parse().unwrap_or_else(|e| e.exit());
    match cli.command {
        Command::Info(args) => info::run(&args),
        Command::Convert(args) => convert::run(args),
        Command::Markers(args) => markers::run(&args),
        Command::Inspect(args) => inspect::run(&args),
        #[cfg(feature = "physio")]
        Command::Signals(args) => signals::run(&args),
        #[cfg(feature = "plot")]
        Command::Plot(args) => plot::run(&args),
    }
}

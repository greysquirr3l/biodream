//! CLI subcommands for the `biopac` binary.
//!
//! Each subcommand lives in its own module and is dispatched from [`run`].
//! All subcommands require the `read` feature (= `std`).

mod convert;
mod info;
mod inspect;
mod markers;

use clap::{Parser, Subcommand};

use convert::ConvertArgs;
use info::InfoArgs;
use inspect::InspectArgs;
use markers::MarkersArgs;

// ---------------------------------------------------------------------------
// Top-level CLI struct
// ---------------------------------------------------------------------------

const LONG_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (git:", env!("GIT_SHA"), ")");

/// biodream CLI — read, inspect, and convert BIOPAC `AcqKnowledge` (.acq) files.
#[derive(Debug, Parser)]
#[command(
    name = "biopac",
    version,
    long_version = LONG_VERSION,
    about = "Read, inspect, and convert BIOPAC AcqKnowledge (.acq) files",
    long_about = None
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
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Parse the command-line arguments and run the selected subcommand.
pub fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Info(args) => info::run(&args),
        Command::Convert(args) => convert::run(args),
        Command::Markers(args) => markers::run(&args),
        Command::Inspect(args) => inspect::run(&args),
    }
}

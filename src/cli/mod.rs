//! CLI subcommands: `biopac info`, `biopac convert`, `biopac inspect`.
//!
//! All subcommands require the `read` feature (= `std`).

use clap::{Parser, Subcommand};

/// biodream CLI — read, inspect, and convert BIOPAC `AcqKnowledge` (.acq) files.
#[derive(Debug, Parser)]
#[command(name = "biopac", version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print channel metadata and recording summary.
    Info {
        /// Path to the .acq file.
        #[arg(value_name = "FILE")]
        path: std::path::PathBuf,
    },
    /// Convert a .acq file to another format.
    ///
    /// Supported output formats: csv (default), arrow, parquet.
    Convert {
        /// Path to the .acq file.
        #[arg(value_name = "FILE")]
        path: std::path::PathBuf,

        /// Output file path. Format is inferred from the extension.
        #[arg(short, long, value_name = "OUTPUT")]
        output: std::path::PathBuf,
    },
    /// Print low-level diagnostic information for debugging format issues.
    Inspect {
        /// Path to the .acq file.
        #[arg(value_name = "FILE")]
        path: std::path::PathBuf,
    },
}

/// Run the CLI. Returns an [`anyhow::Error`] on failure.
pub fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Info { path } => {
            // TODO(T12): implement info subcommand.
            anyhow::bail!("info subcommand not yet implemented (T12): {}", path.display());
        }
        Command::Convert { path, output } => {
            // TODO(T12): implement convert subcommand.
            anyhow::bail!(
                "convert subcommand not yet implemented (T12): {} -> {}",
                path.display(),
                output.display()
            );
        }
        Command::Inspect { path } => {
            // TODO(T12): implement inspect subcommand.
            anyhow::bail!("inspect subcommand not yet implemented (T12): {}", path.display());
        }
    }
}

//! CLI entry point for the `biopac` binary.
//!
//! Requires the `read` feature (which in turn requires `std`).
//! Run `biopac --help` to see available subcommands.

mod cli;

fn main() -> anyhow::Result<()> {
    cli::run()
}

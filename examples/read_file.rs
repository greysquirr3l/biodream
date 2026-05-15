//! Example: read a `.acq` file and print channel info.
//!
//! # Usage
//!
//! ```sh
//! cargo run --example read_file -- path/to/recording.acq
//! ```

use std::{env, process};

fn main() {
    let args: Vec<String> = env::args().collect();
    let path = match args.get(1) {
        Some(p) => p.clone(),
        None => {
            eprintln!("Usage: read_file <path/to/recording.acq>");
            process::exit(1);
        }
    };

    let result = match biodream::read_file(&path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error reading {path}: {e}");
            process::exit(1);
        }
    };

    for w in &result.warnings {
        eprintln!("warning: {w}");
    }

    let df = result.into_value();
    println!(
        "File revision : {}",
        df.metadata.file_revision.display_version()
    );
    println!("Sample rate   : {} Hz", df.metadata.samples_per_second);
    println!("Channels      : {}", df.channels.len());

    if let Some(journal) = &df.journal {
        println!("Journal       : {}", journal.as_text());
    }

    for (i, ch) in df.channels.iter().enumerate() {
        let samples = ch.scaled_samples();
        println!(
            "  [{i}] {:20} | {:>8} samples | {} Hz | units: {}",
            ch.name,
            samples.len(),
            ch.samples_per_second,
            ch.units,
        );
    }
}

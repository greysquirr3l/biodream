//! Example: read a `.acq` file and export to Apache Arrow IPC.
//!
//! Requires the `arrow` feature:
//!
//! ```sh
//! cargo run --example arrow_export --features arrow -- input.acq output.arrow
//! ```

fn main() {
    #[cfg(not(feature = "arrow"))]
    {
        eprintln!(
            "This example requires the `arrow` feature. \
             Re-run with: cargo run --example arrow_export --features arrow -- <input.acq> <output.arrow>"
        );
        std::process::exit(1);
    }

    #[cfg(feature = "arrow")]
    run();
}

#[cfg(feature = "arrow")]
fn run() {
    use std::{env, fs::File, io::BufWriter, process};

    let args: Vec<String> = env::args().collect();
    let (Some(input), Some(output)) = (args.get(1), args.get(2)) else {
        eprintln!("Usage: arrow_export <input.acq> <output.arrow>");
        process::exit(1);
    };

    let df = match biodream::read_file(input) {
        Ok(r) => {
            for w in &r.warnings {
                eprintln!("warning: {w}");
            }
            r.into_value()
        }
        Err(e) => {
            eprintln!("Error reading {input}: {e}");
            process::exit(1);
        }
    };

    let file = match File::create(output) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Cannot create {output}: {e}");
            process::exit(1);
        }
    };

    match biodream::export::arrow::to_arrow_ipc(&df, BufWriter::new(file)) {
        Ok(()) => eprintln!(
            "Wrote {} channels ({} rows) to {output}",
            df.channels.len(),
            df.channels.first().map_or(0, |ch| ch.scaled_samples().len()),
        ),
        Err(e) => {
            eprintln!("Arrow export failed: {e}");
            process::exit(1);
        }
    }
}

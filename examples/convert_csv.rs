//! Example: read a `.acq` file and export to CSV.
//!
//! # Usage
//!
//! ```sh
//! cargo run --example convert_csv -- input.acq output.csv
//! # Or pipe to stdout:
//! cargo run --example convert_csv -- input.acq -
//! ```

use std::{
    env,
    fs::File,
    io::{self, BufWriter, Write},
    process,
};

fn main() {
    let args: Vec<String> = env::args().collect();
    let (Some(input), Some(output)) = (args.get(1), args.get(2)) else {
        eprintln!("Usage: convert_csv <input.acq> <output.csv|->");
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

    let options = biodream::CsvOptions::new();

    let result = if output == "-" {
        let stdout = io::stdout();
        let mut writer = BufWriter::new(stdout.lock());
        let r = biodream::to_csv(&df, &mut writer, &options);
        let _ = writer.flush();
        r
    } else {
        match File::create(output) {
            Ok(f) => {
                let mut writer = BufWriter::new(f);
                let r = biodream::to_csv(&df, &mut writer, &options);
                let _ = writer.flush();
                r
            }
            Err(e) => {
                eprintln!("Cannot create {output}: {e}");
                process::exit(1);
            }
        }
    };

    match result {
        Ok(()) => eprintln!("Wrote {} channels to {output}", df.channels.len()),
        Err(e) => {
            eprintln!("Export failed: {e}");
            process::exit(1);
        }
    }
}

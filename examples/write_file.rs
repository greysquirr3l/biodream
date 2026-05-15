//! Example: create a synthetic `Datafile` and write it to an `.acq` file.
//!
//! Requires the `write` feature:
//!
//! ```sh
//! cargo run --example write_file --features write -- output.acq
//! # Then verify by reading it back:
//! cargo run --example read_file -- output.acq
//! ```

fn main() {
    #[cfg(not(feature = "write"))]
    {
        eprintln!(
            "This example requires the `write` feature. \
             Re-run with: cargo run --example write_file --features write -- <output.acq>"
        );
        std::process::exit(1);
    }

    #[cfg(feature = "write")]
    run();
}

#[cfg(feature = "write")]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "sine amplitude * 1000 fits in i16; usize-to-f64 for time index is intentional"
)]
fn run() {
    use std::{env, process};

    use biodream::{ByteOrder, Channel, ChannelData, Datafile, FileRevision, GraphMetadata};

    let args: Vec<String> = env::args().collect();
    let Some(output) = args.get(1) else {
        eprintln!("Usage: write_file <output.acq>");
        process::exit(1);
    };
    let output = output.clone();

    // Build a synthetic 2-channel recording at 1000 Hz.
    let n_samples: usize = 1000;
    let base_rate = 1000.0_f64;

    // Sine wave at 1 Hz for the ECG channel (scale 0.001 mV/LSB, +/-1 mV peak).
    let ecg_raw: Vec<i16> = (0..n_samples)
        .map(|i| {
            let t = i as f64 / base_rate;
            ((2.0 * std::f64::consts::PI * t).sin() * 1000.0) as i16
        })
        .collect();

    // Slow sine at 0.25 Hz for the respiration channel (half sample rate).
    let resp_n = n_samples / 2;
    let resp_raw: Vec<i16> = (0..resp_n)
        .map(|i| {
            let t = i as f64 / (base_rate / 2.0);
            (0.5 * (2.0 * std::f64::consts::PI * 0.25 * t).sin() * 1000.0) as i16
        })
        .collect();

    let df = Datafile {
        metadata: GraphMetadata {
            file_revision: FileRevision::new(43),
            samples_per_second: base_rate,
            channel_count: 2,
            byte_order: ByteOrder::LittleEndian,
            compressed: false,
            title: None,
            acquisition_datetime: None,
            max_samples_per_second: None,
        },
        channels: vec![
            Channel {
                name: "ECG".to_string(),
                units: "mV".to_string(),
                samples_per_second: base_rate,
                frequency_divider: 1,
                point_count: n_samples,
                data: ChannelData::Scaled {
                    raw: ecg_raw,
                    scale: 0.001,
                    offset: 0.0,
                },
            },
            Channel {
                name: "Respiration".to_string(),
                units: "V".to_string(),
                samples_per_second: base_rate / 2.0,
                frequency_divider: 2,
                point_count: resp_n,
                data: ChannelData::Scaled {
                    raw: resp_raw,
                    scale: 0.001,
                    offset: 0.0,
                },
            },
        ],
        markers: vec![],
        journal: None,
    };

    match biodream::write_file(&df, &output) {
        Ok(()) => eprintln!("Wrote {} channels to {output}", df.channels.len()),
        Err(e) => {
            eprintln!("Write failed: {e}");
            process::exit(1);
        }
    }
}

# Exporting Data

## CSV

Available with the default `csv` feature.

```rust
use biodream::{CsvOptions, TimeFormat};

let df = biodream::read_file("recording.acq")?.into_value();

let opts = CsvOptions {
    time_format: TimeFormat::Seconds,
    include_markers: true,
    delimiter: b',',
};

df.to_csv("output.csv", &opts)?;
```

### TimeFormat

| Variant | Behavior |
|---|---|
| `TimeFormat::Seconds` | Elapsed seconds column (default) |
| `TimeFormat::Milliseconds` | Elapsed milliseconds column |
| `TimeFormat::Samples` | Integer sample index column |

### Multi-rate channels in CSV

When channels have different sampling rates, all channels are resampled to the
base rate (highest sampling rate in the file). Resampling uses linear
interpolation to fill lower-rate channels.

## Apache Arrow IPC

Requires `features = ["arrow"]`.

```rust
use biodream::ArrowOptions;

let df = biodream::read_file("recording.acq")?.into_value();

// Write Arrow IPC stream format
df.to_arrow_ipc("output.arrow", &ArrowOptions::default())?;
```

Arrow IPC files can be read by:

- Python/Polars: `pl.read_ipc("output.arrow")`
- R: `arrow::read_ipc_file("output.arrow")`
- Julia: `Arrow.Table("output.arrow")`

## Parquet

Requires `features = ["arrow", "parquet"]`.

```rust
use biodream::ParquetOptions;

let df = biodream::read_file("recording.acq")?.into_value();

df.to_parquet("output.parquet", &ParquetOptions::default())?;
```

Parquet output is compatible with DuckDB, Apache Spark, Pandas, and Polars.

### Parquet compression

```rust
use biodream::ParquetOptions;
use parquet::basic::Compression;

let opts = ParquetOptions {
    compression: Compression::SNAPPY,
    ..ParquetOptions::default()
};
df.to_parquet("output.parquet", &opts)?;
```

## HDF5

Requires `features = ["hdf5"]` and the system `libhdf5-dev` library.

```rust
use biodream::Hdf5Options;

let df = biodream::read_file("recording.acq")?.into_value();

df.to_hdf5("output.h5", &Hdf5Options::default())?;
```

The HDF5 file has the following layout:

```
/channels/{channel_name}/data   — f64 dataset of scaled samples
/channels/{channel_name}/attrs  — name, units, samples_per_second
/markers/                       — marker dataset with time, label, text
/metadata/                      — revision, recorded_at, byte_order
```

## CLI export

All export formats are also accessible via the `biopac convert` command. See
[CLI Reference](./cli.md) for full details.

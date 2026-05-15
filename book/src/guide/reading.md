# Reading .acq Files

## API surface

The `read` feature exposes three entry points:

| Function | Description |
|---|---|
| `biodream::read_file(path)` | Read a `.acq` file from disk. |
| `biodream::read_stream(reader)` | Read from any `Read + Seek`. |
| `biodream::LazyDatafile::open(path, opts)` | Deferred streaming reader. |

All return `ParseResult<Datafile>` (or `LazyDatafile` for the lazy variant).

## ParseResult and warnings

Biodream distinguishes between **fatal errors** (returned as `Err(BiopacError)`)
and **non-fatal warnings** (accumulated in `ParseResult::warnings`). Warnings
cover things like unknown section types, unexpected padding bytes, or fields
that are out of expected range but still parse cleanly.

```rust
let result = biodream::read_file("recording.acq")?;

if !result.is_clean() {
    for w in &result.warnings {
        eprintln!("warning: {w}");
    }
}

// Consumes result, giving you the Datafile
let df = result.into_value();
```

Always check `result.warnings` — a clean parse on a healthy file returns zero
warnings.

## Datafile structure

```
Datafile
├── metadata: GraphMetadata    — title, date/time, byte order, revision
├── channels: Vec<Channel>     — ordered by channel index
├── markers:  Vec<Marker>      — event markers (may be empty)
└── journal:  Option<Journal>  — free-text journal section
```

### GraphMetadata

```rust
let meta = &df.metadata;
println!("recorded: {}", meta.recorded_at);   // chrono::NaiveDateTime
println!("revision: {}", meta.revision);       // FileRevision (v30–v84+)
println!("byte order: {:?}", meta.byte_order); // ByteOrder::LittleEndian / BigEndian
```

### Channel

```rust
for ch in &df.channels {
    // Scaled f64 samples (raw i16 converted via scale + offset)
    let samples: Vec<f64> = ch.scaled_samples();

    // Raw i16 samples without scaling
    let raw: &[i16] = ch.raw_samples();

    println!(
        "{} [{}]: {} samples @ {} Hz",
        ch.name, ch.units, samples.len(), ch.samples_per_second,
    );
}
```

Channels with a sampling rate lower than the base rate are upsampled to the
base rate via linear interpolation in `scaled_samples()`.

### Mixed sampling rates

AcqKnowledge supports channels at different fractions of the base rate. Each
channel carries a `frequency_divider`; biodream computes `samples_per_second`
correctly for all channels regardless of their divider.

```rust
let base_hz = df.metadata.samples_per_second;
for ch in &df.channels {
    println!("{}: {} Hz (divider {})", ch.name, ch.samples_per_second, ch.frequency_divider);
}
```

## Lazy reader

For recordings with many channels or large sample buffers, `LazyDatafile`
reads only the channel headers on open and loads individual channel data
on demand:

```rust
use biodream::{LazyDatafile, ReadOptions};

let lazy = LazyDatafile::open("large.acq", &ReadOptions::default())?;

// Load channels selectively — no unnecessary I/O
let ecg = lazy.load_channel(0)?;
let resp = lazy.load_channel(2)?;
```

This is the recommended approach for batch-processing pipelines that only
need a subset of channels.

## Byte order

AcqKnowledge pre-4 files were often written on big-endian Mac hardware.
Biodream detects the byte order from the `lVersion` field sign bit and
handles both transparently. The detected order is exposed in
`GraphMetadata::byte_order`.

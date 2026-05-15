# Writing .acq Files

Requires the `write` feature:

```toml
biodream = { version = "0.2", features = ["write"] }
```

## Basic write

```rust
use biodream::{write_file, WriteOptions};

let df = biodream::read_file("input.acq")?.into_value();

// Modify channels, markers, etc.

write_file(&df, "output.acq", &WriteOptions::default())?;
```

## Round-trip fidelity

Every `write → read → write` cycle produces bitwise-identical output. The test
suite verifies this against all 14 synthetic fixture files (v30–v84+, with and
without compression).

```rust
// Verify round-trip
let original = std::fs::read("input.acq")?;
let df = biodream::read_stream(std::io::Cursor::new(&original))?.into_value();

let mut buf = Vec::new();
biodream::write_stream(&df, &mut buf, &WriteOptions::default())?;

assert_eq!(original, buf, "round-trip produced different bytes");
```

## WriteOptions

```rust
use biodream::{WriteOptions, ByteOrder};

let opts = WriteOptions {
    // Override the output byte order (default: preserves the source file's order)
    byte_order: Some(ByteOrder::LittleEndian),
};
```

## Creating a Datafile from scratch

```rust
use biodream::domain::{
    Datafile, GraphMetadata, Channel, ChannelData, FileRevision, ByteOrder,
};

let meta = GraphMetadata {
    revision: FileRevision::V84,
    byte_order: ByteOrder::LittleEndian,
    samples_per_second: 1000.0,
    // ...
    ..GraphMetadata::default()
};

let samples: Vec<i16> = vec![0i16; 5000];
let channel = Channel {
    name: "ECG".to_string(),
    units: "mV".to_string(),
    samples_per_second: 1000.0,
    frequency_divider: 1,
    data: ChannelData::Scaled {
        raw: samples,
        scale: 0.001,
        offset: 0.0,
    },
    ..Channel::default()
};

let df = Datafile {
    metadata: meta,
    channels: vec![channel],
    markers: vec![],
    journal: None,
};

biodream::write_file(&df, "synthetic.acq", &WriteOptions::default())?;
```

## Compression

Compressed output (per-channel zlib) is supported for revision ≥ 68 files when
the source file used compression. To force compressed output on a new file,
set the appropriate flag in `WriteOptions` (see the API docs).

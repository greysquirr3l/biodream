# Quick Start

## Reading a file

```rust
use biodream::read_file;

let result = read_file("recording.acq")?;

// Non-fatal parse warnings (unknown section types, unexpected padding, etc.)
for w in &result.warnings {
    eprintln!("warning: {w}");
}

let df = result.into_value();
println!("{}", df.summary());

for channel in &df.channels {
    let samples = channel.scaled_samples();
    println!(
        "{}: {} samples @ {} Hz",
        channel.name,
        samples.len(),
        channel.samples_per_second,
    );
}
```

## Reading from a stream

Any `Read + Seek` source works:

```rust
use std::io::Cursor;
use biodream::read_stream;

let bytes: Vec<u8> = std::fs::read("recording.acq")?;
let result = read_stream(Cursor::new(&bytes))?;
let df = result.into_value();
```

## Lazy / streaming reader

For large files, load only channel headers up front and stream samples on demand:

```rust
use biodream::{LazyDatafile, ReadOptions};

let opts = ReadOptions::default();
let lazy = LazyDatafile::open("recording.acq", &opts)?;

println!("channels: {}", lazy.channel_count());

// Load one channel at a time — never buffers the entire file
for i in 0..lazy.channel_count() {
    let ch = lazy.load_channel(i)?;
    println!("{}: {} samples", ch.name, ch.scaled_samples().len());
}
```

## Accessing markers

```rust
let df = biodream::read_file("recording.acq")?.into_value();

for marker in &df.markers {
    println!(
        "[{:.3}s] {} — {}",
        marker.time_ms / 1000.0,
        marker.style.label,
        marker.text.as_deref().unwrap_or(""),
    );
}
```

## Error handling

All errors are typed `BiopacError` variants carrying the byte offset where the
problem was detected:

```rust
use biodream::BiopacError;

match biodream::read_file("corrupt.acq") {
    Ok(r) => { /* ... */ }
    Err(BiopacError::UnexpectedEof { offset, expected }) => {
        eprintln!("truncated at byte {offset}: expected {expected} more bytes");
    }
    Err(e) => eprintln!("parse error: {e}"),
}
```

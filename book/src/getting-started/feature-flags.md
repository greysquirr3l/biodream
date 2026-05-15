# Feature Flags

| Flag | Default | Description |
|------|:-------:|-------------|
| `read` | ✅ | Read `.acq` files. Enables the parser, domain types, and `read_file` / `read_stream` API. Requires `std`. |
| `csv` | ✅ | CSV export via `to_csv` / `CsvOptions`. |
| `write` | ❌ | Write `.acq` files. Round-trip fidelity: `read → modify → write → diff = clean`. |
| `arrow` | ❌ | Apache Arrow IPC export (`to_arrow_ipc`). Compatible with Polars, R `arrow`, and Julia. |
| `parquet` | ❌ | Parquet export (`to_parquet`). Requires `arrow`. Compatible with DuckDB, Spark, Pandas. |
| `hdf5` | ❌ | HDF5 export (`to_hdf5`). Requires the `libhdf5-dev` system library. |
| `serde` | ❌ | `Serialize` / `Deserialize` derives on all public domain types. |

## Combining features

```toml
# Read + write + all export formats
biodream = { version = "0.2", features = ["write", "arrow", "parquet", "serde"] }

# Minimal read-only, no CSV
biodream = { version = "0.2", default-features = false, features = ["read"] }

# no_std core (parser + domain only — no I/O or export)
biodream = { version = "0.2", default-features = false }
```

## Feature dependency graph

```
parquet → arrow → read
write   → read
csv     → read
hdf5    → read
serde   (standalone)
```

## CI matrix

The CI suite tests with `--features "read,write,csv,arrow,parquet,serde"`.
The `hdf5` feature is excluded from CI because it requires a system library;
it is tested separately in the full integration suite.

# CLI Reference

The `biopac` binary is the command-line interface for biodream. It provides
four subcommands for inspecting and converting `.acq` files.

## Usage

```
biopac <COMMAND> [OPTIONS] <FILE>
```

## Global flags

| Flag | Description |
|---|---|
| `--help` | Print help |
| `--version` | Print version |
| `-v, --verbose` | Verbose output (repeat for more: `-vv`) |

---

## `biopac info`

Print a summary of the file: revision, channel count, sample rate, duration,
marker count, and journal presence.

```sh
biopac info recording.acq
```

Output:

```
File:       recording.acq
Revision:   v84
Channels:   4
Base rate:  1000.00 Hz
Duration:   60.000 s
Markers:    12
Journal:    yes
Byte order: LittleEndian

Channels:
  [0] ECG    1000.00 Hz  mV   (divider 1)
  [1] EDA    250.00 Hz   µS   (divider 4)
  [2] RESP   125.00 Hz   %    (divider 8)
  [3] TEMP   10.00 Hz    °C   (divider 100)
```

---

## `biopac convert`

Convert a `.acq` file to CSV, Arrow IPC, or Parquet.

```sh
biopac convert recording.acq --format csv --output output.csv
biopac convert recording.acq --format arrow --output output.arrow
biopac convert recording.acq --format parquet --output output.parquet
```

### Flags

| Flag | Default | Description |
|---|---|---|
| `--format <FORMAT>` | `csv` | Output format: `csv`, `arrow`, `parquet` |
| `--output <PATH>` | (derived) | Output file path. Defaults to input stem + new extension. |
| `--time-format <FMT>` | `seconds` | Time column format: `seconds`, `ms`, `samples` |
| `--no-markers` | — | Omit markers column from CSV output |
| `--channels <LIST>` | (all) | Comma-separated channel names or indices to export |

---

## `biopac markers`

Print the event marker list as tab-separated text.

```sh
biopac markers recording.acq
```

Output:

```
TIME_S  LABEL       TEXT
0.000   event       Baseline start
30.450  stim        Stimulus onset
31.200  resp        Response
60.000  event       End
```

### Flags

| Flag | Description |
|---|---|
| `--json` | Output markers as JSON array |
| `--csv` | Output as CSV with header row |

---

## `biopac inspect`

Low-level binary dump: print each section header, its type code, byte offset,
and length. Useful for debugging corrupt or unusual files.

```sh
biopac inspect recording.acq
```

Output:

```
offset=0x0000  type=GraphHeader   len=506
offset=0x01FA  type=ChannelHeader len=182  ch=0 ECG
offset=0x02B0  type=ChannelHeader len=182  ch=1 EDA
offset=0x0366  type=ChannelHeader len=182  ch=2 RESP
offset=0x041C  type=ChannelHeader len=182  ch=3 TEMP
offset=0x04D2  type=DataSection   len=12000000
offset=0xB75D2 type=MarkerSection len=412
```

### Flags

| Flag | Description |
|---|---|
| `--hex` | Print a hex dump of each section header |
| `--unknown` | Print unknown section types encountered (useful for format research) |

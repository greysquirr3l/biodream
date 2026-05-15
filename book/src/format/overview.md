# Format Overview

BIOPAC AcqKnowledge `.acq` files are binary files used to store multi-channel
physiological data recorded with BIOPAC hardware. The format has evolved
through more than 50 revisions since the early 1990s, spanning big-endian
Classic Mac 68k builds through modern Windows/macOS 64-bit applications.

## Format versions

All version identifiers are stored in the `lVersion` field of the graph header
as a signed 32-bit integer. Negative values indicate big-endian byte order
(legacy Mac builds).

| Version range | Era |
|---|---|
| 30–40 | Classic Mac (68k / PPC), big-endian |
| 41–66 | Windows transition era |
| 67–84 | Modern Windows/macOS |
| 84+ | Current AcqKnowledge 5.x |

## File structure (top-level sections)

Every `.acq` file is a flat sequence of typed sections:

```
[GraphHeader]
[ChannelHeader × channel_count]
[CompressionHeader × channel_count]  — only in revision ≥ 68
[DataSection]                         — interleaved sample data
[MarkerSection]                       — event markers (optional)
[JournalSection]                      — free-text journal (optional)
[ForeignDataSection …]                — unknown/vendor-specific sections
```

## Interleave pattern

Sample data is written in interleaved blocks. Each channel writes
`frequency_divider` worth of samples per "tick" of the base rate. A full
interleave block is:

```
ch0_samples[0..n0] | ch1_samples[0..n1] | … | chN_samples[0..nN]
```

where `nI = base_samples_per_block / channel[I].frequency_divider`.

Biodream's streaming reader consumes exactly one interleave block at a time,
so memory usage is O(block\_size) not O(file\_size).

## Further reading

The field-by-field binary layout (graph header, channel header, compression
header, all section types) is documented in the internal ADR and in the parser
source at `src/parser/`.

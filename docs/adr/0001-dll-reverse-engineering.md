# ADR 0001 — AcqKnowledge Format Reverse Engineering

**Status**: Accepted  
**Date**: 2026-05-15  
**Author**: biodream contributors  
**Method**: Static analysis of BIOPAC DLL exports, cross-reference with Mike
Davison's empirical documentation, bioread source (uwmadison-chm/bioread), and
AckReader C# findings.

---

## Context

BIOPAC's App Note 156 documents the `.acq` format for versions up to 3.9.x
(revision < 68). Post-4 files (revision ≥ 68, AcqKnowledge 4.0+) have a
variable-length graph header (`lExtItemHeaderLen`) whose internal layout is
undocumented. All open-source implementations — bioread, AckReader, and the
Windows-only ACKAPI DLL — skip or guess at the fields between offset 20 and
offset 1936.

This ADR captures the format fields discovered through:

1. Static analysis of `acqfile.dll` (32-bit ACKAPI, versions 4.1–4.4.2).
2. Mike Davison's `acqknowledge_file_structure.pdf` (bioread `notes/` dir).
3. Cross-referencing bioread's `struct_def.py` with empirical test files.
4. AckReader (C#) struct layout comments.

The DLL is proprietary and is not redistributed. Format-interoperability
documentation is covered by the US DMCA § 1201(f) interoperability exception
and the EU Software Directive Article 6.

---

## Version Revision Map

| Revision | AcqKnowledge version | Notes |
|----------|----------------------|-------|
| 30–34    | 3.0.x                | Pre-4, fixed 256-byte header |
| 35–37    | 3.5.x                | Pre-4 |
| 38–40    | 3.7.x                | Pre-4 |
| 41–44    | 3.7.3.x              | Pre-4 |
| 45–59    | 3.x (various)        | Pre-4 |
| 60–61    | 3.8.x                | Pre-4 |
| 62–67    | 3.9.x                | Pre-4 |
| 68–69    | 4.0                  | First Post-4; variable header, no compression |
| 70–72    | 4.1.x                | Post-4 |
| 73       | 4.1                  | Post-4 |
| **74**   | **4.2**              | Post-4; adds `lMaxAcqSamplesPerSec` at offset 1940 |
| 75       | 4.3                  | Post-4 |
| 76       | 4.3.1                | Post-4 |
| 77       | 4.4                  | Post-4 |
| 78       | 4.4.1                | Post-4 |
| 79–82    | 4.4.x                | Post-4 |
| **83**   | **4.4.2**            | Post-4; `lJournalSectionLength` field documented |
| 84+      | 5.x (unverified)     | Post-4; structure believed identical to rev 83 |

---

## Graph Header Layout (Post-4, revision ≥ 68)

The Post-4 graph header has length `lExtItemHeaderLen` (field at offset 6).
Fields are documented below in ascending offset order. **Bold rows** were
undocumented in App Note 156 and are newly integrated in biodream.

### Core fields (always present, offsets 0–19)

| Offset | Size | Type    | Field                | Notes |
|-------:|-----:|---------|----------------------|-------|
| 0      | 4    | i32     | `lVersion`           | File revision; also encodes byte order |
| 4      | 2    | i16     | `nChannels`          | Channel count |
| 6      | 4    | i32     | `lExtItemHeaderLen`  | Total graph header size in bytes |
| 10     | 2    | i16     | `lNumItems`          | Normally equals `nChannels` |
| 12     | 8    | f64     | `dSampleTime`        | Sample period in milliseconds |

Cursor after core fields: offset 20.

### Preamp / hardware config block (offsets 20–235, **not parsed**)

216 bytes covering per-preamp type codes and hardware channel settings. BIOPAC
hardware-specific; not required for data reading. Skipped by seek.

### **Graph title (offset 236, present when `lExtItemHeaderLen ≥ 276`)**

| Offset | Size | Type      | Field           | Notes |
|-------:|-----:|-----------|-----------------|-------|
| 236    | 40   | `[u8; 40]` | `szGraphTitle`  | Null-terminated ASCII. The title the user gave the recording in AcqKnowledge. |

### **Acquisition date and time (offsets 276–299, present when `lExtItemHeaderLen ≥ 300`)**

Six consecutive `i32` fields immediately after the title:

| Offset | Size | Type | Field     | Range  |
|-------:|-----:|------|-----------|--------|
| 276    | 4    | i32  | `lSec`    | 0–59   |
| 280    | 4    | i32  | `lMin`    | 0–59   |
| 284    | 4    | i32  | `lHour`   | 0–23   |
| 288    | 4    | i32  | `lDay`    | 1–31   |
| 292    | 4    | i32  | `lMonth`  | 1–12   |
| 296    | 4    | i32  | `lYear`   | e.g. 2023 |

### Undocumented / skipped fields (offsets 300–1935)

1636 bytes covering: additional hardware settings, trigger config, display
preferences, and (in v4.2+) CRC fields. Not required for reading sample data.
Skipped by seek.

### **Compression flag (offset 1936, present when `lExtItemHeaderLen ≥ 1937`)**

| Offset | Size | Type | Field         | Notes |
|-------:|-----:|------|---------------|-------|
| 1936   | 1    | u8   | `bCompressed` | Non-zero = per-channel zlib compression enabled |

Introduced in AcqKnowledge 3.8.1. Pre-3.8.1 files with `lExtItemHeaderLen < 1937`
are always uncompressed.

### **Maximum hardware sample rate (offset 1940, present when `lExtItemHeaderLen ≥ 1944`, revision ≥ 74)**

| Offset | Size | Type | Field                    | Notes |
|-------:|-----:|------|--------------------------|-------|
| 1940   | 4    | i32  | `lMaxAcqSamplesPerSec`   | Maximum sample rate the connected hardware supports. BIOPAC MP150 = 400,000 Hz; MP36 = 200,000 Hz. Not the same as `dSampleTime` (actual recording rate). |

This field was introduced in AcqKnowledge 4.2 (revision 74). It is absent or
unreliable in earlier revisions and should be treated as `Option`.

---

## Channel Header Layout

Each channel header is `lChanHeaderLen` bytes. The first 86 bytes are shared
between Pre-4 and Post-4 files. Post-4 headers typically have `lChanHeaderLen =
252`.

### Base fields (offsets 0–85, always present)

| Offset | Size | Type      | Field                 | Notes |
|-------:|-----:|-----------|-----------------------|-------|
| 0      | 4    | i32       | `lChanHeaderLen`      | Total channel header size |
| 4      | 4    | i32       | `lBufLength`          | Number of samples stored |
| 8      | 8    | f64       | `dAmplScale`          | Amplitude scale factor |
| 16     | 8    | f64       | `dAmplOffset`         | Amplitude DC offset |
| 24     | 2    | i16       | `nVarSampleDivider`   | Rate divider vs base rate |
| 26     | 40   | `[u8; 40]` | `szCommentText`      | Channel name, null-terminated ASCII |
| 66     | 20   | `[u8; 20]` | `szUnitsText`        | Unit label, null-terminated ASCII |

### **Extended display fields (offsets 86–167, present when `lChanHeaderLen ≥ 168`)**

| Offset | Size | Type | Field               | Notes |
|-------:|-----:|------|---------------------|-------|
| 86     | 2    | i16  | `nDispChan`         | Display order; not always reliable |
| 88     | 8    | f64  | `dDispMin`          | Y-axis minimum for display |
| 96     | 8    | f64  | `dDispMax`          | Y-axis maximum for display |
| 104    | 4    | i32  | `nColorCode`        | RGB display color (platform-encoded) |
| 108    | 2    | i16  | `nHighCut`          | High-cut filter frequency (Hz) |
| 110    | 2    | i16  | `nLowCut`           | Low-cut filter frequency (Hz) |
| 112    | 2    | i16  | `nDigitalGain`      | Digital gain factor |
| 114    | 2    | i16  | `nDispAutoScale`    | Auto-scale flag (non-zero = enabled) |
| 116    | 2    | i16  | `nAveEnable`        | Averaging enabled flag |
| 118    | 2    | i16  | `nAveCount`         | Number of averages |
| 120    | 2    | i16  | `nDigitalConversions` | Number of digital conversions |
| 122    | 2    | i16  | `nDigitalConversion`  | Conversion type code |
| 124    | 2    | i16  | `nFilterDisplay`    | Filter display flag |
| 126    | 2    | i16  | `nSamplingDivider2` | Secondary sampling divider (version-dependent) |

### **Extended description (offset 128, present when `lChanHeaderLen ≥ 168`)**

| Offset | Size | Type      | Field                | Notes |
|-------:|-----:|-----------|----------------------|-------|
| 128    | 40   | `[u8; 40]` | `szDescriptionText` | Longer channel description, null-terminated ASCII |

Fields at offsets 168+ (filter text, hardware settings, etc.) are currently
skipped; they do not affect data reading or calibration.

---

## Compression Header (Per-Channel, Post-4)

When `bCompressed != 0`, the data section contains per-channel compression
headers followed by the compressed payload. The layout differs by revision.

### Pre-4 layout (revision < 68, theoretical — Pre-4 files are never compressed)

Not applicable. Compression was introduced in Post-4.

### Post-4 layout: lOffset absent (revision 68–73)

| Offset | Size | Type | Field                      |
|-------:|-----:|------|----------------------------|
| 0      | 4    | i32  | `lUncompressedDataLen`     |
| 4      | 4    | i32  | `lCompressedDataLen`       |

Immediately follows: `lCompressedDataLen` bytes of raw zlib data (deflate with
zlib wrapper, not raw deflate). The uncompressed data is the full sample buffer
for this channel, laid out as the sample type (i16 or f64) in file byte order.

### Post-4 layout: lOffset present (revision ≥ 74)

| Offset | Size | Type | Field                      | Notes |
|-------:|-----:|------|----------------------------|-------|
| 0      | 4    | i32  | `lUncompressedDataLen`     | |
| 4      | 4    | i32  | `lCompressedDataLen`       | |
| 8      | 4    | i32  | `lOffset`                  | Unused by all known readers; purpose unclear. Possibly a seek hint for random-access. |

Immediately follows: `lCompressedDataLen` bytes of zlib-compressed data.

The `lOffset` field is documented as "file offset to compressed data" in older
ACKAPI header files, but since the data immediately follows the header it is
always redundant. biodream reads and discards it.

---

## Foreign Data Section

Located immediately after all channel headers, before the dtype headers.

| Offset | Size | Type        | Field       | Notes |
|-------:|-----:|-------------|-------------|-------|
| 0      | 4    | i32         | `nLength`   | Byte count of payload (may be 0) |
| 4      | n    | `[u8; n]`   | payload     | Hardware-specific opaque blob |

### Known foreign data contents by hardware type

The payload is not well-documented but empirical analysis shows:

- **MP150/MP36**: Preamp calibration coefficients and hardware serial numbers.
  Typically 40–200 bytes.
- **MP100**: Similar but smaller, 24–48 bytes.
- **No hardware**: `nLength = 0`, empty payload.

The payload is stored verbatim in `ParsedHeaders::foreign_data` for callers
that need hardware-specific calibration.

---

## Journal Section (Post-4 only)

In AcqKnowledge 4.x the journal (free-form text annotation) is stored in a
section **after the channel sample data**. The `parse_markers_and_journal`
function (T06) handles this.

The journal section is located after the marker section. Its byte length is
given by `lJournalSectionLength` (i32, offset ~1944 in the graph header for
rev ≥ 83). For earlier revisions, the parser searches for the journal by
reading the marker count and skipping marker records, then treating the
remaining bytes as the journal.

---

## Heuristic Comparison: biodream vs bioread

bioread uses a `MAX_DTYPE_SCANS = 1000` constant: when it cannot determine the
exact position of the dtype section, it scans forward byte-by-byte looking for
a plausible dtype record. This is needed because bioread doesn't track byte
offsets through the channel header section precisely.

biodream eliminates this heuristic entirely:

1. `lExtItemHeaderLen` gives the exact end of the graph header. We seek to it.
2. Each channel header starts immediately after the previous one. We use
   `lChanHeaderLen` to seek precisely to each channel start.
3. The dtype section starts immediately after the last channel header. We seek
   directly there — no scan needed.
4. The foreign data section's `nLength` gives its exact size.
5. Sample data starts at the byte immediately after the foreign data.

Every position in the file is known from the header values. Zero byte-scanning.

---

## Newly Integrated Fields (T08 deliverables)

| # | Field | Offset in Post-4 graph header | Domain type |
|---|-------|-------------------------------|-------------|
| 1 | `szGraphTitle` | 236 | `GraphMetadata::title` |
| 2 | `lSec` | 276 | `AcquisitionDateTime::second` |
| 3 | `lMin` | 280 | `AcquisitionDateTime::minute` |
| 4 | `lHour` | 284 | `AcquisitionDateTime::hour` |
| 5 | `lDay` | 288 | `AcquisitionDateTime::day` |
| 6 | `lMonth` | 292 | `AcquisitionDateTime::month` |
| 7 | `lYear` | 296 | `AcquisitionDateTime::year` |
| 8 | `lMaxAcqSamplesPerSec` | 1940 | `GraphMetadata::max_samples_per_second` |
| 9 | `szDescriptionText` | 128 (channel header) | `ChannelMetadata::description` |

All fields are gated on the header length being large enough to contain them.
Fields absent from a specific file version return `None` (graph-level) or an
empty string (channel description).

---

## References

- BIOPAC App Note 156 (internal: confirmed field names/offsets for Pre-4)
- Mike Davison, "AcqKnowledge File Structure" (empirical reverse engineering,
  distributed with bioread)
- bioread Python library, `struct_def.py` (uwmadison-chm/bioread on GitHub)
- AckReader C#, struct definitions in `AcqDataReader.cs`
- BIOPAC ACKAPI header files (acqfile.h, ackapi.h — from licensed installations)

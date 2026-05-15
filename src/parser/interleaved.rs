//! Streaming reader for uncompressed, interleaved channel data.
//!
//! BIOPAC uncompressed files interleave samples from all channels in a
//! repeating pattern determined by each channel's `frequency_divider`.
//!
//! Example: 3 channels where channel 2 runs at half the base rate:
//! ```text
//! [ch0][ch1][ch2] [ch0][ch2] [ch0][ch1][ch2] [ch0][ch2] …
//! ```
//!
//! Key invariant (from bioread 1.0.0 rewrite note): the file may end mid-pattern.
//! Some channels may have more samples than others. The reader must not trim data
//! to the nearest complete pattern repetition.

use alloc::vec::Vec;

use std::io::{Read, Seek, SeekFrom};

use crate::{
    domain::{ByteOrder, Channel, ChannelData, ChannelMetadata},
    error::{BiopacError, Warning},
    parser::headers::{ParsedHeaders, SampleType},
};

// ---------------------------------------------------------------------------
// LCM / GCD helpers
// ---------------------------------------------------------------------------

const fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

const fn lcm(a: u64, b: u64) -> u64 {
    if a == 0 || b == 0 {
        0
    } else {
        a / gcd(a, b) * b
    }
}

// ---------------------------------------------------------------------------
// Sample pattern
// ---------------------------------------------------------------------------

/// Compute the repeating channel-index sequence for one interleave cycle.
///
/// For each slot `i` in `0..lcm(dividers)`, every channel `j` whose divider
/// `d_j` divides `i` evenly contributes one sample in that slot.
///
/// # Examples
///
/// Two channels, dividers `[1, 2]` → LCM=2 → `[0, 1, 0]`
/// Three channels, dividers `[1, 2, 4]` → LCM=4 → `[0, 1, 2, 0, 0, 1, 0]`
pub fn compute_sample_pattern(dividers: &[u16]) -> Vec<usize> {
    if dividers.is_empty() {
        return Vec::new();
    }

    let cycle_len = dividers
        .iter()
        .fold(1u64, |acc, &d| lcm(acc, u64::from(d.max(1))));

    let mut pattern = Vec::new();
    for slot in 0..cycle_len {
        for (j, &d) in dividers.iter().enumerate() {
            let d = u64::from(d.max(1));
            if slot % d == 0 {
                pattern.push(j);
            }
        }
    }
    pattern
}

// ---------------------------------------------------------------------------
// Interleaved reader
// ---------------------------------------------------------------------------

const CHUNK_BYTES: usize = 1 << 20; // 1 MiB

/// Read uncompressed interleaved sample data, returning one [`Channel`] per
/// entry in `headers.channel_metadata`.
///
/// The reader must already be positioned at `headers.data_start_offset`.
#[expect(
    clippy::too_many_lines,
    reason = "single-pass streaming read loop; splitting would obscure the read/demux/scale pipeline"
)]
#[expect(
    clippy::similar_names,
    reason = "names encode element type; shortening loses precision"
)]
pub(crate) fn read_interleaved<R: Read + Seek>(
    reader: &mut R,
    headers: &ParsedHeaders,
) -> Result<(Vec<Channel>, Vec<Warning>), BiopacError> {
    let meta = &headers.channel_metadata;
    let types = &headers.sample_types;
    let n = meta.len();

    if n == 0 {
        return Ok((Vec::new(), headers.warnings.clone()));
    }

    let byte_order = headers.graph_metadata.byte_order;
    let base_rate = headers.graph_metadata.samples_per_second;

    // Per-channel sample budgets (0 means unbounded — read until EOF).
    let budgets: Vec<u32> = meta.iter().map(|m| m.sample_count).collect();

    // Per-channel accumulator buffers.
    let mut raw_i16: Vec<Vec<i16>> = (0..n).map(|_| Vec::new()).collect();
    let mut raw_f64: Vec<Vec<f64>> = (0..n).map(|_| Vec::new()).collect();

    // Precompute dividers and pattern.
    let dividers: Vec<u16> = meta.iter().map(|m| m.frequency_divider).collect();
    let pattern = compute_sample_pattern(&dividers);

    // Seek to the data start.
    reader.seek(SeekFrom::Start(headers.data_start_offset))?;

    // Read data byte by byte following the pattern, using a refillable
    // intermediate buffer to amortise syscall overhead.
    let mut buf = alloc::vec![0u8; CHUNK_BYTES];
    let mut buf_pos = 0usize; // next unread byte in buf
    let mut buf_end = 0usize; // valid bytes in buf

    let mut pattern_idx = 0usize; // position in pattern

    // Returns the next byte from the internal buffer, refilling from the
    // reader when necessary. Returns `None` at EOF.
    let mut read_byte = |buf: &mut Vec<u8>,
                         buf_pos: &mut usize,
                         buf_end: &mut usize|
     -> Result<Option<u8>, BiopacError> {
        if *buf_pos >= *buf_end {
            let n_read = reader.read(buf)?;
            if n_read == 0 {
                return Ok(None);
            }
            *buf_pos = 0;
            *buf_end = n_read;
        }
        let b = buf.get(*buf_pos).copied().ok_or_else(|| {
            BiopacError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "buffer underflow",
            ))
        })?;
        *buf_pos += 1;
        Ok(Some(b))
    };

    while let Some(&ch_idx) = pattern.get(pattern_idx) {
        let Some(&sample_type) = types.get(ch_idx) else {
            break;
        };

        // Check budget — if this channel is at its limit, we're done for all.
        // (Budget-based termination: stop when any channel with a budget reaches it.)
        let budget = budgets.get(ch_idx).copied().unwrap_or(0);
        let current_count = match sample_type {
            SampleType::I16 => raw_i16.get(ch_idx).map_or(0, Vec::len),
            SampleType::F64 => raw_f64.get(ch_idx).map_or(0, Vec::len),
        };
        if budget > 0 && current_count >= budget as usize {
            // This channel's budget is exhausted — stop reading.
            break;
        }

        // Read one sample's worth of bytes.
        let byte_size = sample_type.byte_size();
        let mut sample_bytes = [0u8; 8]; // max sample size is 8 (f64)
        let mut eof_hit = false;

        for i in 0..byte_size {
            if let Some(b) = read_byte(&mut buf, &mut buf_pos, &mut buf_end)? {
                if let Some(slot) = sample_bytes.get_mut(i) {
                    *slot = b;
                }
            } else {
                // EOF mid-sample or mid-pattern — stop cleanly.
                eof_hit = true;
                break;
            }
        }

        if eof_hit {
            break;
        }

        // Decode and store the sample.
        let sample_buf = sample_bytes
            .get(..byte_size)
            .ok_or_else(|| BiopacError::Validation("sample byte buffer overflow".into()))?;
        #[expect(
            clippy::indexing_slicing,
            reason = "ch_idx is always in-bounds: pattern entries come from 0..n; raw_i16/raw_f64 have n entries"
        )]
        match sample_type {
            SampleType::I16 => {
                let arr: [u8; 2] = [
                    *sample_buf.first().unwrap_or(&0),
                    *sample_buf.get(1).unwrap_or(&0),
                ];
                let v = match byte_order {
                    ByteOrder::LittleEndian => i16::from_le_bytes(arr),
                    ByteOrder::BigEndian => i16::from_be_bytes(arr),
                };
                raw_i16[ch_idx].push(v);
            }
            SampleType::F64 => {
                let arr: [u8; 8] = {
                    let mut a = [0u8; 8];
                    for (dst, src) in a.iter_mut().zip(sample_buf.iter()) {
                        *dst = *src;
                    }
                    a
                };
                let v = match byte_order {
                    ByteOrder::LittleEndian => f64::from_le_bytes(arr),
                    ByteOrder::BigEndian => f64::from_be_bytes(arr),
                };
                raw_f64[ch_idx].push(v);
            }
        }

        // Advance pattern.
        pattern_idx += 1;
        if pattern_idx >= pattern.len() {
            pattern_idx = 0;
        }
    }

    // Build Channel structs.
    let mut channels = Vec::with_capacity(n);
    for (j, m) in meta.iter().enumerate() {
        let sample_type = types.get(j).copied().unwrap_or(SampleType::I16);
        let data = build_channel_data(sample_type, j, &raw_i16, &raw_f64, m);
        let point_count = data.len();
        let ch_rate = base_rate / f64::from(m.frequency_divider.max(1));
        channels.push(Channel {
            name: m.name.clone(),
            units: m.units.clone(),
            samples_per_second: ch_rate,
            frequency_divider: m.frequency_divider,
            data,
            point_count,
        });
    }

    Ok((channels, headers.warnings.clone()))
}

#[expect(
    clippy::similar_names,
    reason = "names encode element type; shortening loses precision"
)]
fn build_channel_data(
    sample_type: SampleType,
    ch_idx: usize,
    raw_i16: &[Vec<i16>],
    raw_f64: &[Vec<f64>],
    meta: &ChannelMetadata,
) -> ChannelData {
    match sample_type {
        SampleType::I16 => {
            let raw = raw_i16.get(ch_idx).cloned().unwrap_or_default();
            let scale = meta.amplitude_scale;
            let offset = meta.amplitude_offset;
            // If scale == 1.0 and offset == 0.0, just store raw.
            let diff_scale = (scale - 1.0_f64).abs();
            let diff_offset = offset.abs();
            if diff_scale < f64::EPSILON && diff_offset < f64::EPSILON {
                ChannelData::Raw(raw)
            } else {
                ChannelData::Scaled { raw, scale, offset }
            }
        }
        SampleType::F64 => ChannelData::Float(raw_f64.get(ch_idx).cloned().unwrap_or_default()),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        domain::{ByteOrder, FileRevision, GraphMetadata},
        parser::headers::ParsedHeaders,
    };
    use alloc::{boxed::Box, vec, vec::Vec};
    use std::io::Cursor;

    fn make_headers(
        meta: Vec<ChannelMetadata>,
        types: Vec<SampleType>,
        byte_order: ByteOrder,
        data_start: u64,
    ) -> ParsedHeaders {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "test controls channel count; always fits u16"
        )]
        let channel_count = meta.len() as u16;
        ParsedHeaders {
            graph_metadata: GraphMetadata {
                file_revision: FileRevision::new(73),
                samples_per_second: 1000.0,
                channel_count,
                byte_order,
                compressed: false,
                title: None,
                acquisition_datetime: None,
                max_samples_per_second: None,
            },
            channel_metadata: meta,
            foreign_data: Vec::new(),
            sample_types: types,
            data_start_offset: data_start,
            warnings: Vec::new(),
        }
    }

    fn ch_meta(name: &str, div: u16, scale: f64, offset: f64, count: u32) -> ChannelMetadata {
        ChannelMetadata {
            name: alloc::string::String::from(name),
            units: alloc::string::String::from("mV"),
            description: alloc::string::String::new(),
            frequency_divider: div,
            amplitude_scale: scale,
            amplitude_offset: offset,
            display_order: 0,
            sample_count: count,
        }
    }

    // -----------------------------------------------------------------------
    // Unit: compute_sample_pattern
    // -----------------------------------------------------------------------

    #[test]
    fn pattern_single_channel() {
        let p = compute_sample_pattern(&[1]);
        assert_eq!(p, vec![0]);
    }

    #[test]
    fn pattern_two_channels_div_1_2() {
        // LCM(1,2)=2 → slots 0:[0,1], 1:[0] → [0,1,0]
        let p = compute_sample_pattern(&[1, 2]);
        assert_eq!(p, vec![0, 1, 0]);
    }

    #[test]
    fn pattern_three_channels_div_1_2_4() {
        // LCM(1,2,4)=4 →
        // slot 0: 0(÷1), 1(÷2), 2(÷4)
        // slot 1: 0(÷1)
        // slot 2: 0(÷1), 1(÷2)
        // slot 3: 0(÷1)
        // → [0,1,2, 0, 0,1, 0]
        let p = compute_sample_pattern(&[1, 2, 4]);
        assert_eq!(p, vec![0, 1, 2, 0, 0, 1, 0]);
    }

    #[test]
    fn pattern_empty_dividers() {
        let p = compute_sample_pattern(&[]);
        assert!(p.is_empty());
    }

    // -----------------------------------------------------------------------
    // Integration: two channels, same rate (div=1)
    // -----------------------------------------------------------------------

    #[test]
    fn two_channels_same_rate_i16_le() -> Result<(), Box<dyn std::error::Error>> {
        // ch0: [10, 20]  ch1: [30, 40]
        // pattern: [0, 1]
        // interleaved bytes (LE i16): 10,0, 30,0, 20,0, 40,0
        let mut bytes = Vec::<u8>::new();
        for v in [10i16, 30i16, 20i16, 40i16] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }

        let meta = vec![
            ch_meta("ch0", 1, 1.0, 0.0, 2),
            ch_meta("ch1", 1, 1.0, 0.0, 2),
        ];
        let types = vec![SampleType::I16, SampleType::I16];
        let headers = make_headers(meta, types, ByteOrder::LittleEndian, 0);

        let mut cur = Cursor::new(&bytes[..]);
        let (channels, _warns) = read_interleaved(&mut cur, &headers)?;
        assert_eq!(channels.len(), 2);

        let ch0 = channels.first().ok_or("missing ch0")?;
        let ch1 = channels.get(1).ok_or("missing ch1")?;
        assert_eq!(ch0.point_count, 2);
        assert_eq!(ch1.point_count, 2);
        assert_eq!(ch0.scaled_samples(), vec![10.0, 20.0]);
        assert_eq!(ch1.scaled_samples(), vec![30.0, 40.0]);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Integration: mixed rates ch0=div1, ch1=div2
    // -----------------------------------------------------------------------

    #[test]
    fn mixed_rate_channels_div_1_2() -> Result<(), Box<dyn std::error::Error>> {
        // ch0: 4 samples @ 1000Hz  ch1: 2 samples @ 500Hz
        // pattern [0,1,0] repeated twice:
        // ch0[0], ch1[0], ch0[1], ch0[2], ch1[1], ch0[3]
        let ch0_vals = [1i16, 2, 3, 4];
        let ch1_vals = [10i16, 20];
        let order = [
            ch0_vals[0],
            ch1_vals[0],
            ch0_vals[1],
            ch0_vals[2],
            ch1_vals[1],
            ch0_vals[3],
        ];
        let mut bytes = Vec::<u8>::new();
        for v in order {
            bytes.extend_from_slice(&v.to_le_bytes());
        }

        let meta = vec![
            ch_meta("ch0", 1, 1.0, 0.0, 4),
            ch_meta("ch1", 2, 1.0, 0.0, 2),
        ];
        let types = vec![SampleType::I16, SampleType::I16];
        let headers = make_headers(meta, types, ByteOrder::LittleEndian, 0);

        let mut cur = Cursor::new(&bytes[..]);
        let (channels, _) = read_interleaved(&mut cur, &headers)?;

        let ch0 = channels.first().ok_or("missing ch0")?;
        let ch1 = channels.get(1).ok_or("missing ch1")?;
        assert_eq!(ch0.point_count, 4);
        assert_eq!(ch1.point_count, 2);
        assert_eq!(ch0.scaled_samples(), vec![1.0, 2.0, 3.0, 4.0]);
        assert_eq!(ch1.scaled_samples(), vec![10.0, 20.0]);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Integration: EOF mid-pattern stops cleanly
    // -----------------------------------------------------------------------

    #[test]
    fn eof_mid_pattern_does_not_panic() -> Result<(), Box<dyn std::error::Error>> {
        // pattern for [1,2]: [0,1,0] — write only the first 3 samples (one full
        // cycle) then cut off before the second ch0 sample.
        let mut bytes = Vec::<u8>::new();
        bytes.extend_from_slice(&5i16.to_le_bytes()); // ch0[0]
        bytes.extend_from_slice(&9i16.to_le_bytes()); // ch1[0]
        bytes.extend_from_slice(&7i16.to_le_bytes()); // ch0[1]
        // EOF here — second cycle would be ch0[2], ch1[1], ch0[3] but truncated

        let meta = vec![
            ch_meta("ch0", 1, 1.0, 0.0, 0), // 0 = unbounded
            ch_meta("ch1", 2, 1.0, 0.0, 0),
        ];
        let types = vec![SampleType::I16, SampleType::I16];
        let headers = make_headers(meta, types, ByteOrder::LittleEndian, 0);

        let mut cur = Cursor::new(&bytes[..]);
        let (channels, _) = read_interleaved(&mut cur, &headers)?;
        let ch0 = channels.first().ok_or("missing ch0")?;
        let ch1 = channels.get(1).ok_or("missing ch1")?;
        assert_eq!(ch0.point_count, 2);
        assert_eq!(ch1.point_count, 1);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Unit: scale/offset applied
    // -----------------------------------------------------------------------

    #[test]
    fn scale_offset_stored_in_scaled_variant() -> Result<(), Box<dyn std::error::Error>> {
        let mut bytes = Vec::<u8>::new();
        bytes.extend_from_slice(&100i16.to_le_bytes());

        let meta = vec![ch_meta("sig", 1, 0.5, 2.0, 1)]; // scale=0.5, offset=2
        let types = vec![SampleType::I16];
        let headers = make_headers(meta, types, ByteOrder::LittleEndian, 0);

        let mut cur = Cursor::new(&bytes[..]);
        let (channels, _) = read_interleaved(&mut cur, &headers)?;
        let ch = channels.first().ok_or("missing channel")?;
        // physical = 100 * 0.5 + 2.0 = 52.0
        let samples = ch.scaled_samples();
        let first = samples.first().copied().ok_or("no samples")?;
        assert!((first - 52.0_f64).abs() < 1e-9);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Unit: f64 channel type
    // -----------------------------------------------------------------------

    #[test]
    fn f64_channel_read_correctly() -> Result<(), Box<dyn std::error::Error>> {
        let val: f64 = core::f64::consts::PI;
        let mut bytes = Vec::<u8>::new();
        bytes.extend_from_slice(&val.to_le_bytes());

        let meta = vec![ch_meta("sig", 1, 1.0, 0.0, 1)];
        let types = vec![SampleType::F64];
        let headers = make_headers(meta, types, ByteOrder::LittleEndian, 0);

        let mut cur = Cursor::new(&bytes[..]);
        let (channels, _) = read_interleaved(&mut cur, &headers)?;
        let ch = channels.first().ok_or("missing channel")?;
        let first = ch.scaled_samples().first().copied().ok_or("no samples")?;
        assert!((first - val).abs() < 1e-12);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Big-endian sample decoding
    // -----------------------------------------------------------------------

    #[test]
    fn big_endian_i16_decoded_correctly() -> Result<(), Box<dyn std::error::Error>> {
        let val: i16 = 0x0102; // BE bytes: [0x01, 0x02]
        let mut bytes = Vec::<u8>::new();
        bytes.extend_from_slice(&val.to_be_bytes());

        let meta = vec![ch_meta("sig", 1, 1.0, 0.0, 1)];
        let types = vec![SampleType::I16];
        let headers = make_headers(meta, types, ByteOrder::BigEndian, 0);

        let mut cur = Cursor::new(&bytes[..]);
        let (channels, _) = read_interleaved(&mut cur, &headers)?;
        let ch = channels.first().ok_or("missing channel")?;
        assert_eq!(ch.scaled_samples(), vec![f64::from(val)]);
        Ok(())
    }
}

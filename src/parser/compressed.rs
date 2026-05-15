//! Compressed channel reader — per-channel zlib decompression.
//!
//! Compressed .acq file layout:
//! ```text
//! Graph Header → Channel Headers → Foreign Data → Channel Dtypes
//! → Marker Header → Markers → Journal
//! → [Compressed Channel 0] → … → [Compressed Channel N-1]
//! ```
//!
//! This is the reverse of uncompressed files where data precedes markers.
//!
//! Each compressed channel blob starts with a compression header (Pre4 and
//! Post4 variants differ), then the raw zlib-compressed samples.
//!
//! Decompressed data is always little-endian regardless of the file's byte
//! order flag. Each channel is decompressed independently using [`flate2`].

use alloc::{string::ToString, vec, vec::Vec};
use std::io::{Read, Seek};

use flate2::Decompress;

use super::headers::{ParsedHeaders, SampleType};
use crate::{
    domain::{Channel, ChannelData},
    error::{BiopacError, CompressionError, Warning},
};

// ---------------------------------------------------------------------------
// Compression header sizes (bytes)
// ---------------------------------------------------------------------------

/// Bytes in the per-channel compression header for Pre-4 files.
///
/// Layout: `lUncompressedDataLen` (4) + `lCompressedDataLen` (4) = 8 bytes.
#[expect(dead_code, reason = "documents the binary layout; not used at runtime")]
const COMP_HDR_PRE4_LEN: usize = 8;

/// Extra bytes in the Post-4 compression header beyond the Pre-4 header.
///
/// The Post-4 format adds `lOffset` (4 bytes) for an additional 4 bytes
/// giving a total of 12 bytes.
const COMP_HDR_POST4_EXTRA: usize = 4;

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn read_i32_le<R: Read>(r: &mut R) -> Result<i32, BiopacError> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf).map_err(BiopacError::Io)?;
    Ok(i32::from_le_bytes(buf))
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Decompress all channels from a compressed `.acq` file.
///
/// The reader must be positioned immediately after the journal section — the
/// start of the compressed channel data area.  Returns a `Vec<Channel>` in
/// channel order and any non-fatal [`Warning`]s.
pub(crate) fn read_compressed<R: Read + Seek>(
    reader: &mut R,
    headers: &ParsedHeaders,
) -> Result<(Vec<Channel>, Vec<Warning>), BiopacError> {
    let mut warnings: Vec<Warning> = Vec::new();
    let channel_count = headers.channel_metadata.len();
    let is_pre4 = headers.graph_metadata.file_revision.is_pre_v4();

    // For Post-4 files we read all compression headers first (they contain
    // absolute offsets), then seek to each channel's data individually.
    // For Pre-4 files the compressed blobs follow sequentially with no offset
    // field so we read them in order.

    let mut channels: Vec<Channel> = Vec::with_capacity(channel_count);

    for ch_idx in 0..channel_count {
        let meta = headers
            .channel_metadata
            .get(ch_idx)
            .ok_or_else(|| BiopacError::Validation("channel index out of range".to_string()))?;
        let sample_type =
            headers.sample_types.get(ch_idx).copied().ok_or_else(|| {
                BiopacError::Validation("sample type index out of range".to_string())
            })?;

        // --- Read compression header ---
        let uncompressed_len = read_i32_le(reader)?;
        let compressed_len = read_i32_le(reader)?;

        if !is_pre4 {
            // Post-4 has an extra `lOffset` field we don't need; skip it.
            let mut skip = [0u8; COMP_HDR_POST4_EXTRA];
            reader.read_exact(&mut skip).map_err(BiopacError::Io)?;
        }

        // Validate lengths
        if compressed_len < 0 || uncompressed_len < 0 {
            return Err(BiopacError::Compression(CompressionError {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "ch_idx is bounded by channel_count which fits u16 in valid .acq files"
                )]
                channel_index: ch_idx as u16,
                message: alloc::format!(
                    "negative length in compression header: \
                     uncompressed={uncompressed_len}, compressed={compressed_len}"
                ),
            }));
        }

        #[expect(clippy::cast_sign_loss, reason = "validated non-negative above")]
        let compressed_bytes = compressed_len as usize;
        #[expect(clippy::cast_sign_loss, reason = "validated non-negative above")]
        let uncompressed_bytes = uncompressed_len as usize;

        // --- Read compressed data ---
        let mut compressed_buf = vec![0u8; compressed_bytes];
        reader
            .read_exact(&mut compressed_buf)
            .map_err(BiopacError::Io)?;

        // --- Decompress ---
        let raw_bytes =
            decompress_channel(ch_idx, &compressed_buf, uncompressed_bytes, &mut warnings)?;

        // --- Decode samples (always little-endian) ---
        let channel_data = decode_samples(ch_idx, sample_type, &raw_bytes, meta, &mut warnings);

        let point_count = channel_data.len();
        channels.push(Channel {
            name: meta.name.clone(),
            units: meta.units.clone(),
            samples_per_second: headers.graph_metadata.samples_per_second
                / f64::from(meta.frequency_divider),
            frequency_divider: meta.frequency_divider,
            data: channel_data,
            point_count,
        });
    }

    Ok((channels, warnings))
}

// ---------------------------------------------------------------------------
// Decompression
// ---------------------------------------------------------------------------

fn decompress_channel(
    ch_idx: usize,
    compressed: &[u8],
    expected_uncompressed: usize,
    warnings: &mut Vec<Warning>,
) -> Result<Vec<u8>, BiopacError> {
    let mut out = vec![0u8; expected_uncompressed];
    let mut decompressor = Decompress::new(true);

    match decompressor.decompress(compressed, &mut out, flate2::FlushDecompress::Finish) {
        Ok(flate2::Status::StreamEnd | flate2::Status::Ok) => {}
        Ok(flate2::Status::BufError) => {
            // BufError means the output buffer was too small or input was
            // exhausted without reaching stream end.  This is non-fatal for
            // files that were written with an incorrect uncompressed length;
            // we keep whatever bytes were produced.
            warnings.push(Warning::new(alloc::format!(
                "channel {ch_idx}: decompression BufError \
                 (expected {expected_uncompressed} bytes)"
            )));
        }
        Err(e) => {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "ch_idx bounded by channel_count which fits u16"
            )]
            return Err(BiopacError::Compression(CompressionError {
                channel_index: ch_idx as u16,
                message: e.to_string(),
            }));
        }
    }

    // Truncate to bytes actually written.
    let produced = decompressor.total_out();
    #[expect(
        clippy::cast_possible_truncation,
        reason = "total_out is u64 but realistic decompressed sizes fit usize on all supported targets"
    )]
    let produced = produced as usize;
    out.truncate(produced);

    Ok(out)
}

// ---------------------------------------------------------------------------
// Sample decoding
// ---------------------------------------------------------------------------

fn decode_samples(
    ch_idx: usize,
    sample_type: SampleType,
    raw: &[u8],
    meta: &crate::domain::ChannelMetadata,
    warnings: &mut Vec<Warning>,
) -> ChannelData {
    match sample_type {
        SampleType::I16 => {
            let byte_size = core::mem::size_of::<i16>();
            if !raw.len().is_multiple_of(byte_size) {
                warnings.push(Warning::new(alloc::format!(
                    "channel {ch_idx}: decompressed byte count {} \
                     is not a multiple of i16 size; truncating",
                    raw.len()
                )));
            }
            let sample_count = raw.len() / byte_size;
            let mut samples: Vec<i16> = Vec::with_capacity(sample_count);
            let chunks = raw.chunks_exact(byte_size);
            for chunk in chunks {
                let arr: [u8; 2] = [*chunk.first().unwrap_or(&0), *chunk.get(1).unwrap_or(&0)];
                samples.push(i16::from_le_bytes(arr));
            }
            ChannelData::Scaled {
                raw: samples,
                scale: meta.amplitude_scale,
                offset: meta.amplitude_offset,
            }
        }
        SampleType::F64 => {
            let byte_size = core::mem::size_of::<f64>();
            if !raw.len().is_multiple_of(byte_size) {
                warnings.push(Warning::new(alloc::format!(
                    "channel {ch_idx}: decompressed byte count {} \
                     is not a multiple of f64 size; truncating",
                    raw.len()
                )));
            }
            let sample_count = raw.len() / byte_size;
            let mut samples: Vec<f64> = Vec::with_capacity(sample_count);
            let chunks = raw.chunks_exact(byte_size);
            for chunk in chunks {
                // SAFETY: we know chunk.len() == 8 from chunks_exact
                let arr: [u8; 8] = [
                    *chunk.first().unwrap_or(&0),
                    *chunk.get(1).unwrap_or(&0),
                    *chunk.get(2).unwrap_or(&0),
                    *chunk.get(3).unwrap_or(&0),
                    *chunk.get(4).unwrap_or(&0),
                    *chunk.get(5).unwrap_or(&0),
                    *chunk.get(6).unwrap_or(&0),
                    *chunk.get(7).unwrap_or(&0),
                ];
                samples.push(f64::from_le_bytes(arr));
            }
            ChannelData::Float(samples)
        }
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
        parser::headers::{ParsedHeaders, SampleType},
    };
    use alloc::{boxed::Box, string::String, vec};
    use std::io::Cursor;

    fn make_headers(
        is_pre4: bool,
        sample_types: Vec<SampleType>,
        channel_count: usize,
    ) -> ParsedHeaders {
        let revision = if is_pre4 { 38 } else { 73 };
        let meta: Vec<crate::domain::ChannelMetadata> = (0..channel_count)
            .map(|i| crate::domain::ChannelMetadata {
                name: alloc::format!("Ch{i}"),
                units: String::from("mV"),
                description: String::new(),
                frequency_divider: 1,
                amplitude_scale: 1.0,
                amplitude_offset: 0.0,
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "test: channel index bounded by channel_count parameter"
                )]
                display_order: i as u16,
                sample_count: 0,
            })
            .collect();
        ParsedHeaders {
            graph_metadata: GraphMetadata {
                file_revision: FileRevision::new(revision),
                samples_per_second: 1000.0,
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "test: channel_count controlled by caller"
                )]
                channel_count: channel_count as u16,
                byte_order: ByteOrder::LittleEndian,
                compressed: true,
                title: None,
                acquisition_datetime: None,
                max_samples_per_second: None,
            },
            channel_metadata: meta,
            foreign_data: vec![],
            sample_types,
            data_start_offset: 0,
            warnings: vec![],
        }
    }

    #[expect(
        clippy::expect_used,
        reason = "test helper: compress on known-valid data cannot fail"
    )]
    fn compress_data(data: &[u8]) -> Vec<u8> {
        use flate2::{Compress, FlushCompress};
        let mut c = Compress::new(flate2::Compression::default(), true);
        let mut out = vec![0u8; data.len() * 2 + 64];
        c.compress(data, &mut out, FlushCompress::Finish)
            .expect("compress");
        #[expect(
            clippy::cast_possible_truncation,
            reason = "test: compressed output fits usize on all supported targets"
        )]
        let n = c.total_out() as usize;
        out.truncate(n);
        out
    }

    fn build_pre4_blob(uncompressed: &[u8]) -> Vec<u8> {
        let compressed = compress_data(uncompressed);
        let mut blob = Vec::new();
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_possible_wrap,
            reason = "test: data sizes fit i32"
        )]
        blob.extend_from_slice(&(uncompressed.len() as i32).to_le_bytes());
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_possible_wrap,
            reason = "test: data sizes fit i32"
        )]
        blob.extend_from_slice(&(compressed.len() as i32).to_le_bytes());
        blob.extend_from_slice(&compressed);
        blob
    }

    fn build_post4_blob(uncompressed: &[u8]) -> Vec<u8> {
        let compressed = compress_data(uncompressed);
        let mut blob = Vec::new();
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_possible_wrap,
            reason = "test: data sizes fit i32"
        )]
        blob.extend_from_slice(&(uncompressed.len() as i32).to_le_bytes());
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_possible_wrap,
            reason = "test: data sizes fit i32"
        )]
        blob.extend_from_slice(&(compressed.len() as i32).to_le_bytes());
        blob.extend_from_slice(&0i32.to_le_bytes()); // lOffset placeholder
        blob.extend_from_slice(&compressed);
        blob
    }

    #[test]
    fn pre4_i16_single_channel() -> Result<(), Box<dyn std::error::Error>> {
        let samples: Vec<i16> = vec![100, 200, -100, 0];
        let raw_bytes: Vec<u8> = samples.iter().flat_map(|&s| s.to_le_bytes()).collect();
        let blob = build_pre4_blob(&raw_bytes);

        let headers = make_headers(true, vec![SampleType::I16], 1);
        let mut cursor = Cursor::new(blob);
        let (channels, warnings) = read_compressed(&mut cursor, &headers)?;

        assert_eq!(warnings.len(), 0);
        assert_eq!(channels.len(), 1);
        let Some(ch0) = channels.first() else {
            return Err("no channel returned".into());
        };
        let ChannelData::Scaled { raw, scale, offset } = &ch0.data else {
            return Err("expected ChannelData::Scaled".into());
        };
        assert_eq!(raw, &samples);
        assert!((scale - 1.0).abs() < f64::EPSILON);
        assert!((offset - 0.0).abs() < f64::EPSILON);
        assert_eq!(ch0.point_count, 4);
        Ok(())
    }

    #[test]
    fn post4_f64_single_channel() -> Result<(), Box<dyn std::error::Error>> {
        let samples: Vec<f64> = vec![1.0, 2.5, -3.2];
        let raw_bytes: Vec<u8> = samples.iter().flat_map(|&s| s.to_le_bytes()).collect();
        let blob = build_post4_blob(&raw_bytes);

        let headers = make_headers(false, vec![SampleType::F64], 1);
        let mut cursor = Cursor::new(blob);
        let (channels, warnings) = read_compressed(&mut cursor, &headers)?;

        assert_eq!(warnings.len(), 0);
        assert_eq!(channels.len(), 1);
        let Some(ch0) = channels.first() else {
            return Err("no channel returned".into());
        };
        let ChannelData::Float(floats) = &ch0.data else {
            return Err("expected ChannelData::Float".into());
        };
        assert_eq!(floats.len(), 3);
        for (a, b) in floats.iter().zip(samples.iter()) {
            assert!((a - b).abs() < 1e-10);
        }
        Ok(())
    }

    #[test]
    fn two_channels_mixed_dtypes() -> Result<(), Box<dyn std::error::Error>> {
        let i16_samples: Vec<i16> = vec![10, 20, 30];
        let f64_samples: Vec<f64> = vec![1.1, 2.2];

        let i16_bytes: Vec<u8> = i16_samples.iter().flat_map(|&s| s.to_le_bytes()).collect();
        let f64_bytes: Vec<u8> = f64_samples.iter().flat_map(|&s| s.to_le_bytes()).collect();

        let mut data = build_post4_blob(&i16_bytes);
        data.extend_from_slice(&build_post4_blob(&f64_bytes));

        let headers = make_headers(false, vec![SampleType::I16, SampleType::F64], 2);
        let mut cursor = Cursor::new(data);
        let (channels, warnings) = read_compressed(&mut cursor, &headers)?;

        assert_eq!(warnings.len(), 0);
        assert_eq!(channels.len(), 2);

        // Ch0 — i16
        let Some(ch0) = channels.first() else {
            return Err("expected channel 0".into());
        };
        assert_eq!(ch0.point_count, 3);
        let ChannelData::Scaled { raw, .. } = &ch0.data else {
            return Err("ch0: expected Scaled".into());
        };
        assert_eq!(raw, &i16_samples);

        // Ch1 — f64
        let Some(ch1) = channels.get(1) else {
            return Err("expected channel 1".into());
        };
        assert_eq!(ch1.point_count, 2);
        let ChannelData::Float(f) = &ch1.data else {
            return Err("ch1: expected Float".into());
        };
        assert_eq!(f.len(), 2);

        Ok(())
    }

    #[test]
    #[expect(
        clippy::panic,
        reason = "test: unreachable given the preceding is_err() assertion"
    )]
    fn decompression_error_returns_compression_error() {
        // Feed garbage bytes as compressed data.
        let bad_data = vec![0xFFu8; 16];
        let mut blob = Vec::new();
        blob.extend_from_slice(&16i32.to_le_bytes()); // uncompressed len
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_possible_wrap,
            reason = "test: bad_data is 16 bytes"
        )]
        blob.extend_from_slice(&(bad_data.len() as i32).to_le_bytes()); // compressed len
        blob.extend_from_slice(&0i32.to_le_bytes()); // lOffset (post4)
        blob.extend_from_slice(&bad_data);

        let headers = make_headers(false, vec![SampleType::I16], 1);
        let mut cursor = Cursor::new(blob);
        let result = read_compressed(&mut cursor, &headers);

        assert!(result.is_err());
        let Err(BiopacError::Compression(e)) = result else {
            panic!("expected CompressionError");
        };
        assert_eq!(e.channel_index, 0);
    }

    #[test]
    fn scale_offset_applied_in_channel_data() -> Result<(), Box<dyn std::error::Error>> {
        let samples: Vec<i16> = vec![100, 200];
        let raw_bytes: Vec<u8> = samples.iter().flat_map(|&s| s.to_le_bytes()).collect();
        let blob = build_post4_blob(&raw_bytes);

        // Use non-trivial scale/offset
        let revision = 73;
        let meta = vec![crate::domain::ChannelMetadata {
            name: String::from("EMG"),
            units: String::from("mV"),
            description: String::new(),
            frequency_divider: 1,
            amplitude_scale: 0.5,
            amplitude_offset: 1.0,
            display_order: 0,
            sample_count: 0,
        }];
        let headers = ParsedHeaders {
            graph_metadata: GraphMetadata {
                file_revision: FileRevision::new(revision),
                samples_per_second: 1000.0,
                channel_count: 1,
                byte_order: ByteOrder::LittleEndian,
                compressed: true,
                title: None,
                acquisition_datetime: None,
                max_samples_per_second: None,
            },
            channel_metadata: meta,
            foreign_data: vec![],
            sample_types: vec![SampleType::I16],
            data_start_offset: 0,
            warnings: vec![],
        };

        let mut cursor = Cursor::new(blob);
        let (channels, _) = read_compressed(&mut cursor, &headers)?;
        let Some(ch0) = channels.first() else {
            return Err("no channel returned".into());
        };
        let ChannelData::Scaled { raw, scale, offset } = &ch0.data else {
            return Err("expected Scaled".into());
        };
        assert_eq!(raw, &samples);
        assert!((scale - 0.5).abs() < f64::EPSILON);
        assert!((offset - 1.0).abs() < f64::EPSILON);
        Ok(())
    }
}

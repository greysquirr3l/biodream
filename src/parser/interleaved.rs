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

// TODO(T05): implement SamplePattern, InterleavedReader<R: Read + Seek>.

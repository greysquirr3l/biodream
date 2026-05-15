//! Apache Arrow IPC export (requires `arrow` feature).
//!
//! Produces a RecordBatch with one column per channel, typed as Float64 or
//! Int16 depending on the channel's ChannelData variant.

// TODO(T11): implement ArrowExporter → RecordBatch / IPC stream.

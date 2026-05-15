//! Pure domain types for biodream.
//!
//! These types carry the semantics of physiological recordings — channels with
//! samples, markers with timestamps, metadata headers — with zero knowledge of
//! binary layout or I/O. All types are `no_std` compatible with `alloc`.

mod channel;
mod datafile;
mod journal;
mod marker;
mod metadata;

pub use channel::{Channel, ChannelData, ChannelMetadata};
pub use datafile::Datafile;
pub use journal::Journal;
pub use marker::{Marker, MarkerStyle, Timestamp};
pub use metadata::{AcquisitionDateTime, ByteOrder, FileRevision, GraphMetadata};

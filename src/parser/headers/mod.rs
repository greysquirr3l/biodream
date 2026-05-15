//! Raw `binrw` header structs — binary layout knowledge lives here.
//!
//! Nothing outside this module should know about byte offsets or field names
//! from the .acq format. Domain types are produced by `From`/`TryFrom` impls
//! at the end of each submodule.
//!
//! # Sub-modules
//!
//! - [`graph`] — graph (file) header, Pre4 and Post4 variants (T04)
//! - [`channel`] — per-channel header, Pre4 and Post4 variants (T04)
//! - [`foreign`] — opaque foreign-data blob (T04)
//! - [`dtype`] — channel data-type descriptors (T04)

// TODO(T04): implement header structs
pub mod channel;
pub mod dtype;
pub mod foreign;
pub mod graph;

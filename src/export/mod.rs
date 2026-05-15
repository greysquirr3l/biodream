//! Export targets for biodream recordings.
//!
//! Each submodule is gated behind its corresponding feature flag.

/// CSV export (requires `csv` feature).
#[cfg(feature = "csv")]
pub mod csv;

/// Apache Arrow IPC export (requires `arrow` feature).
#[cfg(any(feature = "arrow", feature = "parquet"))]
pub mod arrow;

/// Parquet export (requires `parquet` feature).
#[cfg(feature = "parquet")]
pub mod parquet;

/// HDF5 export (requires `hdf5` feature and libhdf5-dev system library).
#[cfg(feature = "hdf5")]
pub mod hdf5;

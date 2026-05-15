//! Write support for .acq files (requires `write` feature).
//!
//! Implements round-trip fidelity: a file read with [`crate::parser::reader`]
//! and written back with this module must produce a bitwise-identical output.

// TODO(T13): implement AcqWriter with round-trip fidelity.

//! GRIB2 encoding.
//!
//! A hand-written encoder that builds each message from a template of fixed
//! section bytes, patching only what varies (spec.md 12.2). Implemented in M4.
//!
//! The corresponding *decoder* exists only under `#[cfg(test)]`, so the shipped
//! binary never depends on an external decoder and invariant 5 holds.

pub mod error;
pub mod packing;
#[cfg(feature = "testing")]
pub mod reader;
pub mod writer;

pub use error::{GribError, Result};
pub use writer::{GridSpec, MessageSpec, Parameter, ReferenceTime};

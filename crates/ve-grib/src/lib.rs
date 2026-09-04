//! GRIB2 encoding, and decoding for import.
//!
//! A hand-written encoder that builds each message from a template of fixed
//! section bytes, patching only what varies (spec.md 12.2). Implemented in M4.
//!
//! `decode` reads real forecast files for the import feature (spec.md 4.8) and
//! `import` turns what it reads into the rasters a layer carries. Both are
//! pure Rust: the shipped binary still depends on no external decoder and
//! invariant 5 holds. `reader` is the older, minimal verifier for the writer's
//! own output and stays test-only.

pub mod decode;
pub mod error;
pub mod icon;
pub mod import;
pub mod packing;
#[cfg(feature = "testing")]
pub mod reader;
pub mod writer;

pub use error::{GribError, Result};
pub use writer::{GridSpec, MessageSpec, Parameter, ReferenceTime};

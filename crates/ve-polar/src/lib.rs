//! Boat polars and sailboat route solving.
//!
//! Implemented in M9. The interesting part is the inverse problem: given a
//! required course and speed, find the wind that makes it achievable
//! (spec.md 11.4). It is under-determined, so the selection strategy is
//! explicit and named rather than implicit.

pub mod error;

pub use error::{PolarError, Result};

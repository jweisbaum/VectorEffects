//! Map data that is looked at and never computed with.
//!
//! An electronic chart under the field, a coastline survey from a shapefile:
//! neither makes wind, neither reaches a scene, a render-cache key or an
//! exported file (spec.md 4.11). They are read here into one small feature
//! model and painted here into RGBA tiles of the application's own pyramid,
//! which the map draws as plain textures beneath everything else — so a chart
//! follows every projection the field does without knowing any of them.
//!
//! Nothing in this crate reaches the network or links a C library.

pub mod error;
pub mod geometry;
pub mod gis;
pub mod paint;
pub mod s57;

pub use error::{ChartError, Result};
pub use geometry::{Bounds, Feature, Geometry, Point, Value};
pub use gis::{Vectors, read as read_gis};
pub use s57::{Cell, read_cell};

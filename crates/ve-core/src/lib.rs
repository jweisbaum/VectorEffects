//! Core types, conventions, and geodesy for VectorEffects.
//!
//! This crate depends on nothing else in the workspace and is the single home
//! for the conventions listed in `CLAUDE.md`. Getting those wrong produces
//! silently incorrect GRIB output rather than a crash, so they are defined once,
//! here, and tested against independent reference values.

pub mod angle;
pub mod annotation;
pub mod canonical;
pub mod capture;
pub mod clipboard;
pub mod colour;
pub mod command;
pub mod document;
pub mod error;
pub mod follow;
pub mod geo;
pub mod history;
pub mod id;
pub mod io;
pub mod keyframe;
pub mod project;
pub mod raster;
pub mod regrid;
pub mod schema;
pub mod units;
pub mod value;
pub mod vector;

pub use angle::Angle;
pub use command::Command;
pub use document::{Geometry, Layer, LocalPoint, Object, StepRange};
pub use error::{CoreError, Result};
pub use geo::{EARTH_RADIUS_M, LonLat};
pub use history::History;
pub use id::Id;
pub use keyframe::{Animatable, Keyframe};
pub use project::{FieldKind, Project, ProjectSettings, Resolution, StepHours};
pub use schema::{PropId, PropSpec, PropertyMap, ToolKind};
pub use value::{Interpolation, PropKind, PropValue};

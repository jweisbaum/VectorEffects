//! Error taxonomy for `ve-core`.
//!
//! Library crates use `thiserror` and return typed errors; only `ve-app`
//! collapses them into an application-level error at the IPC boundary.

use thiserror::Error;

/// Errors produced by core model operations.
#[derive(Debug, Error)]
pub enum CoreError {
    /// A latitude fell outside [-90, 90].
    #[error("latitude {0} is out of range [-90, 90]")]
    LatitudeOutOfRange(f64),

    /// A coordinate component was NaN or infinite.
    #[error("coordinate component was not finite: {0}")]
    NonFiniteCoordinate(&'static str),

    /// A project file declared a schema version this build cannot read.
    #[error("project schema version {found} is newer than supported version {supported}")]
    SchemaTooNew { found: u32, supported: u32 },

    /// A project declared zero steps, or more than the maximum.
    #[error("step count {0} is out of range 1..=240")]
    InvalidStepCount(u32),

    /// An object's active range extends past the project's last step.
    #[error("object '{object}' ends at step {end}, past the last step {last}")]
    RangeOutOfBounds {
        /// The offending object's name.
        object: String,
        /// The range's end step.
        end: u32,
        /// The project's last valid step.
        last: u32,
    },

    /// Reading or writing a project file failed.
    #[error("project file i/o failed: {0}")]
    Io(#[from] std::io::Error),

    /// The project JSON could not be parsed or written.
    #[error("project json is malformed: {0}")]
    Json(#[from] serde_json::Error),

    /// The `.veproj` container was not a readable archive.
    #[error("project archive is malformed: {0}")]
    Archive(String),

    /// A migration could not upgrade a document.
    #[error("could not migrate project from schema version {from}: {reason}")]
    Migration {
        /// The version being migrated from.
        from: u32,
        /// What went wrong.
        reason: String,
    },

    /// A command referenced a layer that is not in the document.
    #[error("no layer with id #{0}")]
    MissingLayer(u64),

    /// A command referenced an object that is not in the document.
    #[error("no object with id #{0}")]
    MissingObject(u64),

    /// A reorder referenced a position outside the collection.
    #[error("index {index} is out of bounds for length {len}")]
    IndexOutOfBounds {
        /// The offending index.
        index: usize,
        /// The collection's length.
        len: usize,
    },

    /// A write was attempted while the document was locked (spec.md 8.7).
    #[error("the document cannot be changed while a macro capture is running")]
    Locked,

    /// A captured field, or its container, was not readable (spec.md 8.5).
    #[error("captured field is malformed: {0}")]
    Capture(String),

    /// A property held a value that cannot be written to JSON.
    #[error("object '{object}' property {property} holds a non-finite value")]
    NonFiniteProperty {
        /// The offending object's name.
        object: String,
        /// The property identifier.
        property: String,
    },
}

/// Convenience alias for results in this crate.
pub type Result<T> = std::result::Result<T, CoreError>;

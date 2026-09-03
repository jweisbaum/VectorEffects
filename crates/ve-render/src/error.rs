//! Error taxonomy for `ve-render`.

use thiserror::Error;

/// Errors produced while evaluating or caching a field.
#[derive(Debug, Error)]
pub enum RenderError {
    /// No GPU adapter met the requirements; the CPU evaluator should be used.
    #[error("no suitable GPU adapter: {0}")]
    NoAdapter(String),

    /// The render cache could not be read or written.
    #[error("render cache i/o failed: {0}")]
    Cache(#[from] std::io::Error),

    /// A bundled asset was missing, truncated, or in an unexpected format.
    #[error("bundled asset is invalid: {0}")]
    Asset(String),

    /// A tile was requested that does not exist in the pyramid.
    #[error("invalid tile address z={z} x={x} y={y}")]
    BadTile {
        /// Zoom level.
        z: u32,
        /// Column.
        x: u32,
        /// Row.
        y: u32,
    },

    /// The backend cannot render this scene and the caller should fall back.
    #[error("unsupported by this backend: {0}")]
    Unsupported(String),

    /// A scene referenced geometry that could not be flattened.
    #[error("scene flattening failed: {0}")]
    Flatten(String),
}

/// Convenience alias for results in this crate.
pub type Result<T> = std::result::Result<T, RenderError>;

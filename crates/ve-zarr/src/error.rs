//! Error taxonomy for `ve-zarr`.

use thiserror::Error;

/// Errors produced while reading one of the history archives.
#[derive(Debug, Error)]
pub enum ZarrError {
    /// The store URL could not be parsed or the store could not be opened.
    #[error("could not open the archive: {0}")]
    Open(String),

    /// A read against the store failed.
    #[error("reading {what} failed: {source}")]
    Read {
        /// What was being read, for the message.
        what: String,
        /// The underlying zarrs error.
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// A blosc chunk could not be decoded.
    #[error("blosc decode failed: {0}")]
    Blosc(String),

    /// The store is not shaped the way this crate requires.
    #[error("unexpected archive layout: {0}")]
    Layout(String),

    /// The requested time range does not lie inside the dataset.
    #[error("{0}")]
    TimeRange(String),

    /// The import was cancelled by the user.
    #[error("cancelled")]
    Cancelled,
}

/// Convenience alias for results in this crate.
pub type Result<T> = std::result::Result<T, ZarrError>;

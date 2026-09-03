//! Error taxonomy for `ve-grib`.

use thiserror::Error;

/// Errors produced while encoding GRIB2 output.
#[derive(Debug, Error)]
pub enum GribError {
    /// Writing the output file failed.
    #[error("grib output i/o failed: {0}")]
    Io(#[from] std::io::Error),

    /// A field contained a non-finite value, which cannot be packed.
    #[error("field contained a non-finite value at grid index {0}")]
    NonFiniteValue(usize),

    /// The requested grid does not correspond to a supported template.
    #[error("unsupported grid: {0}")]
    UnsupportedGrid(String),
}

/// Convenience alias for results in this crate.
pub type Result<T> = std::result::Result<T, GribError>;

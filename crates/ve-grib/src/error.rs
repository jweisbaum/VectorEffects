//! Error taxonomy for `ve-grib`.

use thiserror::Error;

/// Errors produced while encoding GRIB2 output.
#[derive(Debug, Error)]
pub enum GribError {
    /// Writing the output file failed.
    #[error("the GRIB file could not be read or written: {0}")]
    Io(#[from] std::io::Error),

    /// A field contained a non-finite value, which cannot be packed.
    #[error(
        "the field held a value that is not a number, at grid point {0}; a NaN or an infinity cannot be encoded"
    )]
    NonFiniteValue(usize),

    /// The requested grid does not correspond to a supported template.
    #[error("unsupported grid: {0}")]
    UnsupportedGrid(String),

    /// An input file is not well-formed GRIB2.
    #[error("this is not a GRIB2 file this build can read: {0}")]
    Malformed(String),

    /// An input file uses a feature the decoder does not implement.
    #[error("unsupported GRIB2 feature: {0}")]
    Unsupported(String),

    /// An input file holds no wind or current field that could be imported.
    #[error("no wind or current field was found to import: {0}")]
    NoVectorField(String),
}

/// Convenience alias for results in this crate.
pub type Result<T> = std::result::Result<T, GribError>;

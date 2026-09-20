//! What can go wrong reading a chart or a GIS file.

/// An error reading display data.
#[derive(Debug, thiserror::Error)]
pub enum ChartError {
    /// The file could not be read at all.
    #[error("{0}")]
    Io(#[from] std::io::Error),
    /// The file is not what its name says, or is damaged.
    #[error("{what} is malformed: {why}")]
    Malformed {
        /// Which part of which format.
        what: &'static str,
        /// What was found.
        why: String,
    },
    /// The file is well formed and says something this crate does not handle.
    #[error("{0}")]
    Unsupported(String),
}

impl ChartError {
    /// A malformed-file error, for the many places that find one.
    pub fn malformed(what: &'static str, why: impl Into<String>) -> Self {
        Self::Malformed {
            what,
            why: why.into(),
        }
    }
}

/// The crate's result type.
pub type Result<T> = std::result::Result<T, ChartError>;

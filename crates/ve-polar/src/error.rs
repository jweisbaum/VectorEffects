//! Error taxonomy for `ve-polar`.

use thiserror::Error;

/// Errors produced while parsing polars or solving routes.
#[derive(Debug, Error)]
pub enum PolarError {
    /// The polar file could not be read.
    #[error("polar i/o failed: {0}")]
    Io(#[from] std::io::Error),

    /// The polar file was not in a recognised layout.
    #[error("could not parse polar: {0}")]
    Parse(String),

    /// No wind within the cap makes this leg achievable.
    #[error("leg requires {required_kt:.1} kt but the polar tops out at {best_kt:.1} kt")]
    LegInfeasible { required_kt: f64, best_kt: f64 },
}

/// Convenience alias for results in this crate.
pub type Result<T> = std::result::Result<T, PolarError>;

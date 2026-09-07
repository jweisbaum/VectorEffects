//! Field evaluation and rendering.
//!
//! The engine is a pure function of a flattened scene: one evaluation semantic,
//! two backends (spec.md 7.8). The trait seam is defined here from the start so
//! the GPU and CPU implementations added in M3 cannot drift apart structurally,
//! only numerically -- and the parity test covers the numeric case.

pub mod aeqd;
pub mod basemap;
pub mod cache;
pub mod cpu;
pub mod cull;
pub mod error;
pub mod evaluator;
pub mod gpu;
pub mod preview;
pub mod scene;
pub mod sdf;
pub mod synthetic;
pub mod tile;

pub use cpu::CpuEvaluator;
pub use error::{RenderError, Result};
pub use evaluator::{FieldEvaluator, SamplePoint};
pub use scene::Scene;
pub use tile::{TILE_SIZE, TileId};

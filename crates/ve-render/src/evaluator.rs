//! The evaluation seam shared by every backend.

use ve_core::LonLat;
use ve_core::vector::Uv;

use crate::error::Result;
use crate::scene::Scene;

/// A point at which the field is sampled.
///
/// The same type serves both callers: preview passes tile pixel centres,
/// export passes every global grid cell. There is no second code path for
/// export — the view is a proxy, never a source (spec.md, invariant 3).
pub type SamplePoint = LonLat;

/// Evaluates a scene at a set of sample points.
///
/// Backends must agree *perceptually*, not numerically: speed within the
/// greater of 0.25 m/s and 2%, direction within 2° (spec.md 7.9). The preview
/// is therefore free to approximate; the export is not, and always uses the CPU
/// evaluator.
pub trait FieldEvaluator: Send + Sync + std::fmt::Debug {
    /// Human-readable backend name, surfaced in About and in logs.
    fn backend_name(&self) -> &'static str;

    /// Evaluates `scene` at each point, returning one vector each.
    fn evaluate(&self, scene: &Scene, points: &[SamplePoint]) -> Result<Vec<Uv>>;
}

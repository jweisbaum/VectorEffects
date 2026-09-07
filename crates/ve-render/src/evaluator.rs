//! The evaluation seam shared by every backend.

use ve_core::LonLat;
use ve_core::project::FieldKind;
use ve_core::vector::Uv;

use crate::error::Result;
use crate::scene::Scene;

/// A point at which the field is sampled.
///
/// The same type serves both callers: preview passes tile pixel centres,
/// export passes every global grid cell. There is no second code path for
/// export — the view is a proxy, never a source (spec.md, invariant 3).
pub type SamplePoint = LonLat;

/// What a cell composites to (M31, spec.md 7.6).
///
/// The field, how much of the cell was written — one where a layer covers
/// it, zero where nothing does, in between across a feathered edge — and the
/// kind of the layer that wrote it. Zero and undefined are different things
/// (D58): a calm cell some layer wrote has a coverage of one and is drawn
/// opaque; a cell nothing wrote has none and shows the map beneath.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    /// The field, `(u, v)` in m/s. Calm where nothing wrote.
    pub uv: Uv,
    /// How much of the cell was written, 0 to 1.
    pub coverage: f32,
    /// The kind of the layer the cell shows: the topmost that covers at
    /// least half of it, or the topmost that touches it at all. Wind where
    /// nothing wrote.
    pub kind: FieldKind,
}

impl Default for Sample {
    fn default() -> Self {
        Self {
            uv: Uv::default(),
            coverage: 0.0,
            kind: FieldKind::Wind,
        }
    }
}

/// Evaluates a scene at a set of sample points.
///
/// Backends must agree *perceptually*, not numerically: speed within the
/// greater of 0.25 m/s and 2%, direction within 2° (spec.md 7.9). The preview
/// is therefore free to approximate; the export is not, and always uses the CPU
/// evaluator.
pub trait FieldEvaluator: Send + Sync + std::fmt::Debug {
    /// Human-readable backend name, surfaced in About and in logs.
    fn backend_name(&self) -> &'static str;

    /// Evaluates `scene` at each point: the field, its coverage and its kind
    /// (M31). What a tile is encoded from.
    fn evaluate_samples(&self, scene: &Scene, points: &[SamplePoint]) -> Result<Vec<Sample>>;

    /// Evaluates `scene` at each point, returning one vector each. What the
    /// export writes: an uncovered cell is calm.
    fn evaluate(&self, scene: &Scene, points: &[SamplePoint]) -> Result<Vec<Uv>> {
        Ok(self
            .evaluate_samples(scene, points)?
            .into_iter()
            .map(|sample| sample.uv)
            .collect())
    }
}

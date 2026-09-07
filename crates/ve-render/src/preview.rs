//! Coarse-lattice preview rendering.
//!
//! A tile is 65,536 pixels, but a preview does not have to evaluate every one:
//! the view is a proxy, never a source, so it only has to *look* right
//! (spec.md, invariant 3 and 7.9). Evaluating a coarse lattice and
//! interpolating between the nodes cuts the work by the square of the stride.
//!
//! # Why it is adaptive, and on what
//!
//! Plain interpolation would smear every hard edge over a whole cell, and a
//! brush with no feather has an edge that is genuinely a step. So some cells
//! must be re-evaluated exactly.
//!
//! The test for which ones is **non-linearity, not variation**. An earlier
//! version refined any cell whose corners disagreed by more than a threshold,
//! which was wrong: a smooth linear ramp has a large corner spread and
//! interpolates perfectly. In a busy field that criterion fired almost
//! everywhere, so the renderer paid for the lattice *and* full evaluation —
//! measurably slower than doing nothing clever at all.
//!
//! Instead each cell's centre is evaluated and compared against what bilinear
//! interpolation predicts there. Agreement means the field is locally linear
//! and the cell can be interpolated however fast it is changing; disagreement
//! means an edge or a curve, and the cell is refined. One extra sample per
//! cell buys a criterion that measures the thing that actually matters.

use ve_core::LonLat;
use ve_core::vector::Uv;

use crate::error::Result;
use crate::evaluator::{FieldEvaluator, Sample};
use crate::scene::Scene;
use crate::tile::{TILE_SIZE, TileId};

/// Interpolation error, in m/s, above which a cell is evaluated exactly.
///
/// Set below the preview fidelity tolerance of 0.25 m/s (spec.md 7.9), so a
/// cell is refined before the error could become visible.
const REFINE_THRESHOLD: f32 = 0.15;

/// How coarse the preview lattice is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Quality {
    /// Every pixel evaluated. Used for anything that must be exact.
    Exact,
    /// One node every 4 pixels: about a sixteenth of the work.
    Standard,
    /// One node every 8 pixels, for while the camera is moving.
    Draft,
}

impl Quality {
    /// Pixels between lattice nodes.
    pub fn stride(self) -> u32 {
        match self {
            Self::Exact => 1,
            Self::Standard => 4,
            Self::Draft => 8,
        }
    }
}

/// Interpolation error in coverage above which a cell on the edge of the
/// written field is evaluated exactly (M31): a written cell beside an
/// unwritten one is an edge whatever the field does there, and interpolating
/// across it would feather a hard one. Only a cell with an unwritten corner
/// or centre is tested — inside the field the coverage is at most a little
/// off across overlapping feathered rims, which moves the alpha by less than
/// the eye can see, and testing it there refined a third of a dense tile and
/// cost the coarse path its advantage.
const COVERAGE_REFINE_THRESHOLD: f32 = 0.2;

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Bilinear blend of four corner samples.
///
/// The field and the coverage interpolate; the kind cannot, so the nearest
/// corner's is taken — a cell whose corners disagree about it is refined
/// anyway, so the choice only ever decides a pixel inside a uniform cell.
fn blend(c00: Sample, c10: Sample, c01: Sample, c11: Sample, tx: f32, ty: f32) -> Sample {
    let top_u = lerp(c00.uv.u, c10.uv.u, tx);
    let top_v = lerp(c00.uv.v, c10.uv.v, tx);
    let bottom_u = lerp(c01.uv.u, c11.uv.u, tx);
    let bottom_v = lerp(c01.uv.v, c11.uv.v, tx);
    let top_c = lerp(c00.coverage, c10.coverage, tx);
    let bottom_c = lerp(c01.coverage, c11.coverage, tx);
    let nearest = match (tx < 0.5, ty < 0.5) {
        (true, true) => c00,
        (false, true) => c10,
        (true, false) => c01,
        (false, false) => c11,
    };
    Sample {
        uv: Uv {
            u: lerp(top_u, bottom_u, ty),
            v: lerp(top_v, bottom_v, ty),
        },
        coverage: lerp(top_c, bottom_c, ty),
        kind: nearest.kind,
    }
}

/// Whether the true centre value departs from the interpolated one by more
/// than the eye is allowed to see — in the field, in the coverage, or in
/// the kind, which either agrees across the cell or does not.
fn needs_refining(corners: [Sample; 4], centre: Sample) -> bool {
    let predicted = blend(corners[0], corners[1], corners[2], corners[3], 0.5, 0.5);
    let field_error = (predicted.uv.u - centre.uv.u)
        .abs()
        .max((predicted.uv.v - centre.uv.v).abs());
    let on_the_edge = centre.coverage <= 0.0 || corners.iter().any(|corner| corner.coverage <= 0.0);
    field_error > REFINE_THRESHOLD
        || (on_the_edge && (predicted.coverage - centre.coverage).abs() > COVERAGE_REFINE_THRESHOLD)
        || corners.iter().any(|corner| corner.kind != centre.kind)
}

/// Lattice nodes and the cells that need exact evaluation.
struct CellPlan {
    nodes: u32,
    node_values: Vec<Sample>,
    refine: Vec<(u32, u32)>,
}

fn plan_cells(
    evaluator: &dyn FieldEvaluator,
    scene: &Scene,
    id: TileId,
    stride: u32,
) -> Result<CellPlan> {
    let bounds = id.bounds();
    let cells = TILE_SIZE / stride;
    let nodes = cells + 1;

    // Lattice nodes span the tile's full extent, so every pixel falls inside a
    // cell rather than needing extrapolation at the far edge.
    let node_positions: Vec<LonLat> = (0..nodes)
        .flat_map(|row| {
            (0..nodes).map(move |column| {
                let fx = f64::from(column) / f64::from(cells);
                let fy = f64::from(row) / f64::from(cells);
                LonLat {
                    lon: ve_core::geo::normalize_lon(
                        bounds.west + fx * (bounds.east - bounds.west),
                    ),
                    lat: (bounds.north - fy * (bounds.north - bounds.south)).clamp(-90.0, 90.0),
                }
            })
        })
        .collect();

    let node_values = evaluator.evaluate_samples(scene, &node_positions)?;
    let node = |column: u32, row: u32| -> Sample {
        node_values
            .get((row * nodes + column) as usize)
            .copied()
            .unwrap_or_default()
    };

    // One sample at each cell centre, to test whether the field is locally
    // linear there. This is the criterion that decides refinement.
    let centre_positions: Vec<LonLat> = (0..cells)
        .flat_map(|row| {
            (0..cells).map(move |column| {
                let fx = (f64::from(column) + 0.5) / f64::from(cells);
                let fy = (f64::from(row) + 0.5) / f64::from(cells);
                LonLat {
                    lon: ve_core::geo::normalize_lon(
                        bounds.west + fx * (bounds.east - bounds.west),
                    ),
                    lat: (bounds.north - fy * (bounds.north - bounds.south)).clamp(-90.0, 90.0),
                }
            })
        })
        .collect();
    let centre_values = evaluator.evaluate_samples(scene, &centre_positions)?;

    let mut refine: Vec<(u32, u32)> = Vec::new();
    for row in 0..cells {
        for column in 0..cells {
            let corners = [
                node(column, row),
                node(column + 1, row),
                node(column, row + 1),
                node(column + 1, row + 1),
            ];
            let centre = centre_values
                .get((row * cells + column) as usize)
                .copied()
                .unwrap_or_default();
            if needs_refining(corners, centre) {
                refine.push((column, row));
            }
        }
    }

    Ok(CellPlan {
        nodes,
        node_values,
        refine,
    })
}

/// Diagnostic: what fraction of cells needed exact evaluation.
///
/// Reported so the value of the coarse path can be measured rather than
/// assumed. If most cells refine, the lattice is pure overhead.
pub fn refined_fraction(
    evaluator: &dyn FieldEvaluator,
    scene: &Scene,
    id: TileId,
    quality: Quality,
) -> Result<f64> {
    let stride = quality.stride();
    if stride <= 1 {
        return Ok(1.0);
    }
    let cells = TILE_SIZE / stride;
    let plan = plan_cells(evaluator, scene, id, stride)?;
    Ok(plan.refine.len() as f64 / f64::from(cells * cells))
}

/// Renders one tile's worth of samples, row-major from the north-west corner.
pub fn render_tile(
    evaluator: &dyn FieldEvaluator,
    scene: &Scene,
    id: TileId,
    quality: Quality,
) -> Result<Vec<Sample>> {
    let stride = quality.stride();
    if stride <= 1 {
        return evaluator.evaluate_samples(scene, &id.sample_positions());
    }

    let plan = plan_cells(evaluator, scene, id, stride)?;
    let nodes = plan.nodes;
    let cells = TILE_SIZE / stride;
    let node = |column: u32, row: u32| -> Sample {
        plan.node_values
            .get((row * nodes + column) as usize)
            .copied()
            .unwrap_or_default()
    };
    let refine = plan.refine;

    let mut out = vec![Sample::default(); (TILE_SIZE * TILE_SIZE) as usize];

    // Interpolate everything first, then overwrite the refined cells.
    for py in 0..TILE_SIZE {
        let fy = (f64::from(py) + 0.5) / f64::from(TILE_SIZE) * f64::from(cells);
        let row = (fy.floor() as u32).min(cells - 1);
        let ty = (fy - f64::from(row)) as f32;

        for px in 0..TILE_SIZE {
            let fx = (f64::from(px) + 0.5) / f64::from(TILE_SIZE) * f64::from(cells);
            let column = (fx.floor() as u32).min(cells - 1);
            let tx = (fx - f64::from(column)) as f32;

            out[(py * TILE_SIZE + px) as usize] = blend(
                node(column, row),
                node(column + 1, row),
                node(column, row + 1),
                node(column + 1, row + 1),
                tx,
                ty,
            );
        }
    }

    if !refine.is_empty() {
        let mut positions = Vec::with_capacity(refine.len() * (stride * stride) as usize);
        let mut targets = Vec::with_capacity(positions.capacity());
        for (column, row) in &refine {
            for offset_y in 0..stride {
                for offset_x in 0..stride {
                    let px = column * stride + offset_x;
                    let py = row * stride + offset_y;
                    if px >= TILE_SIZE || py >= TILE_SIZE {
                        continue;
                    }
                    positions.push(id.pixel_position(px, py));
                    targets.push((py * TILE_SIZE + px) as usize);
                }
            }
        }

        for (index, value) in targets
            .iter()
            .zip(evaluator.evaluate_samples(scene, &positions)?)
        {
            out[*index] = value;
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::CpuEvaluator;
    use crate::scene::{DirectionMode, EdgeMode, FlatObject, Motion, OffsetMode, SpeedMode};
    use crate::sdf::Shape;
    use ve_core::angle::Angle;

    fn tile() -> TileId {
        TileId::new(3, 4, 3).expect("valid tile")
    }

    /// A hard-edged disc: the case plain interpolation would smear.
    fn disc_scene(speed: f64) -> Scene {
        let centre = tile().pixel_position(128, 128);
        Scene {
            rasters: Vec::new(),
            objects: vec![FlatObject {
                erased: Vec::new(),
                layer: 0,
                kind: ve_core::project::FieldKind::Wind,
                frame: crate::aeqd::Frame::new(centre, 0.0, 100.0),
                shape: Shape::Disc {
                    radius_m: 400_000.0,
                },
                cap_radius_m: 400_000.0,
                speed: SpeedMode::Constant(speed),
                direction: DirectionMode::Constant(Angle::new(90.0)),
                feather: 0.0,
                edge_mode: EdgeMode::Blend,
                gradient_axis: Angle::new(90.0),
                gradient_extent: 400_000.0,
                path: Vec::new(),
                clone_source: None,
                clone_offset: OffsetMode::Aligned,
                modifier: None,
                smear: Vec::new(),
                invert: false,
                capture: None,
                erases: false,
                motion: Motion::default(),
            }],
        }
    }

    fn render(scene: &Scene, quality: Quality) -> Vec<Uv> {
        render_tile(&CpuEvaluator, scene, tile(), quality)
            .expect("renders")
            .into_iter()
            .map(|sample| sample.uv)
            .collect()
    }

    #[test]
    fn every_quality_produces_a_full_tile() {
        let scene = disc_scene(20.0);
        for quality in [Quality::Exact, Quality::Standard, Quality::Draft] {
            assert_eq!(
                render(&scene, quality).len(),
                (TILE_SIZE * TILE_SIZE) as usize
            );
        }
    }

    /// An empty scene is uniform, so it must interpolate exactly.
    #[test]
    fn a_calm_tile_is_identical_at_every_quality() {
        let empty = Scene::default();
        let exact = render(&empty, Quality::Exact);
        for quality in [Quality::Standard, Quality::Draft] {
            assert_eq!(render(&empty, quality), exact);
        }
    }

    /// The property the whole design rests on: a coarse render must stay within
    /// the preview fidelity tolerance of an exact one (spec.md 7.9).
    #[test]
    fn coarse_rendering_stays_within_the_fidelity_tolerance() {
        let scene = disc_scene(25.0);
        let exact = render(&scene, Quality::Exact);

        for quality in [Quality::Standard, Quality::Draft] {
            let coarse = render(&scene, quality);
            let mut worst = 0.0f32;
            for (a, b) in exact.iter().zip(&coarse) {
                worst = worst.max((a.u - b.u).abs()).max((a.v - b.v).abs());
            }
            assert!(
                worst <= 0.25,
                "{quality:?} differed by {worst} m/s, tolerance is 0.25"
            );
        }
    }

    /// A hard edge must stay hard: refinement exists precisely so a disc's rim
    /// does not come out looking feathered.
    #[test]
    fn a_hard_edge_is_not_smeared() {
        let scene = disc_scene(25.0);
        let coarse = render(&scene, Quality::Standard);

        // Across the middle row, count pixels that are neither clearly inside
        // nor clearly outside. A smeared edge would produce many.
        let middle = 128 * TILE_SIZE as usize;
        let transitional = (0..TILE_SIZE as usize)
            .map(|x| coarse[middle + x].u)
            .filter(|u| *u > 1.0 && *u < 24.0)
            .count();
        assert!(transitional <= 4, "edge spread over {transitional} pixels");
    }

    #[test]
    fn a_smooth_gradient_interpolates_cleanly() {
        let centre = tile().pixel_position(128, 128);
        let scene = Scene {
            rasters: Vec::new(),
            objects: vec![FlatObject {
                erased: Vec::new(),
                layer: 0,
                kind: ve_core::project::FieldKind::Wind,
                frame: crate::aeqd::Frame::new(centre, 0.0, 100.0),
                shape: Shape::Disc {
                    radius_m: 5_000_000.0,
                },
                cap_radius_m: 5_000_000.0,
                speed: SpeedMode::Radial {
                    centre: 0.0,
                    edge: 30.0,
                    extent: 5_000_000.0,
                },
                direction: DirectionMode::Constant(Angle::new(45.0)),
                feather: 0.0,
                edge_mode: EdgeMode::Blend,
                gradient_axis: Angle::new(90.0),
                gradient_extent: 5_000_000.0,
                path: Vec::new(),
                clone_source: None,
                clone_offset: OffsetMode::Aligned,
                modifier: None,
                smear: Vec::new(),
                invert: false,
                capture: None,
                erases: false,
                motion: Motion::default(),
            }],
        };

        let exact = render(&scene, Quality::Exact);
        let coarse = render(&scene, Quality::Standard);
        let worst = exact
            .iter()
            .zip(&coarse)
            .map(|(a, b)| (a.u - b.u).abs().max((a.v - b.v).abs()))
            .fold(0.0f32, f32::max);
        assert!(worst <= 0.25, "smooth field differed by {worst}");
    }

    #[test]
    fn strides_divide_the_tile_evenly() {
        for quality in [Quality::Exact, Quality::Standard, Quality::Draft] {
            assert_eq!(TILE_SIZE % quality.stride(), 0, "{quality:?}");
        }
    }
}

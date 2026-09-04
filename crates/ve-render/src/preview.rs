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
use crate::evaluator::FieldEvaluator;
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

/// Bilinear blend of four corner vectors.
fn blend(c00: Uv, c10: Uv, c01: Uv, c11: Uv, tx: f32, ty: f32) -> Uv {
    let top_u = c00.u + (c10.u - c00.u) * tx;
    let top_v = c00.v + (c10.v - c00.v) * tx;
    let bottom_u = c01.u + (c11.u - c01.u) * tx;
    let bottom_v = c01.v + (c11.v - c01.v) * tx;
    Uv {
        u: top_u + (bottom_u - top_u) * ty,
        v: top_v + (bottom_v - top_v) * ty,
    }
}

/// How far the true centre value departs from the interpolated one.
fn interpolation_error(corners: [Uv; 4], centre: Uv) -> f32 {
    let predicted = blend(corners[0], corners[1], corners[2], corners[3], 0.5, 0.5);
    (predicted.u - centre.u)
        .abs()
        .max((predicted.v - centre.v).abs())
}

/// Lattice nodes and the cells that need exact evaluation.
struct CellPlan {
    nodes: u32,
    node_values: Vec<Uv>,
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

    let node_values = evaluator.evaluate(scene, &node_positions)?;
    let node = |column: u32, row: u32| -> Uv {
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
    let centre_values = evaluator.evaluate(scene, &centre_positions)?;

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
            if interpolation_error(corners, centre) > REFINE_THRESHOLD {
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
) -> Result<Vec<Uv>> {
    let stride = quality.stride();
    if stride <= 1 {
        return evaluator.evaluate(scene, &id.sample_positions());
    }

    let plan = plan_cells(evaluator, scene, id, stride)?;
    let nodes = plan.nodes;
    let cells = TILE_SIZE / stride;
    let node = |column: u32, row: u32| -> Uv {
        plan.node_values
            .get((row * nodes + column) as usize)
            .copied()
            .unwrap_or_default()
    };
    let refine = plan.refine;

    let mut out = vec![Uv::default(); (TILE_SIZE * TILE_SIZE) as usize];

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

        for (index, value) in targets.iter().zip(evaluator.evaluate(scene, &positions)?) {
            out[*index] = value;
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::CpuEvaluator;
    use crate::scene::{DirectionMode, EdgeMode, FlatObject, OffsetMode, SpeedMode};
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
                invert: false,
            }],
        }
    }

    fn render(scene: &Scene, quality: Quality) -> Vec<Uv> {
        render_tile(&CpuEvaluator, scene, tile(), quality).expect("renders")
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
                invert: false,
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

//! The CPU field evaluator.
//!
//! Authoritative. Every exported GRIB is produced by this code, so it is the
//! definition of what a project means — the GPU backend is an approximation of
//! it for preview only (spec.md 7.8).

use rayon::prelude::*;
use ve_core::angle::Angle;
use ve_core::vector::{Uv, uv_from_speed_azimuth};
use ve_core::{LonLat, geo};

use crate::aeqd::{Local, M_PER_DEGREE};
use crate::error::Result;
use crate::evaluator::{FieldEvaluator, SamplePoint};
use crate::scene::{
    DirectionMode, EdgeMode, FlatCapture, FlatObject, Modifier, OffsetMode, Scene, SpeedMode, Warp,
};

/// How deep a clone stamp may read through other clone stamps.
///
/// The z-order dependency graph is acyclic by construction — a stamp only ever
/// reads *below* itself — so this is a cost guard, not a correctness one. Past
/// it the source reads as calm (spec.md 7.6).
const MAX_CLONE_DEPTH: u32 = 4;

/// Smallest worthwhile parallel chunk.
///
/// Below this the scheduling costs more than the work.
const MIN_CHUNK: usize = 256;

/// Chunk size for `count` points.
///
/// Sized from the thread count rather than fixed, so a small batch still
/// spreads across the pool. A fixed 4,096 left the preview's 4,225-node pass on
/// two threads and its 4,096 cell centres on *one*, while a full tile used
/// sixteen — which made the coarse preview path slower than evaluating every
/// pixel, for reasons that had nothing to do with how much work it saved.
fn chunk_size(count: usize) -> usize {
    let threads = rayon::current_num_threads().max(1);
    // Several chunks per thread, so an uneven scene still balances.
    (count / (threads * 4)).max(MIN_CHUNK)
}

/// Smooth Hermite interpolation between two edges.
fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    if edge1 <= edge0 {
        return if x >= edge1 { 1.0 } else { 0.0 };
    }
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How strongly an object writes at a point inside it.
///
/// Zero at the edge, one well inside. Feather widens the band; a feather of
/// zero gives a hard edge.
fn feather_weight(signed_distance: f64, feather: f64, reference: f64) -> f64 {
    let band = feather * reference;
    if band <= 0.0 {
        return 1.0;
    }
    smoothstep(0.0, band, -signed_distance)
}

/// Position along an object's gradient axis, 0 to 1.
fn axis_fraction(object: &FlatObject, local: Local) -> f64 {
    if object.gradient_extent <= 0.0 {
        return 0.5;
    }
    // The axis is a spatial parameter, not a flow direction, so taking it in
    // the local frame is correct here.
    let axis = (object.gradient_axis.degrees() - object.frame.rotation_deg).to_radians();
    let along = local[0] * axis.sin() + local[1] * axis.cos();
    (0.5 + along / (2.0 * object.gradient_extent)).clamp(0.0, 1.0)
}

/// Bearing of the object's path nearest to `local`.
///
/// Taken between the segment's endpoints *on the globe* rather than from a
/// local frame angle, so it is a true azimuth at any distance from the anchor.
fn path_bearing(object: &FlatObject, local: Local) -> Angle {
    if object.path.len() < 2 {
        return Angle::new(object.frame.rotation_deg);
    }

    let mut best = (f64::INFINITY, 0usize);
    for (index, pair) in object.path.windows(2).enumerate() {
        let (a, b) = (pair[0], pair[1]);
        let (pax, pay) = (local[0] - a[0], local[1] - a[1]);
        let (bax, bay) = (b[0] - a[0], b[1] - a[1]);
        let denom = bax * bax + bay * bay;
        let t = if denom <= f64::EPSILON {
            0.0
        } else {
            ((pax * bax + pay * bay) / denom).clamp(0.0, 1.0)
        };
        let distance = (pax - bax * t).hypot(pay - bay * t);
        if distance < best.0 {
            best = (distance, index);
        }
    }

    let (a, b) = (object.path[best.1], object.path[best.1 + 1]);
    object
        .frame
        .to_global(a)
        .initial_bearing(object.frame.to_global(b))
}

/// The object's speed at a point.
fn speed_at(object: &FlatObject, local: Local) -> f64 {
    match object.speed {
        SpeedMode::Constant(speed) => speed,
        SpeedMode::Radial {
            centre,
            edge,
            extent,
        } => {
            let t = if extent > 0.0 {
                (local[0].hypot(local[1]) / extent).clamp(0.0, 1.0)
            } else {
                0.0
            };
            centre + (edge - centre) * t
        }
        SpeedMode::Axis { start, end } => {
            let t = axis_fraction(object, local);
            start + (end - start) * t
        }
    }
}

/// The object's direction at a point, as a true azimuth-toward.
fn direction_at(object: &FlatObject, position: LonLat, local: Local) -> Angle {
    match &object.direction {
        DirectionMode::Constant(bearing) => *bearing,
        // Exact everywhere: the bearing from the cell to the target.
        DirectionMode::Toward(target) => position.initial_bearing(*target),
        // The reciprocal of that, which is the outward tangent to the same
        // great circle. Not `target.initial_bearing(position)`: that is the
        // outward bearing measured at the *target*, and the two differ by the
        // meridian convergence between the two points.
        DirectionMode::Away(target) => {
            Angle::new(position.initial_bearing(*target).degrees() + 180.0)
        }
        DirectionMode::Axis { start, end } => {
            start.lerp_shortest(*end, axis_fraction(object, local))
        }
        DirectionMode::AlongPath { offset } => {
            Angle::new(path_bearing(object, local).degrees() + offset.degrees())
        }
        DirectionMode::Tangential { clockwise } => {
            // 90 degrees off the outward bearing, which side depending on sense.
            let radial = object.frame.radial_bearing(position).degrees();
            Angle::new(radial + if *clockwise { 90.0 } else { -90.0 })
        }
    }
}

/// Samples one object, returning its vector and how strongly it writes.
///
/// `None` when the point lies outside the object entirely.
fn sample_object(object: &FlatObject, position: LonLat) -> Option<(Uv, f64)> {
    let weight = coverage(object, position)?;
    let local = object.frame.to_local(position);
    let speed = speed_at(object, local).max(0.0);
    let bearing = direction_at(object, position, local);
    let vector = uv_from_speed_azimuth(speed, bearing);

    Some((vector, weight))
}

/// How strongly an object writes at a position, or `None` where it does not.
///
/// The one place coverage is decided, for every kind of object: the cap cull,
/// the signed distance, the feather ramp — and the inversion, which turns all
/// three inside out (spec.md 6.2). An inverted object covers everything outside
/// its footprint, so the cap cull that rejects a distant cell for every other
/// object *accepts* it here, at full weight and without touching the SDF.
fn coverage(object: &FlatObject, position: LonLat) -> Option<f64> {
    // Spherical-cap cull first: cheap, and rejects almost every cell for a
    // typical object.
    if object.frame.distance_m(position) > object.cap_radius_m {
        return if object.invert { Some(1.0) } else { None };
    }

    let local = object.frame.to_local(position);
    let signed_distance = object.shape.distance(local);
    if signed_distance > 0.0 && !object.invert {
        return None;
    }

    // The ramp runs over the same band either way; inverting reflects it, so
    // the two sides of one edge always add to a full weight and a mask and its
    // inverse leave no seam between them.
    let weight = feather_weight(
        signed_distance,
        object.feather,
        object.shape.feather_reference_m(),
    );
    Some(if object.invert { 1.0 - weight } else { weight })
}

/// Evaluates a whole scene at one position.
pub fn sample_scene(scene: &Scene, position: LonLat) -> Uv {
    sample_upto(scene, position, scene.objects.len(), 0).0
}

/// Evaluates a scene, and says whether anything wrote at the position.
///
/// **Zero and undefined are different things** (spec.md 8.5, D58). A cell no
/// object and no raster wrote is undefined, and so is one a mask removed — the
/// mask exists to let what is beneath show through, and a capture that turned
/// that into a real zero would overwrite whatever it is later pasted over.
///
/// Only the capture asks. The field itself is unchanged: an undefined cell
/// composites as calm, which is what it has always done, so this cannot move a
/// single exported byte.
pub fn sample_scene_covered(scene: &Scene, position: LonLat) -> Option<Uv> {
    let (uv, coverage) = sample_upto(scene, position, scene.objects.len(), 0);
    (coverage > COVERAGE_EPSILON).then_some(uv)
}

/// How much of a cell has to be written for it to count as written.
///
/// Small rather than a half: the rule is "anything wrote here", so the faded
/// outer edge of a feathered stroke is captured — faded, but there. A patch's
/// own feather is how it is softened again.
const COVERAGE_EPSILON: f32 = 1e-4;

/// Evaluates the first `upto` objects of a scene.
///
/// `depth` counts nested clone-stamp reads. A clone stamp evaluates the scene
/// beneath *itself*, which is exactly `upto = its own index` — so "everything
/// below me in z-order" needs no separate sub-scene to be built.
fn sample_upto(scene: &Scene, position: LonLat, upto: usize, depth: u32) -> (Uv, f32) {
    // Unwritten cells are calm.
    let mut accumulated = Uv::default();
    // ...and uncovered. Coverage accumulates exactly as the field does, so
    // that "was anything written here" has the same answer at a feathered edge
    // that the field has. Only a capture reads it (spec.md 8.5, D58); it costs
    // a few floating-point operations per object and moves no field value.
    let mut coverage = 0.0f32;

    // Imported fields are interleaved with the objects by `z`: a raster at
    // `z` is applied just before object `z`, and every raster at or below
    // `upto` is beneath the object that asked. Where the grid has a value it
    // overwrites outright.
    let mut rasters = scene.rasters.iter().peekable();
    let mut apply_rasters_below = |z: usize, accumulated: &mut Uv, coverage: &mut f32| {
        while let Some(raster) = rasters.next_if(|r| r.z <= z) {
            if let Some(uv) = raster.grid.sample(position.lon, position.lat) {
                // A sample outside the layer's speed band is treated as a
                // missing one, so the field beneath shows through exactly as it
                // does outside a regional grid (spec.md 4.8). Filtered here and
                // not inside `RasterGrid::sample`, which is the lattice's own
                // reading of itself and is shared with the GPU port.
                if raster
                    .speed_range
                    .is_none_or(|band| band.keeps(uv.u.hypot(uv.v)))
                {
                    *accumulated = uv;
                    *coverage = 1.0;
                }
            }
        }
    };

    for (index, object) in scene.objects.iter().take(upto).enumerate() {
        apply_rasters_below(index, &mut accumulated, &mut coverage);
        // A modifier has no field of its own either: it rewrites what the
        // accumulation buffer already holds, which at this point in the loop is
        // exactly everything below it in z-order (spec.md 6.3, 7.6).
        if let Some(modifier) = object.modifier {
            let Some(weight) = operator_weight(object, position) else {
                continue;
            };
            let w = weight as f32;
            let modified = match modifier {
                // A warp reads from somewhere else, so it needs the sub-scene
                // evaluated at that point — the clone stamp's machinery, with a
                // displacement that varies across the footprint instead of a
                // constant offset. The depth cap is shared, and for the same
                // reason: there is no recursion budget beyond it.
                Modifier::Warp(warp) => {
                    if depth >= MAX_CLONE_DEPTH {
                        accumulated
                    } else {
                        let read = warp_source_position(object, warp, position, weight);
                        sample_upto(scene, read, index, depth + 1).0
                    }
                }
                _ => modified_vector(modifier, object, position, accumulated),
            };
            // One rule for all four: fade from what was there to what the
            // modifier makes of it. At the footprint's edge the weight is zero
            // and the field is untouched, which is what keeps a modifier from
            // showing its own outline.
            accumulated = Uv {
                u: accumulated.u + (modified.u - accumulated.u) * w,
                v: accumulated.v + (modified.v - accumulated.v) * w,
            };
            continue;
        }
        // A clone stamp has no field of its own: it copies whatever lies
        // beneath it, from an offset position.
        if let Some(source) = object.clone_source {
            let Some(weight) = operator_weight(object, position) else {
                continue;
            };
            let vector = if depth >= MAX_CLONE_DEPTH {
                Uv::default()
            } else {
                let sampled = clone_source_position(object, source, position);
                sample_upto(scene, sampled, index, depth + 1).0
            };
            // A moving clone stamp carries its own motion into what it copies
            // (spec.md 9.3), added before the edge so the feather fades the
            // sum rather than the two separately.
            let vector = with_motion(object, position, vector);
            let w = weight as f32;
            accumulated = match object.edge_mode {
                EdgeMode::Blend => Uv {
                    u: accumulated.u + (vector.u - accumulated.u) * w,
                    v: accumulated.v + (vector.v - accumulated.v) * w,
                },
                EdgeMode::Replace => Uv {
                    u: vector.u * w,
                    v: vector.v * w,
                },
            };
            coverage = covered(object, coverage, w);
            continue;
        }

        // A patch replays a captured field rather than computing one
        // (spec.md 8.5, M14). Where the capture is undefined it writes
        // *nothing* — that is the whole of D58 — so a cell its source never
        // covered leaves what is beneath alone, exactly as a raster's gap
        // does.
        if let Some(patch) = object.capture.as_ref() {
            let Some(weight) = operator_weight(object, position) else {
                continue;
            };
            let Some(vector) = patch_sample(object, patch, position) else {
                continue;
            };
            let vector = with_motion(object, position, vector);
            let w = weight as f32;
            coverage = covered(object, coverage, w);
            accumulated = match object.edge_mode {
                EdgeMode::Blend => Uv {
                    u: accumulated.u + (vector.u - accumulated.u) * w,
                    v: accumulated.v + (vector.v - accumulated.v) * w,
                },
                EdgeMode::Replace => Uv {
                    u: vector.u * w,
                    v: vector.v * w,
                },
            };
            continue;
        }

        let Some((vector, weight)) = sample_object(object, position) else {
            continue;
        };
        let vector = with_motion(object, position, vector);
        let w = weight as f32;
        coverage = covered(object, coverage, w);
        accumulated = match object.edge_mode {
            // Fade into whatever is underneath. Identical to Replace where the
            // field below is calm, which is why it is the default (D12).
            EdgeMode::Blend => Uv {
                u: accumulated.u + (vector.u - accumulated.u) * w,
                v: accumulated.v + (vector.v - accumulated.v) * w,
            },
            // Overwrite, fading the object's own speed toward zero.
            EdgeMode::Replace => Uv {
                u: vector.u * w,
                v: vector.v * w,
            },
        };
    }
    apply_rasters_below(upto, &mut accumulated, &mut coverage);
    (accumulated, coverage)
}

/// A patch's captured value at a position, in the object's own frame.
///
/// The frame does the moving: `to_local` gives metres in the object's space,
/// which for the projected space a region always has is degrees of the map
/// times `M_PER_DEGREE`. Dividing that back out gives the degrees the capture
/// indexed itself by, so a patch that has been dragged, turned or scaled reads
/// the same samples through a different transform rather than resampling them.
///
/// `None` where the capture has nothing: outside its lattice, or at a cell its
/// source never covered.
fn patch_sample(object: &FlatObject, patch: &FlatCapture, position: LonLat) -> Option<Uv> {
    let local = object.frame.to_local(position);
    // The lattice is read in the object's own frame, and a capture that
    // recorded a moving region has already moved the *frame* — anchor,
    // footprint and all (spec.md 8.7) — so there is nothing to subtract here.
    let sample =
        patch
            .capture
            .sample_pick(patch.pick, local[0] / M_PER_DEGREE, local[1] / M_PER_DEGREE)?;
    Some(Uv {
        u: sample[0],
        v: sample[1],
    })
}

/// How much of a cell an object has written, after it (spec.md 8.5, D58).
///
/// The same shape as the field's own blend, so coverage and field agree at
/// every feathered edge. A mask **subtracts**: it is there to let what is
/// beneath show through, and what it removes is undefined rather than calm —
/// so a patch captured over one is transparent exactly where the mask was, and
/// does not paint a hole of dead air over whatever it is pasted onto.
fn covered(object: &FlatObject, before: f32, weight: f32) -> f32 {
    if object.erases {
        return before * (1.0 - weight);
    }
    match object.edge_mode {
        EdgeMode::Blend => before + (1.0 - before) * weight,
        EdgeMode::Replace => weight,
    }
}

/// Adds an object's own movement to the vector it paints (spec.md 9.3, M13).
///
/// One vector addition per cell, in the cell's own east/north frame, which is
/// what makes a tailwind strengthen, a headwind weaken and a crosswind turn by
/// the same rule — and what makes the sum right at every relative angle rather
/// than only along the axis of travel.
///
/// Before the feather and the edge mode, so the edge fades the sum. A still
/// object pays one comparison.
fn with_motion(object: &FlatObject, position: LonLat, vector: Uv) -> Uv {
    if object.motion.is_still() {
        return vector;
    }
    let added = object.motion.velocity_at(&object.frame, position);
    Uv {
        u: vector.u + added.u,
        v: vector.v + added.v,
    }
}

/// What a modifier makes of the vector beneath it.
///
/// Every variant is a function of the vector it was handed, so a modifier over
/// calm water leaves calm water: nothing here can conjure a field where there
/// is none, which is what separates a modifier from a tool that paints
/// (spec.md 6.3).
fn modified_vector(modifier: Modifier, object: &FlatObject, position: LonLat, beneath: Uv) -> Uv {
    match modifier {
        Modifier::Gain(gain) => {
            let factor = (1.0 + gain) as f32;
            Uv {
                u: beneath.u * factor,
                v: beneath.v * factor,
            }
        }
        // Outward from the anchor, at a fraction of the local speed. The
        // bearing is the frame's own radial one — the same the circle's
        // rotation is a quarter turn off (spec.md 7.5) — so the two tools
        // agree about which way "out" is at a given cell.
        Modifier::Radial(fraction) => {
            let speed = f64::from(beneath.u.hypot(beneath.v));
            let radial =
                uv_from_speed_azimuth(speed * fraction, object.frame.radial_bearing(position));
            Uv {
                u: beneath.u + radial.u,
                v: beneath.v + radial.v,
            }
        }
        // Turning an azimuth by `d` is a rotation of (u, v) by `d` clockwise:
        // u = s·sin(az) and v = s·cos(az), so expanding sin(az + d) and
        // cos(az + d) gives exactly this pair. Written out rather than routed
        // through speed and azimuth so a calm cell stays calm instead of
        // acquiring a direction from atan2(0, 0).
        Modifier::Turn(degrees) => {
            let (sin, cos) = degrees.to_radians().sin_cos();
            let (sin, cos) = (sin as f32, cos as f32);
            Uv {
                u: beneath.u * cos + beneath.v * sin,
                v: beneath.v * cos - beneath.u * sin,
            }
        }
        // Handled by the caller, which has the scene a warp has to re-read.
        Modifier::Warp(_) => beneath,
    }
}

/// Where a warp reads from, for a point inside it.
///
/// The displacement fades with the same weight the result is blended by, so it
/// is zero at the footprint's edge: a warp that displaced uniformly would tear
/// the field along its own outline. A push reads from *behind* the direction it
/// pushes — the field at `p` is what used to be at `p - d` — and a twist reads
/// from the position rotated back the other way, for the same reason.
fn warp_source_position(object: &FlatObject, warp: Warp, position: LonLat, weight: f64) -> LonLat {
    let local = object.frame.to_local(position);
    let moved = match warp {
        Warp::Push { x, y } => [local[0] - x * weight, local[1] - y * weight],
        Warp::Twist { degrees } => {
            let (sin, cos) = (-degrees * weight).to_radians().sin_cos();
            // Clockwise on the map is negative in the local right-handed frame,
            // where x is east and y is north.
            [
                local[0] * cos + local[1] * sin,
                local[1] * cos - local[0] * sin,
            ]
        }
    };
    object.frame.to_global(moved)
}

/// Coverage and feather for an operator, without evaluating any field.
///
/// Shared by the clone stamp and the modifiers: all of them need to know how
/// strongly they write at a cell before they know what they are writing. The
/// same [`coverage`] every other object uses — an operator is not a second
/// notion of "inside".
fn operator_weight(object: &FlatObject, position: LonLat) -> Option<f64> {
    coverage(object, position)
}

/// Where a clone stamp reads from, for a point inside it.
///
/// The displacement is preserved in the object's own frame and replayed from
/// the source anchor, so the copied patch keeps its shape and orientation on
/// the globe rather than being sheared by however far apart the two points are.
///
/// Which displacement depends on the offset mode (spec.md 6.2). `Aligned`
/// measures it from the object's anchor, so the source travels with the brush
/// and a long stroke copies a correspondingly long band. `Fixed` measures it
/// from the nearest point of the stroke's own skeleton — the stamp centre for
/// this cell — so the source stays where it was put and every stamp along the
/// stroke reads the same neighbourhood of it.
fn clone_source_position(object: &FlatObject, source: LonLat, position: LonLat) -> LonLat {
    let mut local = object.frame.to_local(position);
    if object.clone_offset == OffsetMode::Fixed
        && let Some(centre) = object.shape.nearest_on_skeleton(local)
    {
        local = [local[0] - centre[0], local[1] - centre[1]];
    }
    let source_frame = crate::aeqd::Frame {
        anchor: source,
        // The displacement is replayed in the object's own space: a projected
        // patch is a map-space patch wherever it is read from.
        space: object.frame.space,
        rotation_deg: object.frame.rotation_deg,
        scale: object.frame.scale,
    };
    source_frame.to_global(local)
}

/// The reference field evaluator.
#[derive(Debug, Default, Clone, Copy)]
pub struct CpuEvaluator;

impl FieldEvaluator for CpuEvaluator {
    fn backend_name(&self) -> &'static str {
        "cpu"
    }

    fn evaluate(&self, scene: &Scene, points: &[SamplePoint]) -> Result<Vec<Uv>> {
        // Chunked rather than per-point so the scheduler overhead is amortised;
        // a global grid at 0.1 degrees is 6.5M samples. The chunk is sized from
        // the thread count so small batches parallelise too.
        Ok(points
            .par_chunks(chunk_size(points.len()))
            .flat_map_iter(|chunk| chunk.iter().map(|p| sample_scene(scene, *p)))
            .collect())
    }
}

/// Great-circle distance, re-exported so callers need not reach into `ve-core`.
pub fn distance_m(from: LonLat, to: LonLat) -> f64 {
    geo::LonLat::distance_m(from, to)
}

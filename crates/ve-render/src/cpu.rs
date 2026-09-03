//! The CPU field evaluator.
//!
//! Authoritative. Every exported GRIB is produced by this code, so it is the
//! definition of what a project means — the GPU backend is an approximation of
//! it for preview only (spec.md 7.8).

use rayon::prelude::*;
use ve_core::angle::Angle;
use ve_core::vector::{Uv, uv_from_speed_azimuth};
use ve_core::{LonLat, geo};

use crate::aeqd::Local;
use crate::error::Result;
use crate::evaluator::{FieldEvaluator, SamplePoint};
use crate::scene::{DirectionMode, EdgeMode, FlatObject, OffsetMode, Scene, SpeedMode};

/// Below this distance from the anchor, radial and tangential directions are
/// undefined, so divergence and curl are skipped rather than producing a
/// singularity at the centre of every circle.
const RADIAL_EPSILON_M: f64 = 1.0;

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
    // Spherical-cap cull first: cheap, and rejects almost every cell for a
    // typical object.
    let ground_distance = object.frame.distance_m(position);
    if ground_distance > object.cap_radius_m {
        return None;
    }

    let local = object.frame.to_local(position);
    let signed_distance = object.shape.distance(local);
    if signed_distance > 0.0 {
        return None;
    }

    let weight = feather_weight(
        signed_distance,
        object.feather,
        object.shape.feather_reference_m(),
    );
    let speed = speed_at(object, local).max(0.0);
    let bearing = direction_at(object, position, local);
    let mut vector = uv_from_speed_azimuth(speed, bearing);

    // Divergence pushes outward from the anchor, curl runs tangential to it.
    // Both are defined against the true bearing from the anchor, so they stay
    // correct at any distance.
    if (object.divergence != 0.0 || object.curl != 0.0) && ground_distance > RADIAL_EPSILON_M {
        let radial = object.frame.radial_bearing(position);
        let tangential = Angle::new(radial.degrees() + 90.0);
        let out = uv_from_speed_azimuth(object.divergence * speed, radial);
        let round = uv_from_speed_azimuth(object.curl * speed, tangential);
        vector = Uv {
            u: vector.u + out.u + round.u,
            v: vector.v + out.v + round.v,
        };
    }

    Some((vector, weight))
}

/// Evaluates a whole scene at one position.
pub fn sample_scene(scene: &Scene, position: LonLat) -> Uv {
    sample_upto(scene, position, scene.objects.len(), 0)
}

/// Evaluates the first `upto` objects of a scene.
///
/// `depth` counts nested clone-stamp reads. A clone stamp evaluates the scene
/// beneath *itself*, which is exactly `upto = its own index` — so "everything
/// below me in z-order" needs no separate sub-scene to be built.
fn sample_upto(scene: &Scene, position: LonLat, upto: usize, depth: u32) -> Uv {
    // Unwritten cells are calm.
    let mut accumulated = Uv::default();

    for (index, object) in scene.objects.iter().take(upto).enumerate() {
        // A clone stamp has no field of its own: it copies whatever lies
        // beneath it, from an offset position.
        if let Some(source) = object.clone_source {
            let Some(weight) = clone_weight(object, position) else {
                continue;
            };
            let vector = if depth >= MAX_CLONE_DEPTH {
                Uv::default()
            } else {
                let sampled = clone_source_position(object, source, position);
                sample_upto(scene, sampled, index, depth + 1)
            };
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
            continue;
        }

        let Some((vector, weight)) = sample_object(object, position) else {
            continue;
        };
        let w = weight as f32;
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
    accumulated
}

/// Coverage and feather for a clone stamp, without evaluating any field.
fn clone_weight(object: &FlatObject, position: LonLat) -> Option<f64> {
    if object.frame.distance_m(position) > object.cap_radius_m {
        return None;
    }
    let local = object.frame.to_local(position);
    let signed_distance = object.shape.distance(local);
    if signed_distance > 0.0 {
        return None;
    }
    Some(feather_weight(
        signed_distance,
        object.feather,
        object.shape.feather_reference_m(),
    ))
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

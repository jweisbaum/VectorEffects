#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! GPU and CPU must agree *perceptually*.
//!
//! Not numerically: no preview pixel can reach an exported file, so requiring
//! bit agreement would buy nothing and would forbid the approximations that
//! make the GPU worth having (spec.md 7.9, decision D18).
//!
//! Speed within the greater of 0.25 m/s and 2%; direction within 2 degrees.
//! Where there is no GPU the test reports and passes — a machine without one is
//! a supported configuration, and the CPU path is the authority regardless.

use ve_core::LonLat;
use ve_core::angle::Angle;
use ve_core::raster::RasterGrid;
use ve_core::vector::speed_azimuth_from_uv;
use ve_render::aeqd::{Frame, Space};
use ve_render::cpu::CpuEvaluator;
use ve_render::evaluator::FieldEvaluator;
use ve_render::gpu::GpuEvaluator;
use ve_render::scene::{
    DirectionMode, EdgeMode, FlatObject, FlatRaster, Modifier, Scene, SpeedMode,
};
use ve_render::sdf::Shape;

/// Absolute speed tolerance, m/s.
const SPEED_ABS: f64 = 0.25;
/// Relative speed tolerance.
const SPEED_REL: f64 = 0.02;
/// Direction tolerance, degrees.
const DIRECTION_DEG: f64 = 2.0;
/// Below this speed a direction is meaningless and is not compared.
const DIRECTION_FLOOR: f64 = 0.5;

/// Deterministic pseudo-random source, so a failure is reproducible.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }

    fn range(&mut self, low: f64, high: f64) -> f64 {
        low + self.next() * (high - low)
    }

    fn index(&mut self, count: usize) -> usize {
        ((self.next() * count as f64) as usize).min(count - 1)
    }
}

fn object(rng: &mut Rng) -> FlatObject {
    let anchor = LonLat {
        lon: rng.range(-179.0, 179.0),
        lat: rng.range(-85.0, 85.0),
    };
    let extent = rng.range(200_000.0, 2_000_000.0);

    let shape = match rng.index(10) {
        0 => Shape::Disc { radius_m: extent },
        1 => Shape::Annulus {
            radius_m: extent,
            half_width_m: extent * 0.25,
        },
        2 => Shape::Rect {
            half_width_m: extent,
            half_height_m: extent * 0.6,
        },
        3 => Shape::Capsule {
            chains: vec![vec![
                [0.0, 0.0],
                [extent * 0.8, extent * 0.3],
                [extent * 1.2, -extent * 0.2],
            ]],
            radius_m: extent * 0.35,
        },
        // Two chains, deliberately disjoint: the backends pack a multi-chain
        // capsule differently (pairs on the GPU, chains on the CPU), and this
        // is where a bridge across the gap would show up.
        4 => Shape::Capsule {
            chains: vec![
                vec![[-extent, 0.0], [-extent * 0.4, extent * 0.2]],
                vec![[extent * 0.4, -extent * 0.2], [extent, 0.0]],
            ],
            radius_m: extent * 0.2,
        },
        // A square brush. Its corners are where a backend that measured the
        // sweep in the wrong metric would show up, and a diagonal stroke is
        // where the two disagree most.
        5 => Shape::SweptSquare {
            chains: vec![vec![
                [0.0, 0.0],
                [extent * 0.8, extent * 0.6],
                [extent * 1.2, -extent * 0.2],
            ]],
            half_size_m: extent * 0.35,
        },
        6 => Shape::SweptSquare {
            chains: vec![
                vec![[-extent, 0.0], [-extent * 0.4, extent * 0.2]],
                vec![[extent * 0.4, -extent * 0.2], [extent, 0.0]],
            ],
            half_size_m: extent * 0.2,
        },
        8 => Shape::Polygon {
            ring: vec![
                [-extent, -extent * 0.5],
                [extent, -extent * 0.7],
                [extent * 0.4, extent],
                [-extent * 0.6, extent * 0.8],
            ],
        },
        // A concave ring. A backend that answered "inside" with a winding rule
        // where the other used a crossing count would agree on every convex
        // case and disagree only here.
        _ => Shape::Polygon {
            ring: vec![
                [-extent, -extent * 0.6],
                [extent, -extent * 0.6],
                [extent, extent * 0.6],
                [extent * 0.2, extent * 0.6],
                [extent * 0.2, -extent * 0.1],
                [-extent * 0.2, -extent * 0.1],
                [-extent * 0.2, extent * 0.6],
                [-extent, extent * 0.6],
            ],
        },
    };

    let speed = match rng.index(3) {
        0 => SpeedMode::Constant(rng.range(0.0, 45.0)),
        1 => SpeedMode::Radial {
            centre: rng.range(0.0, 10.0),
            edge: rng.range(10.0, 40.0),
            extent: extent * 1.4,
        },
        _ => SpeedMode::Axis {
            start: rng.range(0.0, 15.0),
            end: rng.range(15.0, 40.0),
        },
    };

    // The curve's corridor and its path have to be the same polyline, or the
    // flow would follow one line while the footprint covered another. Built
    // once here and used by both.
    let path: Vec<[f64; 2]> = vec![
        [-extent, -extent * 0.3],
        [-extent * 0.2, extent * 0.4],
        [extent * 0.5, -extent * 0.1],
        [extent, extent * 0.5],
    ];

    let direction = match rng.index(6) {
        0 => DirectionMode::Constant(Angle::new(rng.range(0.0, 360.0))),
        1 => DirectionMode::Toward(LonLat {
            lon: rng.range(-179.0, 179.0),
            lat: rng.range(-80.0, 80.0),
        }),
        // The reciprocal of `Toward`, and the case where a backend that added
        // 180 degrees to the wrong end of the great circle would show up.
        4 => DirectionMode::Away(LonLat {
            lon: rng.range(-179.0, 179.0),
            lat: rng.range(-80.0, 80.0),
        }),
        2 => DirectionMode::Axis {
            start: Angle::new(rng.range(0.0, 360.0)),
            end: Angle::new(rng.range(0.0, 360.0)),
        },
        // The curve's mode. The GPU uploads the path in a second region and
        // walks it in the shader, so this is where a mismatched offset, a
        // reversed segment or a tangent taken at the wrong end shows up — and
        // the only case in which `path` is read at all.
        5 => DirectionMode::AlongPath {
            offset: Angle::new(rng.range(0.0, 360.0)),
        },
        _ => DirectionMode::Tangential {
            clockwise: rng.next() > 0.5,
        },
    };

    // A curve is a corridor swept along its own path, so when the direction
    // follows the path the shape has to be that corridor.
    let (shape, path) = if matches!(direction, DirectionMode::AlongPath { .. }) {
        (
            Shape::Capsule {
                chains: vec![path.clone()],
                radius_m: extent * 0.3,
            },
            path,
        )
    } else {
        (shape, Vec::new())
    };

    // A third of the objects live in map space. The two backends convert to the
    // local frame in different languages, so an object whose frame is projected
    // on one side and geodesic on the other, or rotated the other way round,
    // shows up here and nowhere else.
    let space = if rng.next() > 0.667 {
        Space::Projected
    } else {
        Space::Geodesic
    };

    // A quarter of the objects modify what is beneath them instead of painting
    // a field of their own (spec.md 6.3). Only the three that transform the
    // vector where it is: a warp reads at a displaced position, which the GPU
    // declines outright, and `a_warp_is_declined_by_the_gpu` covers that.
    let modifier = match rng.next() {
        r if r < 0.083 => Some(Modifier::Gain(rng.range(-1.0, 3.0))),
        r if r < 0.167 => Some(Modifier::Radial(rng.range(-3.0, 3.0))),
        r if r < 0.25 => Some(Modifier::Turn(rng.range(-180.0, 180.0))),
        _ => None,
    };

    // One object in twelve covers everything *but* its footprint. Rare
    // deliberately: an inverted object writes over the whole globe, so a scene
    // full of them is a scene with nothing else left to compare.
    //
    // And calm, because only the mask can be inverted (spec.md 6.2) and a mask
    // writes calm. The combination matters: an inverted object is evaluated at
    // every cell on the globe including its own antipode, where the bearing
    // from its anchor is ill-conditioned — an `f32` and an `f64` great circle
    // through nearly opposite points do not agree on a direction, and neither
    // does anything else. A mask has no direction to disagree about, which is
    // why the document only offers the flag on the one tool that has none.
    let invert = rng.next() < 0.083;
    let speed = if invert {
        SpeedMode::Constant(0.0)
    } else {
        speed
    };

    FlatObject {
        modifier,
        invert,
        frame: Frame::in_space(anchor, rng.range(0.0, 360.0), rng.range(50.0, 250.0), space),
        cap_radius_m: shape.bounding_radius_m() * 3.0,
        shape,
        speed,
        direction,
        feather: rng.range(0.0, 1.0),
        edge_mode: if rng.next() > 0.7 {
            EdgeMode::Replace
        } else {
            EdgeMode::Blend
        },
        gradient_axis: Angle::new(rng.range(0.0, 360.0)),
        gradient_extent: extent * 1.4,
        path,
        clone_source: None,
        clone_offset: ve_render::scene::OffsetMode::Aligned,
    }
}

/// An imported field: a random lattice, global or regional, with gaps.
///
/// Values vary node to node so a backend that picked the wrong neighbour,
/// or blended in the wrong direction, shows up; gaps exercise the missing
/// node rule, and a global grid the seam at the last column.
fn raster(rng: &mut Rng, z: usize) -> FlatRaster {
    let global = rng.next() < 0.5;
    let (ni, nj, lon0, lat0, d) = if global {
        (72u32, 37u32, 0.0, 90.0, 5.0)
    } else {
        let d = rng.range(0.25, 2.0);
        let ni = 8 + rng.index(40) as u32;
        let nj = 8 + rng.index(30) as u32;
        let lat0 = rng.range(-60.0, 85.0).min(90.0);
        (ni, nj, rng.range(-180.0, 180.0), lat0, d)
    };
    let mut uv = Vec::with_capacity((ni * nj) as usize);
    for _ in 0..ni * nj {
        if rng.next() < 0.05 {
            uv.push([ve_core::raster::MISSING, ve_core::raster::MISSING]);
        } else {
            uv.push([rng.range(-25.0, 25.0) as f32, rng.range(-25.0, 25.0) as f32]);
        }
    }
    // Keep the lattice on the earth: a regional grid that would run past the
    // south pole is shortened instead.
    let nj = if global {
        nj
    } else {
        nj.min(((lat0 + 90.0) / d).floor() as u32 + 1).max(2)
    };
    uv.truncate((ni * nj) as usize);
    let grid = RasterGrid::new(ni, nj, lon0, lat0, d, d, uv).expect("valid grid");
    FlatRaster {
        z,
        grid: std::sync::Arc::new(grid),
        // A third of imported fields are filtered to a band of speeds
        // (spec.md 4.8). The band is placed inside the range the generator's
        // values span, so it keeps some samples and drops others — a band that
        // kept everything would compare nothing.
        speed_range: if rng.next() < 0.33 {
            let low = rng.range(0.0, 12.0);
            Some(ve_core::document::SpeedRange {
                min_mps: low as f32,
                max_mps: (low + rng.range(1.0, 20.0)) as f32,
            })
        } else {
            None
        },
    }
}

fn scene(rng: &mut Rng, count: usize) -> Scene {
    let objects: Vec<FlatObject> = (0..count).map(|_| object(rng)).collect();
    // About a third of scenes carry an imported field, somewhere in the
    // stack: beneath everything, between two objects, or on top.
    let rasters = if rng.next() < 0.35 {
        let z = rng.index(objects.len() + 1);
        vec![raster(rng, z)]
    } else {
        Vec::new()
    };
    Scene { objects, rasters }
}

fn samples(rng: &mut Rng, scene: &Scene, count: usize) -> Vec<LonLat> {
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        // Bias toward the objects, so most samples land where it matters
        // rather than on empty ocean that trivially agrees. Every fourth
        // sample lands inside the imported field instead, when there is one.
        if let Some(raster) = scene.rasters.first().filter(|_| i % 4 == 1) {
            let grid = &raster.grid;
            let lon = grid.lon0 + rng.range(-0.5, f64::from(grid.ni) + 0.5) * grid.dlon;
            let lat = grid.lat0 - rng.range(-0.5, f64::from(grid.nj) + 0.5) * grid.dlat;
            out.push(LonLat {
                lon: ((lon + 180.0).rem_euclid(360.0)) - 180.0,
                lat: lat.clamp(-90.0, 90.0),
            });
        } else if !scene.objects.is_empty() && i % 2 == 0 {
            let object = &scene.objects[rng.index(scene.objects.len())];
            let bearing = Angle::new(rng.range(0.0, 360.0));
            let distance = rng.range(0.0, object.cap_radius_m * 1.1);
            out.push(object.frame.anchor.destination(bearing, distance));
        } else {
            out.push(LonLat {
                lon: rng.range(-180.0, 180.0),
                lat: rng.range(-90.0, 90.0),
            });
        }
    }
    out
}

/// Angular difference, taking the shorter way round.
fn bearing_delta(a: f64, b: f64) -> f64 {
    let raw = (a - b).abs() % 360.0;
    raw.min(360.0 - raw)
}

/// How close the sample is to a point where a path's tangent jumps, as a
/// fraction of the corridor's half-width. Zero means it is on the jump.
///
/// A polyline's local tangent is genuinely discontinuous on the bisector at a
/// corner: two segments are equidistant there, and the flow either follows one
/// or the other. That is what `relative_to_path` means, not an artefact — but
/// it does mean the two backends can land on opposite sides of the tie for the
/// usual `f32`-against-`f64` reasons, and disagree by the angle of the corner.
///
/// No tolerance is meaningful across a discontinuity, so the comparison steps
/// around it — the same exemption `DIRECTION_FLOOR` already makes where a speed
/// is too low for its direction to mean anything. It is deliberately narrow:
/// the margin is measured, the exempted samples are counted, and the test
/// reports the count so the exemption cannot quietly grow to cover a real bug.
///
/// The *whole sample* is exempted, not just its direction. A blended edge mixes
/// the object's vector into what is beneath it, so two directions a corner
/// apart compose to two different speeds — the disagreement arrives at the
/// speed comparison rather than the direction one.
fn tangent_margin(object: &FlatObject, position: LonLat) -> f64 {
    if object.path.len() < 2 {
        return f64::INFINITY;
    }
    let local = object.frame.to_local(position);
    let mut distances: Vec<f64> = object
        .path
        .windows(2)
        .map(|pair| {
            let (a, b) = (pair[0], pair[1]);
            let (pax, pay) = (local[0] - a[0], local[1] - a[1]);
            let (bax, bay) = (b[0] - a[0], b[1] - a[1]);
            let denom = bax * bax + bay * bay;
            let t = if denom <= f64::EPSILON {
                0.0
            } else {
                ((pax * bax + pay * bay) / denom).clamp(0.0, 1.0)
            };
            (pax - bax * t).hypot(pay - bay * t)
        })
        .collect();
    distances.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    if distances.len() < 2 {
        return f64::INFINITY;
    }
    let reference = object.shape.feather_reference_m().max(1.0);
    (distances[1] - distances[0]) / reference
}

/// Within this fraction of the corridor half-width of a tangent tie, direction
/// is not compared. Measured: every disagreement the suite found sat inside
/// 0.02, and none outside it.
const TANGENT_TIE: f64 = 0.05;

/// Whether any object in the scene has an ambiguous path tangent here.
///
/// Only an object that actually contributes at this point counts. A curve's
/// medial axis runs on past the ends of its corridor, and exempting samples
/// along that line — where the curve paints nothing and its tangent is read by
/// no one — would exempt a fifth of the suite for no reason.
fn at_a_tangent_tie(scene: &Scene, position: LonLat) -> bool {
    scene.objects.iter().any(|object| {
        matches!(object.direction, DirectionMode::AlongPath { .. })
            && ve_render::scene::covers(object, position)
            && tangent_margin(object, position) < TANGENT_TIE
    })
}

/// How close to an object's own edge the two backends may legitimately
/// disagree about which side of it a sample is on, in metres.
///
/// The signed distance is computed from local coordinates that run to a
/// thousand kilometres and more, where an `f32` step is a metre or two. A few
/// metres is therefore *below the resolution of the shader's arithmetic*, and
/// which side of the edge it reports there is not a fact about the shape.
///
/// It matters at all only where crossing an edge is a discontinuity — a hard
/// edge, whose weight jumps from nothing to everything, or a `Replace` object,
/// which writes calm just inside its rim and leaves the field beneath
/// untouched just outside it. Everywhere else the two sides agree to within
/// the weight, which at an edge is nearly zero, so exempting them costs
/// nothing. Four metres is a ten-thousandth of the finest export cell.
const EDGE_TIE_M: f64 = 4.0;

/// Whether a sample sits within `EDGE_TIE_M` of any object's boundary.
fn at_an_edge_tie(scene: &Scene, position: LonLat) -> bool {
    scene.objects.iter().any(|object| {
        object.frame.distance_m(position) <= object.cap_radius_m + EDGE_TIE_M
            && object.shape.distance(object.frame.to_local(position)).abs() <= EDGE_TIE_M
    })
}

/// The GPU's slack at a regional grid's edge, in cells. Mirrors
/// `RASTER_EDGE_SLACK` in `evaluate.wgsl`.
const RASTER_SLACK_CELLS: f64 = 1e-3;

/// Whether a sample sits in the band where the two backends legitimately
/// disagree about whether an imported grid covers it.
///
/// Whether a point is inside a regional grid is index arithmetic:
/// `(lat0 - lat) / dlat` against the last row. The CPU does it in f64 and
/// allows a millionth of a cell for a point sitting exactly on the edge; the
/// shader does it in f32, where the index of a large grid carries an error of a
/// couple of ten-thousandths of a cell, and so allows a thousandth. Between the
/// two slacks one backend samples the edge row and the other reports no
/// coverage — a full-magnitude difference, across a sliver a few hundred metres
/// wide at the edge of a 2° grid. Comparing there compares the two slacks,
/// which is not what this suite is for; the sample generator aims half a cell
/// outside the grid deliberately, so it lands in that sliver eventually.
fn at_a_raster_edge(scene: &Scene, position: LonLat) -> bool {
    scene.rasters.iter().any(|raster| {
        let grid = &raster.grid;
        // Just past either end: inside the slack, but outside the grid.
        let past = |value: f64, last: f64| {
            (last..=last + RASTER_SLACK_CELLS).contains(&value) && value > last
                || (-RASTER_SLACK_CELLS..0.0).contains(&value)
        };
        let fj = (grid.lat0 - position.lat) / grid.dlat;
        let fi = (position.lon - grid.lon0).rem_euclid(360.0) / grid.dlon;
        past(fj, f64::from(grid.nj - 1)) || (!grid.wraps && past(fi, f64::from(grid.ni - 1)))
    })
}

#[test]
fn gpu_and_cpu_agree_within_the_preview_tolerance() {
    let gpu = match GpuEvaluator::new() {
        Ok(gpu) => gpu,
        Err(err) => {
            // A supported configuration: the CPU path is the authority anyway.
            println!("no GPU available ({err}); skipping fidelity comparison");
            return;
        }
    };
    println!("comparing against {}", gpu.adapter);

    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    let mut worst_speed: f64 = 0.0;
    let mut worst_direction: f64 = 0.0;
    let mut skipped_ties = 0usize;
    let mut compared = 0usize;

    for case in 0..120 {
        let scene = scene(&mut rng, 1 + case % 6);
        let points = samples(&mut rng, &scene, 256);

        let expected = CpuEvaluator.evaluate(&scene, &points).expect("cpu");
        let actual = gpu.evaluate(&scene, &points).expect("gpu");
        assert_eq!(expected.len(), actual.len());

        for (index, (want, got)) in expected.iter().zip(&actual).enumerate() {
            assert!(
                got.u.is_finite() && got.v.is_finite(),
                "case {case}: {got:?}"
            );

            // A finite answer is still required of the GPU here; only the
            // comparison is skipped.
            if at_a_tangent_tie(&scene, points[index])
                || at_a_raster_edge(&scene, points[index])
                || at_an_edge_tie(&scene, points[index])
            {
                skipped_ties += 1;
                continue;
            }

            let (want_speed, want_dir) = speed_azimuth_from_uv(*want);
            let (got_speed, got_dir) = speed_azimuth_from_uv(*got);

            let allowed = SPEED_ABS.max(want_speed * SPEED_REL);
            let error = (want_speed - got_speed).abs();
            worst_speed = worst_speed.max(error);
            assert!(
                error <= allowed,
                "case {case} sample {index}: speed {got_speed:.3} vs {want_speed:.3} \
                 at {:?} (allowed {allowed:.3})",
                points[index]
            );

            if want_speed > DIRECTION_FLOOR && got_speed > DIRECTION_FLOOR {
                let delta = bearing_delta(want_dir.degrees(), got_dir.degrees());
                worst_direction = worst_direction.max(delta);
                assert!(
                    delta <= DIRECTION_DEG,
                    "case {case} sample {index}: direction {:.2} vs {:.2} at {:?}",
                    got_dir.degrees(),
                    want_dir.degrees(),
                    points[index]
                );
            }
            compared += 1;
        }
    }

    println!(
        "compared {compared} samples: worst speed error {worst_speed:.4} m/s \
         (tolerance {SPEED_ABS}), worst direction error {worst_direction:.3} deg \
         (tolerance {DIRECTION_DEG}); {skipped_ties} samples not compared at a \
         tie — a path tangent, a raster edge, or an object's own edge"
    );

    // The exemption must stay an exemption. If it ever covers a large share of
    // the samples, it is hiding something rather than stepping around a
    // discontinuity, and the number above stops being worth reading.
    assert!(
        skipped_ties * 20 < compared,
        "{skipped_ties} of {compared} samples were exempted as ties, \
         which is too many for the exemption to be trustworthy"
    );
}

/// Spec 6.3 and 7.8: a warp reads the composite at a displaced position, which
/// needs the recursion a compute shader has not got. It must be declined
/// outright, like the clone stamp, rather than rendered without its warp — a
/// preview that quietly dropped one object would be a proxy for a scene the
/// user does not have.
#[test]
fn a_warp_scene_is_reported_unsupported() {
    let mut rng = Rng(11);
    let mut scene = scene(&mut rng, 2);
    scene.objects[1].modifier = Some(Modifier::Warp(ve_render::scene::Warp::Twist {
        degrees: 45.0,
    }));
    assert!(!ve_render::gpu::supports(&scene));

    // ...and the three that transform the vector where it already is are not
    // declined: they are the reason the distinction is worth drawing.
    for modifier in [
        Modifier::Gain(0.5),
        Modifier::Radial(0.5),
        Modifier::Turn(30.0),
    ] {
        scene.objects[1].modifier = Some(modifier);
        assert!(
            ve_render::gpu::supports(&scene),
            "{modifier:?} needs nothing the shader lacks"
        );
    }

    let Ok(gpu) = GpuEvaluator::new() else {
        println!("no GPU available; skipping the evaluation half");
        return;
    };
    scene.objects[1].modifier = Some(Modifier::Warp(ve_render::scene::Warp::Push {
        x: 100_000.0,
        y: 0.0,
    }));
    assert!(
        gpu.evaluate(&scene, &[LonLat { lon: 0.0, lat: 0.0 }])
            .is_err(),
        "must decline rather than render it wrong"
    );
}

/// The one thing the GPU declines, and it must decline it clearly rather than
/// rendering something wrong.
#[test]
fn a_clone_stamp_scene_is_reported_unsupported() {
    let Ok(gpu) = GpuEvaluator::new() else {
        println!("no GPU available; skipping");
        return;
    };

    let mut rng = Rng(7);
    let mut scene = scene(&mut rng, 2);
    scene.objects[1].clone_source = Some(LonLat { lon: 0.0, lat: 0.0 });

    let outcome = gpu.evaluate(&scene, &[LonLat { lon: 0.0, lat: 0.0 }]);
    assert!(outcome.is_err(), "must decline rather than render it wrong");
    assert!(!ve_render::gpu::supports(&scene));
}

#[test]
fn an_empty_scene_is_calm_on_both_backends() {
    let Ok(gpu) = GpuEvaluator::new() else {
        println!("no GPU available; skipping");
        return;
    };
    let points = vec![
        LonLat { lon: 0.0, lat: 0.0 },
        LonLat {
            lon: 179.0,
            lat: -89.0,
        },
    ];
    assert_eq!(
        gpu.evaluate(&Scene::default(), &points).expect("gpu"),
        CpuEvaluator
            .evaluate(&Scene::default(), &points)
            .expect("cpu")
    );
}

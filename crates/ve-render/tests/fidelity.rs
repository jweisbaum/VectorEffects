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
use ve_core::vector::speed_azimuth_from_uv;
use ve_render::aeqd::{Frame, Space};
use ve_render::cpu::CpuEvaluator;
use ve_render::evaluator::FieldEvaluator;
use ve_render::gpu::GpuEvaluator;
use ve_render::scene::{DirectionMode, EdgeMode, FlatObject, Scene, SpeedMode};
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

    let shape = match rng.index(8) {
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
        _ => Shape::Polygon {
            ring: vec![
                [-extent, -extent * 0.5],
                [extent, -extent * 0.7],
                [extent * 0.4, extent],
                [-extent * 0.6, extent * 0.8],
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

    let direction = match rng.index(5) {
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
        _ => DirectionMode::Tangential {
            clockwise: rng.next() > 0.5,
        },
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

    FlatObject {
        frame: Frame::in_space(anchor, rng.range(0.0, 360.0), rng.range(50.0, 250.0), space),
        cap_radius_m: shape.bounding_radius_m() * 3.0,
        shape,
        speed,
        direction,
        feather: rng.range(0.0, 1.0),
        divergence: rng.range(-1.0, 1.0),
        curl: rng.range(-1.0, 1.0),
        edge_mode: if rng.next() > 0.7 {
            EdgeMode::Replace
        } else {
            EdgeMode::Blend
        },
        gradient_axis: Angle::new(rng.range(0.0, 360.0)),
        gradient_extent: extent * 1.4,
        path: Vec::new(),
        clone_source: None,
    }
}

fn scene(rng: &mut Rng, count: usize) -> Scene {
    Scene {
        objects: (0..count).map(|_| object(rng)).collect(),
    }
}

fn samples(rng: &mut Rng, scene: &Scene, count: usize) -> Vec<LonLat> {
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        // Bias toward the objects, so most samples land where it matters
        // rather than on empty ocean that trivially agrees.
        if !scene.objects.is_empty() && i % 2 == 0 {
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
         (tolerance {DIRECTION_DEG})"
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

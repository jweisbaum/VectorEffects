#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test setup; clippy's allow-in-tests does not reach helpers in tests/"
)]
//! An object's own movement, added to the field it paints (spec.md 9.3, M13).
//!
//! **Every number here is hand-computed, never read off the evaluator.** The
//! whole risk of this feature is a sign or a frame that is quietly wrong in a
//! way that still looks like weather, which is the same class of mistake as an
//! inverted direction convention — so the arithmetic is done in the comments
//! and the code is asked to agree with it.

use ve_core::angle::Angle;
use ve_core::document::{Geometry, Layer, LocalPoint, MotionFlags, Object};
use ve_core::project::{FieldKind, Project, ProjectSettings, Resolution, StepHours};
use ve_core::schema::{PropId, ToolKind};
use ve_core::value::Interpolation;
use ve_core::{LonLat, PropValue};
use ve_render::cpu::sample_scene;
use ve_render::scene::flatten;

/// A 3-hourly project: one step is 10,800 seconds.
fn project(steps: u32) -> Project {
    Project::new(
        "Motion",
        ProjectSettings::new(FieldKind::Wind, Resolution::Deg1, StepHours::H3, steps),
    )
}

/// A single brush stamp painting a constant vector, wide enough to cover the
/// sample points. The brush is the tool with a plain `Direction`; a circle's
/// direction is defined against the bearing from its anchor, which would put
/// a second rotation in the way of reading the motion off.
fn circle(anchor: LonLat, speed_kt: f64, azimuth: f64, radius_km: f64) -> Object {
    let mut object = Object::new(ToolKind::Brush, "Stamp", 8);
    object.geometry = Geometry::Stroke {
        chains: vec![vec![LocalPoint::new(0.0, 0.0)]],
    };
    set(&mut object, PropId::Position, PropValue::LonLat(anchor));
    set(
        &mut object,
        PropId::SizeKm,
        PropValue::F32((radius_km * 2.0) as f32),
    );
    set(&mut object, PropId::Speed, PropValue::F32(speed_kt as f32));
    set(
        &mut object,
        PropId::Direction,
        PropValue::Angle(Angle::new(azimuth)),
    );
    set(&mut object, PropId::Feather, PropValue::F32(0.0));
    object
}

fn set(object: &mut Object, id: PropId, value: PropValue) {
    object
        .props
        .get_mut(id)
        .expect("the tool has the property")
        .set_base(value);
}

fn key(object: &mut Object, id: PropId, step: u32, value: PropValue, interp: Interpolation) {
    object
        .props
        .get_mut(id)
        .expect("the tool has the property")
        .set_key(step, value, interp);
}

/// The field at a position, at a step, through the whole flatten path.
fn at(project: &Project, step: u32, lon: f64, lat: f64) -> (f64, f64) {
    let uv = sample_scene(&flatten(project, step), LonLat::new(lon, lat).unwrap());
    (f64::from(uv.u), f64::from(uv.v))
}

fn with(object: Object) -> Project {
    let mut project = project(8);
    let mut layer = Layer::new("Paint");
    layer.objects.push(object);
    project.layers.push(layer);
    project
}

/// A stroke travelling east adds its own speed eastward: a tailwind
/// strengthens, a headwind weakens, and a crosswind turns.
///
/// 300 km in one 3-hour step is 300_000 / 10_800 = 27.78 m/s, and the wind is
/// 20 m/s — the property is in m/s, as everything below the IPC boundary is.
#[test]
fn a_travelling_object_adds_its_own_velocity() {
    let start = LonLat::new(0.0, 0.0).unwrap();
    // 300 km east of the start, on the equator, where a degree is
    // 6_371_229 · π / 180 = 111_194.9 m.
    let east_300 = 300_000.0 / (6_371_229.0f64 * std::f64::consts::PI / 180.0);

    for (azimuth, expect_u, expect_v, what) in [
        (90.0, 47.78, 0.0, "a tailwind strengthens"),
        (270.0, 7.78, 0.0, "a headwind weakens"),
        (0.0, 27.78, 20.0, "a crosswind turns"),
    ] {
        let mut object = circle(start, 20.0, azimuth, 800.0);
        object.motion = MotionFlags {
            position: true,
            ..Default::default()
        };
        key(
            &mut object,
            PropId::Position,
            0,
            PropValue::LonLat(start),
            Interpolation::Linear,
        );
        key(
            &mut object,
            PropId::Position,
            1,
            PropValue::LonLat(LonLat::new(east_300, 0.0).unwrap()),
            Interpolation::Linear,
        );
        let project = with(object);
        let (u, v) = at(&project, 0, 0.0, 0.0);
        assert!(
            (u - expect_u).abs() < 0.05 && (v - expect_v).abs() < 0.05,
            "{what}: ({u:.2}, {v:.2}), expected ({expect_u:.2}, {expect_v:.2})"
        );
    }

    // Painted northward, the sum is 27.78 east and 20 north: 34.24 m/s on
    // atan2(27.78, 20) = 54.25°.
    let mut object = circle(start, 20.0, 0.0, 800.0);
    object.motion = MotionFlags {
        position: true,
        ..Default::default()
    };
    key(
        &mut object,
        PropId::Position,
        0,
        PropValue::LonLat(start),
        Interpolation::Linear,
    );
    key(
        &mut object,
        PropId::Position,
        1,
        PropValue::LonLat(LonLat::new(east_300, 0.0).unwrap()),
        Interpolation::Linear,
    );
    let project = with(object);
    let (u, v) = at(&project, 0, 0.0, 0.0);
    let speed = u.hypot(v);
    let azimuth = u.atan2(v).to_degrees();
    assert!((speed - 34.24).abs() < 0.05, "speed came out {speed:.2}");
    assert!(
        (azimuth - 54.25).abs() < 0.1,
        "azimuth came out {azimuth:.2}"
    );
}

/// A turn about the anchor is tangential and clockwise.
///
/// 30° per 3-hour step is 0.5236 rad / 10_800 s = 4.848e-5 rad/s. At 200 km
/// that is 4.848e-5 × 200_000 = 9.70 m/s. Due north of the anchor a clockwise
/// turn points east; due east of it, south.
#[test]
fn a_turning_object_adds_a_tangential_velocity() {
    let anchor = LonLat::new(0.0, 0.0).unwrap();
    let mut object = circle(anchor, 0.0, 0.0, 800.0);
    object.motion = MotionFlags {
        rotation: true,
        ..Default::default()
    };
    for (step, degrees) in [(0u32, 0.0f64), (1, 30.0)] {
        key(
            &mut object,
            PropId::RotationDeg,
            step,
            PropValue::Angle(Angle::new(degrees)),
            Interpolation::Linear,
        );
    }
    let project = with(object);
    // 200 km north and 200 km east of the anchor, in degrees at the equator.
    let d = 200_000.0 / (6_371_229.0f64 * std::f64::consts::PI / 180.0);

    let (u, v) = at(&project, 0, 0.0, d);
    assert!(
        (u - 9.70).abs() < 0.05 && v.abs() < 0.05,
        "north of the anchor: ({u:.2}, {v:.2}), expected (9.70, 0)"
    );
    let (u, v) = at(&project, 0, d, 0.0);
    assert!(
        u.abs() < 0.05 && (v + 9.70).abs() < 0.05,
        "east of the anchor: ({u:.2}, {v:.2}), expected (0, -9.70)"
    );
}

/// The same turn, with the object sitting on the antimeridian: the frame is a
/// rotation of the sphere, so the seam is not a case.
#[test]
fn a_turn_across_the_seam_is_the_same_turn() {
    let anchor = LonLat::new(180.0, 0.0).unwrap();
    let mut object = circle(anchor, 0.0, 0.0, 800.0);
    object.motion = MotionFlags {
        rotation: true,
        ..Default::default()
    };
    for (step, degrees) in [(0u32, 0.0f64), (1, 30.0)] {
        key(
            &mut object,
            PropId::RotationDeg,
            step,
            PropValue::Angle(Angle::new(degrees)),
            Interpolation::Linear,
        );
    }
    let project = with(object);
    let d = 200_000.0 / (6_371_229.0f64 * std::f64::consts::PI / 180.0);
    // Due north of an anchor at 180°: still 9.70 m/s east.
    let (u, v) = at(&project, 0, 180.0, d);
    assert!(
        (u - 9.70).abs() < 0.05 && v.abs() < 0.05,
        "on the seam: ({u:.2}, {v:.2})"
    );
    // 200 km east of it, which is longitude -179.2: still 9.70 m/s south.
    let (u, v) = at(&project, 0, -180.0 + d, 0.0);
    assert!(
        u.abs() < 0.05 && (v + 9.70).abs() < 0.05,
        "east of the seam: ({u:.2}, {v:.2})"
    );
}

/// Growth adds a radial velocity: `ṡ/s · r`, outward.
///
/// 100% to 150% over one 3-hour step is a rate of 0.5 / (1.0 × 10_800) =
/// 4.63e-5 per second at step 0, where the scale is 100%. At 200 km that is
/// 9.26 m/s outward.
#[test]
fn a_growing_object_adds_a_radial_velocity() {
    let anchor = LonLat::new(0.0, 0.0).unwrap();
    let mut object = circle(anchor, 0.0, 0.0, 800.0);
    object.motion = MotionFlags {
        scale: true,
        ..Default::default()
    };
    for (step, pct) in [(0u32, 100.0f32), (1, 150.0)] {
        key(
            &mut object,
            PropId::ScalePct,
            step,
            PropValue::F32(pct),
            Interpolation::Linear,
        );
    }
    let project = with(object);
    let d = 200_000.0 / (6_371_229.0f64 * std::f64::consts::PI / 180.0);
    let (u, v) = at(&project, 0, 0.0, d);
    assert!(
        u.abs() < 0.05 && (v - 9.26).abs() < 0.05,
        "north of the anchor: ({u:.2}, {v:.2}), expected (0, 9.26) outward"
    );
    let (u, v) = at(&project, 0, d, 0.0);
    assert!(
        (u - 9.26).abs() < 0.05 && v.abs() < 0.05,
        "east of the anchor: ({u:.2}, {v:.2})"
    );
}

/// Three things that must add nothing: a held segment, an unkeyed property,
/// and a flag left off.
#[test]
fn nothing_moves_that_was_not_asked_to() {
    let start = LonLat::new(0.0, 0.0).unwrap();
    let east_300 = 300_000.0 / (6_371_229.0f64 * std::f64::consts::PI / 180.0);
    let moved = LonLat::new(east_300, 0.0).unwrap();

    let build = |flags: MotionFlags, interp: Interpolation, keyed: bool| {
        let mut object = circle(start, 20.0, 90.0, 800.0);
        object.motion = flags;
        if keyed {
            key(
                &mut object,
                PropId::Position,
                0,
                PropValue::LonLat(start),
                interp,
            );
            key(
                &mut object,
                PropId::Position,
                1,
                PropValue::LonLat(moved),
                interp,
            );
        }
        with(object)
    };
    let on = MotionFlags {
        position: true,
        ..Default::default()
    };

    // A held segment is a teleport, not a wind: 300 km in an hour would
    // otherwise read as 83 m/s.
    let held = build(on, Interpolation::Step, true);
    assert!(
        (at(&held, 0, 0.0, 0.0).0 - 20.0).abs() < 0.05,
        "a held segment"
    );

    // A property with no keys never moves.
    let still = build(on, Interpolation::Linear, false);
    assert!((at(&still, 0, 0.0, 0.0).0 - 20.0).abs() < 0.05, "no keys");

    // And the flag off is the default, which is what every existing project
    // has: the field is exactly what it was.
    let off = build(MotionFlags::default(), Interpolation::Linear, true);
    assert!(
        (at(&off, 0, 0.0, 0.0).0 - 20.0).abs() < 0.05,
        "the flag is off"
    );
}

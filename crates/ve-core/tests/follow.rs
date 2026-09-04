#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! Objects that follow other objects (spec.md 9.3, M13).
//!
//! A follower is a **rigid part of** its primary: it keeps its offset in the
//! primary's frame, so a turning primary carries it round rather than sliding
//! it sideways. That is the whole of what these check, plus the three ways a
//! link can end — unlinked by hand, its primary deleted, or a loop refused.

use ve_core::angle::Angle;
use ve_core::document::{Geometry, Layer, LocalPoint, Object};
use ve_core::follow;
use ve_core::keyframe::{Follow, FollowOffset};
use ve_core::project::{FieldKind, Project, ProjectSettings, Resolution, StepHours};
use ve_core::schema::{PropId, ToolKind};
use ve_core::value::Interpolation;
use ve_core::{Id, LonLat, PropValue};

fn project() -> Project {
    Project::new(
        "Follow",
        ProjectSettings::new(FieldKind::Wind, Resolution::Deg1, StepHours::H3, 12),
    )
}

fn stamp(name: &str, at: LonLat) -> Object {
    let mut object = Object::new(ToolKind::Brush, name, 12);
    object.geometry = Geometry::Stroke {
        chains: vec![vec![LocalPoint::new(0.0, 0.0)]],
    };
    object
        .props
        .get_mut(PropId::Position)
        .expect("position")
        .set_base(PropValue::LonLat(at));
    object
}

fn key(object: &mut Object, id: PropId, step: u32, value: PropValue) {
    object
        .props
        .get_mut(id)
        .expect("the property")
        .set_key(step, value, Interpolation::Linear);
}

fn link(project: &mut Project, follower: Id, primary: Id, prop: PropId, step: u32) {
    let offset = follow::offset_at(project, follower, primary, prop, step).expect("an offset");
    project
        .object_mut(follower)
        .expect("follower")
        .props
        .get_mut(prop)
        .expect("the property")
        .set_follow(Some(Follow { primary, offset }));
}

fn with(objects: Vec<Object>) -> Project {
    let mut project = project();
    let mut layer = Layer::new("Paint");
    layer.objects = objects;
    project.layers.push(layer);
    project
}

fn position_of(project: &Project, id: Id, step: u32) -> LonLat {
    follow::resolve(project, step)
        .of(id)
        .position
        .unwrap_or_else(|| {
            let object = project.object(id).expect("object");
            object
                .props
                .value_at(object.tool, PropId::Position, step)
                .and_then(PropValue::as_lonlat)
                .expect("a position")
        })
}

/// A follower 500 km east of a primary at 45°N stays 500 km from it, on the
/// primary's own bearing, through 3,000 km of travel and a 90° turn.
#[test]
fn a_follower_stays_a_rigid_part_of_its_primary() {
    let start = LonLat::new(0.0, 45.0).unwrap();
    let mut primary = stamp("Primary", start);
    // 3,000 km east over ten steps, and a quarter turn over the same span.
    key(&mut primary, PropId::Position, 0, PropValue::LonLat(start));
    key(
        &mut primary,
        PropId::Position,
        10,
        PropValue::LonLat(start.destination(Angle::new(90.0), 3_000_000.0)),
    );
    key(
        &mut primary,
        PropId::RotationDeg,
        0,
        PropValue::Angle(Angle::new(0.0)),
    );
    key(
        &mut primary,
        PropId::RotationDeg,
        10,
        PropValue::Angle(Angle::new(90.0)),
    );
    let primary_id = primary.id;

    let follower = stamp("Follower", start.destination(Angle::new(90.0), 500_000.0));
    let follower_id = follower.id;

    let mut project = with(vec![primary, follower]);
    link(&mut project, follower_id, primary_id, PropId::Position, 0);

    for step in 0..=10 {
        let p = position_of(&project, primary_id, step);
        let f = position_of(&project, follower_id, step);
        let distance = p.distance_m(f);
        assert!(
            (distance - 500_000.0).abs() < 100.0,
            "step {step}: the follower is {distance:.0} m from its primary"
        );
        // The bearing to the follower turns with the primary: it started due
        // east of an unturned primary, so at 90° of turn it is due south.
        let turned = f64::from(step) * 9.0;
        let bearing = p.initial_bearing(f).degrees();
        let expected = (90.0 + turned) % 360.0;
        let error = ((bearing - expected + 540.0) % 360.0) - 180.0;
        assert!(
            error.abs() < 0.5,
            "step {step}: the follower is on {bearing:.1}°, expected {expected:.1}°"
        );
    }
}

/// A rotation link keeps the difference of the two rotations.
#[test]
fn a_rotation_link_keeps_its_difference() {
    let at = LonLat::new(10.0, 20.0).unwrap();
    let mut primary = stamp("Primary", at);
    key(
        &mut primary,
        PropId::RotationDeg,
        0,
        PropValue::Angle(Angle::new(0.0)),
    );
    key(
        &mut primary,
        PropId::RotationDeg,
        4,
        PropValue::Angle(Angle::new(120.0)),
    );
    let primary_id = primary.id;

    let mut follower = stamp("Follower", at);
    follower
        .props
        .get_mut(PropId::RotationDeg)
        .expect("rotation")
        .set_base(PropValue::Angle(Angle::new(30.0)));
    let follower_id = follower.id;

    let mut project = with(vec![primary, follower]);
    link(
        &mut project,
        follower_id,
        primary_id,
        PropId::RotationDeg,
        0,
    );

    for (step, expected) in [(0u32, 30.0f64), (2, 90.0), (4, 150.0)] {
        let derived = follow::resolve(&project, step)
            .of(follower_id)
            .rotation
            .expect("a derived rotation");
        assert!(
            (derived - expected).abs() < 1e-6,
            "step {step}: rotation came out {derived}"
        );
    }
}

/// A link never moves anything at the moment it is made: the offset is read
/// from where the two objects already are.
#[test]
fn linking_leaves_the_follower_where_it_was() {
    let primary = stamp("Primary", LonLat::new(0.0, 0.0).unwrap());
    let follower = stamp("Follower", LonLat::new(3.0, 1.0).unwrap());
    let (p, f) = (primary.id, follower.id);
    let mut project = with(vec![primary, follower]);
    let was = position_of(&project, f, 0);
    link(&mut project, f, p, PropId::Position, 0);
    let now = position_of(&project, f, 0);
    assert!(
        was.distance_m(now) < 1.0,
        "linking moved the follower from {was:?} to {now:?}"
    );
}

/// A loop has no meaning — every object in it would be defined by the others
/// — so it is refused when it is made.
#[test]
fn a_cycle_is_refused() {
    let a = stamp("A", LonLat::new(0.0, 0.0).unwrap());
    let b = stamp("B", LonLat::new(1.0, 0.0).unwrap());
    let c = stamp("C", LonLat::new(2.0, 0.0).unwrap());
    let d = stamp("D", LonLat::new(3.0, 0.0).unwrap());
    let (ida, idb, idc, idd) = (a.id, b.id, c.id, d.id);
    let mut project = with(vec![a, b, c, d]);

    link(&mut project, idb, ida, PropId::Position, 0);
    link(&mut project, idc, idb, PropId::Position, 0);

    assert!(
        follow::would_cycle(&project, ida, idc, PropId::Position),
        "A following C closes the loop A <- B <- C"
    );
    assert!(
        follow::would_cycle(&project, ida, ida, PropId::Position),
        "an object cannot follow itself"
    );
    assert!(
        follow::would_cycle(&project, idb, idc, PropId::Position),
        "B following C closes the shorter loop B <- C"
    );
    // D follows nothing and nothing follows it, so joining the chain anywhere
    // is a chain and not a loop.
    assert!(!follow::would_cycle(&project, idd, ida, PropId::Position));
    assert!(!follow::would_cycle(&project, idd, idc, PropId::Position));
}

/// A chain resolves in dependency order, however the objects are ordered in
/// the document.
#[test]
fn a_chain_resolves_through_its_links() {
    let start = LonLat::new(0.0, 0.0).unwrap();
    let mut primary = stamp("Primary", start);
    key(&mut primary, PropId::Position, 0, PropValue::LonLat(start));
    key(
        &mut primary,
        PropId::Position,
        2,
        PropValue::LonLat(start.destination(Angle::new(90.0), 1_000_000.0)),
    );
    let middle = stamp("Middle", start.destination(Angle::new(90.0), 200_000.0));
    let end = stamp("End", start.destination(Angle::new(90.0), 400_000.0));
    let (p, m, e) = (primary.id, middle.id, end.id);

    // Deliberately in the wrong order in the document: the resolver must not
    // depend on the layer's ordering.
    let mut project = with(vec![end, middle, primary]);
    link(&mut project, m, p, PropId::Position, 0);
    link(&mut project, e, m, PropId::Position, 0);

    let at2 = |id: Id| position_of(&project, id, 2);
    let (pp, mm, ee) = (at2(p), at2(m), at2(e));
    assert!((pp.distance_m(mm) - 200_000.0).abs() < 100.0);
    assert!((mm.distance_m(ee) - 200_000.0).abs() < 100.0);
    // The whole chain travelled the primary's 1,000 km.
    assert!((start.distance_m(pp) - 1_000_000.0).abs() < 100.0);
}

/// A link to an object that is not there is inert rather than fatal: the
/// property falls back to the follower's own keys.
#[test]
fn a_link_to_a_missing_object_is_inert() {
    let mut follower = stamp("Follower", LonLat::new(5.0, 5.0).unwrap());
    follower
        .props
        .get_mut(PropId::Position)
        .expect("position")
        .set_follow(Some(Follow {
            primary: Id::from_raw(999_999),
            offset: FollowOffset::Position {
                distance_m: 100_000.0,
                bearing_deg: 0.0,
            },
        }));
    let id = follower.id;
    let project = with(vec![follower]);
    let at = position_of(&project, id, 0);
    assert!(
        (at.lon - 5.0).abs() < 1e-9 && (at.lat - 5.0).abs() < 1e-9,
        "a broken link should leave the object on its own keys, got {at:?}"
    );
}

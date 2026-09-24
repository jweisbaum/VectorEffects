#![allow(clippy::unwrap_used, reason = "test fixtures")]
//! Independent field references for the selection-and-move Liquify operator.
use ve_core::{
    LonLat, PropValue,
    angle::Angle,
    document::{Geometry, LocalPoint, Object},
    schema::{PropId, ToolKind},
};
use ve_render::{
    aeqd::{Frame, Space},
    cpu::sample_scene,
    relocate::Transition,
    scene::{Scene, flatten_object},
    sdf::Shape,
};

fn object(tool: ToolKind, anchor: LonLat, space: Space, origin: LonLat) -> Object {
    let mut o = Object::new(tool, "Fixture", 4);
    for (id, value) in [
        (PropId::Position, PropValue::LonLat(anchor)),
        (PropId::StampSpace, PropValue::Enum(space.choice())),
        (PropId::StampOrigin, PropValue::LonLat(origin)),
    ] {
        o.props
            .insert(id, ve_core::keyframe::Animatable::constant(value));
    }
    o
}
fn number(o: &mut Object, id: PropId, value: f32) {
    o.props.get_mut(id).unwrap().set_base(PropValue::F32(value));
}
fn fixture(
    anchor: LonLat,
    space: Space,
    origin: LonLat,
    delta: [f64; 2],
    distance: f32,
) -> (Scene, Scene, Frame) {
    let mut base = object(ToolKind::Brush, anchor, space, origin);
    base.geometry = Geometry::Stroke {
        chains: vec![vec![LocalPoint::new(0.0, 0.0)]],
    };
    number(&mut base, PropId::SizeKm, 20000.0);
    number(&mut base, PropId::Speed, 10.0);
    let mut feature = object(ToolKind::ShapeFill, anchor, space, origin);
    feature.geometry = Geometry::Disc {
        radius_m: Some(90_000.0),
    };
    feature
        .props
        .get_mut(PropId::VectorMode)
        .unwrap()
        .set_base(PropValue::Enum(1));
    number(&mut feature, PropId::SpeedStart, 12.0);
    number(&mut feature, PropId::SpeedEnd, 35.0);
    feature
        .props
        .get_mut(PropId::DirectionStart)
        .unwrap()
        .set_base(PropValue::Angle(Angle::new(0.0)));
    feature
        .props
        .get_mut(PropId::DirectionEnd)
        .unwrap()
        .set_base(PropValue::Angle(Angle::new(90.0)));
    let mut move_ = object(ToolKind::Liquify, anchor, space, origin);
    move_.geometry = Geometry::Stroke {
        chains: vec![vec![LocalPoint::new(0.0, 0.0)]],
    };
    number(&mut move_, PropId::SizeKm, 200.0);
    move_
        .props
        .get_mut(PropId::DisplacementPosition)
        .unwrap()
        .set_base(PropValue::Offset(delta.map(|v| (v / 1000.0) as f32)));
    number(&mut move_, PropId::InterpolationDistanceKm, distance);
    let before = Scene {
        objects: vec![
            flatten_object(&base, 0).unwrap(),
            flatten_object(&feature, 0).unwrap(),
        ],
        ..Default::default()
    };
    let flat = flatten_object(&move_, 0).unwrap();
    let frame = flat.frame;
    let mut after = before.clone();
    after.objects.push(flat);
    (before, after, frame)
}
#[test]
fn exact_interior_healed_hole_and_unchanged_exterior_including_globe_poles_and_dateline() {
    for (origin, anchor, space) in [
        (
            LonLat::new(0.0, 0.0).unwrap(),
            LonLat::new(0.0, 0.0).unwrap(),
            Space::Geodesic,
        ),
        (
            LonLat::new(179.0, 82.0).unwrap(),
            LonLat::new(-179.0, 84.0).unwrap(),
            Space::Geodesic,
        ),
        (
            LonLat::new(179.0, 65.0).unwrap(),
            LonLat::new(179.0, 65.0)
                .unwrap()
                .destination(Angle::new(90.0), 6_500_000.0),
            Space::Orthographic,
        ),
    ] {
        let delta = [-400_000.0, 100_000.0];
        for distance in [0.0, 150.0] {
            let (before, after, frame) = fixture(anchor, space, origin, delta, distance);
            let hash = ve_render::cache::scene_hash(&after);
            assert!(!ve_render::gpu::supports(&after));
            assert!(ve_render::scene::covers(
                after.objects.last().unwrap(),
                frame.to_global(delta)
            ));
            let start = std::time::Instant::now();
            for p in [[0.0, 0.0], [70_000.0, 20_000.0], [-20_000.0, 80_000.0]] {
                let source = sample_scene(&before, frame.to_global(p));
                let moved =
                    sample_scene(&after, frame.to_global([p[0] + delta[0], p[1] + delta[1]]));
                assert!(
                    (source.u - moved.u).abs() < 1e-4 && (source.v - moved.v).abs() < 1e-4,
                    "interior changed: {source:?} -> {moved:?}"
                );
            }
            let healed = sample_scene(&after, frame.to_global([0.0, 0.0]));
            assert!(
                (ve_core::vector::speed_azimuth_from_uv(healed).0 - 10.0).abs() < 0.01,
                "vacated region retains source: {healed:?}"
            );
            for p in [[300_000.0, 300_000.0], [-700_000.0, 300_000.0]] {
                assert_eq!(
                    sample_scene(&before, frame.to_global(p)),
                    sample_scene(&after, frame.to_global(p))
                );
            }
            assert_eq!(ve_render::cache::scene_hash(&after), hash);
            eprintln!(
                "Liquify {space:?} distance {distance}: build and samples {:?}",
                start.elapsed()
            );
        }
    }
}
#[test]
fn overlap_uses_original_source_before_any_displacement() {
    let anchor = LonLat::new(0.0, 0.0).unwrap();
    let (before, after, frame) = fixture(anchor, Space::Geodesic, anchor, [80_000.0, 0.0], 150.0);
    for x in [-70_000.0, 0.0, 70_000.0] {
        assert_eq!(
            sample_scene(&after, frame.to_global([x + 80_000.0, 0.0])),
            sample_scene(&before, frame.to_global([x, 0.0]))
        );
    }
    assert!(
        ve_core::vector::speed_azimuth_from_uv(sample_scene(
            &after,
            frame.to_global([-80_000.0, 0.0])
        ))
        .0 < 35.0
    );
}
#[test]
fn transition_follows_smooth_harmonic_reference_and_fills_missing_source_coverage() {
    let shape = Shape::Disc { radius_m: 100.0 };
    for shift in [400.0, 40_000_000.0] {
        let grid = Transition::build(&shape, [shift, 0.0], 200.0, 0.0, |p| {
            let speed = if shape.distance(p) <= 0.0 { 30.0 } else { 10.0 };
            (ve_core::vector::Uv { u: speed, v: 0.0 }, 1.0)
        });
        // Concentric annulus: the analytic harmonic solution is logarithmic.
        for radius in [120.0f64, 160.0, 200.0, 250.0, 280.0] {
            let (uv, c) = grid.sample([shift, radius]);
            let expected = 10.0 + 20.0 * (300.0 / radius).ln() / 3.0f64.ln();
            assert!(
                (f64::from(uv.u) - expected).abs() < 0.7,
                "{radius}: {} != {expected}",
                uv.u
            );
            assert!((c - 1.0).abs() < 0.001);
        }
    }
    let empty = Transition::build(&shape, [400.0, 0.0], 0.0, 0.0, |_| {
        (ve_core::vector::Uv::default(), 0.0)
    });
    assert_eq!(
        empty.sample([0.0, 0.0]),
        (ve_core::vector::Uv::default(), 0.0)
    );
}

#[test]
fn feather_softens_the_moved_rim_but_preserves_the_core_and_heals_the_source() {
    let anchor = LonLat::new(179.0, 72.0).unwrap();
    for space in [Space::Geodesic, Space::Orthographic] {
        for distance in [0.0, 150.0] {
            let (before, mut after, frame) =
                fixture(anchor, space, anchor, [400_000.0, 0.0], distance);
            // A uniform feature isolates the edge falloff from source gradients.
            after.objects[1].speed = ve_render::scene::SpeedMode::Constant(30.0);
            after.objects[1].direction = ve_render::scene::DirectionMode::Constant(Angle::new(0.0));
            let mut source = before;
            source.objects[1] = after.objects[1].clone();
            for feather in [0.7, 1.0] {
                after.objects.last_mut().unwrap().feather = feather;
                after.objects.last_mut().unwrap().transition = Default::default();
                let hard = sample_scene(&source, frame.to_global([0.0, 0.0]));
                let core = sample_scene(&after, frame.to_global([400_000.0, 0.0]));
                assert_eq!(core, hard, "the feather must not dilute the core");
                let rim = sample_scene(&after, frame.to_global([485_000.0, 0.0]));
                assert!(
                    rim.v > 10.1 && rim.v < 29.5,
                    "the rim should blend: {rim:?}"
                );
                let healed = sample_scene(&after, frame.to_global([0.0, 0.0]));
                assert!(
                    (healed.v - 10.0).abs() < 0.02,
                    "source must be healed even at distance zero: {healed:?}"
                );
                let exterior = frame.to_global([800_000.0, 0.0]);
                assert_eq!(
                    sample_scene(&after, exterior),
                    sample_scene(&source, exterior)
                );
            }
        }
    }
}

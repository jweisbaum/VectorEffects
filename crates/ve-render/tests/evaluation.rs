#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test helpers; clippy's allow-in-tests does not reach tests/"
)]
//! Behaviour of the field evaluator, including M3's acceptance criteria.
//!
//! Objects are built directly rather than through the UI, so these exercise the
//! evaluation semantics themselves: footprint, feather, compositing, and the
//! geodesic behaviour that makes the poles and the dateline ordinary.

use ve_core::angle::Angle;
use ve_core::document::{Geometry, LocalPoint, Object, PathNode};
use ve_core::project::{FieldKind, Project, ProjectSettings, Resolution, StepHours};
use ve_core::schema::{PropId, ToolKind};
use ve_core::vector::speed_azimuth_from_uv;
use ve_core::{LonLat, PropValue};
use ve_render::cpu::sample_scene;
use ve_render::scene::{Scene, flatten, flatten_object};

const STEP: u32 = 0;

fn ll(lon: f64, lat: f64) -> LonLat {
    LonLat::new(lon, lat).unwrap()
}

fn set(object: &mut Object, id: PropId, value: PropValue) {
    object
        .props
        .get_mut(id)
        .expect("property exists")
        .set_base(value);
}

fn set_num(object: &mut Object, id: PropId, value: f32) {
    set(object, id, PropValue::F32(value));
}

/// A brush stroke along `points`, with a hard edge unless feathered.
fn brush(anchor: LonLat, points: Vec<[f64; 2]>, size_km: f32, speed: f32, dir: f64) -> Object {
    let mut object = Object::new(ToolKind::Brush, "brush", 24);
    object.geometry = Geometry::Stroke {
        chains: vec![
            points
                .into_iter()
                .map(|p| LocalPoint::new(p[0], p[1]))
                .collect(),
        ],
    };
    set(&mut object, PropId::Position, PropValue::LonLat(anchor));
    set_num(&mut object, PropId::SizeKm, size_km);
    set_num(&mut object, PropId::Speed, speed);
    set(
        &mut object,
        PropId::Direction,
        PropValue::Angle(Angle::new(dir)),
    );
    set_num(&mut object, PropId::Feather, 0.0);
    object
}

/// A filled disc of `diameter_km` at `anchor`, flowing at `speed`.
fn disc(anchor: LonLat, diameter_km: f32, speed: f32) -> Object {
    let mut object = Object::new(ToolKind::Circle, "disc", 24);
    object.geometry = Geometry::Disc { radius_m: None };
    set(&mut object, PropId::Position, PropValue::LonLat(anchor));
    set_num(&mut object, PropId::DiameterKm, diameter_km);
    set_num(&mut object, PropId::Speed, speed);
    set_num(&mut object, PropId::Feather, 0.0);
    object
}

fn scene_of(objects: Vec<Object>) -> Scene {
    Scene {
        objects: objects
            .iter()
            .filter_map(|o| flatten_object(o, STEP))
            .collect(),
    }
}

fn speed_at(scene: &Scene, position: LonLat) -> f64 {
    speed_azimuth_from_uv(sample_scene(scene, position)).0
}

fn azimuth_at(scene: &Scene, position: LonLat) -> f64 {
    speed_azimuth_from_uv(sample_scene(scene, position))
        .1
        .degrees()
}

/// Distance from `anchor` along `bearing` at which coverage ends, by bisection.
fn edge_distance_m(scene: &Scene, anchor: LonLat, bearing: f64, limit: f64) -> f64 {
    let covered = |d: f64| speed_at(scene, anchor.destination(Angle::new(bearing), d)) > 0.01;
    assert!(covered(1.0), "the anchor itself must be covered");

    let (mut lo, mut hi) = (1.0, limit);
    for _ in 0..60 {
        let mid = (lo + hi) / 2.0;
        if covered(mid) { lo = mid } else { hi = mid }
    }
    (lo + hi) / 2.0
}

// --- Basic coverage ---------------------------------------------------------

#[test]
fn an_empty_scene_is_calm() {
    let scene = Scene::default();
    for lat in [-89.0, -45.0, 0.0, 45.0, 89.0] {
        for lon in [-179.0, -90.0, 0.0, 90.0, 179.0] {
            assert_eq!(
                sample_scene(&scene, ll(lon, lat)),
                ve_core::vector::Uv::default()
            );
        }
    }
}

#[test]
fn a_stroke_writes_inside_and_leaves_the_rest_calm() {
    let anchor = ll(0.0, 0.0);
    let scene = scene_of(vec![brush(anchor, vec![[0.0, 0.0]], 1000.0, 12.0, 90.0)]);

    assert!((speed_at(&scene, anchor) - 12.0).abs() < 1e-4);
    assert!((azimuth_at(&scene, anchor) - 90.0).abs() < 1e-4);

    // Well outside the 500 km radius.
    let far = anchor.destination(Angle::new(90.0), 2_000_000.0);
    assert_eq!(speed_at(&scene, far), 0.0);
}

/// M3 acceptance: a 500 km radius is 500 km on the ground at any latitude.
#[test]
fn an_objects_size_is_the_same_ground_distance_at_any_latitude() {
    for lat in [0.0, 70.0, -70.0, 85.0] {
        let anchor = ll(0.0, lat);
        let scene = scene_of(vec![brush(anchor, vec![[0.0, 0.0]], 1000.0, 10.0, 0.0)]);

        for bearing in [0.0, 90.0, 180.0, 270.0] {
            let edge = edge_distance_m(&scene, anchor, bearing, 2_000_000.0);
            assert!(
                (edge - 500_000.0).abs() < 2_000.0,
                "at {lat}N bearing {bearing}: edge at {edge} m, expected 500 km"
            );
        }
    }
}

// --- The cases that break naive lat/lon implementations ---------------------

/// M3 acceptance: a stroke crossing the dateline renders continuously.
#[test]
fn a_stroke_across_the_antimeridian_is_continuous() {
    let anchor = ll(180.0, 0.0);
    let scene = scene_of(vec![brush(anchor, vec![[0.0, 0.0]], 2000.0, 15.0, 45.0)]);

    // Sweep straight through the seam; coverage must not blink.
    let mut covered = 0;
    for step in 0..=40 {
        let lon = 175.0 + f64::from(step) * 0.25;
        let speed = speed_at(&scene, ll(ve_core::geo::normalize_lon(lon), 0.0));
        if speed > 0.0 {
            covered += 1;
            assert!((speed - 15.0).abs() < 1e-4, "speed dipped at lon {lon}");
        }
    }
    assert!(
        covered > 30,
        "expected a continuous band, got {covered} covered samples"
    );

    // Symmetric about the seam: 179E and 179W are the same distance out.
    let east = speed_at(&scene, ll(179.0, 0.0));
    let west = speed_at(&scene, ll(-179.0, 0.0));
    assert!((east - west).abs() < 1e-6, "{east} != {west}");
}

/// M3 acceptance: a stroke centred on the pole renders without artefacts.
#[test]
fn a_stroke_on_the_pole_covers_every_meridian() {
    let anchor = ll(0.0, 90.0);
    let scene = scene_of(vec![brush(anchor, vec![[0.0, 0.0]], 2000.0, 20.0, 0.0)]);

    // A cap centred on the pole must cover all longitudes equally at a given
    // latitude, and nothing beyond its radius.
    for lon in (-180..180).step_by(15) {
        let inside = speed_at(&scene, ll(f64::from(lon), 85.0));
        assert!((inside - 20.0).abs() < 1e-4, "gap at lon {lon}");

        let outside = speed_at(&scene, ll(f64::from(lon), 70.0));
        assert_eq!(outside, 0.0, "leaked past the radius at lon {lon}");
    }
}

#[test]
fn every_sample_on_the_globe_is_finite() {
    let scene = scene_of(vec![
        brush(
            ll(0.0, 89.0),
            vec![[0.0, 0.0], [400_000.0, 0.0]],
            900.0,
            30.0,
            200.0,
        ),
        brush(ll(179.9, -60.0), vec![[0.0, 0.0]], 1500.0, 18.0, 10.0),
    ]);
    for lat in (-90..=90).step_by(5) {
        for lon in (-180..180).step_by(5) {
            let uv = sample_scene(&scene, ll(f64::from(lon), f64::from(lat)));
            assert!(
                uv.u.is_finite() && uv.v.is_finite(),
                "at {lon},{lat}: {uv:?}"
            );
        }
    }
}

// --- Feather and compositing ------------------------------------------------

/// Decision D12 rests on this: over calm water the two edge modes are the same
/// function, so Blend can be the default without changing existing behaviour.
#[test]
fn blend_and_replace_agree_over_calm() {
    let anchor = ll(20.0, 10.0);
    let mut blend = brush(anchor, vec![[0.0, 0.0]], 1200.0, 20.0, 270.0);
    set_num(&mut blend, PropId::Feather, 0.6);
    let mut replace = blend.clone();
    set(&mut replace, PropId::EdgeMode, PropValue::Enum(1));

    let blend_scene = scene_of(vec![blend]);
    let replace_scene = scene_of(vec![replace]);

    for d in [0.0, 100_000.0, 300_000.0, 500_000.0, 580_000.0] {
        let p = anchor.destination(Angle::new(45.0), d);
        let a = sample_scene(&blend_scene, p);
        let b = sample_scene(&replace_scene, p);
        assert!(
            (a.u - b.u).abs() < 1e-6 && (a.v - b.v).abs() < 1e-6,
            "at {d} m: {a:?} vs {b:?}"
        );
    }
}

/// And over existing wind they differ, which is the reason the default matters.
#[test]
fn blend_and_replace_differ_over_existing_wind() {
    let anchor = ll(0.0, 0.0);
    let background = brush(anchor, vec![[0.0, 0.0]], 6000.0, 10.0, 90.0);

    let mut soft = brush(anchor, vec![[0.0, 0.0]], 1200.0, 30.0, 90.0);
    set_num(&mut soft, PropId::Feather, 0.9);
    let mut hard = soft.clone();
    set(&mut hard, PropId::EdgeMode, PropValue::Enum(1));

    let blend_scene = scene_of(vec![background.clone(), soft]);
    let replace_scene = scene_of(vec![background, hard]);

    // Just inside the small object's edge, where the feather is strongest.
    let p = anchor.destination(Angle::new(0.0), 590_000.0);
    let blended = speed_at(&blend_scene, p);
    let replaced = speed_at(&replace_scene, p);

    assert!(
        blended > replaced + 1.0,
        "blend {blended} should exceed replace {replaced}"
    );
    // Replace fades toward calm; blend fades toward the 10 m/s underneath.
    assert!(
        replaced < 5.0,
        "replace should approach calm, got {replaced}"
    );
    assert!(
        blended > 8.0,
        "blend should approach the background, got {blended}"
    );
}

#[test]
fn a_hard_edge_writes_full_speed_right_up_to_the_boundary() {
    let anchor = ll(0.0, 0.0);
    let scene = scene_of(vec![brush(anchor, vec![[0.0, 0.0]], 1000.0, 25.0, 0.0)]);
    let just_inside = anchor.destination(Angle::new(0.0), 499_000.0);
    assert!((speed_at(&scene, just_inside) - 25.0).abs() < 1e-4);
}

#[test]
fn the_topmost_object_wins() {
    let anchor = ll(0.0, 0.0);
    let lower = brush(anchor, vec![[0.0, 0.0]], 2000.0, 10.0, 90.0);
    let upper = brush(anchor, vec![[0.0, 0.0]], 1000.0, 30.0, 180.0);
    let scene = scene_of(vec![lower, upper]);

    assert!((speed_at(&scene, anchor) - 30.0).abs() < 1e-4);
    assert!((azimuth_at(&scene, anchor) - 180.0).abs() < 1e-4);

    // Outside the upper object, the lower one still shows.
    let outer = anchor.destination(Angle::new(90.0), 800_000.0);
    assert!((speed_at(&scene, outer) - 10.0).abs() < 1e-4);
}

#[test]
fn an_eraser_writes_calm_over_existing_wind() {
    let anchor = ll(0.0, 0.0);
    let background = brush(anchor, vec![[0.0, 0.0]], 4000.0, 20.0, 90.0);

    let mut eraser = Object::new(ToolKind::Eraser, "eraser", 24);
    eraser.geometry = Geometry::Stroke {
        chains: vec![vec![LocalPoint::new(0.0, 0.0)]],
    };
    set(&mut eraser, PropId::Position, PropValue::LonLat(anchor));
    set_num(&mut eraser, PropId::SizeKm, 1000.0);
    set_num(&mut eraser, PropId::Feather, 0.0);

    let scene = scene_of(vec![background, eraser]);
    assert!(
        speed_at(&scene, anchor) < 1e-4,
        "the eraser must leave calm"
    );

    let outside = anchor.destination(Angle::new(0.0), 1_000_000.0);
    assert!(
        (speed_at(&scene, outside) - 20.0).abs() < 1e-4,
        "only inside it"
    );
}

// --- Direction behaviour ----------------------------------------------------

#[test]
fn toward_point_aims_every_vector_at_the_target() {
    let anchor = ll(0.0, 0.0);
    let target = ll(0.0, 40.0);
    let mut object = brush(anchor, vec![[0.0, 0.0]], 3000.0, 10.0, 0.0);
    set(&mut object, PropId::DirectionMode, PropValue::Enum(1));
    set(&mut object, PropId::Target, PropValue::LonLat(target));
    let scene = scene_of(vec![object]);

    // Due south of the anchor, the target is due north.
    let south = anchor.destination(Angle::new(180.0), 500_000.0);
    assert!(
        (azimuth_at(&scene, south) - 0.0).abs() < 0.5,
        "{}",
        azimuth_at(&scene, south)
    );

    // West of the anchor the target is still mostly north: 800 km at the
    // equator is only 7.2 degrees of longitude against 40 degrees of latitude,
    // so the bearing leans only slightly east of north.
    let west = anchor.destination(Angle::new(270.0), 800_000.0);
    let bearing = azimuth_at(&scene, west);
    assert!(
        (2.0..30.0).contains(&bearing),
        "expected just east of north, got {bearing}"
    );
}

/// Spec 7.5: nothing shapes a flow with a radial or tangential component any
/// more. A circle turns about its centre and a shape fill flows the way it was
/// given; neither has anything added on top.
///
/// Checked as a *field* property rather than by looking for absent properties,
/// because that is what the removal was for: a disc's flow is purely
/// tangential, so at any point on it the vector is perpendicular to the bearing
/// from the centre. Any outward or inward component would be divergence by
/// another name, whatever it was called.
#[test]
fn a_circle_flows_purely_about_its_centre() {
    let anchor = ll(0.0, 20.0);
    let scene = scene_of(vec![disc(anchor, 2000.0, 10.0)]);

    for bearing in [0.0, 45.0, 90.0, 180.0, 270.0] {
        let at = anchor.destination(Angle::new(bearing), 400_000.0);
        let uv = sample_scene(&scene, at);
        let (speed, azimuth) = ve_core::vector::speed_azimuth_from_uv(uv);
        assert!(speed > 1.0, "the disc paints nothing at {bearing}");

        // The outward bearing at this point, and the flow's angle to it.
        let outward = anchor.initial_bearing(at).degrees();
        let off = ((azimuth.degrees() - outward + 540.0) % 360.0) - 180.0;
        assert!(
            (off.abs() - 90.0).abs() < 1.0,
            "at {bearing} the flow is {off} degrees off the outward bearing, \
             not the 90 a pure rotation makes"
        );
    }
}

/// A bearing from the anchor is undefined *at* the anchor, and a circle's flow
/// is taken from one. The centre of every circle is therefore the case that has
/// to stay finite — it used to be guarded by an epsilon, which went with the
/// divergence and curl terms it was written for.
#[test]
fn the_centre_of_a_circle_stays_finite() {
    let anchor = ll(0.0, 0.0);
    let scene = scene_of(vec![disc(anchor, 2000.0, 10.0)]);

    let uv = sample_scene(&scene, anchor);
    assert!(uv.u.is_finite() && uv.v.is_finite(), "{uv:?}");

    // ...and just off it, where the bearing is well defined again.
    for metres in [0.5, 1.0, 2.0, 100.0] {
        let near = anchor.destination(Angle::new(37.0), metres);
        let uv = sample_scene(&scene, near);
        assert!(
            uv.u.is_finite() && uv.v.is_finite(),
            "{metres} m out: {uv:?}"
        );
    }
}

// --- Transform --------------------------------------------------------------

#[test]
fn rotation_turns_both_the_shape_and_its_direction() {
    let anchor = ll(0.0, 0.0);
    let stroke = vec![[0.0, 0.0], [800_000.0, 0.0]];

    let upright = scene_of(vec![brush(anchor, stroke.clone(), 200.0, 10.0, 0.0)]);
    let mut turned_object = brush(anchor, stroke, 200.0, 10.0, 0.0);
    set(
        &mut turned_object,
        PropId::RotationDeg,
        PropValue::Angle(Angle::new(90.0)),
    );
    let turned = scene_of(vec![turned_object]);

    // Unrotated the stroke runs east; rotated 90 degrees it runs south.
    let east = anchor.destination(Angle::new(90.0), 600_000.0);
    let south = anchor.destination(Angle::new(180.0), 600_000.0);

    assert!(speed_at(&upright, east) > 0.0 && speed_at(&upright, south) == 0.0);
    assert!(speed_at(&turned, south) > 0.0 && speed_at(&turned, east) == 0.0);

    // And the direction turns with it.
    assert!((azimuth_at(&turned, anchor) - 90.0).abs() < 1e-4);
}

#[test]
fn scale_grows_the_footprint_on_the_ground() {
    let anchor = ll(0.0, 0.0);
    let mut object = brush(anchor, vec![[0.0, 0.0]], 1000.0, 10.0, 0.0);
    set_num(&mut object, PropId::ScalePct, 200.0);
    let scene = scene_of(vec![object]);

    let edge = edge_distance_m(&scene, anchor, 0.0, 4_000_000.0);
    assert!(
        (edge - 1_000_000.0).abs() < 5_000.0,
        "expected 1000 km, got {edge}"
    );
}

// --- Other tools ------------------------------------------------------------

#[test]
fn a_circle_gradient_ramps_from_the_centre_outward() {
    let anchor = ll(0.0, 0.0);
    let mut object = Object::new(ToolKind::Circle, "low", 24);
    object.geometry = Geometry::Disc { radius_m: None };
    set(&mut object, PropId::Position, PropValue::LonLat(anchor));
    set(&mut object, PropId::FillMode, PropValue::Enum(2));
    set_num(&mut object, PropId::DiameterKm, 2000.0);
    set_num(&mut object, PropId::SpeedMin, 0.0);
    set_num(&mut object, PropId::SpeedMax, 40.0);
    set_num(&mut object, PropId::Feather, 0.0);
    let scene = scene_of(vec![object]);

    let centre = speed_at(&scene, anchor);
    let middle = speed_at(&scene, anchor.destination(Angle::new(0.0), 500_000.0));
    let outer = speed_at(&scene, anchor.destination(Angle::new(0.0), 950_000.0));

    assert!(centre < 1.0, "centre should be calm, got {centre}");
    assert!(
        (middle - 20.0).abs() < 1.5,
        "halfway should be about 20, got {middle}"
    );
    assert!(outer > 35.0, "edge should approach 40, got {outer}");
}

/// A rotating circle must flow *around* its anchor, not drift with an
/// accidental default direction.
#[test]
fn a_circle_rotates_around_its_anchor() {
    let anchor = ll(0.0, 0.0);
    let mut object = Object::new(ToolKind::Circle, "low", 24);
    object.geometry = Geometry::Disc { radius_m: None };
    set(&mut object, PropId::Position, PropValue::LonLat(anchor));
    set_num(&mut object, PropId::DiameterKm, 2000.0);
    set_num(&mut object, PropId::Speed, 20.0);
    set_num(&mut object, PropId::Feather, 0.0);
    let scene = scene_of(vec![object]);

    // Clockwise (the default): north of the anchor flows east, east flows south.
    let north = anchor.destination(Angle::new(0.0), 500_000.0);
    assert!(
        (azimuth_at(&scene, north) - 90.0).abs() < 1.0,
        "{}",
        azimuth_at(&scene, north)
    );
    let east = anchor.destination(Angle::new(90.0), 500_000.0);
    assert!(
        (azimuth_at(&scene, east) - 180.0).abs() < 1.0,
        "{}",
        azimuth_at(&scene, east)
    );

    // Speed is the rotation speed, not a diagonal sum of two contributions.
    assert!(
        (speed_at(&scene, north) - 20.0).abs() < 1e-3,
        "{}",
        speed_at(&scene, north)
    );

    // Counter-clockwise reverses it.
    let mut ccw = Object::new(ToolKind::Circle, "high", 24);
    ccw.geometry = Geometry::Disc { radius_m: None };
    set(&mut ccw, PropId::Position, PropValue::LonLat(anchor));
    set_num(&mut ccw, PropId::DiameterKm, 2000.0);
    set_num(&mut ccw, PropId::Speed, 20.0);
    set_num(&mut ccw, PropId::Feather, 0.0);
    set(&mut ccw, PropId::RotationSense, PropValue::Enum(1));
    let scene = scene_of(vec![ccw]);
    assert!(
        (azimuth_at(&scene, north) - 270.0).abs() < 1.0,
        "{}",
        azimuth_at(&scene, north)
    );
}

#[test]
fn a_circle_perimeter_leaves_its_hole_empty() {
    let anchor = ll(0.0, 0.0);
    let mut object = Object::new(ToolKind::Circle, "ring", 24);
    object.geometry = Geometry::Disc { radius_m: None };
    set(&mut object, PropId::Position, PropValue::LonLat(anchor));
    set(&mut object, PropId::FillMode, PropValue::Enum(1));
    set_num(&mut object, PropId::DiameterKm, 2000.0);
    set_num(&mut object, PropId::RingWidthKm, 200.0);
    set_num(&mut object, PropId::Speed, 15.0);
    set_num(&mut object, PropId::Feather, 0.0);
    let scene = scene_of(vec![object]);

    assert_eq!(speed_at(&scene, anchor), 0.0, "the hole must stay empty");
    let on_ring = anchor.destination(Angle::new(0.0), 1_000_000.0);
    assert!((speed_at(&scene, on_ring) - 15.0).abs() < 1e-4);
}

#[test]
fn a_curve_can_follow_its_own_path() {
    let anchor = ll(0.0, 0.0);
    let mut object = Object::new(ToolKind::Curve, "jet", 24);
    object.geometry = Geometry::Path {
        nodes: vec![
            PathNode {
                point: LocalPoint::new(0.0, 0.0),
                in_handle: None,
                out_handle: None,
            },
            PathNode {
                point: LocalPoint::new(1_500_000.0, 0.0),
                in_handle: None,
                out_handle: None,
            },
        ],
    };
    set(&mut object, PropId::Position, PropValue::LonLat(anchor));
    set_num(&mut object, PropId::WidthKm, 400.0);
    set_num(&mut object, PropId::Speed, 22.0);
    set_num(&mut object, PropId::Feather, 0.0);
    // Mode 1 is "relative to the path", with a zero offset: straight along it.
    set(&mut object, PropId::CurveDirectionMode, PropValue::Enum(1));
    set(
        &mut object,
        PropId::Direction,
        PropValue::Angle(Angle::new(0.0)),
    );
    let scene = scene_of(vec![object]);

    let midpoint = anchor.destination(Angle::new(90.0), 700_000.0);
    assert!((speed_at(&scene, midpoint) - 22.0).abs() < 1e-4);
    // The path runs east, so the flow should too.
    let bearing = azimuth_at(&scene, midpoint);
    assert!(
        (bearing - 90.0).abs() < 5.0,
        "expected roughly east, got {bearing}"
    );
}

// --- Project integration ----------------------------------------------------

#[test]
fn hidden_layers_and_inactive_objects_contribute_nothing() {
    let mut project = Project::new(
        "T",
        ProjectSettings::new(FieldKind::Wind, Resolution::Deg1, StepHours::H3, 24),
    );
    let anchor = ll(0.0, 0.0);
    project.layers[0]
        .objects
        .push(brush(anchor, vec![[0.0, 0.0]], 2000.0, 10.0, 90.0));

    assert!(speed_at(&flatten(&project, STEP), anchor) > 0.0);

    project.layers[0].visible = false;
    assert_eq!(
        speed_at(&flatten(&project, STEP), anchor),
        0.0,
        "hidden layer"
    );

    project.layers[0].visible = true;
    project.layers[0].objects[0].active_range = ve_core::StepRange::new(5, 10);
    assert_eq!(
        speed_at(&flatten(&project, STEP), anchor),
        0.0,
        "outside its range"
    );
    assert!(
        speed_at(&flatten(&project, 7), anchor) > 0.0,
        "inside its range"
    );
}

// --- Clone stamp ------------------------------------------------------------

/// A clone stamp centred on `anchor`, copying from `source`.
fn clone(anchor: LonLat, source: LonLat, size_km: f32) -> Object {
    let mut object = Object::new(ToolKind::CloneStamp, "clone", 24);
    object.geometry = Geometry::Stroke {
        chains: vec![vec![LocalPoint::new(0.0, 0.0)]],
    };
    set(&mut object, PropId::Position, PropValue::LonLat(anchor));
    set(&mut object, PropId::SourcePoint, PropValue::LonLat(source));
    set_num(&mut object, PropId::SizeKm, size_km);
    set_num(&mut object, PropId::Feather, 0.0);
    object
}

#[test]
fn a_clone_stamp_copies_the_field_beneath_it() {
    let source = ll(0.0, 0.0);
    let destination = ll(60.0, 0.0);

    // Something distinctive to copy, and an empty patch to copy it into.
    let painted = brush(source, vec![[0.0, 0.0]], 2000.0, 23.0, 135.0);
    let scene = scene_of(vec![painted, clone(destination, source, 1500.0)]);

    assert!(
        (speed_at(&scene, source) - 23.0).abs() < 1e-4,
        "the original"
    );
    assert!(
        (speed_at(&scene, destination) - 23.0).abs() < 1e-4,
        "the copy should carry the same speed, got {}",
        speed_at(&scene, destination)
    );
    assert!(
        (azimuth_at(&scene, destination) - 135.0).abs() < 1e-4,
        "and the same direction, got {}",
        azimuth_at(&scene, destination)
    );
}

/// The stamp reads *below* itself, not above. An object painted on top of the
/// stamp must not feed back into what it copies.
#[test]
fn a_clone_stamp_reads_only_what_is_below_it() {
    let source = ll(0.0, 0.0);
    let destination = ll(60.0, 0.0);

    let scene = scene_of(vec![
        brush(source, vec![[0.0, 0.0]], 2000.0, 10.0, 90.0),
        clone(destination, source, 1500.0),
        // Painted over the source *after* the stamp: invisible to it.
        brush(source, vec![[0.0, 0.0]], 2000.0, 40.0, 270.0),
    ]);

    assert!(
        (speed_at(&scene, source) - 40.0).abs() < 1e-4,
        "the top object wins here"
    );
    assert!(
        (speed_at(&scene, destination) - 10.0).abs() < 1e-4,
        "the copy must show what was below the stamp, got {}",
        speed_at(&scene, destination)
    );
}

#[test]
fn cloning_calm_water_produces_calm() {
    let scene = scene_of(vec![clone(ll(30.0, 30.0), ll(-30.0, -30.0), 1500.0)]);
    assert_eq!(speed_at(&scene, ll(30.0, 30.0)), 0.0);
}

/// A clone stamp has no speed or direction of its own — it copies, it does not
/// invent — so the schema gives it neither.
#[test]
fn a_clone_stamp_has_no_field_of_its_own() {
    let object = clone(ll(120.0, 60.0), ll(0.0, 0.0), 1000.0);
    assert!(
        object.props.get(PropId::Speed).is_none(),
        "a clone stamp must not have a speed property"
    );
    assert!(
        object.props.get(PropId::Direction).is_none(),
        "nor a direction property"
    );

    // And with nothing beneath it, it produces nothing.
    let scene = scene_of(vec![object]);
    assert_eq!(
        speed_at(&scene, ll(120.0, 60.0)),
        0.0,
        "nothing beneath to copy"
    );
}

/// A stamp may copy a patch that itself contains a stamp.
#[test]
fn clone_stamps_can_be_chained() {
    let origin = ll(0.0, 0.0);
    let first = ll(40.0, 0.0);
    let second = ll(80.0, 0.0);

    let scene = scene_of(vec![
        brush(origin, vec![[0.0, 0.0]], 2000.0, 17.0, 45.0),
        clone(first, origin, 1500.0),
        clone(second, first, 1500.0),
    ]);

    assert!((speed_at(&scene, first) - 17.0).abs() < 1e-4, "first copy");
    assert!(
        (speed_at(&scene, second) - 17.0).abs() < 1e-4,
        "a copy of a copy, got {}",
        speed_at(&scene, second)
    );
}

/// Beyond the depth cap the source reads as calm rather than recursing forever.
#[test]
fn chained_clones_stop_at_the_depth_cap() {
    let origin = ll(0.0, 0.0);
    let mut objects = vec![brush(origin, vec![[0.0, 0.0]], 1500.0, 12.0, 0.0)];

    // Six links: more than the cap of four.
    let mut previous = origin;
    let mut positions = Vec::new();
    for i in 1..=6 {
        let next = ll(f64::from(i) * 25.0, 0.0);
        objects.push(clone(next, previous, 1200.0));
        positions.push(next);
        previous = next;
    }
    let scene = scene_of(objects);

    // Early links still resolve.
    assert!(
        (speed_at(&scene, positions[0]) - 12.0).abs() < 1e-4,
        "first link"
    );
    assert!(
        (speed_at(&scene, positions[2]) - 12.0).abs() < 1e-4,
        "third link"
    );

    // Deep ones stop rather than hanging, and every sample stays finite.
    for position in &positions {
        let uv = sample_scene(&scene, *position);
        assert!(uv.u.is_finite() && uv.v.is_finite(), "at {position:?}");
    }
}

/// The copied patch keeps its shape: an offset does not shear it.
#[test]
fn a_clone_preserves_the_shape_of_what_it_copies() {
    let source = ll(0.0, 0.0);
    let destination = ll(50.0, 20.0);

    // A narrow east-west stroke, copied whole.
    let painted = brush(
        source,
        vec![[-600_000.0, 0.0], [600_000.0, 0.0]],
        300.0,
        20.0,
        90.0,
    );
    let scene = scene_of(vec![painted, clone(destination, source, 2000.0)]);

    // Along the copied stroke's axis it is covered; across it, not.
    let along = destination.destination(Angle::new(90.0), 400_000.0);
    let across = destination.destination(Angle::new(0.0), 400_000.0);
    assert!(speed_at(&scene, along) > 19.0, "along the copied stroke");
    assert!(speed_at(&scene, across) < 1.0, "across it should be clear");
}

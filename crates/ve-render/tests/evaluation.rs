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
        rasters: Vec::new(),
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

/// A mask stamp at `anchor`, with a hard edge.
fn mask(anchor: LonLat, size_km: f32, invert: bool) -> Object {
    let mut object = Object::new(ToolKind::Mask, "mask", 24);
    object.geometry = Geometry::Stroke {
        chains: vec![vec![LocalPoint::new(0.0, 0.0)]],
    };
    set(&mut object, PropId::Position, PropValue::LonLat(anchor));
    set_num(&mut object, PropId::SizeKm, size_km);
    set_num(&mut object, PropId::Feather, 0.0);
    set(&mut object, PropId::Invert, PropValue::Bool(invert));
    object
}

#[test]
fn a_mask_writes_calm_over_existing_wind() {
    let anchor = ll(0.0, 0.0);
    let background = brush(anchor, vec![[0.0, 0.0]], 4000.0, 20.0, 90.0);

    let scene = scene_of(vec![background, mask(anchor, 1000.0, false)]);
    assert!(speed_at(&scene, anchor) < 1e-4, "the mask must leave calm");

    let outside = anchor.destination(Angle::new(0.0), 1_000_000.0);
    assert!(
        (speed_at(&scene, outside) - 20.0).abs() < 1e-4,
        "only inside it"
    );
}

/// Spec 6.2: an inverted mask covers everything *except* its footprint, which
/// is how a field is confined to a region rather than cut out of one. Exactly
/// the complement of the same mask uninverted — including out at the far side
/// of the globe, where the spherical-cap cull would otherwise have skipped the
/// object entirely.
#[test]
fn an_inverted_mask_covers_everything_but_its_footprint() {
    let anchor = ll(0.0, 0.0);
    let background = brush(anchor, vec![[0.0, 0.0]], 8000.0, 20.0, 90.0);
    let scene = scene_of(vec![background, mask(anchor, 1000.0, true)]);

    assert!(
        (speed_at(&scene, anchor) - 20.0).abs() < 1e-4,
        "inside an inverted mask the field stands"
    );
    let outside = anchor.destination(Angle::new(0.0), 1_000_000.0);
    assert!(
        speed_at(&scene, outside) < 1e-4,
        "and outside it there is nothing left: {} m/s",
        speed_at(&scene, outside)
    );
    // Past the cap the object would have been culled. It is not.
    let far = anchor.destination(Angle::new(90.0), 3_000_000.0);
    assert!(
        speed_at(&scene, far) < 1e-4,
        "the cap cull skipped an inverted mask: {} m/s at {far:?}",
        speed_at(&scene, far)
    );
}

/// A mask and its inverse are complements: at every cell, what one of them
/// covers the other leaves, and their two weights add to exactly one.
///
/// Read off the field, which is the only place a weight is observable: over a
/// uniform 20 m/s easterly a mask of weight `w` leaves `20(1 - w)`, so the two
/// halves of one edge must always leave 20 m/s *between them* — at the centre,
/// out past the cap, and everywhere across a wide feathered rim, which is where
/// a reflected ramp would show up as a seam if it were wrong.
#[test]
fn a_mask_and_its_inverse_are_exact_complements() {
    let anchor = ll(10.0, -20.0);
    for feather in [0.0f32, 0.5, 1.0] {
        let scene_of_mask = |invert: bool| {
            let background = brush(anchor, vec![[0.0, 0.0]], 8000.0, 20.0, 90.0);
            let mut object = mask(anchor, 1500.0, invert);
            set_num(&mut object, PropId::Feather, feather);
            scene_of(vec![background, object])
        };
        let plain = scene_of_mask(false);
        let inverted = scene_of_mask(true);

        for bearing in [0.0, 90.0, 180.0, 270.0] {
            for distance in [1.0, 400_000.0, 700_000.0, 749_000.0, 751_000.0, 2_000_000.0] {
                let at = anchor.destination(Angle::new(bearing), distance);
                let total = speed_at(&plain, at) + speed_at(&inverted, at);
                assert!(
                    (total - 20.0).abs() < 1e-3,
                    "feather {feather}: the two weights sum to {}, not 1, \
                     at {distance} m on {bearing}",
                    total / 20.0
                );
            }
        }
    }
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

// --- Imported fields --------------------------------------------------------

use std::sync::Arc;

use ve_core::raster::{RasterFrame, RasterGrid, RasterSequence};
use ve_render::scene::FlatRaster;

/// A 5° global lattice whose `u` is the column index and `v` the row index,
/// so a sample's value says which nodes it was blended from.
fn indexed_grid() -> Arc<RasterGrid> {
    let (ni, nj) = (72, 37);
    let mut uv = Vec::new();
    for j in 0..nj {
        for i in 0..ni {
            uv.push([i as f32, j as f32]);
        }
    }
    Arc::new(RasterGrid::new(ni, nj, 0.0, 90.0, 5.0, 5.0, uv).unwrap())
}

/// A uniform regional lattice over the North Atlantic.
fn atlantic(u: f32, v: f32) -> Arc<RasterGrid> {
    Arc::new(RasterGrid::new(41, 31, -60.0, 60.0, 1.0, 1.0, vec![[u, v]; 41 * 31]).unwrap())
}

#[test]
fn an_imported_field_is_sampled_bilinearly_between_its_nodes() {
    let scene = Scene {
        objects: Vec::new(),
        rasters: vec![FlatRaster {
            erased: Vec::new(),
            layer: 0,
            kind: ve_core::project::FieldKind::Wind,
            z: 0,
            grid: indexed_grid(),
            speed_range: None,
        }],
    };
    // Exactly on a node.
    let at = sample_scene(&scene, ll(10.0, 80.0));
    assert_eq!((at.u, at.v), (2.0, 2.0));
    // Halfway between columns 2 and 3, a fifth of the way from row 2 to 3.
    let between = sample_scene(&scene, ll(12.5, 79.0));
    assert!((between.u - 2.5).abs() < 1e-5, "{}", between.u);
    assert!((between.v - 2.2).abs() < 1e-5, "{}", between.v);
    // Across the seam: between column 71 and column 0.
    let seam = sample_scene(&scene, ll(-2.5, 90.0));
    assert!((seam.u - 35.5).abs() < 1e-5, "{}", seam.u);
}

#[test]
fn an_imported_field_overwrites_what_is_beneath_and_yields_to_what_is_above() {
    let inside = ll(-30.0, 45.0);
    let stroke = brush(inside, vec![[0.0, 0.0]], 1000.0, 20.0, 180.0);
    let painted = flatten_object(&stroke, STEP).unwrap();

    // Raster above the stroke: the raster wins.
    let above = Scene {
        objects: vec![painted.clone()],
        rasters: vec![FlatRaster {
            erased: Vec::new(),
            layer: 0,
            kind: ve_core::project::FieldKind::Wind,
            z: 1,
            grid: atlantic(5.0, 0.0),
            speed_range: None,
        }],
    };
    let s = sample_scene(&above, inside);
    assert!((s.u - 5.0).abs() < 1e-5 && s.v.abs() < 1e-5, "{s:?}");

    // Raster beneath the stroke: the stroke wins inside its footprint, and
    // the raster shows through outside it.
    let beneath = Scene {
        objects: vec![painted],
        rasters: vec![FlatRaster {
            erased: Vec::new(),
            layer: 0,
            kind: ve_core::project::FieldKind::Wind,
            z: 0,
            grid: atlantic(5.0, 0.0),
            speed_range: None,
        }],
    };
    let s = sample_scene(&beneath, inside);
    assert!((s.v + 20.0).abs() < 1e-3 && s.u.abs() < 1e-3, "{s:?}");
    let s = sample_scene(&beneath, ll(-50.0, 40.0));
    assert!((s.u - 5.0).abs() < 1e-5, "{s:?}");

    // Outside the regional grid the field beneath is untouched: calm here.
    let s = sample_scene(&beneath, ll(120.0, 0.0));
    assert_eq!((s.u, s.v), (0.0, 0.0));
}

#[test]
fn a_raster_beneath_a_clone_stamp_is_what_the_stamp_copies() {
    let anchor = ll(-40.0, 45.0);
    let source = ll(-30.0, 40.0);
    let stamp = flatten_object(&clone(anchor, source, 500.0), STEP).unwrap();
    let scene = Scene {
        objects: vec![stamp],
        rasters: vec![FlatRaster {
            erased: Vec::new(),
            layer: 0,
            kind: ve_core::project::FieldKind::Wind,
            z: 0,
            grid: atlantic(3.0, 4.0),
            speed_range: None,
        }],
    };
    let s = sample_scene(&scene, anchor);
    assert!(
        (s.u - 3.0).abs() < 1e-5 && (s.v - 4.0).abs() < 1e-5,
        "{s:?}"
    );
}

/// A 3-hourly file in an hourly project, and an hourly file in a 3-hourly one:
/// each step shows the message valid *at* its forecast hour, and nothing at a
/// step the file has no message for (spec.md 4.8).
#[test]
fn a_step_shows_only_the_message_valid_at_its_own_hour() {
    let sequence = |offsets: &[f64]| {
        let frames = offsets
            .iter()
            .map(|&h| RasterFrame {
                offset_hours: h,
                valid_unix_s: (h * 3600.0) as i64,
                grid: atlantic(h as f32, 0.0),
            })
            .collect();
        Arc::new(RasterSequence::new(FieldKind::Wind, frames).unwrap())
    };
    let at = ll(-30.0, 45.0);
    let u_at = |project: &Project, step: u32| sample_scene(&flatten(project, step), at).u;

    let mut hourly = Project::new(
        "hourly",
        ProjectSettings::new(FieldKind::Wind, Resolution::Deg1, StepHours::H1, 12),
    );
    hourly.layers[0].raster = Some(sequence(&[0.0, 3.0, 6.0]));
    // Hours 1, 2, 4, 5, 7, 8 fall between messages: nothing there.
    let seen: Vec<f32> = (0..9).map(|s| u_at(&hourly, s)).collect();
    assert_eq!(seen, vec![0.0, 0.0, 0.0, 3.0, 0.0, 0.0, 6.0, 0.0, 0.0]);
    assert_eq!(
        u_at(&hourly, 11),
        0.0,
        "past the file's end there is no imported field at all"
    );

    let mut three_hourly = Project::new(
        "3h",
        ProjectSettings::new(FieldKind::Wind, Resolution::Deg1, StepHours::H3, 4),
    );
    three_hourly.layers[0].raster = Some(sequence(&[0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0]));
    // 0, 3 and 6 are read; the messages between them are never shown, and the
    // fourth step at 9 h is past the file.
    let seen: Vec<f32> = (0..4).map(|s| u_at(&three_hourly, s)).collect();
    assert_eq!(seen, vec![0.0, 3.0, 6.0, 0.0]);
}

#[test]
fn a_hidden_grib_layer_contributes_nothing_and_a_missing_file_is_calm() {
    let mut project = Project::new(
        "T",
        ProjectSettings::new(FieldKind::Wind, Resolution::Deg1, StepHours::H3, 4),
    );
    let at = ll(-30.0, 45.0);
    let frame = RasterFrame {
        offset_hours: 0.0,
        valid_unix_s: 0,
        grid: atlantic(7.0, 0.0),
    };
    project.layers[0].raster = Some(Arc::new(
        RasterSequence::new(FieldKind::Wind, vec![frame]).unwrap(),
    ));
    assert_eq!(sample_scene(&flatten(&project, 0), at).u, 7.0);
    project.layers[0].visible = false;
    assert_eq!(sample_scene(&flatten(&project, 0), at).u, 0.0);
    // A GRIB layer whose file could not be read carries no raster.
    project.layers[0].visible = true;
    project.layers[0].raster = None;
    assert_eq!(sample_scene(&flatten(&project, 0), at).u, 0.0);
}

// --- Modifiers (spec.md 6.3) ------------------------------------------------
//
// A modifier has no field of its own: it reads the composite beneath it and
// writes back a transformed version. Every assertion here is against a
// hand-computed vector rather than against what the evaluator happens to
// produce — the whole point of these tools is that the arithmetic is
// predictable.

/// A one-stamp modifier stroke centred on `anchor`, with a hard edge.
///
/// A modifier is painted like the brush, so a single stamp is a chain of one
/// point — the same construction a click with the brush produces.
fn modifier(tool: ToolKind, anchor: LonLat, size_km: f32) -> Object {
    let mut object = Object::new(tool, "modifier", 24);
    object.geometry = Geometry::Stroke {
        chains: vec![vec![LocalPoint::new(0.0, 0.0)]],
    };
    set(&mut object, PropId::Position, PropValue::LonLat(anchor));
    set_num(&mut object, PropId::SizeKm, size_km);
    set_num(&mut object, PropId::Feather, 0.0);
    object
}

/// Spec 6.3: the intensity modifier scales the speed beneath it and leaves the
/// direction alone. +100% is twice as fast, -50% is half, -100% is calm.
#[test]
fn intensify_scales_the_speed_beneath_it_and_reduce_takes_it_back() {
    let anchor = ll(0.0, 0.0);
    for (percent, expected) in [(100.0, 40.0), (-50.0, 10.0), (-100.0, 0.0), (0.0, 20.0)] {
        let background = brush(anchor, vec![[0.0, 0.0]], 4000.0, 20.0, 90.0);
        let mut gain = modifier(ToolKind::Intensity, anchor, 1000.0);
        set_num(&mut gain, PropId::Gain, percent);

        let scene = scene_of(vec![background, gain]);
        assert!(
            (speed_at(&scene, anchor) - expected).abs() < 1e-3,
            "{percent}% of 20 m/s should be {expected}, got {}",
            speed_at(&scene, anchor)
        );
        if expected > 0.0 {
            assert!(
                (azimuth_at(&scene, anchor) - 90.0).abs() < 1e-3,
                "the direction is not the intensity's business"
            );
        }
        // And only inside its footprint.
        let outside = anchor.destination(Angle::new(0.0), 900_000.0);
        assert!((speed_at(&scene, outside) - 20.0).abs() < 1e-3);
    }
}

/// A modifier cannot conjure a field: every one of them is a function of what
/// it was handed, so over calm water it hands calm back. This is the line
/// between a modifier and a tool that paints (spec.md 6.3).
#[test]
fn a_modifier_over_calm_water_leaves_calm_water() {
    let anchor = ll(20.0, -10.0);
    for (tool, prop, amount) in [
        (ToolKind::Intensity, PropId::Gain, 300.0),
        (ToolKind::Divergence, PropId::Radial, 300.0),
        (ToolKind::Turn, PropId::TurnAmountDeg, 90.0),
    ] {
        let mut object = modifier(tool, anchor, 2000.0);
        set_num(&mut object, prop, amount);
        let scene = scene_of(vec![object]);
        assert!(
            speed_at(&scene, anchor) < 1e-6,
            "{tool:?} painted {} m/s onto empty ocean",
            speed_at(&scene, anchor)
        );
    }

    // The warp too, which reads elsewhere rather than transforming in place.
    let mut warp = modifier(ToolKind::Warp, anchor, 2000.0);
    set(
        &mut warp,
        PropId::PushTo,
        PropValue::LonLat(anchor.destination(Angle::new(90.0), 400_000.0)),
    );
    assert!(speed_at(&scene_of(vec![warp]), anchor) < 1e-6);
}

/// Spec 6.3: diverging adds a component pointing away from the anchor, at a
/// fraction of the local speed; converging adds the same component inward.
///
/// Hand-computed: a 10 m/s northward flow, sampled due east of the anchor where
/// "outward" is a bearing of 90°, plus 100% of 10 m/s outward is (10, 10) —
/// 14.142 m/s on a bearing of 45°. Converging by the same amount gives
/// (-10, 10): the same speed, 45° the other side of north.
#[test]
fn diverging_bends_the_flow_outward_and_converging_bends_it_in() {
    let anchor = ll(0.0, 0.0);
    let east = anchor.destination(Angle::new(90.0), 300_000.0);

    for (percent, expected_azimuth) in [(100.0, 45.0), (-100.0, 315.0)] {
        let background = brush(anchor, vec![[0.0, 0.0]], 4000.0, 10.0, 0.0);
        let mut radial = modifier(ToolKind::Divergence, anchor, 2000.0);
        set_num(&mut radial, PropId::Radial, percent);

        let scene = scene_of(vec![background, radial]);
        let speed = speed_at(&scene, east);
        let azimuth = azimuth_at(&scene, east);
        assert!(
            (speed - 200.0f64.sqrt()).abs() < 0.05,
            "{percent}%: speed {speed}, expected {}",
            200.0f64.sqrt()
        );
        assert!(
            (azimuth - expected_azimuth).abs() < 0.5,
            "{percent}%: azimuth {azimuth}, expected {expected_azimuth}"
        );
    }
}

/// Spec 8.1 (M29): an erasure takes away what it covers and nothing else.
/// A stamp of 400 km at 1,000 km east of a 4,000 km disc's centre leaves the
/// disc's centre and its western half exactly as they were, and where the
/// stamp is nothing writes at all — undefined, not calm.
#[test]
fn an_erasure_removes_what_it_covers_and_nothing_else() {
    let anchor = ll(0.0, 0.0);
    let mut object = brush(anchor, vec![[0.0, 0.0]], 4000.0, 12.0, 90.0);
    object.erased.push(ve_core::document::Erasure {
        chains: vec![vec![LocalPoint {
            x: 1_000_000.0,
            y: 0.0,
        }]],
        radius_m: 400_000.0,
        square: false,
        feather: 0.0,
        step: None,
    });
    let scene = scene_of(vec![object]);
    let east = anchor.destination(Angle::new(90.0), 1_000_000.0);
    let west = anchor.destination(Angle::new(270.0), 1_000_000.0);
    assert!(
        ve_render::cpu::sample_scene_covered(&scene, east).is_none(),
        "under the eraser nothing writes"
    );
    assert!(
        (speed_at(&scene, anchor) - 12.0).abs() < 1e-3,
        "the centre is untouched"
    );
    assert!(
        (speed_at(&scene, west) - 12.0).abs() < 1e-3,
        "and so is the far side"
    );
    // Just outside the stamp's rim the stroke is whole again.
    let rim = anchor.destination(Angle::new(90.0), 1_450_000.0);
    assert!((speed_at(&scene, rim) - 12.0).abs() < 1e-3);
}

/// An erasure made with the frame held applies at that step alone (M29).
#[test]
fn a_step_erasure_applies_at_its_step_alone() {
    let anchor = ll(0.0, 0.0);
    let mut object = brush(anchor, vec![[0.0, 0.0]], 4000.0, 12.0, 90.0);
    object.erased.push(ve_core::document::Erasure {
        chains: vec![vec![LocalPoint { x: 0.0, y: 0.0 }]],
        radius_m: 3_000_000.0,
        square: true,
        feather: 0.0,
        step: Some(3),
    });
    let at = |step: u32| Scene {
        rasters: Vec::new(),
        objects: flatten_object(&object, step).into_iter().collect(),
    };
    assert!(
        ve_render::cpu::sample_scene_covered(&at(3), anchor).is_none(),
        "gone at the step it was erased on"
    );
    assert!(
        (speed_at(&at(4), anchor) - 12.0).abs() < 1e-3,
        "there on the next"
    );
    assert!(
        (speed_at(&at(2), anchor) - 12.0).abs() < 1e-3,
        "and on the one before"
    );
}

/// Spec 6.3: the turn modifier rotates every vector beneath it by a fixed
/// angle, clockwise or counter-clockwise as its sense says (M29), and does
/// not touch the speed.
#[test]
fn rotating_turns_the_flow_by_the_angle_it_is_given() {
    let anchor = ll(-30.0, 40.0);
    for (turn, expected) in [(30.0, 120.0), (-30.0, 60.0), (180.0, 270.0)] {
        let background = brush(anchor, vec![[0.0, 0.0]], 4000.0, 12.0, 90.0);
        let mut object = modifier(ToolKind::Turn, anchor, 1500.0);
        set_num(&mut object, PropId::TurnAmountDeg, f32::abs(turn));
        set(
            &mut object,
            PropId::TurnSense,
            PropValue::Enum(u8::from(turn < 0.0)),
        );

        let scene = scene_of(vec![background, object]);
        assert!(
            (speed_at(&scene, anchor) - 12.0).abs() < 1e-3,
            "a turn is not a change of speed"
        );
        let azimuth = azimuth_at(&scene, anchor);
        assert!(
            (azimuth - expected).abs() < 1e-2,
            "turning 90° by {turn}° should give {expected}, got {azimuth}"
        );
    }
}

/// Spec 7.6: a modifier reads what is *beneath* it in z-order, so one placed
/// below an object does not touch it.
#[test]
fn a_modifier_only_changes_what_is_below_it() {
    let anchor = ll(0.0, 0.0);
    let background = brush(anchor, vec![[0.0, 0.0]], 4000.0, 20.0, 90.0);
    let mut gain = modifier(ToolKind::Intensity, anchor, 2000.0);
    set_num(&mut gain, PropId::Gain, 100.0);

    let above = scene_of(vec![background.clone(), gain.clone()]);
    let below = scene_of(vec![gain, background]);
    assert!((speed_at(&above, anchor) - 40.0).abs() < 1e-3);
    assert!(
        (speed_at(&below, anchor) - 20.0).abs() < 1e-3,
        "a modifier beneath an object must not reach up into it"
    );
}

/// Spec 6.3: a warp displaces the position the field is read from, so the patch
/// beneath it moves. Hand-computed: a 500 km disc of wind at the anchor, and a
/// warp pushing 600 km east, means the wind is found 600 km east of where it
/// was painted and no longer at the anchor.
#[test]
fn a_warp_pushes_the_field_beneath_it_along_a_bearing() {
    let anchor = ll(0.0, 0.0);
    let background = brush(anchor, vec![[0.0, 0.0]], 1000.0, 15.0, 0.0);
    // Where the patch should end up: 600 km east of where it was painted.
    let moved_to = anchor.destination(Angle::new(90.0), 600_000.0);
    let mut warp = modifier(ToolKind::Warp, anchor, 6000.0);
    set(&mut warp, PropId::WarpMode, PropValue::Enum(0));
    set(&mut warp, PropId::PushTo, PropValue::LonLat(moved_to));

    let before = scene_of(vec![background.clone()]);
    let after = scene_of(vec![background, warp]);

    assert!(
        speed_at(&before, moved_to) < 1e-6 && (speed_at(&before, anchor) - 15.0).abs() < 1e-3,
        "the unwarped patch is at the anchor and nowhere else"
    );
    assert!(
        (speed_at(&after, moved_to) - 15.0).abs() < 0.2,
        "the warp did not carry the patch east: {} m/s there",
        speed_at(&after, moved_to)
    );
    assert!(
        speed_at(&after, anchor) < 0.2,
        "and it did not leave a copy behind: {} m/s",
        speed_at(&after, anchor)
    );
}

/// A twist rotates the field about the warp's anchor. Hand-computed: a patch
/// due north of the anchor, twisted 90° clockwise, is found due east.
#[test]
fn a_warp_twists_the_field_about_its_anchor() {
    let anchor = ll(0.0, 0.0);
    let north = anchor.destination(Angle::new(0.0), 800_000.0);
    let east = anchor.destination(Angle::new(90.0), 800_000.0);
    let background = brush(north, vec![[0.0, 0.0]], 600.0, 18.0, 45.0);

    let mut warp = modifier(ToolKind::Warp, anchor, 6000.0);
    set(&mut warp, PropId::WarpMode, PropValue::Enum(1));
    set_num(&mut warp, PropId::TwistDeg, 90.0);

    let scene = scene_of(vec![background, warp]);
    assert!(
        (speed_at(&scene, east) - 18.0).abs() < 0.5,
        "a 90° twist should have carried the patch from north to east: {} m/s",
        speed_at(&scene, east)
    );
    assert!(
        speed_at(&scene, north) < 0.5,
        "and away from where it was: {} m/s",
        speed_at(&scene, north)
    );
}

/// Spec 6.3: a modifier fades out with its feather, so it has no visible edge
/// of its own — at the rim of its footprint the field is exactly what it was.
#[test]
fn a_modifier_fades_to_nothing_at_the_edge_of_its_feather() {
    let anchor = ll(0.0, 0.0);
    let background = brush(anchor, vec![[0.0, 0.0]], 8000.0, 20.0, 90.0);
    let mut gain = modifier(ToolKind::Intensity, anchor, 2000.0);
    set_num(&mut gain, PropId::Gain, 100.0);
    set_num(&mut gain, PropId::Feather, 1.0);

    let scene = scene_of(vec![background, gain]);
    // Fully inside: doubled. At the rim: untouched. Between: in between.
    assert!((speed_at(&scene, anchor) - 40.0).abs() < 1e-3);
    let rim = anchor.destination(Angle::new(45.0), 999_000.0);
    assert!(
        (speed_at(&scene, rim) - 20.0).abs() < 0.2,
        "the modifier showed its own edge: {} m/s at the rim",
        speed_at(&scene, rim)
    );
    let middle = anchor.destination(Angle::new(45.0), 700_000.0);
    let half = speed_at(&scene, middle);
    assert!(
        half > 20.5 && half < 39.5,
        "the feather should ramp, not switch: {half} m/s"
    );
}

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! The brush's shape and direction options, end to end (spec.md 6.2).
//!
//! Painted through the same command the map calls, then read back out of the
//! authoritative CPU evaluator — so these cover the whole path from the wire
//! type to the field, not just the property that got written.

use ve_app::commands::AppState;
use ve_app::edit::{self, BrushDirectionMode, BrushShape, BrushStroke, StampSpace};
use ve_app::error::AppError;
use ve_app::paths::AppPaths;
use ve_app::projects::{self, NewProjectRequest};
use ve_core::LonLat;
use ve_core::angle::Angle;

struct TempRoot(std::path::PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ve-brush-{}-{label}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("temp root");
        Self(dir)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn project(label: &str) -> (TempRoot, AppState) {
    let root = TempRoot::new(label);
    let state = AppState::new(AppPaths::in_directory(&root.0).expect("paths"));
    projects::create(
        &state,
        NewProjectRequest {
            name: "Brush".to_owned(),
            field_kind: "wind".to_owned(),
            resolution: "1.0".to_owned(),
            step_hours: 3,
            step_count: 4,
        },
        false,
    )
    .expect("create");
    (root, state)
}

fn ll(lon: f64, lat: f64) -> LonLat {
    LonLat::new(lon, lat).expect("position")
}

/// Speed and azimuth-toward at a position, from the authoritative evaluator.
fn sample(state: &AppState, position: LonLat) -> (f64, f64) {
    let project = {
        let mut session = state.session.lock().expect("lock");
        session.require_open().expect("open").project.clone()
    };
    let scene = ve_render::scene::flatten(&project, 0);
    let uv = ve_render::cpu::sample_scene(&scene, position);
    let (speed, azimuth) = ve_core::vector::speed_azimuth_from_uv(uv);
    (speed, azimuth.degrees())
}

/// A one-click stroke at the origin, so the footprint is a single stamp.
fn stamp(shape: BrushShape, size_km: f32) -> BrushStroke {
    BrushStroke {
        points: vec![[0.0, 0.0]],
        size_km,
        speed_mps: 20.0,
        direction_toward_deg: 90.0,
        // A hard edge: coverage is then the only thing a sampled speed reports.
        feather: 0.0,
        shape,
        ..Default::default()
    }
}

// --- Brush shape ------------------------------------------------------------

/// The stamp is a square of side `size_km`, so its corner sits at
/// `(half, half)` in the object's local frame — inside a square of half-side
/// 500 km, and outside the disc of radius 500 km that the round brush paints.
#[test]
fn a_square_brush_paints_the_corner_a_round_one_misses() {
    // 480 km along both local axes: comfortably inside the square, and 679 km
    // from the anchor, which is well outside the 500 km disc.
    let diagonal_m = 480_000.0_f64.hypot(480_000.0);
    let corner = ll(0.0, 0.0).destination(Angle::new(45.0), diagonal_m);

    let (_root, round) = project("round-corner");
    edit::paint(&round, stamp(BrushShape::Circle, 1000.0)).expect("paint");
    assert_eq!(
        sample(&round, corner).0,
        0.0,
        "a round brush does not reach its bounding box's corner"
    );

    let (_root, square) = project("square-corner");
    edit::paint(&square, stamp(BrushShape::Square, 1000.0)).expect("paint");
    assert!(
        (sample(&square, corner).0 - 20.0).abs() < 1e-6,
        "a square brush covers the corner at full speed"
    );
}

/// Size is a diameter for the disc and a side for the square, so the two agree
/// across the flats and differ only at the corners.
#[test]
fn both_shapes_are_the_same_size_across_the_flats() {
    let inside = ll(0.0, 0.0).destination(Angle::new(90.0), 480_000.0);
    let outside = ll(0.0, 0.0).destination(Angle::new(90.0), 520_000.0);

    for shape in [BrushShape::Circle, BrushShape::Square] {
        let (_root, state) = project("flats");
        edit::paint(&state, stamp(shape, 1000.0)).expect("paint");
        assert!(
            sample(&state, inside).0 > 0.0,
            "{shape:?} must cover 480 km out from a 1000 km stamp"
        );
        assert_eq!(
            sample(&state, outside).0,
            0.0,
            "{shape:?} must not cover 520 km out from a 1000 km stamp"
        );
    }
}

/// Two strokes that differ only in brush shape look different, so absorbing
/// one into the other would change the layer (spec.md 6.1).
#[test]
fn a_square_stroke_does_not_merge_into_a_round_one() {
    let (_root, state) = project("merge");
    edit::paint(&state, stamp(BrushShape::Circle, 1000.0)).expect("paint");
    edit::paint(&state, stamp(BrushShape::Square, 1000.0)).expect("paint");

    let tree = ve_app::document::tree(&state, 0).expect("tree");
    assert_eq!(
        tree.layers[0].objects.len(),
        2,
        "different shapes must stay separate objects"
    );

    // The control: the same stroke twice does still merge, so the assertion
    // above is about the shape and not about merging having stopped working.
    let (_root, state) = project("merge-control");
    edit::paint(&state, stamp(BrushShape::Square, 1000.0)).expect("paint");
    edit::paint(&state, stamp(BrushShape::Square, 1000.0)).expect("paint");
    let tree = ve_app::document::tree(&state, 0).expect("tree");
    assert_eq!(tree.layers[0].objects.len(), 1);
}

// --- Direction mode ---------------------------------------------------------

/// Aiming at the north pole must produce due north everywhere, whatever the
/// longitude — a reference that needs no bearing formula to check.
#[test]
fn aiming_at_the_pole_points_every_vector_north() {
    let (_root, state) = project("pole");
    edit::paint(
        &state,
        BrushStroke {
            points: vec![[-40.0, 10.0], [0.0, 10.0], [40.0, 10.0]],
            size_km: 600.0,
            speed_mps: 20.0,
            // Deliberately not north: the constant bearing must be ignored.
            direction_toward_deg: 180.0,
            feather: 0.0,
            direction_mode: BrushDirectionMode::TowardPoint,
            target: Some([0.0, 90.0]),
            ..Default::default()
        },
    )
    .expect("paint");

    for lon in [-40.0, -20.0, 0.0, 20.0, 40.0] {
        let (speed, azimuth) = sample(&state, ll(lon, 10.0));
        assert!(speed > 0.0, "the stroke covers {lon}");
        let off_north = (azimuth + 180.0) % 360.0 - 180.0;
        assert!(
            off_north.abs() < 1e-6,
            "at {lon} the vector points {azimuth}, not north"
        );
    }
}

/// Along the equator the initial bearing to a point due east is exactly 90.
#[test]
fn aiming_along_the_equator_points_due_east() {
    let (_root, state) = project("equator");
    edit::paint(
        &state,
        BrushStroke {
            points: vec![[0.0, 0.0]],
            size_km: 600.0,
            speed_mps: 12.0,
            direction_toward_deg: 0.0,
            feather: 0.0,
            direction_mode: BrushDirectionMode::TowardPoint,
            target: Some([30.0, 0.0]),
            ..Default::default()
        },
    )
    .expect("paint");

    let (speed, azimuth) = sample(&state, ll(0.0, 0.0));
    assert!((speed - 12.0).abs() < 1e-6);
    assert!((azimuth - 90.0).abs() < 1e-6, "got {azimuth}");
}

/// A constant-direction stroke keeps its bearing, and ignores any target that
/// came along with it.
/// Aiming away from the pole points every vector due south, wherever on the
/// stroke it is measured: the reciprocal of aiming at it.
#[test]
fn aiming_away_from_the_pole_points_every_vector_south() {
    let (_root, state) = project("away-pole");
    edit::paint(
        &state,
        BrushStroke {
            points: vec![[-40.0, 10.0], [0.0, 10.0], [40.0, 10.0]],
            size_km: 600.0,
            speed_mps: 20.0,
            // Deliberately not south: the constant bearing must be ignored.
            direction_toward_deg: 0.0,
            feather: 0.0,
            direction_mode: BrushDirectionMode::AwayFromPoint,
            target: Some([0.0, 90.0]),
            ..Default::default()
        },
    )
    .expect("paint");

    for lon in [-40.0, -20.0, 0.0, 20.0, 40.0] {
        let (speed, azimuth) = sample(&state, ll(lon, 10.0));
        assert!(speed > 0.0, "the stroke covers {lon}");
        let off_south = (azimuth - 180.0 + 180.0).rem_euclid(360.0) - 180.0;
        assert!(
            off_south.abs() < 1e-6,
            "at {lon} the vector points {azimuth}, not south"
        );
    }
}

/// The two aimed modes are exact opposites at every cell.
///
/// The check that catches taking the outward bearing at the *target* instead of
/// at the cell: those differ by the meridian convergence between the two, which
/// is tens of degrees for a target this far off.
#[test]
fn away_is_the_reciprocal_of_toward_everywhere() {
    let stroke = |mode| BrushStroke {
        points: vec![[-30.0, 55.0], [10.0, 40.0], [40.0, 20.0]],
        size_km: 700.0,
        speed_mps: 20.0,
        direction_toward_deg: 0.0,
        feather: 0.0,
        direction_mode: mode,
        target: Some([-120.0, -20.0]),
        ..Default::default()
    };

    let (_root, toward) = project("toward-recip");
    edit::paint(&toward, stroke(BrushDirectionMode::TowardPoint)).expect("paint");
    let (_root, away) = project("away-recip");
    edit::paint(&away, stroke(BrushDirectionMode::AwayFromPoint)).expect("paint");

    for point in [
        ll(-30.0, 55.0),
        ll(-10.0, 48.0),
        ll(10.0, 40.0),
        ll(40.0, 20.0),
    ] {
        let (speed, at) = sample(&toward, point);
        assert!(speed > 0.0, "the stroke covers {point:?}");
        let (_, from) = sample(&away, point);
        let apart = (from - at - 180.0 + 180.0).rem_euclid(360.0) - 180.0;
        assert!(
            apart.abs() < 1e-6,
            "at {point:?}: toward {at}, away {from}, {apart} off the reciprocal"
        );
    }
}

/// A mode that aims at a point needs one, whichever way it aims.
#[test]
fn aiming_away_from_no_point_is_refused() {
    let (_root, state) = project("away-no-target");
    let refused = edit::paint(
        &state,
        BrushStroke {
            points: vec![[0.0, 0.0]],
            size_km: 400.0,
            speed_mps: 10.0,
            direction_toward_deg: 0.0,
            feather: 0.0,
            direction_mode: BrushDirectionMode::AwayFromPoint,
            target: None,
            ..Default::default()
        },
    );
    assert!(refused.is_err(), "a target is not optional here");
}

#[test]
fn a_constant_stroke_ignores_a_target() {
    let (_root, state) = project("constant");
    edit::paint(
        &state,
        BrushStroke {
            points: vec![[0.0, 0.0]],
            size_km: 600.0,
            speed_mps: 12.0,
            direction_toward_deg: 45.0,
            feather: 0.0,
            direction_mode: BrushDirectionMode::Constant,
            target: Some([0.0, 90.0]),
            ..Default::default()
        },
    )
    .expect("paint");

    let (_, azimuth) = sample(&state, ll(0.0, 0.0));
    assert!((azimuth - 45.0).abs() < 1e-6, "got {azimuth}");
}

/// Falling back to the constant bearing would paint a stroke nobody asked for,
/// so the missing point is refused instead.
#[test]
fn aiming_at_no_point_is_refused() {
    let (_root, state) = project("no-target");
    let outcome = edit::paint(
        &state,
        BrushStroke {
            points: vec![[0.0, 0.0]],
            size_km: 600.0,
            speed_mps: 12.0,
            direction_toward_deg: 45.0,
            feather: 0.0,
            direction_mode: BrushDirectionMode::TowardPoint,
            target: None,
            ..Default::default()
        },
    );
    assert!(matches!(
        outcome,
        Err(AppError::BadOption {
            field: "target",
            ..
        })
    ));

    let tree = ve_app::document::tree(&state, 0).expect("tree");
    assert!(
        tree.layers[0].objects.is_empty(),
        "a refused stroke must not half-land"
    );
}

// --- Stamp space ------------------------------------------------------------

/// A stamp at `lat`, in `space`, of `size_km` north-south.
fn stamp_at(lat: f64, space: StampSpace, size_km: f32) -> BrushStroke {
    BrushStroke {
        points: vec![[0.0, lat]],
        size_km,
        speed_mps: 20.0,
        direction_toward_deg: 90.0,
        feather: 0.0,
        space,
        ..Default::default()
    }
}

/// A degree of latitude on this earth, in kilometres. `EARTH_RADIUS_M` times
/// pi over 180, to the precision an `f32` size property can hold.
const DEGREE_KM: f32 = 111.198_92;

/// The property the projected space exists for: the footprint reaches the same
/// number of *degrees* in both axes, which on an equirectangular map is the
/// same number of pixels — a circle on screen, at any latitude.
#[test]
fn a_projected_stamp_is_round_on_the_map() {
    // A degree across, so its edge is half a degree out in either axis.
    for lat in [0.0, 45.0, 70.0] {
        let (_root, state) = project(&format!("projected-{lat}"));
        edit::paint(&state, stamp_at(lat, StampSpace::Projected, DEGREE_KM)).expect("paint");

        for (label, inside, outside) in [
            ("north", ll(0.0, lat + 0.4), ll(0.0, lat + 0.6)),
            ("east", ll(0.4, lat), ll(0.6, lat)),
        ] {
            assert!(
                sample(&state, inside).0 > 0.0,
                "0.4 degrees {label} must be inside at lat {lat}"
            );
            assert_eq!(
                sample(&state, outside).0,
                0.0,
                "0.6 degrees {label} must be outside at lat {lat}"
            );
        }
    }
}

/// The same stamp on the ground is a disc, and a disc covers more *longitude*
/// the further from the equator: at 70 N it reaches past 0.6 degrees east while
/// stopping short of 0.6 degrees north. That ellipse is the deformation the
/// projected space removes, asserted rather than described.
#[test]
fn a_geodesic_stamp_is_not_round_on_the_map() {
    let lat = 70.0;
    let (_root, state) = project("geodesic-at-70");
    edit::paint(&state, stamp_at(lat, StampSpace::Geodesic, DEGREE_KM)).expect("paint");

    assert_eq!(
        sample(&state, ll(0.0, lat + 0.6)).0,
        0.0,
        "0.6 degrees north is 67 km out, past the 55.6 km radius"
    );
    assert!(
        sample(&state, ll(0.6, lat)).0 > 0.0,
        "0.6 degrees east at 70 N is only 23 km out, well inside the same radius"
    );
}

/// The two spaces agree on the axis the projection leaves alone, so `size_km`
/// means one thing whichever space the stamp is in.
#[test]
fn both_spaces_agree_north_south() {
    let size_km = 1000.0;
    let lat = 55.0;
    let inside = ll(0.0, lat).destination(Angle::new(0.0), 480_000.0);
    let outside = ll(0.0, lat).destination(Angle::new(0.0), 520_000.0);

    for space in [StampSpace::Geodesic, StampSpace::Projected] {
        let (_root, state) = project(&format!("north-south-{space:?}"));
        edit::paint(&state, stamp_at(lat, space, size_km)).expect("paint");
        assert!(
            sample(&state, inside).0 > 0.0,
            "{space:?} must cover 480 km north of a 1000 km stamp"
        );
        assert_eq!(
            sample(&state, outside).0,
            0.0,
            "{space:?} must not cover 520 km north of a 1000 km stamp"
        );
    }
}

/// A projected stroke's points are map-space metres. Converting them with a
/// ground frame would bend the stroke off the path that was drawn, which a
/// point on the straight line between its ends is what catches.
#[test]
fn a_projected_stroke_follows_the_path_it_was_drawn_along() {
    let (_root, state) = project("projected-path");
    edit::paint(
        &state,
        BrushStroke {
            // A long east-west stroke well off the equator, where a ground
            // frame and a map frame disagree most.
            points: vec![[-20.0, 60.0], [20.0, 60.0]],
            size_km: 100.0,
            speed_mps: 20.0,
            direction_toward_deg: 90.0,
            feather: 0.0,
            space: StampSpace::Projected,
            ..Default::default()
        },
    )
    .expect("paint");

    // Every point on the drawn line is under the stroke...
    for lon in [-20.0, -10.0, 0.0, 10.0, 20.0] {
        assert!(
            sample(&state, ll(lon, 60.0)).0 > 0.0,
            "the stroke must cover the line it was drawn along, at {lon}"
        );
    }
    // ...and the great circle between its ends, which bows about 100 km north
    // of the line at this latitude, is not.
    assert_eq!(
        sample(&state, ll(0.0, 61.0)).0,
        0.0,
        "a degree north of the drawn line is outside a 100 km stroke"
    );
}

/// Two strokes that differ only in space look different, so absorbing one into
/// the other would change the layer (spec.md 6.1).
#[test]
fn a_projected_stroke_does_not_merge_into_a_geodesic_one() {
    let (_root, state) = project("space-merge");
    let summary = edit::paint(&state, stamp_at(40.0, StampSpace::Geodesic, 800.0)).expect("paint");
    assert_eq!(summary.object_count, 1);

    let summary = edit::paint(&state, stamp_at(40.0, StampSpace::Projected, 800.0)).expect("paint");
    assert_eq!(
        summary.object_count, 2,
        "a map-space stamp is a different footprint, so it stays its own object"
    );
}

// --- Editing the aim point after the fact -----------------------------------

/// The aim point is an ordinary property, so it can be moved after the stroke
/// exists — which is the only way to correct one, since the stroke froze the
/// tool's option at creation (spec.md 6.1).
#[test]
fn the_aim_point_can_be_moved_after_the_stroke_exists() {
    for mode in [
        BrushDirectionMode::TowardPoint,
        BrushDirectionMode::AwayFromPoint,
    ] {
        let (_root, state) = project(&format!("retarget-{mode:?}"));
        edit::paint(
            &state,
            BrushStroke {
                points: vec![[0.0, 0.0]],
                size_km: 600.0,
                speed_mps: 20.0,
                direction_toward_deg: 0.0,
                feather: 0.0,
                direction_mode: mode,
                // Due east of the stroke to begin with.
                target: Some([30.0, 0.0]),
                ..Default::default()
            },
        )
        .expect("paint");
        let id = ve_app::document::tree(&state, 0).expect("tree").layers[0].objects[0].id;

        let toward_east = matches!(mode, BrushDirectionMode::TowardPoint);
        let (_, azimuth) = sample(&state, ll(0.0, 0.0));
        let expected = if toward_east { 90.0 } else { 270.0 };
        assert!(
            (azimuth - expected).abs() < 1e-6,
            "{mode:?} should start pointing {expected}, got {azimuth}"
        );

        // Move the point to due north, the way the inspector does.
        ve_app::document::set_property(
            &state,
            id,
            "Target",
            ve_app::document::PropertyValue::Position {
                lon: 0.0,
                lat: 30.0,
            },
        )
        .expect("retarget");

        let (speed, azimuth) = sample(&state, ll(0.0, 0.0));
        assert!(speed > 0.0, "the stroke is still there");
        let expected = if toward_east { 0.0 } else { 180.0 };
        let off = (azimuth - expected + 180.0).rem_euclid(360.0) - 180.0;
        assert!(
            off.abs() < 1e-6,
            "{mode:?} should now point {expected}, got {azimuth}"
        );
    }
}

/// And the move is undoable, like every other property edit.
#[test]
fn moving_the_aim_point_is_undoable() {
    let (_root, state) = project("retarget-undo");
    edit::paint(
        &state,
        BrushStroke {
            points: vec![[0.0, 0.0]],
            size_km: 600.0,
            speed_mps: 20.0,
            direction_toward_deg: 0.0,
            feather: 0.0,
            direction_mode: BrushDirectionMode::TowardPoint,
            target: Some([30.0, 0.0]),
            ..Default::default()
        },
    )
    .expect("paint");
    let id = ve_app::document::tree(&state, 0).expect("tree").layers[0].objects[0].id;

    ve_app::document::set_property(
        &state,
        id,
        "Target",
        ve_app::document::PropertyValue::Position {
            lon: 0.0,
            lat: 30.0,
        },
    )
    .expect("retarget");
    assert!((sample(&state, ll(0.0, 0.0)).1 - 0.0).abs() < 1e-6);

    edit::undo_for_test(&state).expect("undo");
    assert!(
        (sample(&state, ll(0.0, 0.0)).1 - 90.0).abs() < 1e-6,
        "undo must put the aim point back"
    );
}

// --- What the inspector is shown --------------------------------------------

/// The ids the inspector would list for the object.
fn listed(state: &AppState, object: u64) -> Vec<String> {
    ve_app::document::properties(state, object, 0)
        .expect("properties")
        .into_iter()
        .map(|p| p.id)
        .collect()
}

/// An aimed stroke never reads its constant bearing, and a constant one never
/// reads its target. Listing an inert value invites editing it and watching
/// nothing happen.
#[test]
fn the_inspector_lists_only_the_direction_options_in_use() {
    let aimed = |mode| BrushStroke {
        points: vec![[0.0, 0.0]],
        size_km: 400.0,
        speed_mps: 10.0,
        direction_toward_deg: 90.0,
        feather: 0.0,
        direction_mode: mode,
        target: matches!(mode, BrushDirectionMode::Constant)
            .then_some(None)
            .flatten(),
        ..Default::default()
    };

    for mode in [
        BrushDirectionMode::TowardPoint,
        BrushDirectionMode::AwayFromPoint,
    ] {
        let (_root, state) = project(&format!("listed-{mode:?}"));
        edit::paint(
            &state,
            BrushStroke {
                target: Some([30.0, 0.0]),
                ..aimed(mode)
            },
        )
        .expect("paint");
        let id = ve_app::document::tree(&state, 0).expect("tree").layers[0].objects[0].id;
        let ids = listed(&state, id);
        assert!(
            ids.contains(&"Target".to_owned()),
            "{mode:?} aims at a target, so it must be editable: {ids:?}"
        );
        assert!(
            !ids.contains(&"Direction".to_owned()),
            "{mode:?} never reads a constant bearing: {ids:?}"
        );
    }

    let (_root, state) = project("listed-constant");
    edit::paint(&state, aimed(BrushDirectionMode::Constant)).expect("paint");
    let id = ve_app::document::tree(&state, 0).expect("tree").layers[0].objects[0].id;
    let ids = listed(&state, id);
    assert!(ids.contains(&"Direction".to_owned()), "{ids:?}");
    assert!(
        !ids.contains(&"Target".to_owned()),
        "a constant bearing has no target to aim at: {ids:?}"
    );
}

/// Switching the mode changes what is listed, without losing the other value:
/// the property is hidden, not deleted, so flipping back restores it.
#[test]
fn hiding_a_direction_option_does_not_discard_it() {
    let (_root, state) = project("listed-flip");
    edit::paint(
        &state,
        BrushStroke {
            points: vec![[0.0, 0.0]],
            size_km: 400.0,
            speed_mps: 10.0,
            direction_toward_deg: 90.0,
            feather: 0.0,
            direction_mode: BrushDirectionMode::TowardPoint,
            target: Some([30.0, 0.0]),
            ..Default::default()
        },
    )
    .expect("paint");
    let id = ve_app::document::tree(&state, 0).expect("tree").layers[0].objects[0].id;

    ve_app::document::set_property(
        &state,
        id,
        "DirectionMode",
        ve_app::document::PropertyValue::Choice { index: 0 },
    )
    .expect("to constant");
    // The stroke now flows along the bearing it was painted with.
    assert!((sample(&state, ll(0.0, 0.0)).1 - 90.0).abs() < 1e-6);

    ve_app::document::set_property(
        &state,
        id,
        "DirectionMode",
        ve_app::document::PropertyValue::Choice { index: 1 },
    )
    .expect("back to aimed");
    let ids = listed(&state, id);
    assert!(ids.contains(&"Target".to_owned()), "{ids:?}");
    // And the target it was painted with is still there, aiming due east.
    assert!((sample(&state, ll(0.0, 0.0)).1 - 90.0).abs() < 1e-6);
}

/// The brush's options fall into three kinds, and the panel treats each
/// differently: gone, fixed, or editable.
#[test]
fn the_edit_panel_offers_only_what_can_be_edited() {
    let (_root, state) = project("frozen");
    edit::paint(
        &state,
        BrushStroke {
            points: vec![[0.0, 0.0]],
            size_km: 400.0,
            speed_mps: 10.0,
            direction_toward_deg: 90.0,
            feather: 0.0,
            shape: BrushShape::Square,
            ..Default::default()
        },
    )
    .expect("paint");
    let id = ve_app::document::tree(&state, 0).expect("tree").layers[0].objects[0].id;
    let ids = listed(&state, id);

    // Gone from the tool entirely: the brush has no centre to define them
    // about (spec.md 7.5).
    for absent in ["Divergence", "Curl"] {
        assert!(
            !ids.contains(&absent.to_owned()),
            "{absent} is not a brush property at all: {ids:?}"
        );
        assert!(
            ve_app::document::set_property(
                &state,
                id,
                absent,
                ve_app::document::PropertyValue::Number { value: 1.0 },
            )
            .is_err(),
            "{absent} must not be writable on a brush"
        );
    }

    // Fixed at creation: part of the geometry the stroke painted.
    for fixed in ["BrushShape", "StampSpace"] {
        assert!(
            !ids.contains(&fixed.to_owned()),
            "{fixed} is fixed at creation, so the edit panel must not list it: {ids:?}"
        );
        assert!(
            ve_app::document::set_property(
                &state,
                id,
                fixed,
                ve_app::document::PropertyValue::Choice { index: 0 },
            )
            .is_err(),
            "{fixed} must refuse a write"
        );
    }

    // Editable, and still offered.
    for editable in ["EdgeMode", "SizeKm", "Speed", "Feather", "Direction"] {
        assert!(
            ids.contains(&editable.to_owned()),
            "{editable} missing: {ids:?}"
        );
    }

    // The refusals changed nothing: the stroke is still the square it was
    // painted as, at a corner a round stamp would miss.
    let corner = ll(0.0, 0.0).destination(Angle::new(45.0), 190_000.0_f64.hypot(190_000.0));
    assert!(
        sample(&state, corner).0 > 0.0,
        "the square stamp must still cover its corner"
    );
}

// --- Persistence ------------------------------------------------------------

/// Every option has to survive the file, or the stroke reopens as something
/// else.
#[test]
fn the_options_survive_a_save_and_load() {
    let (root, state) = project("save");
    edit::paint(
        &state,
        BrushStroke {
            points: vec![[5.0, 5.0], [15.0, 12.0]],
            size_km: 750.0,
            speed_mps: 18.0,
            direction_toward_deg: 0.0,
            feather: 0.25,
            shape: BrushShape::Square,
            space: StampSpace::Projected,
            direction_mode: BrushDirectionMode::TowardPoint,
            target: Some([-30.0, 45.0]),
            ..Default::default()
        },
    )
    .expect("paint");

    let path = root.0.join("brush.veproj");
    projects::save_as(&state, path.to_string_lossy().into_owned()).expect("save");

    let before = sample(&state, ll(5.0, 5.0));
    projects::close(&state).expect("close");
    projects::open(&state, path.to_string_lossy().into_owned(), false).expect("open");
    let after = sample(&state, ll(5.0, 5.0));

    assert!((before.0 - after.0).abs() < 1e-6, "{before:?} vs {after:?}");
    assert!((before.1 - after.1).abs() < 1e-6, "{before:?} vs {after:?}");

    // And the values themselves, not just that the two agree.
    let project = {
        let mut session = state.session.lock().expect("lock");
        session.require_open().expect("open").project.clone()
    };
    let props = &project.layers[0].objects[0].props;
    let value = |id| props.get(id).map(ve_core::keyframe::Animatable::base);
    use ve_core::PropValue;
    use ve_core::schema::PropId;
    assert_eq!(value(PropId::BrushShape), Some(PropValue::Enum(1)));
    assert_eq!(value(PropId::StampSpace), Some(PropValue::Enum(1)));
    assert_eq!(value(PropId::DirectionMode), Some(PropValue::Enum(1)));
    assert_eq!(
        value(PropId::Target),
        Some(PropValue::LonLat(ll(-30.0, 45.0)))
    );
    assert_eq!(value(PropId::SizeKm), Some(PropValue::F32(750.0)));
}

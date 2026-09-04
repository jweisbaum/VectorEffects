#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! Keyframes, end to end (spec.md 4.4, 4.5, 9.3).
//!
//! Driven through the same functions the timeline's commands call, and read
//! back through the evaluator where the question is what a step *looks like* —
//! an interpolation that produced the right number and the wrong field would
//! pass a value check and fail on screen.
//!
//! M7's acceptance names five kinds — position, scale, rotation, an enum and a
//! boolean — and asks that each behave per spec 4.5. Each has its own test
//! here, asserting the rule the spec states for it rather than whatever the
//! code happens to do.

use ve_app::animation::{self, InterpolationView};
use ve_app::commands::AppState;
use ve_app::create::{self, Gesture, NewObject, Tool, ToolOption};
use ve_app::document::{self, PropertyValue};
use ve_app::edit;
use ve_app::paths::AppPaths;
use ve_app::projects::{self, NewProjectRequest};
use ve_core::LonLat;
use ve_core::schema::PropId;

struct TempRoot(std::path::PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ve-anim-{}-{label}-{}",
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

/// A twelve-step project.
fn project(label: &str) -> (TempRoot, AppState) {
    let root = TempRoot::new(label);
    let state = AppState::new(AppPaths::in_directory(&root.0).expect("paths"));
    projects::create(
        &state,
        NewProjectRequest {
            name: "Anim".to_owned(),
            field_kind: "wind".to_owned(),
            resolution: "1.0".to_owned(),
            step_hours: 3,
            step_count: 12,
        },
        false,
    )
    .expect("create");
    (root, state)
}

fn ll(lon: f64, lat: f64) -> LonLat {
    LonLat::new(lon, lat).expect("position")
}

fn document_of(state: &AppState) -> ve_core::project::Project {
    let mut session = state.session.lock().expect("lock");
    session.require_open().expect("open").project.clone()
}

/// The object most recently created: a new object joins the end of its layer.
fn newest_object(state: &AppState) -> u64 {
    document_of(state).layers[0]
        .objects
        .last()
        .expect("an object")
        .id
        .raw()
}

/// A circle stamp: it has an enum (`FillMode`) as well as the common set.
fn circle(state: &AppState, lon: f64, lat: f64) -> u64 {
    create::create(
        state,
        NewObject {
            tool: Tool::Circle,
            gesture: Gesture::Point { at: [lon, lat] },
            options: vec![
                ToolOption {
                    property: "DiameterKm".to_owned(),
                    value: PropertyValue::Number { value: 800.0 },
                },
                ToolOption {
                    property: "Speed".to_owned(),
                    value: PropertyValue::Number { value: 12.0 },
                },
                ToolOption {
                    property: "Feather".to_owned(),
                    value: PropertyValue::Number { value: 0.0 },
                },
            ],
            layer: None,
        },
    )
    .expect("circle");
    newest_object(state)
}

/// A one-stamp brush stroke flowing east at 12 m/s.
fn stamp(state: &AppState, lon: f64, lat: f64) -> u64 {
    edit::paint(
        state,
        edit::BrushStroke {
            points: vec![[lon, lat]],
            size_km: 600.0,
            speed_mps: 12.0,
            direction_toward_deg: 90.0,
            feather: 0.0,
            ..Default::default()
        },
    )
    .expect("paint");
    newest_object(state)
}

/// The value a property resolves to at `step`, from the document.
fn value_at(state: &AppState, object: u64, prop: PropId, step: u32) -> ve_core::PropValue {
    let doc = document_of(state);
    let target = doc.object(ve_core::Id::from_raw(object)).expect("present");
    target
        .props
        .value_at(target.tool, prop, step)
        .expect("declared")
}

/// Speed and azimuth-toward at a position and step, from the evaluator.
fn sample(state: &AppState, position: LonLat, step: u32) -> (f64, f64) {
    let scene = ve_render::scene::flatten(&document_of(state), step);
    let uv = ve_render::cpu::sample_scene(&scene, position);
    let (speed, azimuth) = ve_core::vector::speed_azimuth_from_uv(uv);
    (speed, azimuth.degrees())
}

fn key(state: &AppState, object: u64, prop: &str, step: u32, value: PropertyValue) {
    animation::key_at(state, object, prop, step, Some(value)).expect("key");
}

// --- Spec 4.5, per kind --------------------------------------------------------

/// A position slerps along the great circle between its keys. Checked on the
/// field: a stamp keyed from one place to another must, half way through, be
/// painting at the great-circle midpoint and not at the degree-space one — the
/// two are 250 km apart for these endpoints.
#[test]
fn a_keyed_position_moves_along_the_great_circle() {
    let (_root, state) = project("position");
    let id = stamp(&state, 0.0, 0.0);
    key(
        &state,
        id,
        "Position",
        2,
        PropertyValue::Position {
            lon: -60.0,
            lat: 50.0,
        },
    );
    key(
        &state,
        id,
        "Position",
        10,
        PropertyValue::Position {
            lon: 60.0,
            lat: 50.0,
        },
    );

    let midpoint = value_at(&state, id, PropId::Position, 6)
        .as_lonlat()
        .expect("a position");
    let great_circle = ve_core::value::slerp(ll(-60.0, 50.0), ll(60.0, 50.0), 0.5);
    assert!(
        midpoint.distance_m(great_circle) < 1_000.0,
        "midpoint {midpoint:?} is {} km from the great-circle midpoint",
        midpoint.distance_m(great_circle) / 1000.0
    );
    // The great circle between two points at 50°N bows *north* of the parallel;
    // a degree-space average would sit on it. The bow is the whole difference.
    assert!(
        midpoint.lat > 55.0,
        "the path did not bow north: {midpoint:?}"
    );

    // ...and the field is there. Painted at the midpoint, calm at the origin.
    assert!(
        sample(&state, midpoint, 6).0 > 1.0,
        "nothing painted at the midpoint"
    );
    assert!(
        sample(&state, ll(0.0, 0.0), 6).0 < 0.01,
        "still painting at the origin"
    );
}

/// Scale interpolates linearly by default and holds outside its keys.
#[test]
fn a_keyed_scale_ramps_linearly_and_holds_past_its_keys() {
    let (_root, state) = project("scale");
    let id = circle(&state, 0.0, 0.0);
    key(
        &state,
        id,
        "ScalePct",
        2,
        PropertyValue::Number { value: 100.0 },
    );
    key(
        &state,
        id,
        "ScalePct",
        6,
        PropertyValue::Number { value: 300.0 },
    );

    let at = |step| {
        value_at(&state, id, PropId::ScalePct, step)
            .as_f32()
            .unwrap()
    };
    assert!(
        (at(4) - 200.0).abs() < 1e-3,
        "half way should be 200, got {}",
        at(4)
    );
    assert!(
        (at(3) - 150.0).abs() < 1e-3,
        "a quarter in should be 150, got {}",
        at(3)
    );
    // No extrapolation: the nearest key holds (spec.md 4.5).
    assert_eq!(at(0), 100.0, "before the first key, the first key holds");
    assert_eq!(at(11), 300.0, "after the last key, the last key holds");

    // On the field: at step 6 the disc reaches three times as far.
    let reach = |step: u32| {
        let scene = ve_render::scene::flatten(&document_of(&state), step);
        scene.objects[0].cap_radius_m
    };
    assert!(
        (reach(6) / reach(2) - 3.0).abs() < 0.01,
        "{} vs {}",
        reach(6),
        reach(2)
    );
}

/// Rotation takes the shortest arc through 0/360. From 350° to 10° the half-way
/// bearing is 0°, not 180° — the difference between a stamp that turns twenty
/// degrees and one that spins most of the way round.
#[test]
fn a_keyed_rotation_takes_the_shortest_arc() {
    let (_root, state) = project("rotation");
    let id = stamp(&state, 0.0, 0.0);
    key(
        &state,
        id,
        "RotationDeg",
        2,
        PropertyValue::Angle { degrees: 350.0 },
    );
    key(
        &state,
        id,
        "RotationDeg",
        6,
        PropertyValue::Angle { degrees: 10.0 },
    );

    let mid = value_at(&state, id, PropId::RotationDeg, 4)
        .as_angle()
        .unwrap()
        .degrees();
    let off = ((mid + 540.0) % 360.0) - 180.0;
    assert!(
        off.abs() < 1e-6,
        "half way round the short arc is 0, got {mid}"
    );

    // A stroke's rotation turns its constant flow with it (the frame turns the
    // bearing), so the field at step 4 flows east plus the rotation, which at
    // the midpoint is east exactly.
    let (_, azimuth) = sample(&state, ll(0.0, 0.0), 4);
    let error = ((azimuth - 90.0 + 540.0) % 360.0) - 180.0;
    assert!(
        error.abs() < 1.0,
        "flow at the midpoint is {azimuth}, not east"
    );
}

/// An enum holds its earlier key until the next one: there is no half-way
/// between a filled disc and a ring. Asserted on the field, where a blended
/// mode would have to invent one.
#[test]
fn a_keyed_enum_holds_until_its_next_key() {
    let (_root, state) = project("enum");
    let id = circle(&state, 0.0, 0.0);
    // Fill mode 0 is filled, 1 is a ring.
    key(
        &state,
        id,
        "FillMode",
        2,
        PropertyValue::Choice { index: 0 },
    );
    key(
        &state,
        id,
        "FillMode",
        8,
        PropertyValue::Choice { index: 1 },
    );

    for step in 2..8 {
        assert_eq!(
            value_at(&state, id, PropId::FillMode, step).as_enum(),
            Some(0),
            "step {step} should still be filled"
        );
        assert!(
            sample(&state, ll(0.0, 0.0), step).0 > 1.0,
            "the disc has a hole at {step}"
        );
    }
    assert_eq!(value_at(&state, id, PropId::FillMode, 8).as_enum(), Some(1));
    assert!(
        sample(&state, ll(0.0, 0.0), 8).0 < 0.01,
        "the ring has no hole at 8"
    );

    // And it cannot be asked to do otherwise: the only interpolation offered is
    // step, and any other is refused rather than silently coerced.
    let tracks = animation::tracks_of(&state, id, 2).expect("tracks");
    let fill = tracks
        .tracks
        .iter()
        .find(|t| t.property == "FillMode")
        .unwrap();
    assert_eq!(fill.interpolations, vec![InterpolationView::Step]);
    assert!(animation::ease_from(&state, id, "FillMode", 2, InterpolationView::Linear).is_err());
}

/// A boolean holds too, and switching `enabled` off at a step removes the
/// object from the field at that step and every one after until it comes back.
///
/// Note the rule at the start: outside the keyed range the *nearest key* holds,
/// not the base (spec.md 4.5). A lone "off" key at step 3 therefore switches
/// the object off from step 0, which is why the first key here says "on" — and
/// why the timeline marks where a value is held rather than keyed.
#[test]
fn a_keyed_boolean_switches_the_object_off_and_on() {
    let (_root, state) = project("bool");
    let id = stamp(&state, 0.0, 0.0);
    key(
        &state,
        id,
        "Enabled",
        3,
        PropertyValue::Bool { value: false },
    );
    key(
        &state,
        id,
        "Enabled",
        7,
        PropertyValue::Bool { value: true },
    );

    let painted = |step| sample(&state, ll(0.0, 0.0), step).0 > 1.0;
    assert!(
        !painted(0) && !painted(2),
        "the first key holds before it: off"
    );
    assert!(!painted(3) && !painted(5) && !painted(6), "off from step 3");
    assert!(painted(7) && painted(11), "on again from step 7");

    key(
        &state,
        id,
        "Enabled",
        0,
        PropertyValue::Bool { value: true },
    );
    assert!(painted(0) && painted(2), "now keyed on at the start");
    assert!(!painted(3), "and still off from step 3");
    assert!(animation::ease_from(&state, id, "Enabled", 3, InterpolationView::EaseIn).is_err());
}

// --- Editing keys -------------------------------------------------------------

/// "Key this here" without a value pins what is on screen: between two keys,
/// that is the interpolated value, not the base and not a neighbour's.
#[test]
fn keying_without_a_value_pins_the_interpolated_value() {
    let (_root, state) = project("pin");
    let id = circle(&state, 0.0, 0.0);
    key(
        &state,
        id,
        "Speed",
        0,
        PropertyValue::Number { value: 10.0 },
    );
    key(
        &state,
        id,
        "Speed",
        10,
        PropertyValue::Number { value: 30.0 },
    );

    animation::key_at(&state, id, "Speed", 5, None).expect("pin");
    let tracks = animation::tracks_of(&state, id, 5).expect("tracks");
    let speed = tracks
        .tracks
        .iter()
        .find(|t| t.property == "Speed")
        .unwrap();
    assert_eq!(speed.keys.len(), 3);
    let pinned = &speed.keys[1];
    assert_eq!(pinned.step, 5);
    assert!(matches!(pinned.value, PropertyValue::Number { value } if (value - 20.0).abs() < 1e-3));
    assert!(speed.keyed_here);
    assert!(!speed.interpolated_here);
}

/// Spec 9.3: a row says when the current step's value is interpolated rather
/// than keyed. That is true strictly between keys and nowhere else.
#[test]
fn a_track_reports_where_its_value_is_interpolated() {
    let (_root, state) = project("interpolated");
    let id = circle(&state, 0.0, 0.0);
    key(
        &state,
        id,
        "Speed",
        3,
        PropertyValue::Number { value: 10.0 },
    );
    key(
        &state,
        id,
        "Speed",
        7,
        PropertyValue::Number { value: 30.0 },
    );

    let flags = |step: u32| {
        let tracks = animation::tracks_of(&state, id, step).expect("tracks");
        let speed = tracks
            .tracks
            .iter()
            .find(|t| t.property == "Speed")
            .unwrap();
        (speed.keyed_here, speed.interpolated_here)
    };
    assert_eq!(
        flags(1),
        (false, false),
        "before the keys: held, not interpolated"
    );
    assert_eq!(flags(3), (true, false), "on a key");
    assert_eq!(flags(5), (false, true), "between keys");
    assert_eq!(flags(7), (true, false), "on the last key");
    assert_eq!(flags(9), (false, false), "after the keys: held");
}

/// Auto-key (spec.md 9.3): an ordinary property write, made while the
/// timeline is on a step, becomes a key at that step and leaves the rest of
/// the animation as it was.
#[test]
fn an_auto_keyed_write_keys_the_current_step_only() {
    let (_root, state) = project("autokey");
    let id = circle(&state, 0.0, 0.0);
    key(
        &state,
        id,
        "Speed",
        0,
        PropertyValue::Number { value: 10.0 },
    );
    key(
        &state,
        id,
        "Speed",
        10,
        PropertyValue::Number { value: 10.0 },
    );

    document::set_property_with(
        &state,
        id,
        "Speed",
        PropertyValue::Number { value: 40.0 },
        None,
        5,
        true,
    )
    .expect("auto-key");

    let at = |step| value_at(&state, id, PropId::Speed, step).as_f32().unwrap();
    assert_eq!(at(5), 40.0, "the step being edited");
    assert_eq!(at(0), 10.0, "the first key is untouched");
    assert_eq!(at(10), 10.0, "the last key is untouched");
    assert!(
        (at(2) - 22.0).abs() < 1e-3,
        "and the ramp now passes through the new key"
    );
}

/// Without auto-key, the same write to a property that *has* keys still lands
/// on the current step: once a key exists the base is not what any step shows,
/// so writing it would be an edit with no visible effect (spec 9.3, D42).
#[test]
fn a_plain_write_to_a_keyed_property_never_touches_the_base() {
    let (_root, state) = project("base");
    let id = circle(&state, 0.0, 0.0);
    key(
        &state,
        id,
        "Speed",
        2,
        PropertyValue::Number { value: 10.0 },
    );
    // `set_property` is the step-0, auto-key-off write.
    document::set_property(&state, id, "Speed", PropertyValue::Number { value: 40.0 })
        .expect("write");
    assert_eq!(keys_of(&state, id, PropId::Speed), vec![0, 2]);
    assert_eq!(value_at(&state, id, PropId::Speed, 0).as_f32(), Some(40.0));
    assert_eq!(value_at(&state, id, PropId::Speed, 2).as_f32(), Some(10.0));
    let speed = animation::tracks_of(&state, id, 2)
        .expect("tracks")
        .tracks
        .into_iter()
        .find(|t| t.property == "Speed")
        .unwrap();
    // The base is the 12 m/s the circle was created with, untouched.
    assert!(matches!(speed.base, PropertyValue::Number { value } if value == 12.0));
}

#[test]
fn a_key_can_be_moved_removed_and_re_eased() {
    let (_root, state) = project("edit");
    let id = circle(&state, 0.0, 0.0);
    key(
        &state,
        id,
        "Speed",
        2,
        PropertyValue::Number { value: 10.0 },
    );
    key(
        &state,
        id,
        "Speed",
        6,
        PropertyValue::Number { value: 30.0 },
    );

    animation::move_key(&state, id, "Speed", 6, 10, None).expect("move");
    let at = |step| value_at(&state, id, PropId::Speed, step).as_f32().unwrap();
    assert_eq!(at(10), 30.0);
    assert!((at(6) - 20.0).abs() < 1e-3, "the ramp now spans 2..10");

    animation::ease_from(&state, id, "Speed", 2, InterpolationView::EaseInOut).expect("ease");
    // Ease-in-out is symmetric, so the midpoint is unchanged and a quarter in
    // is below linear.
    assert!((at(6) - 20.0).abs() < 1e-3);
    assert!(
        at(4) < 15.0,
        "ease-in-out should lag linear at a quarter: {}",
        at(4)
    );

    animation::unkey_at(&state, id, "Speed", 10).expect("remove");
    assert_eq!(
        at(10),
        10.0,
        "with one key left, its value holds everywhere"
    );

    assert!(animation::move_key(&state, id, "Speed", 99, 3, None).is_err());
    assert!(
        animation::move_key(&state, id, "Speed", 2, 99, None).is_err(),
        "past the end"
    );
    assert!(
        animation::key_at(&state, id, "Speed", 99, None).is_err(),
        "past the end"
    );
}

/// A drag moves a key through many steps and is one history entry.
#[test]
fn a_dragged_key_is_one_undo() {
    let (_root, state) = project("drag");
    let id = circle(&state, 0.0, 0.0);
    key(
        &state,
        id,
        "Speed",
        2,
        PropertyValue::Number { value: 10.0 },
    );
    let entries = document::history_of(&state).expect("history").entries.len();

    let mut from = 2;
    for to in 3..=8 {
        animation::move_key(&state, id, "Speed", from, to, Some("drag:1".to_owned()))
            .expect("move");
        from = to;
    }
    document::finish_gesture(&state).expect("end");
    assert_eq!(
        document::history_of(&state).expect("history").entries.len(),
        entries + 1,
        "six moves, one entry"
    );

    edit::undo_for_test(&state).expect("undo");
    let tracks = animation::tracks_of(&state, id, 0).expect("tracks");
    let speed = tracks
        .tracks
        .iter()
        .find(|t| t.property == "Speed")
        .unwrap();
    assert_eq!(
        speed.keys[0].step, 2,
        "one undo returns the key to where the drag began"
    );
}

/// Every keyframe edit is undoable, and undo restores the property exactly —
/// keys, interpolations and base together.
#[test]
fn every_keyframe_edit_undoes_exactly() {
    let (_root, state) = project("undo");
    let id = circle(&state, 0.0, 0.0);
    let snapshot = || {
        document_of(&state)
            .object(ve_core::Id::from_raw(id))
            .unwrap()
            .props
            .clone()
    };

    type Edit<'a> = Box<dyn Fn() + 'a>;
    let edits: Vec<(&str, Edit<'_>)> = vec![
        (
            "key",
            Box::new(|| {
                key(
                    &state,
                    id,
                    "Speed",
                    4,
                    PropertyValue::Number { value: 22.0 },
                )
            }),
        ),
        (
            "pin",
            Box::new(|| {
                animation::key_at(&state, id, "Speed", 6, None)
                    .map(|_| ())
                    .unwrap()
            }),
        ),
        (
            "move",
            Box::new(|| {
                animation::move_key(&state, id, "Speed", 6, 8, None)
                    .map(|_| ())
                    .unwrap()
            }),
        ),
        (
            "ease",
            Box::new(|| {
                animation::ease_from(&state, id, "Speed", 4, InterpolationView::EaseOut)
                    .map(|_| ())
                    .unwrap()
            }),
        ),
        (
            "remove",
            Box::new(|| {
                animation::unkey_at(&state, id, "Speed", 8)
                    .map(|_| ())
                    .unwrap()
            }),
        ),
    ];
    for (name, edit) in edits {
        let before = snapshot();
        edit();
        assert_ne!(snapshot(), before, "{name} changed nothing");
        edit::undo_for_test(&state).expect("undo");
        assert_eq!(snapshot(), before, "{name} did not undo exactly");
        // Redo, so the next edit starts from the edited state.
        edit::redo_for_test(&state).expect("redo");
    }
}

/// A creation-only property cannot be keyed: a key is an edit spread over
/// time, and the write path refuses the edit however it arrives (spec.md 6.1).
#[test]
fn a_creation_only_property_cannot_be_keyed() {
    let (_root, state) = project("frozen");
    let id = stamp(&state, 0.0, 0.0);
    assert!(animation::key_at(&state, id, "BrushShape", 3, None).is_err());
    let tracks = animation::tracks_of(&state, id, 0).expect("tracks");
    assert!(
        tracks
            .tracks
            .iter()
            .all(|t| t.property != "BrushShape" && t.property != "StampSpace"),
        "the timeline lists a frozen property"
    );
}

// --- Step count (spec 4.1, decision D13) ----------------------------------------

/// The confirmation states the exact count and names the objects, and the
/// shrink then deletes exactly that. The two read one predicate, and this holds
/// them to it.
#[test]
fn shrinking_deletes_exactly_what_the_impact_reported_and_undoes() {
    let (_root, state) = project("shrink");
    let a = circle(&state, 0.0, 0.0);
    key(&state, a, "Speed", 3, PropertyValue::Number { value: 10.0 });
    key(&state, a, "Speed", 9, PropertyValue::Number { value: 20.0 });
    key(
        &state,
        a,
        "Speed",
        11,
        PropertyValue::Number { value: 30.0 },
    );
    // A second object entirely inside the new range — keyed early, and with a
    // lifetime that ends before the cut: untouched, and so unlisted.
    let b = circle(&state, 30.0, 0.0);
    key(&state, b, "Speed", 1, PropertyValue::Number { value: 5.0 });
    document::object_range(&state, b, 0, 4).expect("range");
    let b_name = document_of(&state)
        .object(ve_core::Id::from_raw(b))
        .unwrap()
        .name
        .clone();
    // A third with no keys but a lifetime reaching the end: clamped, listed.
    let c = circle(&state, 60.0, 0.0);
    document::object_range(&state, c, 4, 11).expect("range");

    let impact = animation::shrink_impact(&state, 6).expect("impact");
    assert_eq!(impact.keyframes, 2, "two of a's keys are past step 5");
    assert_eq!(
        impact.clamped_ranges, 2,
        "a's default range and c's reach past 5"
    );
    assert_eq!(impact.objects.len(), 2);
    assert!(
        !impact.objects.contains(&b_name),
        "b is untouched and must not be listed"
    );

    let before = document_of(&state);
    animation::resize_steps(&state, 6).expect("shrink");
    let after = document_of(&state);
    assert_eq!(after.settings.step_count, 6);
    let keys_of = |doc: &ve_core::project::Project, id: u64| {
        doc.object(ve_core::Id::from_raw(id))
            .unwrap()
            .props
            .get(PropId::Speed)
            .unwrap()
            .keys()
            .len()
    };
    assert_eq!(
        keys_of(&after, a),
        1,
        "exactly the two reported keys are gone"
    );
    assert_eq!(keys_of(&after, b), 1);
    assert_eq!(
        after
            .object(ve_core::Id::from_raw(c))
            .unwrap()
            .active_range
            .end,
        5,
        "c's lifetime is clamped to the new end"
    );

    edit::undo_for_test(&state).expect("undo");
    assert_eq!(
        document_of(&state),
        before,
        "the shrink undoes exactly, keys and ranges alike"
    );
}

#[test]
fn an_impossible_step_count_is_refused() {
    let (_root, state) = project("bad-count");
    assert!(animation::resize_steps(&state, 0).is_err());
    assert!(animation::resize_steps(&state, ve_core::project::MAX_STEPS + 1).is_err());
}

// --- Start time (spec 9.1) -----------------------------------------------------

#[test]
fn the_start_time_is_set_cleared_and_undone() {
    let (_root, state) = project("start");
    let summary = animation::start_at(&state, Some(1_700_000_000)).expect("set");
    assert_eq!(summary.start_unix_s, Some(1_700_000_000));

    let summary = animation::start_at(&state, None).expect("clear");
    assert_eq!(summary.start_unix_s, None);

    let summary = edit::undo_for_test(&state).expect("undo");
    assert_eq!(summary.start_unix_s, Some(1_700_000_000));

    // It is document state: it survives a save and a load.
    let path = _root.0.join("start.veproj");
    ve_core::io::save(&document_of(&state), &path).expect("save");
    let loaded = ve_core::io::load(&path).expect("load");
    assert_eq!(loaded.settings.start_unix_s, Some(1_700_000_000));
}

// --- The map's transforms write into the animation (spec 9.3) -----------------

use ve_app::transform::{self, TransformKind};

/// Drags an object with the hand, as the map does: begin, one update, end.
fn drag(state: &AppState, object: u64, step: u32, auto_key: bool, to: (f64, f64)) {
    let from = transform::transform_of(state, &[object], step)
        .expect("transform")
        .expect("present");
    transform::start_transform(
        state,
        &[object],
        step,
        TransformKind::Move,
        from.lon,
        from.lat,
        auto_key,
    )
    .expect("begin");
    transform::update_transform(state, to.0, to.1).expect("drag");
    document::finish_gesture(state).expect("end");
}

fn keys_of(state: &AppState, object: u64, prop: PropId) -> Vec<u32> {
    document_of(state)
        .object(ve_core::Id::from_raw(object))
        .unwrap()
        .props
        .get(prop)
        .unwrap()
        .keys()
        .iter()
        .map(|key| key.step)
        .collect()
}

/// The bug as reported: key a position at two steps, then move the object
/// with the hand — and every key vanished. A drag must write *into* the
/// animation: the keys stay, and the step being edited gains one.
#[test]
fn moving_a_keyed_object_keeps_its_keys_and_keys_the_current_step() {
    let (_root, state) = project("drag-keyed");
    let id = stamp(&state, 0.0, 0.0);
    key(
        &state,
        id,
        "Position",
        2,
        PropertyValue::Position { lon: 0.0, lat: 0.0 },
    );
    key(
        &state,
        id,
        "Position",
        10,
        PropertyValue::Position {
            lon: 40.0,
            lat: 0.0,
        },
    );

    drag(&state, id, 6, false, (20.0, 15.0));

    assert_eq!(keys_of(&state, id, PropId::Position), vec![2, 6, 10]);
    let at = |step| {
        value_at(&state, id, PropId::Position, step)
            .as_lonlat()
            .unwrap()
    };
    assert!(
        at(6).distance_m(ll(20.0, 15.0)) < 1_000.0,
        "step 6 is where it was dragged to"
    );
    assert!(
        at(2).distance_m(ll(0.0, 0.0)) < 1_000.0,
        "the first key is untouched"
    );
    assert!(
        at(10).distance_m(ll(40.0, 0.0)) < 1_000.0,
        "the last key is untouched"
    );
    // And the field follows: painted at the new place at step 6, not at 2.
    assert!(sample(&state, ll(20.0, 15.0), 6).0 > 1.0);
    assert!(sample(&state, ll(20.0, 15.0), 2).0 < 0.01);
}

/// With no keys and auto-key off, a drag changes the base as it always did.
#[test]
fn moving_an_unkeyed_object_without_auto_key_moves_its_base() {
    let (_root, state) = project("drag-base");
    let id = stamp(&state, 0.0, 0.0);
    drag(&state, id, 6, false, (20.0, 15.0));
    assert_eq!(keys_of(&state, id, PropId::Position), Vec::<u32>::new());
    for step in [0, 6, 11] {
        assert!(
            value_at(&state, id, PropId::Position, step)
                .as_lonlat()
                .unwrap()
                .distance_m(ll(20.0, 15.0))
                < 1_000.0
        );
    }
}

/// With auto-key on, the same drag keys the current step instead (spec 9.3).
#[test]
fn moving_with_auto_key_keys_the_current_step() {
    let (_root, state) = project("drag-autokey");
    let id = stamp(&state, 0.0, 0.0);
    drag(&state, id, 6, true, (20.0, 15.0));
    assert_eq!(keys_of(&state, id, PropId::Position), vec![6]);
}

/// Rotate and scale write the same way, and the position key the drag adds
/// alongside them does not disturb the others.
#[test]
fn rotating_and_scaling_a_keyed_object_keys_the_current_step() {
    let (_root, state) = project("drag-rotate");
    let id = circle(&state, 0.0, 0.0);
    key(
        &state,
        id,
        "RotationDeg",
        2,
        PropertyValue::Angle { degrees: 0.0 },
    );
    key(
        &state,
        id,
        "RotationDeg",
        10,
        PropertyValue::Angle { degrees: 90.0 },
    );
    key(
        &state,
        id,
        "ScalePct",
        2,
        PropertyValue::Number { value: 100.0 },
    );
    key(
        &state,
        id,
        "ScalePct",
        10,
        PropertyValue::Number { value: 100.0 },
    );

    let handles = transform::transform_of(&state, &[id], 6).unwrap().unwrap();
    transform::start_transform(
        &state,
        &[id],
        6,
        TransformKind::Rotate,
        handles.lon + 5.0,
        handles.lat,
        false,
    )
    .expect("begin");
    // Turn the pointer a quarter turn about the pivot.
    transform::update_transform(&state, handles.lon, handles.lat - 5.0).expect("drag");
    document::finish_gesture(&state).expect("end");

    assert_eq!(keys_of(&state, id, PropId::RotationDeg), vec![2, 6, 10]);
    assert_eq!(
        keys_of(&state, id, PropId::ScalePct),
        vec![2, 10],
        "scale was not touched"
    );

    let handles = transform::transform_of(&state, &[id], 6).unwrap().unwrap();
    transform::start_transform(
        &state,
        &[id],
        6,
        TransformKind::Scale,
        handles.lon + 5.0,
        handles.lat,
        false,
    )
    .expect("begin");
    transform::update_transform(&state, handles.lon + 10.0, handles.lat).expect("drag");
    document::finish_gesture(&state).expect("end");
    assert_eq!(keys_of(&state, id, PropId::ScalePct), vec![2, 6, 10]);
    let scale = value_at(&state, id, PropId::ScalePct, 6).as_f32().unwrap();
    assert!(
        (scale / 200.0 - 1.0).abs() < 0.05,
        "doubled at step 6, got {scale}"
    );
    assert_eq!(
        value_at(&state, id, PropId::ScalePct, 2).as_f32(),
        Some(100.0)
    );
}

/// Undo after a drag restores the property exactly — keys, base and easings.
/// The old drag fabricated `before` as a constant, so undo lost the keys too.
#[test]
fn undoing_a_drag_restores_the_keys_exactly() {
    let (_root, state) = project("drag-undo");
    let id = stamp(&state, 0.0, 0.0);
    key(
        &state,
        id,
        "Position",
        2,
        PropertyValue::Position { lon: 0.0, lat: 0.0 },
    );
    key(
        &state,
        id,
        "Position",
        10,
        PropertyValue::Position {
            lon: 40.0,
            lat: 0.0,
        },
    );
    animation::ease_from(&state, id, "Position", 2, InterpolationView::EaseInOut).unwrap();
    let before = document_of(&state);

    drag(&state, id, 6, false, (20.0, 15.0));
    assert_ne!(document_of(&state), before);
    edit::undo_for_test(&state).expect("undo");
    assert_eq!(document_of(&state), before);
}

/// Repinning the anchor goes through the same write, so it keys too.
#[test]
fn repinning_a_keyed_object_keys_the_current_step() {
    let (_root, state) = project("drag-anchor");
    let id = stamp(&state, 0.0, 0.0);
    key(
        &state,
        id,
        "Position",
        2,
        PropertyValue::Position { lon: 0.0, lat: 0.0 },
    );
    key(
        &state,
        id,
        "Position",
        10,
        PropertyValue::Position { lon: 0.0, lat: 0.0 },
    );

    let from = transform::transform_of(&state, &[id], 6).unwrap().unwrap();
    transform::start_transform(
        &state,
        &[id],
        6,
        TransformKind::Anchor,
        from.lon,
        from.lat,
        false,
    )
    .expect("begin");
    transform::update_transform(&state, 3.0, 2.0).expect("drag");
    document::finish_gesture(&state).expect("end");
    assert_eq!(keys_of(&state, id, PropId::Position), vec![2, 6, 10]);
}

/// The inspector follows the same rule as the drags: with keys present, an
/// edit at step 5 keys step 5 even with auto-key off — a base write would be
/// invisible, since the nearest key holds everywhere (spec 9.3, D42).
#[test]
fn editing_an_animated_property_without_auto_key_keys_the_current_step() {
    let (_root, state) = project("inspector-keyed");
    let id = stamp(&state, 0.0, 0.0);
    key(
        &state,
        id,
        "Speed",
        2,
        PropertyValue::Number { value: 10.0 },
    );
    key(
        &state,
        id,
        "Speed",
        10,
        PropertyValue::Number { value: 10.0 },
    );

    document::set_property_with(
        &state,
        id,
        "Speed",
        PropertyValue::Number { value: 40.0 },
        None,
        5,
        false,
    )
    .expect("edit");

    assert_eq!(keys_of(&state, id, PropId::Speed), vec![2, 5, 10]);
    let at = |step| value_at(&state, id, PropId::Speed, step).as_f32().unwrap();
    assert_eq!(at(5), 40.0);
    assert_eq!(at(2), 10.0);
    assert_eq!(at(10), 10.0);
}

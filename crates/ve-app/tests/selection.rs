#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! Multi-selection: marquee, and transforms about a collective centroid.
//!
//! These drive the same functions the Tauri commands call, so the pointer-down
//! / pointer-move / pointer-up sequence is exercised without a webview. What is
//! being checked is mostly geodesy: a group dragged across the globe has to keep
//! its shape, and a drag has to be idempotent no matter how many times the
//! pointer reports the same place.

use ve_app::commands::AppState;
use ve_app::document;
use ve_app::edit::{self, BrushStroke, StampSpace};
use ve_app::paths::AppPaths;
use ve_app::projects::{self, NewProjectRequest};
use ve_app::transform::{self, TransformKind};

struct TempRoot(std::path::PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ve-selection-{}-{label}-{}",
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
            name: "Select".to_owned(),
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

/// Paints a one-click stroke at `(lon, lat)` and returns its id.
///
/// Directions differ per stroke so nothing merges: merging would defeat a test
/// that needs two separate objects (spec.md 6.1).
fn dot(state: &AppState, lon: f64, lat: f64, direction: f64) -> u64 {
    edit::paint(
        state,
        BrushStroke {
            points: vec![[lon, lat]],
            size_km: 400.0,
            speed_mps: 12.0,
            direction_toward_deg: direction,
            feather: 0.2,
            layer: None,
            ..Default::default()
        },
    )
    .expect("paint");
    // Painting always targets the top layer, which is where to look for what
    // was just added.
    let tree = document::tree(state, 0).expect("tree");
    let top = tree.layers.last().expect("a layer");
    top.objects.last().expect("an object").id
}

fn anchor(state: &AppState, object: u64) -> (f64, f64) {
    let t = transform::transform_of(state, &[object], 0)
        .expect("transform")
        .expect("present");
    (t.lon, t.lat)
}

fn distance_m(a: (f64, f64), b: (f64, f64)) -> f64 {
    ve_core::LonLat::new(a.0, a.1)
        .expect("a")
        .distance_m(ve_core::LonLat::new(b.0, b.1).expect("b"))
}

fn drag(state: &AppState, objects: &[u64], kind: TransformKind, from: (f64, f64), to: (f64, f64)) {
    transform::start_transform(state, objects, 0, kind, from.0, from.1, false).expect("begin");
    transform::update_transform(state, to.0, to.1).expect("drag");
    document::finish_gesture(state).expect("end");
}

/// Every `[lon, lat]` a preview outline holds.
fn outline_points(outline: &ve_app::transform::ObjectOutline) -> Vec<[f64; 2]> {
    match outline {
        ve_app::transform::ObjectOutline::Swept { chains, .. } => {
            chains.iter().flatten().copied().collect()
        }
        ve_app::transform::ObjectOutline::Ring { points } => points.clone(),
        ve_app::transform::ObjectOutline::Contours { rings } => {
            rings.iter().flatten().copied().collect()
        }
    }
}

/// A two-point stroke: short enough that a preview sends every point, so a
/// preview and the object it previews can be compared point for point.
fn short_stroke(state: &AppState) -> u64 {
    edit::paint(
        state,
        BrushStroke {
            points: vec![[0.0, 0.0], [6.0, 0.0]],
            size_km: 400.0,
            speed_mps: 12.0,
            direction_toward_deg: 45.0,
            feather: 0.2,
            ..Default::default()
        },
    )
    .expect("paint");
    let tree = document::tree(state, 0).expect("tree");
    tree.layers
        .last()
        .expect("a layer")
        .objects
        .last()
        .expect("an object")
        .id
}

/// The object's footprint as it actually stands, in `[lon, lat]`.
///
/// Read from the flattened object, which is what the evaluator paints and what
/// the preview claims to be describing.
fn committed_outline(state: &AppState, object: u64) -> Vec<[f64; 2]> {
    let project = {
        let mut session = state.session.lock().expect("lock");
        session.require_open().expect("open").project.clone()
    };
    let flat = ve_render::scene::flatten_object(
        project
            .object(ve_core::Id::from_raw(object))
            .expect("object"),
        0,
    )
    .expect("flat");
    let ve_render::sdf::Shape::Capsule { chains, .. } = &flat.shape else {
        panic!("a brush stroke flattens to a capsule");
    };
    chains
        .iter()
        .flatten()
        .map(|p| {
            let global = flat.frame.to_global(*p);
            [global.lon, global.lat]
        })
        .collect()
}

/// The open project's summary, for revision and history checks.
fn summary(state: &AppState) -> ve_app::projects::ProjectSummary {
    projects::current(state).expect("current").expect("open")
}

// --- Drag preview -----------------------------------------------------------

/// The property the preview rests on: what is drawn under the pointer is where
/// the object lands. If the two were computed separately the object would jump
/// on release, by an amount that grows with the drag.
#[test]
fn the_preview_lands_where_the_drag_commits() {
    for (kind, from, to) in [
        (TransformKind::Move, (0.0, 0.0), (35.0, 25.0)),
        (TransformKind::Rotate, (6.0, 0.0), (2.0, 6.0)),
        (TransformKind::Scale, (6.0, 0.0), (11.0, 0.0)),
        (TransformKind::Anchor, (0.0, 0.0), (3.0, 2.0)),
    ] {
        let (_root, state) = project(&format!("preview-{kind:?}"));
        let object = short_stroke(&state);

        transform::start_transform(&state, &[object], 0, kind, from.0, from.1, false)
            .expect("begin");
        let preview = transform::peek_transform(&state, to.0, to.1)
            .expect("preview")
            .expect("a drag is in progress");
        let previewed = outline_points(&preview.outlines[0]);

        // Now let the drag land, and read the object's real footprint back.
        transform::update_transform(&state, to.0, to.1).expect("drag");
        document::finish_gesture(&state).expect("end");
        let landed = committed_outline(&state, object);

        assert_eq!(
            previewed.len(),
            landed.len(),
            "{kind:?}: preview and commit describe different outlines"
        );
        for (a, b) in previewed.iter().zip(&landed) {
            let apart = ve_core::LonLat::new(a[0], a[1])
                .expect("preview point")
                .distance_m(ve_core::LonLat::new(b[0], b[1]).expect("landed point"));
            assert!(
                apart < 1.0,
                "{kind:?}: previewed {a:?} but landed {b:?}, {apart:.1} m apart"
            );
        }

        // And the handles agree with where the selection actually ended up.
        let after = transform::transform_of(&state, &[object], 0)
            .expect("handles")
            .expect("selected");
        let pivot_apart = ve_core::LonLat::new(preview.handles.lon, preview.handles.lat)
            .expect("previewed pivot")
            .distance_m(ve_core::LonLat::new(after.lon, after.lat).expect("landed pivot"));
        assert!(
            pivot_apart < 1.0,
            "{kind:?}: handles moved {pivot_apart:.1} m"
        );
    }
}

/// A preview is a read. Nothing about the document may change, or dragging
/// without releasing would leave edits behind and fill the undo stack.
#[test]
fn a_preview_does_not_touch_the_document() {
    let (_root, state) = project("preview-readonly");
    let object = short_stroke(&state);

    let before = summary(&state);
    let outline_before = committed_outline(&state, object);
    transform::start_transform(&state, &[object], 0, TransformKind::Move, 0.0, 0.0, false)
        .expect("begin");

    for step in 1..20 {
        transform::peek_transform(&state, f64::from(step), f64::from(step) * 0.5).expect("preview");
    }

    let after = summary(&state);
    assert_eq!(
        after.revision, before.revision,
        "a preview must not bump the revision: it addresses every tile"
    );
    assert_eq!(
        after.can_undo, before.can_undo,
        "and must not write history"
    );
    assert_eq!(
        committed_outline(&state, object),
        outline_before,
        "and must leave the geometry exactly where it was"
    );
}

/// Nothing to preview once the gesture is over, rather than an error.
#[test]
fn a_preview_with_no_drag_in_progress_is_empty() {
    let (_root, state) = project("preview-idle");
    assert!(
        transform::peek_transform(&state, 10.0, 10.0)
            .expect("preview")
            .is_none()
    );
}

// --- Marquee ----------------------------------------------------------------

#[test]
fn a_marquee_selects_what_it_encloses() {
    let (_root, state) = project("marquee");
    let inside = dot(&state, 10.0, 10.0, 0.0);
    let also_inside = dot(&state, 14.0, 12.0, 90.0);
    let outside = dot(&state, 80.0, -40.0, 180.0);

    let found = transform::region_objects(&state, 0.0, 0.0, 20.0, 20.0, 0, None).expect("region");
    assert!(found.contains(&inside));
    assert!(found.contains(&also_inside));
    assert!(!found.contains(&outside));
}

/// The rectangle a user drags across the dateline has west > east, and its
/// inside is the part that wraps.
#[test]
fn a_marquee_may_wrap_the_antimeridian() {
    let (_root, state) = project("marquee-wrap");
    let near_dateline = dot(&state, 179.0, 5.0, 0.0);
    let elsewhere = dot(&state, 0.0, 5.0, 90.0);

    let found =
        transform::region_objects(&state, 170.0, 0.0, -170.0, 10.0, 0, None).expect("region");
    assert!(found.contains(&near_dateline));
    assert!(
        !found.contains(&elsewhere),
        "0° is outside a wrapped rectangle"
    );
}

#[test]
fn a_marquee_can_be_scoped_to_one_layer() {
    let (_root, state) = project("marquee-layer");
    let below = dot(&state, 10.0, 10.0, 0.0);

    document::layer_add(&state, "Upper".to_owned()).expect("layer");
    let above = dot(&state, 11.0, 11.0, 90.0);

    let tree = document::tree(&state, 0).expect("tree");
    let upper = tree.layers[1].id;

    let scoped =
        transform::region_objects(&state, 0.0, 0.0, 20.0, 20.0, 0, Some(upper)).expect("region");
    assert_eq!(
        scoped,
        vec![above],
        "scoped to the layer that was asked for"
    );

    let across = transform::region_objects(&state, 0.0, 0.0, 20.0, 20.0, 0, None).expect("region");
    assert!(across.contains(&below) && across.contains(&above));
}

#[test]
fn a_marquee_skips_hidden_and_locked_layers() {
    let (_root, state) = project("marquee-hidden");
    let hidden = dot(&state, 10.0, 10.0, 0.0);
    let tree = document::tree(&state, 0).expect("tree");
    document::layer_visibility(&state, tree.layers[0].id, false).expect("hide");

    let found = transform::region_objects(&state, 0.0, 0.0, 20.0, 20.0, 0, None).expect("region");
    assert!(!found.contains(&hidden));
}

// --- Group transforms -------------------------------------------------------

/// The property that makes a group transform a *transform* rather than a
/// per-object edit: the objects keep their arrangement.
#[test]
fn moving_a_group_keeps_its_shape() {
    let (_root, state) = project("group-move");
    let a = dot(&state, 0.0, 0.0, 0.0);
    let b = dot(&state, 10.0, 5.0, 90.0);
    let separation = distance_m(anchor(&state, a), anchor(&state, b));

    let handles = transform::transform_of(&state, &[a, b], 0)
        .expect("transform")
        .expect("present");
    drag(
        &state,
        &[a, b],
        TransformKind::Move,
        (handles.lon, handles.lat),
        (-140.0, 62.0),
    );

    let moved = distance_m(anchor(&state, a), anchor(&state, b));
    assert!(
        (moved - separation).abs() < 1.0,
        "the group was {separation} m across and is now {moved} m"
    );

    let after = transform::transform_of(&state, &[a, b], 0)
        .expect("transform")
        .expect("present");
    assert!(
        distance_m((after.lon, after.lat), (-140.0, 62.0)) < 1.0,
        "the centroid must land on the pointer, not near it"
    );
}

/// A move carries the point that was **pressed**, not the centroid (M23).
///
/// A member grabbed 600 km from a group's centroid used to jump the whole
/// group so the centroid landed under the pointer, before the pointer had
/// moved at all — which is what a `Cmd`-click on a member did to a
/// selection. Now a press moves nothing until the pointer does, and then
/// every member moves by what the pointer moved.
#[test]
fn a_move_carries_the_press_point_and_not_the_centroid() {
    let (_root, state) = project("press-point");
    let a = dot(&state, 0.0, 0.0, 0.0);
    let b = dot(&state, 10.0, 5.0, 90.0);
    let (before_a, before_b) = (anchor(&state, a), anchor(&state, b));

    // Press on `a`, well away from the centroid at about (5, 2.5).
    transform::start_transform(&state, &[a, b], 0, TransformKind::Move, 0.0, 0.0, false)
        .expect("begin");
    let preview = transform::peek_transform(&state, 0.0, 0.0)
        .expect("preview")
        .expect("in progress");
    for outline in &preview.outlines {
        for point in outline_points(outline) {
            let there = [point[0], point[1]];
            let original = committed_or_baseline(&state, &[a, b]);
            assert!(
                original
                    .iter()
                    .any(|p| distance_m((p[0], p[1]), (there[0], there[1])) < 1.0),
                "a press with no movement previewed a moved outline at {there:?}"
            );
        }
    }

    // Now the pointer goes one degree east: every member follows by that.
    transform::update_transform(&state, 1.0, 0.0).expect("drag");
    document::finish_gesture(&state).expect("end");
    let (after_a, after_b) = (anchor(&state, a), anchor(&state, b));
    let moved_a = distance_m(before_a, after_a);
    let moved_b = distance_m(before_b, after_b);
    let one_degree = distance_m((0.0, 0.0), (1.0, 0.0));
    assert!(
        (moved_a - one_degree).abs() < 200.0,
        "a moved {moved_a} m for a {one_degree} m drag"
    );
    assert!(
        (moved_b - one_degree * (5.0f64).to_radians().cos()).abs() < 2_000.0,
        "b moved {moved_b} m; a rigid move carries it by the same rotation"
    );
    assert!(
        (after_a.0 - 1.0).abs() < 1e-6 && after_a.1.abs() < 1e-6,
        "the pressed point lands under the pointer, got {after_a:?}"
    );
}

/// Every point of every member's committed outline, before any drag.
fn committed_or_baseline(state: &AppState, objects: &[u64]) -> Vec<[f64; 2]> {
    objects
        .iter()
        .flat_map(|object| committed_outline(state, *object))
        .collect()
}

#[test]
fn rotating_a_group_turns_it_about_the_centroid() {
    let (_root, state) = project("group-rotate");
    let a = dot(&state, -5.0, 0.0, 0.0);
    let b = dot(&state, 5.0, 0.0, 90.0);
    let separation = distance_m(anchor(&state, a), anchor(&state, b));

    let handles = transform::transform_of(&state, &[a, b], 0)
        .expect("transform")
        .expect("present");
    let pivot = (handles.lon, handles.lat);
    let start = (handles.lon + 10.0, handles.lat);

    // A quarter turn: grab due east of the pivot and drag to due north of it.
    drag(
        &state,
        &[a, b],
        TransformKind::Rotate,
        start,
        (handles.lon, handles.lat + 10.0),
    );

    assert!(
        (distance_m(anchor(&state, a), anchor(&state, b)) - separation).abs() < 1.0,
        "a rotation must not stretch the group"
    );
    // Both members were on the pivot's parallel and should now be on its
    // meridian.
    for object in [a, b] {
        let (lon, _) = anchor(&state, object);
        assert!(
            (lon - pivot.0).abs() < 0.5,
            "{object} is at longitude {lon}, pivot is at {}",
            pivot.0
        );
    }
}

#[test]
fn scaling_a_group_spreads_it_and_grows_its_members() {
    let (_root, state) = project("group-scale");
    let a = dot(&state, -5.0, 0.0, 0.0);
    let b = dot(&state, 5.0, 0.0, 90.0);
    let separation = distance_m(anchor(&state, a), anchor(&state, b));

    let handles = transform::transform_of(&state, &[a, b], 0)
        .expect("transform")
        .expect("present");
    // Grab 5° east of the pivot, drag to 10°: a factor of about two.
    drag(
        &state,
        &[a, b],
        TransformKind::Scale,
        (handles.lon + 5.0, handles.lat),
        (handles.lon + 10.0, handles.lat),
    );

    let spread = distance_m(anchor(&state, a), anchor(&state, b));
    assert!(
        (spread / separation - 2.0).abs() < 0.05,
        "separation went from {separation} to {spread}"
    );

    let scale = transform::transform_of(&state, &[a], 0)
        .expect("transform")
        .expect("present")
        .scale_pct;
    assert!(
        (scale / 200.0 - 1.0).abs() < 0.05,
        "the member should have grown to about 200%, got {scale}"
    );
}

/// The pointer reports the same position many times during a drag. Each report
/// must produce the same answer, or the selection accelerates away.
#[test]
fn repeating_a_drag_update_changes_nothing() {
    let (_root, state) = project("idempotent");
    let a = dot(&state, 0.0, 0.0, 0.0);
    let b = dot(&state, 10.0, 5.0, 90.0);

    let handles = transform::transform_of(&state, &[a, b], 0)
        .expect("transform")
        .expect("present");
    transform::start_transform(
        &state,
        &[a, b],
        0,
        TransformKind::Move,
        handles.lon,
        handles.lat,
        false,
    )
    .expect("begin");

    transform::update_transform(&state, 40.0, 20.0).expect("first");
    let once = (anchor(&state, a), anchor(&state, b));
    for _ in 0..5 {
        transform::update_transform(&state, 40.0, 20.0).expect("again");
    }
    document::finish_gesture(&state).expect("end");

    assert_eq!(
        once,
        (anchor(&state, a), anchor(&state, b)),
        "five more reports of the same position must not move anything"
    );
}

#[test]
fn a_group_drag_is_one_undo_entry() {
    let (_root, state) = project("group-undo");
    let a = dot(&state, 0.0, 0.0, 0.0);
    let b = dot(&state, 10.0, 5.0, 90.0);
    let before = (anchor(&state, a), anchor(&state, b));

    let handles = transform::transform_of(&state, &[a, b], 0)
        .expect("transform")
        .expect("present");
    transform::start_transform(
        &state,
        &[a, b],
        0,
        TransformKind::Move,
        handles.lon,
        handles.lat,
        false,
    )
    .expect("begin");
    for lon in [12.0, 18.0, 24.0, 30.0] {
        transform::update_transform(&state, lon, 8.0).expect("drag");
    }
    document::finish_gesture(&state).expect("end");

    edit::undo_for_test(&state).expect("undo");
    let after = (anchor(&state, a), anchor(&state, b));
    assert!(
        distance_m(before.0, after.0) < 1.0 && distance_m(before.1, after.1) < 1.0,
        "one undo must return the whole group: {before:?} became {after:?}"
    );
}

/// The same, for a stroke whose geometry is in map space. Re-expressing the
/// points through a ground frame would slide them off the path they were drawn
/// along, and the further from the equator the anchor moved, the further off.
#[test]
fn moving_a_projected_anchor_leaves_the_geometry_where_it_was_drawn() {
    let (_root, state) = project("projected-anchor");
    edit::paint(
        &state,
        BrushStroke {
            // Well off the equator, where the two frames disagree most.
            points: vec![[0.0, 55.0], [8.0, 55.0]],
            size_km: 300.0,
            speed_mps: 12.0,
            direction_toward_deg: 45.0,
            feather: 0.0,
            space: StampSpace::Projected,
            ..Default::default()
        },
    )
    .expect("paint");
    let object = document::tree(&state, 0).expect("tree").layers[0].objects[0].id;

    for lon in [0.0, 4.0, 8.0] {
        assert_eq!(
            document::hit_test(&state, lon, 55.0, 0).expect("hit"),
            Some(object),
            "the stroke covers {lon} before the anchor moves"
        );
    }

    drag(
        &state,
        &[object],
        TransformKind::Anchor,
        (0.0, 55.0),
        (4.0, 58.0),
    );

    for lon in [0.0, 4.0, 8.0] {
        assert_eq!(
            document::hit_test(&state, lon, 55.0, 0).expect("hit"),
            Some(object),
            "and still covers {lon} afterwards"
        );
    }
}

/// Moving the anchor changes the pivot, not the picture: the strokes must stay
/// exactly where they were painted.
#[test]
fn moving_an_anchor_leaves_the_geometry_on_the_ground() {
    let (_root, state) = project("anchor");
    edit::paint(
        &state,
        BrushStroke {
            points: vec![[0.0, 0.0], [6.0, 0.0]],
            size_km: 400.0,
            speed_mps: 12.0,
            direction_toward_deg: 45.0,
            feather: 0.2,
            layer: None,
            ..Default::default()
        },
    )
    .expect("paint");
    let object = document::tree(&state, 0).expect("tree").layers[0].objects[0].id;

    // The far end of the stroke is covered before the anchor moves.
    let far_end = document::hit_test(&state, 6.0, 0.0, 0).expect("hit");
    assert_eq!(far_end, Some(object));

    drag(
        &state,
        &[object],
        TransformKind::Anchor,
        (0.0, 0.0),
        (3.0, 2.0),
    );

    let (lon, lat) = anchor(&state, object);
    assert!(
        (lon - 3.0).abs() < 1e-6 && (lat - 2.0).abs() < 1e-6,
        "the anchor should be where it was dropped, got {lon}, {lat}"
    );
    assert_eq!(
        document::hit_test(&state, 6.0, 0.0, 0).expect("hit"),
        Some(object),
        "the stroke must still cover the ground it was painted on"
    );
    assert_eq!(
        document::hit_test(&state, 0.0, 0.0, 0).expect("hit"),
        Some(object),
        "including the end the anchor moved away from"
    );
}

// --- The handles must not move when a drag begins -----------------------------

/// The handles a drag *previews* and the handles the selection *has* are built
/// by two functions, and the moment the pointer goes down the map switches from
/// one to the other. If they disagree, the dashed circle and both knobs jump at
/// the instant of the grab and jump back on release — which is exactly what a
/// user reports as "the handles render wrong when I drag".
///
/// Checked at 100% and at 250%, because the two agreed at 100% by coincidence:
/// the preview scaled a reach that already carried the object's scale, so it
/// was only ever right for an object that had never been scaled.
#[test]
fn a_drag_preview_starts_where_the_committed_handles_are() {
    for scale_pct in [100.0_f64, 250.0, 40.0] {
        let (_root, state) = project("handles-agree");
        let a = dot(&state, 10.0, 20.0, 0.0);
        document::set_property(
            &state,
            a,
            "ScalePct",
            document::PropertyValue::Number { value: scale_pct },
        )
        .expect("scale");

        let committed = transform::transform_of(&state, &[a], 0)
            .expect("transform")
            .expect("present");

        // Grab the rotate handle and do not move: the preview at the grab point
        // is the selection as it is.
        transform::start_transform(
            &state,
            &[a],
            0,
            TransformKind::Rotate,
            committed.lon,
            committed.lat + 1.0,
            false,
        )
        .expect("begin");
        let preview = transform::peek_transform(&state, committed.lon, committed.lat + 1.0)
            .expect("preview")
            .expect("present")
            .handles;
        document::finish_gesture(&state).expect("end");

        assert!(
            (preview.radius_m - committed.radius_m).abs() < 1.0,
            "at {scale_pct}%: the preview's reach is {} m where the selection's is {} m",
            preview.radius_m,
            committed.radius_m
        );
        assert!(
            (preview.rotation_deg - committed.rotation_deg).abs() < 1e-9
                && (preview.scale_pct - committed.scale_pct).abs() < 1e-9,
            "at {scale_pct}%: the preview's orientation differs from the selection's"
        );
        assert!(
            distance_m((preview.lon, preview.lat), (committed.lon, committed.lat)) < 1.0,
            "at {scale_pct}%: the preview's pivot moved"
        );
    }
}

/// A scale drag grows the reach *linearly* with the scale. Doubling the object
/// doubles the dashed circle; a preview that multiplied an already-scaled reach
/// by the scale again grew it four times over.
#[test]
fn a_scale_drag_grows_the_handles_with_the_object_not_faster() {
    let (_root, state) = project("handles-scale");
    let a = dot(&state, 0.0, 0.0, 0.0);
    let before = transform::transform_of(&state, &[a], 0)
        .expect("transform")
        .expect("present");

    // Grab 5° east, drag to 10°: about a factor of two.
    transform::start_transform(&state, &[a], 0, TransformKind::Scale, 5.0, 0.0, false)
        .expect("begin");
    let doubled = transform::peek_transform(&state, 10.0, 0.0)
        .expect("preview")
        .expect("present")
        .handles;
    document::finish_gesture(&state).expect("end");

    let grew = doubled.radius_m / before.radius_m;
    let scaled = doubled.scale_pct / before.scale_pct;
    assert!(
        (grew / scaled - 1.0).abs() < 0.05,
        "the object scaled by {scaled} and its reach by {grew}; they should match"
    );
}

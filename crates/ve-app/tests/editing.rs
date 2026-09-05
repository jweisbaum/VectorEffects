#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! The editing commands behind the layer panel and the inspector.

use ve_app::commands::AppState;
use ve_app::document::{self, PropertyValue};
use ve_app::edit::{self, BrushStroke};
use ve_app::paths::AppPaths;
use ve_app::projects::{self, NewProjectRequest};
use ve_app::transform;

struct TempRoot(std::path::PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ve-editing-{}-{label}-{}",
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

/// A project with one painted stroke.
fn painted(label: &str) -> (TempRoot, AppState) {
    let root = TempRoot::new(label);
    let state = AppState::new(AppPaths::in_directory(&root.0).expect("paths"));
    projects::create(
        &state,
        NewProjectRequest {
            name: "Edit".to_owned(),
            field_kind: "wind".to_owned(),
            resolution: "1.0".to_owned(),
            step_hours: 3,
            step_count: 12,
        },
        false,
    )
    .expect("create");
    edit::paint(
        &state,
        BrushStroke {
            points: vec![[0.0, 0.0], [10.0, 5.0]],
            size_km: 800.0,
            speed_mps: 15.0,
            direction_toward_deg: 90.0,
            feather: 0.3,
            layer: None,
            ..Default::default()
        },
    )
    .expect("paint");
    (root, state)
}

fn first_object(state: &AppState) -> u64 {
    document::tree(state, 0).expect("tree").layers[0].objects[0].id
}

// --- The tree ---------------------------------------------------------------

#[test]
fn the_tree_reflects_the_document() {
    let (_root, state) = painted("tree");
    let tree = document::tree(&state, 0).expect("tree");

    assert_eq!(tree.layers.len(), 1);
    let layer = &tree.layers[0];
    assert!(layer.visible && !layer.locked);
    assert_eq!(layer.objects.len(), 1);

    let object = &layer.objects[0];
    assert_eq!(object.tool, "brush");
    assert_eq!(object.tool_label, "Brush");
    assert!(object.active_here);
    assert_eq!((object.start_step, object.end_step), (0, 11));
}

/// Layers come back bottom first — the order they composite in.
#[test]
fn layers_are_reported_bottom_first() {
    let (_root, state) = painted("order");
    document::layer_add(&state, "Upper".to_owned()).expect("add");

    let tree = document::tree(&state, 0).expect("tree");
    assert_eq!(tree.layers.len(), 2);
    assert_eq!(tree.layers[1].name, "Upper", "a new layer goes on top");
    assert!(
        !tree.layers[0].objects.is_empty(),
        "the painted stroke stays below"
    );
}

// --- Layers -----------------------------------------------------------------

#[test]
fn layers_can_be_added_renamed_and_removed() {
    let (_root, state) = painted("layers");
    let added = document::layer_add(&state, String::new()).expect("add");
    assert_eq!(added.layer_count, 2);

    let id = document::tree(&state, 0).expect("tree").layers[1].id;
    document::layer_rename(&state, id, "Jet stream".to_owned()).expect("rename");
    assert_eq!(
        document::tree(&state, 0).expect("tree").layers[1].name,
        "Jet stream"
    );

    let removed = document::layer_remove(&state, id).expect("remove");
    assert_eq!(removed.layer_count, 1);
}

/// A project must always have somewhere to put an object.
#[test]
fn the_last_layer_cannot_be_removed() {
    let (_root, state) = painted("last-layer");
    let id = document::tree(&state, 0).expect("tree").layers[0].id;
    assert!(document::layer_remove(&state, id).is_err());
    assert_eq!(document::tree(&state, 0).expect("tree").layers.len(), 1);
}

#[test]
fn layers_can_be_hidden_and_locked() {
    let (_root, state) = painted("visibility");
    let id = document::tree(&state, 0).expect("tree").layers[0].id;

    document::layer_visibility(&state, id, false).expect("hide");
    document::layer_lock(&state, id, true).expect("lock");

    let layer = &document::tree(&state, 0).expect("tree").layers[0];
    assert!(!layer.visible);
    assert!(layer.locked);
}

#[test]
fn layers_can_be_reordered() {
    let (_root, state) = painted("reorder");
    document::layer_add(&state, "Second".to_owned()).expect("add");
    document::layer_move(&state, 1, 0).expect("move");

    let tree = document::tree(&state, 0).expect("tree");
    assert_eq!(tree.layers[0].name, "Second", "moved to the bottom");
}

#[test]
fn reordering_past_the_end_is_refused() {
    let (_root, state) = painted("reorder-bad");
    assert!(document::layer_move(&state, 0, 9).is_err());
}

// --- Objects ----------------------------------------------------------------

#[test]
fn objects_can_be_renamed_and_deleted() {
    let (_root, state) = painted("objects");
    let id = first_object(&state);

    document::object_rename(&state, id, "Trade winds".to_owned()).expect("rename");
    assert_eq!(
        document::tree(&state, 0).expect("tree").layers[0].objects[0].name,
        "Trade winds"
    );

    let after = document::object_remove(&state, id).expect("remove");
    assert_eq!(after.object_count, 0);
}

/// A duplicate must be a separate object, or every later command would edit
/// both at once.
#[test]
fn a_duplicate_is_a_distinct_object() {
    let (_root, state) = painted("duplicate");
    let id = first_object(&state);
    document::object_duplicate(&state, id).expect("duplicate");

    let objects = &document::tree(&state, 0).expect("tree").layers[0].objects;
    assert_eq!(objects.len(), 2);
    assert_ne!(
        objects[0].id, objects[1].id,
        "the copy needs its own identity"
    );
    assert!(objects[1].name.ends_with("copy"));
    assert_eq!(objects[0].id, id, "and sits directly above the original");
}

#[test]
fn objects_can_move_between_layers() {
    let (_root, state) = painted("move-object");
    document::layer_add(&state, "Upper".to_owned()).expect("add");

    let tree = document::tree(&state, 0).expect("tree");
    let object = tree.layers[0].objects[0].id;
    let upper = tree.layers[1].id;

    document::object_move(&state, object, upper, 0).expect("move");

    let tree = document::tree(&state, 0).expect("tree");
    assert!(tree.layers[0].objects.is_empty());
    assert_eq!(tree.layers[1].objects.len(), 1);
}

#[test]
fn an_active_range_is_clamped_to_the_project() {
    let (_root, state) = painted("range");
    let id = first_object(&state);
    document::object_range(&state, id, 2, 999).expect("range");

    let object = &document::tree(&state, 0).expect("tree").layers[0].objects[0];
    assert_eq!((object.start_step, object.end_step), (2, 11));
    assert!(!object.active_here, "step 0 is now outside its lifetime");
    assert!(document::tree(&state, 5).expect("tree").layers[0].objects[0].active_here);
}

// --- Properties -------------------------------------------------------------

/// The inspector is built from the schema, so a tool's properties are exactly
/// the ones the schema declares.
#[test]
fn properties_come_from_the_schema() {
    let (_root, state) = painted("properties");
    let id = first_object(&state);
    let properties = document::properties(&state, id, 0).expect("properties");

    let names: Vec<&str> = properties.iter().map(|p| p.id.as_str()).collect();
    assert!(names.contains(&"Speed"), "{names:?}");
    assert!(names.contains(&"Position"));
    assert!(names.contains(&"Feather"));
    assert!(names.contains(&"EdgeMode"));
    // The circle's ring width belongs to the circle, not the brush.
    assert!(!names.contains(&"RingWidthKm"), "{names:?}");

    let speed = properties.iter().find(|p| p.id == "Speed").expect("speed");
    assert_eq!(speed.label, "Speed");
    assert_eq!(speed.unit, "speed", "stored m/s, shown in knots");
    assert!(matches!(speed.value, PropertyValue::Number { value } if (value - 15.0).abs() < 1e-3));
    assert!(!speed.animated);

    let edge = properties
        .iter()
        .find(|p| p.id == "EdgeMode")
        .expect("edge mode");
    assert_eq!(edge.variants, vec!["blend", "replace"]);
}

#[test]
fn a_property_can_be_changed_and_undone() {
    let (_root, state) = painted("set-property");
    let id = first_object(&state);

    let before = document::properties(&state, id, 0).expect("properties");
    let summary =
        document::set_property(&state, id, "Speed", PropertyValue::Number { value: 33.0 })
            .expect("set");
    assert!(summary.can_undo);
    assert!(summary.dirty);

    let after = document::properties(&state, id, 0).expect("properties");
    let speed = after.iter().find(|p| p.id == "Speed").expect("speed");
    assert!(matches!(speed.value, PropertyValue::Number { value } if (value - 33.0).abs() < 1e-3));

    edit::undo_for_test(&state).expect("undo");
    let restored = document::properties(&state, id, 0).expect("properties");
    assert_eq!(
        format!(
            "{:?}",
            restored.iter().find(|p| p.id == "Speed").map(|p| &p.value)
        ),
        format!(
            "{:?}",
            before.iter().find(|p| p.id == "Speed").map(|p| &p.value)
        ),
    );
}

/// A value of the wrong shape must be refused, not coerced into nonsense.
#[test]
fn a_mismatched_property_value_is_refused() {
    let (_root, state) = painted("mismatch");
    let id = first_object(&state);

    assert!(
        document::set_property(&state, id, "Speed", PropertyValue::Bool { value: true }).is_err(),
        "a boolean is not a speed"
    );
    assert!(
        document::set_property(&state, id, "Position", PropertyValue::Number { value: 1.0 })
            .is_err(),
        "a number is not a position"
    );
}

#[test]
fn an_unknown_property_is_refused() {
    let (_root, state) = painted("unknown");
    let id = first_object(&state);
    assert!(
        document::set_property(&state, id, "Nonsense", PropertyValue::Number { value: 1.0 })
            .is_err()
    );
}

/// Every structural edit goes through the undo stack.
#[test]
fn structural_edits_are_undoable() {
    let (_root, state) = painted("undo");
    document::layer_add(&state, "Second".to_owned()).expect("add");
    assert_eq!(document::tree(&state, 0).expect("tree").layers.len(), 2);

    edit::undo_for_test(&state).expect("undo");
    assert_eq!(document::tree(&state, 0).expect("tree").layers.len(), 1);
}

// --- Selection --------------------------------------------------------------

#[test]
fn clicking_inside_a_stroke_selects_it() {
    let (_root, state) = painted("hit");
    let id = first_object(&state);

    // The stroke runs from (0,0) to (10,5) with a 400 km radius.
    assert_eq!(
        document::hit_test(&state, 0.0, 0.0, 0).expect("hit"),
        Some(id)
    );
    assert_eq!(
        document::hit_test(&state, 5.0, 2.5, 0).expect("hit"),
        Some(id)
    );
    assert_eq!(
        document::hit_test(&state, 90.0, 60.0, 0).expect("hit"),
        None
    );
}

/// What a click selects must be what the user can see: the topmost object.
#[test]
fn hit_testing_returns_the_topmost_object() {
    let (_root, state) = painted("hit-order");
    let lower = first_object(&state);
    edit::paint(
        &state,
        BrushStroke {
            points: vec![[0.0, 0.0]],
            size_km: 600.0,
            speed_mps: 20.0,
            direction_toward_deg: 0.0,
            feather: 0.0,
            layer: None,
            ..Default::default()
        },
    )
    .expect("paint");

    let objects = &document::tree(&state, 0).expect("tree").layers[0].objects;
    let upper = objects[1].id;
    assert_ne!(upper, lower);
    assert_eq!(
        document::hit_test(&state, 0.0, 0.0, 0).expect("hit"),
        Some(upper)
    );
}

/// If you cannot see it or edit it, you cannot select it by clicking.
#[test]
fn hidden_and_locked_layers_are_not_selectable() {
    let (_root, state) = painted("hit-hidden");
    let layer = document::tree(&state, 0).expect("tree").layers[0].id;

    document::layer_visibility(&state, layer, false).expect("hide");
    assert_eq!(document::hit_test(&state, 0.0, 0.0, 0).expect("hit"), None);

    document::layer_visibility(&state, layer, true).expect("show");
    document::layer_lock(&state, layer, true).expect("lock");
    assert_eq!(document::hit_test(&state, 0.0, 0.0, 0).expect("hit"), None);
}

/// Outside its lifetime an object is not there to be clicked.
#[test]
fn an_inactive_object_is_not_selectable() {
    let (_root, state) = painted("hit-range");
    let id = first_object(&state);
    document::object_range(&state, id, 5, 8).expect("range");

    assert_eq!(document::hit_test(&state, 0.0, 0.0, 0).expect("hit"), None);
    assert_eq!(
        document::hit_test(&state, 0.0, 0.0, 6).expect("hit"),
        Some(id)
    );
}

// --- Transform --------------------------------------------------------------

#[test]
fn an_objects_transform_is_reported_for_the_handles() {
    let (_root, state) = painted("transform");
    let id = first_object(&state);
    let transform = transform::transform_of(&state, &[id], 0)
        .expect("query")
        .expect("present");

    // The stroke was anchored at its first point.
    assert!((transform.lon - 0.0).abs() < 1e-6);
    assert!((transform.lat - 0.0).abs() < 1e-6);
    assert!((transform.scale_pct - 100.0).abs() < 1e-6);
    // It runs to (10, 5) with a 400 km brush radius, so it reaches well past
    // a megametre.
    assert!(transform.radius_m > 1_000_000.0, "{}", transform.radius_m);
}

/// The radius drives where the scale handle sits, so it must follow scale.
#[test]
fn the_reported_radius_includes_scale() {
    let (_root, state) = painted("transform-scale");
    let id = first_object(&state);
    let before = transform::transform_of(&state, &[id], 0)
        .expect("query")
        .expect("present");

    document::set_property(
        &state,
        id,
        "ScalePct",
        PropertyValue::Number { value: 200.0 },
    )
    .expect("scale");
    let after = transform::transform_of(&state, &[id], 0)
        .expect("query")
        .expect("present");

    assert!((after.scale_pct - 200.0).abs() < 1e-6);
    assert!(
        (after.radius_m - before.radius_m * 2.0).abs() < 1.0,
        "{} vs {}",
        after.radius_m,
        before.radius_m
    );
}

#[test]
fn an_object_outside_its_lifetime_has_no_handles() {
    let (_root, state) = painted("transform-inactive");
    let id = first_object(&state);
    document::object_range(&state, id, 5, 8).expect("range");

    assert!(
        transform::transform_of(&state, &[id], 0)
            .expect("query")
            .is_none()
    );
    assert!(
        transform::transform_of(&state, &[id], 6)
            .expect("query")
            .is_some()
    );
}

/// A drag emits a change per pointer event but must be a single undo.
#[test]
fn a_gesture_collapses_into_one_undo_entry() {
    let (_root, state) = painted("gesture");
    let id = first_object(&state);
    let start = transform::transform_of(&state, &[id], 0)
        .expect("query")
        .expect("present");

    for degrees in [10.0, 20.0, 30.0, 45.0] {
        document::set_property_with(
            &state,
            id,
            "RotationDeg",
            PropertyValue::Angle { degrees },
            Some("rotate:1".to_owned()),
            0,
            false,
        )
        .expect("rotate");
    }
    document::finish_gesture(&state).expect("end");

    let rotated = transform::transform_of(&state, &[id], 0)
        .expect("query")
        .expect("present");
    assert!((rotated.rotation_deg - 45.0).abs() < 1e-6);

    // One undo must return to where the drag started, not to 30 degrees.
    edit::undo_for_test(&state).expect("undo");
    let restored = transform::transform_of(&state, &[id], 0)
        .expect("query")
        .expect("present");
    assert!(
        (restored.rotation_deg - start.rotation_deg).abs() < 1e-6,
        "one undo should span the whole drag, got {}",
        restored.rotation_deg
    );
}

/// Ending a gesture starts a new entry, so two drags undo separately.
#[test]
fn separate_gestures_undo_separately() {
    let (_root, state) = painted("two-gestures");
    let id = first_object(&state);

    document::set_property_with(
        &state,
        id,
        "RotationDeg",
        PropertyValue::Angle { degrees: 30.0 },
        Some("rotate:1".to_owned()),
        0,
        false,
    )
    .expect("first");
    document::finish_gesture(&state).expect("end");

    document::set_property_with(
        &state,
        id,
        "RotationDeg",
        PropertyValue::Angle { degrees: 90.0 },
        Some("rotate:2".to_owned()),
        0,
        false,
    )
    .expect("second");
    document::finish_gesture(&state).expect("end");

    edit::undo_for_test(&state).expect("undo");
    let after = transform::transform_of(&state, &[id], 0)
        .expect("query")
        .expect("present");
    assert!(
        (after.rotation_deg - 30.0).abs() < 1e-6,
        "the first drag should survive, got {}",
        after.rotation_deg
    );
}

/// M5 acceptance: rotation is a true bearing rotation, not a shear of lat/lon
/// space. Checked away from the equator, where the two differ.
#[test]
fn rotation_is_a_true_bearing_rotation_at_high_latitude() {
    use ve_core::angle::Angle;

    let root = TempRoot::new("rotate-60n");
    let state = AppState::new(AppPaths::in_directory(&root.0).expect("paths"));
    projects::create(
        &state,
        NewProjectRequest {
            name: "Rotate".to_owned(),
            field_kind: "wind".to_owned(),
            resolution: "1.0".to_owned(),
            step_hours: 3,
            step_count: 4,
        },
        false,
    )
    .expect("create");

    // A stroke running due east from 60N.
    let anchor = ve_core::LonLat::new(0.0, 60.0).expect("position");
    let east = anchor.destination(Angle::new(90.0), 600_000.0);
    edit::paint(
        &state,
        BrushStroke {
            points: vec![[anchor.lon, anchor.lat], [east.lon, east.lat]],
            size_km: 200.0,
            speed_mps: 10.0,
            direction_toward_deg: 90.0,
            feather: 0.0,
            layer: None,
            ..Default::default()
        },
    )
    .expect("paint");
    let id = first_object(&state);

    // Before rotating: covered to the east, clear to the south.
    let south = anchor.destination(Angle::new(180.0), 600_000.0);
    assert_eq!(
        document::hit_test(&state, east.lon, east.lat, 0).expect("hit"),
        Some(id)
    );
    assert_eq!(
        document::hit_test(&state, south.lon, south.lat, 0).expect("hit"),
        None
    );

    document::set_property(
        &state,
        id,
        "RotationDeg",
        PropertyValue::Angle { degrees: 90.0 },
    )
    .expect("rotate");

    // After a 90 degree turn the stroke runs due south — a true bearing away
    // from the anchor. A lat/lon-space shear would land somewhere else, since
    // 600 km is 10.8 degrees of longitude at 60N but only 5.4 of latitude.
    assert_eq!(
        document::hit_test(&state, south.lon, south.lat, 0).expect("hit"),
        Some(id),
        "the rotated stroke should now run south"
    );
    assert_eq!(
        document::hit_test(&state, east.lon, east.lat, 0).expect("hit"),
        None,
        "and no longer east"
    );
}

// --- Clipboard and history --------------------------------------------------

#[test]
fn objects_can_be_copied_and_pasted() {
    let (_root, state) = painted("clipboard");
    let id = first_object(&state);

    let clipboard = document::clipboard_copy(&state, &[id], 0).expect("copy");
    assert_eq!(clipboard.count, 1);

    let after = document::clipboard_paste(&state, None, 0, false).expect("paste");
    assert_eq!(after.object_count, 2);

    let objects = &document::tree(&state, 0).expect("tree").layers[0].objects;
    assert_ne!(
        objects[0].id, objects[1].id,
        "the copy needs its own identity"
    );
    assert!(objects[1].name.ends_with("copy"));
}

/// A paste is one action, so it is one undo.
#[test]
fn a_paste_is_a_single_undo() {
    let (_root, state) = painted("paste-undo");
    let id = first_object(&state);
    document::object_duplicate(&state, id).expect("duplicate");

    let objects: Vec<u64> = document::tree(&state, 0).expect("tree").layers[0]
        .objects
        .iter()
        .map(|o| o.id)
        .collect();
    document::clipboard_copy(&state, &objects, 0).expect("copy");
    document::clipboard_paste(&state, None, 0, false).expect("paste");
    assert_eq!(
        document::tree(&state, 0).expect("tree").layers[0]
            .objects
            .len(),
        4
    );

    edit::undo_for_test(&state).expect("undo");
    assert_eq!(
        document::tree(&state, 0).expect("tree").layers[0]
            .objects
            .len(),
        2,
        "one undo should remove both pasted objects"
    );
}

#[test]
fn cutting_copies_and_then_removes() {
    let (_root, state) = painted("cut");
    let id = first_object(&state);

    let after = document::clipboard_cut(&state, &[id], 0).expect("cut");
    assert_eq!(after.object_count, 0);

    document::clipboard_paste(&state, None, 0, false).expect("paste");
    assert_eq!(
        document::tree(&state, 0).expect("tree").layers[0]
            .objects
            .len(),
        1
    );
}

/// One clipboard (spec.md 8.5, M23): copying objects drops a held capture
/// and capturing a region drops the copied objects, so `Cmd`-`V` can ask
/// what is held. Before this a capture, once taken, answered every paste for
/// the rest of the session and objects stopped copying.
#[test]
fn a_capture_and_an_object_copy_share_one_clipboard() {
    use ve_app::capture::{self, RegionShape};
    use ve_app::document::ClipboardKind;
    let (_root, state) = painted("one-clipboard");
    let id = first_object(&state);

    assert_eq!(
        document::kind_held(&state).expect("kind"),
        ClipboardKind::Empty
    );
    document::clipboard_copy(&state, &[id], 0).expect("copy");
    assert_eq!(
        document::kind_held(&state).expect("kind"),
        ClipboardKind::Objects
    );

    capture::region_capture(
        &state,
        RegionShape::Rect {
            centre: [5.0, 2.0],
            half_width_deg: 6.0,
            half_height_deg: 4.0,
        },
        0,
    )
    .expect("capture");
    assert_eq!(
        document::kind_held(&state).expect("kind"),
        ClipboardKind::Capture
    );
    assert!(
        document::clipboard_paste(&state, None, 0, false).is_err(),
        "the objects are gone from the clipboard"
    );

    document::clipboard_copy(&state, &[id], 0).expect("copy again");
    assert_eq!(
        document::kind_held(&state).expect("kind"),
        ClipboardKind::Objects
    );
    assert!(!capture::capture_held(&state).expect("state").has_capture);
    let after = document::clipboard_paste(&state, None, 0, false).expect("paste");
    assert_eq!(after.object_count, 2, "and the objects paste, keys and all");
}

/// `Delete` on a selection is one history entry (M23).
#[test]
fn deleting_a_selection_is_one_undo() {
    let (_root, state) = painted("delete-selection");
    let id = first_object(&state);
    document::object_duplicate(&state, id).expect("duplicate");
    document::object_duplicate(&state, id).expect("duplicate");
    let objects: Vec<u64> = document::tree(&state, 0).expect("tree").layers[0]
        .objects
        .iter()
        .map(|o| o.id)
        .collect();
    assert_eq!(objects.len(), 3);

    // In selection order, which is not stack order: the removal has to sort.
    let after =
        document::objects_remove(&state, &[objects[0], objects[2], objects[1]]).expect("remove");
    assert_eq!(after.object_count, 0);

    edit::undo_for_test(&state).expect("undo");
    let back: Vec<u64> = document::tree(&state, 0).expect("tree").layers[0]
        .objects
        .iter()
        .map(|o| o.id)
        .collect();
    assert_eq!(back, objects, "one undo returns all three, in their order");
}

#[test]
fn pasting_an_empty_clipboard_is_refused() {
    let (_root, state) = painted("paste-empty");
    assert!(document::clipboard_paste(&state, None, 0, false).is_err());
}

/// Pasting into another layer puts the copy there, not back where it came from.
#[test]
fn a_paste_can_target_a_layer() {
    let (_root, state) = painted("paste-layer");
    let id = first_object(&state);
    document::layer_add(&state, "Upper".to_owned()).expect("add");
    document::clipboard_copy(&state, &[id], 0).expect("copy");

    let upper = document::tree(&state, 0).expect("tree").layers[1].id;
    document::clipboard_paste(&state, Some(upper), 0, false).expect("paste");

    let tree = document::tree(&state, 0).expect("tree");
    assert_eq!(tree.layers[0].objects.len(), 1);
    assert_eq!(tree.layers[1].objects.len(), 1);
}

/// Relative timing moves a pasted object's lifetime to the paste step.
#[test]
fn pasting_at_another_step_moves_the_lifetime() {
    let (_root, state) = painted("paste-step");
    let id = first_object(&state);
    document::object_range(&state, id, 0, 3).expect("range");
    document::clipboard_copy(&state, &[id], 0).expect("copy");

    document::clipboard_paste(&state, None, 6, false).expect("paste");
    let objects = &document::tree(&state, 6).expect("tree").layers[0].objects;
    let pasted = objects
        .iter()
        .find(|o| o.name.ends_with("copy"))
        .expect("copy");
    assert_eq!((pasted.start_step, pasted.end_step), (6, 9));

    // Absolute timing keeps the original steps instead.
    document::clipboard_paste(&state, None, 6, true).expect("paste absolute");
    let objects = &document::tree(&state, 0).expect("tree").layers[0].objects;
    let absolute = objects.last().expect("last");
    assert_eq!((absolute.start_step, absolute.end_step), (0, 3));
}

#[test]
fn the_history_reports_what_has_happened() {
    let (_root, state) = painted("history");
    let id = first_object(&state);
    document::object_rename(&state, id, "Renamed".to_owned()).expect("rename");

    let history = document::history_of(&state).expect("history");
    assert_eq!(history.entries.len(), 2, "the paint and the rename");
    assert_eq!(history.cursor, 2);
    assert!(history.entries.iter().all(|entry| entry.applied));
    assert!(history.entries[1].label.contains("Rename"));
}

#[test]
fn the_history_can_be_jumped_through() {
    let (_root, state) = painted("history-jump");
    let id = first_object(&state);
    document::object_rename(&state, id, "One".to_owned()).expect("rename");
    document::object_rename(&state, id, "Two".to_owned()).expect("rename");
    document::finish_gesture(&state).expect("end");

    // Back to just after the paint.
    document::history_jump(&state, 1).expect("jump");
    let history = document::history_of(&state).expect("history");
    assert_eq!(history.cursor, 1);
    assert!(!history.entries[1].applied);

    // And forward again.
    document::history_jump(&state, 3).expect("jump");
    assert_eq!(
        document::tree(&state, 0).expect("tree").layers[0].objects[0].name,
        "Two"
    );
}

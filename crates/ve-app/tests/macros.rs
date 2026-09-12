#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test setup: a panic naming the failed step is the right outcome, and clippy's allow-expect-in-tests does not reach helpers in tests/"
)]
//! Macros: capture a run of frames, keep them in a library, insert them
//! anywhere (spec.md 8.7, M16).
//!
//! The two that matter most, and are easiest to get subtly wrong:
//!
//! * **Static versus record movement.** A region dragged to follow a moving
//!   stroke, captured static, must yield that stroke *standing still*.
//! * **Capture mode is a lockout.** While a capture runs, every document write
//!   is refused — by the backend, not by a disabled button.

use std::path::PathBuf;

use ve_app::capture::RegionShape;
use ve_app::commands::AppState;
use ve_app::create::{self, Gesture, NewObject, Tool, ToolOption};
use ve_app::document::{self, PropertyValue};
use ve_app::edit::{self, BrushStroke};
use ve_app::paths::AppPaths;
use ve_app::projects::{self, NewProjectRequest};
use ve_app::{macros, settings};
use ve_core::LonLat;
use ve_core::schema::ToolKind;
use ve_render::cpu::sample_scene;
use ve_render::scene::flatten;

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ve-macros-{}-{label}-{}",
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

fn app(root: &TempRoot) -> AppState {
    let state = AppState::new(AppPaths::in_directory(&root.0).expect("paths"));
    // The library goes inside the temp root, so nothing escapes the test.
    settings::macro_directory_set(
        &state,
        root.0.join("library").to_string_lossy().into_owned(),
    )
    .expect("library directory");
    project(&state);
    state
}

fn project(state: &AppState) {
    projects::create(
        state,
        NewProjectRequest {
            name: "Macros".to_owned(),
            field_kind: "wind".to_owned(),
            resolution: "1.0".to_owned(),
            step_hours: 3,
            step_count: 6,
        },
        true,
    )
    .expect("create");
}

fn number(property: &str, value: f64) -> ToolOption {
    ToolOption {
        property: property.to_owned(),
        value: PropertyValue::Number { value },
    }
}

/// A stroke that travels east: keyed at step 0 and step 2, 20° apart.
fn travelling_stroke(state: &AppState, speed: f64) {
    create::create(
        state,
        NewObject {
            tool: Tool::Brush,
            gesture: Gesture::Stroke {
                points: vec![[0.0, 0.0]],
            },
            options: vec![
                number("SizeKm", 600.0),
                number("Speed", speed),
                number("Feather", 0.0),
                ToolOption {
                    property: "Direction".to_owned(),
                    value: PropertyValue::Angle { degrees: 90.0 },
                },
            ],
            layer: None,
        },
    )
    .expect("a stroke");
    let object = {
        let session = state.session.lock().expect("lock");
        session.open.as_ref().expect("open").project.layers[0]
            .objects
            .last()
            .expect("an object")
            .id
            .raw()
    };
    for (step, lon) in [(0u32, 0.0f64), (2, 20.0)] {
        ve_app::animation::key_at(
            state,
            object,
            "Position",
            step,
            Some(PropertyValue::Position { lon, lat: 0.0 }),
        )
        .expect("key");
    }
}

fn region(lon: f64, lat: f64) -> RegionShape {
    RegionShape::Rect {
        centre: [lon, lat],
        half_width_deg: 8.0,
        half_height_deg: 8.0,
    }
}

fn field(state: &AppState, step: u32, lon: f64, lat: f64) -> (f32, f32) {
    let session = state.session.lock().expect("lock");
    let project = &session.open.as_ref().expect("open").project;
    let uv = sample_scene(&flatten(project, step), LonLat::new(lon, lat).unwrap());
    (uv.u, uv.v)
}

/// A macro's bar is as wide as the macro has frames — which for a macro
/// recorded over a whole timeline is the whole timeline (M34).
///
/// The user's own case, reported as a bug three times: a 24-step project of
/// three-hourly steps, a macro recorded from step 0 with the playhead taken
/// to the end, so the run is 24 frames spanning 69 hours. Placed at step 0
/// it covers every step, because it has a frame for every step; placed at
/// step 13 it starts there and is cut off by the end of the timeline. The
/// length of the *run* is what decides this, and the run is the steps from
/// the first to wherever the playhead is when the preview is taken.
#[test]
fn a_macro_is_as_wide_as_its_frames_even_when_that_is_everything() {
    let root = TempRoot::new("full-length");
    let app = AppState::new(AppPaths::in_directory(&root.0).expect("paths"));
    settings::macro_directory_set(&app, root.0.join("library").to_string_lossy().into_owned())
        .expect("library directory");
    projects::create(
        &app,
        NewProjectRequest {
            name: "Long".to_owned(),
            field_kind: "wind".to_owned(),
            resolution: "1.0".to_owned(),
            step_hours: 3,
            step_count: 24,
        },
        false,
    )
    .expect("create");
    create::create(
        &app,
        NewObject {
            tool: Tool::Brush,
            gesture: Gesture::Stroke {
                points: vec![[0.0, 0.0]],
            },
            options: vec![
                number("SizeKm", 900.0),
                number("Speed", 18.0),
                number("Feather", 0.0),
            ],
            layer: None,
        },
    )
    .expect("a stroke");

    // Recorded from step 0, previewed with the playhead at the last step.
    macros::capture_start(&app, region(0.0, 0.0), 0, false, None).expect("start");
    macros::capture_place(&app, 1, 2.0, 0.0).expect("place");
    macros::capture_place(&app, 2, 4.0, 0.0).expect("place");
    let library = macros::capture_finish(&app, "Macro 0".to_owned(), 23).expect("finish");
    let entry = &library.entries[0];
    assert_eq!(
        (entry.frames, entry.span_hours),
        (24, 69.0),
        "a run to the end of a 24-step, three-hourly project"
    );

    let bar = |at: u32| {
        macros::macro_insert(&app, &entry.id, -70.5, 27.5, at, None).expect("insert");
        let tree = ve_app::document::tree(&app, 0).expect("tree");
        let node = tree.layers[0]
            .objects
            .last()
            .cloned()
            .expect("the macro is in the tree");
        (node.start_step, node.end_step)
    };
    assert_eq!(bar(0), (0, 23), "24 frames from step 0 is every step");
    assert_eq!(bar(13), (13, 23), "and from step 13, what is left of them");
}

/// A capture takes every kind under its region (M34).
///
/// The map shows every kind the project holds, so a copy of what is on
/// screen is a copy of all of it: a project of wind and current layers
/// yields a macro holding both, a plane of samples each, and placing it
/// puts an object in a layer of each kind.
#[test]
fn a_capture_takes_every_kind_under_it() {
    use ve_core::project::FieldKind;

    let root = TempRoot::new("both-kinds");
    let app = app(&root);
    // Wind in layer 0, and a current layer of its own beneath the same water.
    travelling_stroke(&app, 18.0);
    ve_app::document::layer_add(&app, "Current".to_owned()).expect("layer");
    let current = ve_app::document::tree(&app, 0).expect("tree").layers[1].id;
    ve_app::document::layer_parameter(&app, current, "current").expect("parameter");
    create::create(
        &app,
        NewObject {
            tool: Tool::Brush,
            gesture: Gesture::Stroke {
                points: vec![[0.0, 0.0]],
            },
            options: vec![
                number("SizeKm", 600.0),
                number("Speed", 4.0),
                number("Feather", 0.0),
                ToolOption {
                    property: "Direction".to_owned(),
                    value: PropertyValue::Angle { degrees: 90.0 },
                },
            ],
            layer: Some(current),
        },
    )
    .expect("a current stroke");

    macros::capture_start(&app, region(0.0, 0.0), 0, false, None).expect("start");
    let library = macros::capture_finish(&app, "Both".to_owned(), 0).expect("finish");
    let entry = &library.entries[0];
    macros::macro_insert(&app, &entry.id, 100.0, 0.0, 0, None).expect("insert");

    let project = {
        let session = app.session.lock().expect("lock");
        session.open.as_ref().expect("open").project.clone()
    };
    let capture = project.captures.values().next().expect("the capture");
    assert_eq!(
        capture.kinds,
        vec![FieldKind::Wind, FieldKind::Current],
        "both kinds, wind first"
    );
    assert_eq!(
        capture.frames[0].uv.len(),
        capture.node_count() * 2,
        "a plane of samples each"
    );

    // One object per kind, each in a layer of that kind.
    assert_eq!(
        project.layers[0].objects.len(),
        2,
        "wind: the stroke and the macro"
    );
    assert_eq!(project.layers[1].objects.len(), 2, "current: the same");

    // And each paints its own kind's field where it was placed.
    let at = |kind: FieldKind| {
        let scene = ve_render::scene::flatten_kind(&project, 0, kind);
        sample_scene(&scene, LonLat::new(100.0, 0.0).unwrap()).u
    };
    assert!(
        (at(FieldKind::Wind) - 18.0).abs() < 0.6,
        "the wind that was under the region: {}",
        at(FieldKind::Wind)
    );
    assert!(
        (at(FieldKind::Current) - 4.0).abs() < 0.6,
        "and the current: {}",
        at(FieldKind::Current)
    );
}

/// The plan's first acceptance: a region dragged to follow a moving stroke,
/// captured **static**, inserts as that stroke standing still.
#[test]
fn a_static_capture_of_a_followed_stroke_stands_still() {
    let root = TempRoot::new("static");
    let app = app(&root);
    travelling_stroke(&app, 18.0);
    // The stroke is at 0° at step 0 and 20° at step 2.
    assert!((field(&app, 0, 0.0, 0.0).0 - 18.0).abs() < 0.6);
    assert!((field(&app, 2, 20.0, 0.0).0 - 18.0).abs() < 0.6);

    macros::capture_start(&app, region(0.0, 0.0), 0, false, None).expect("start");
    // Drag the region to follow the stroke at each step.
    macros::capture_place(&app, 1, 10.0, 0.0).expect("place");
    macros::capture_place(&app, 2, 20.0, 0.0).expect("place");
    let library = macros::capture_finish(&app, "Followed".to_owned(), 2).expect("finish");
    assert_eq!(library.entries.len(), 1);
    let entry = &library.entries[0];
    assert_eq!(entry.frames, 3);
    assert!(!entry.moves, "a static capture records no movement");

    // Insert somewhere else entirely: every step shows the stroke at the same
    // place, because the region followed it and the macro did not record that.
    macros::macro_insert(&app, &entry.id, 100.0, 0.0, 0, None).expect("insert");
    for step in 0..3 {
        let (u, _) = field(&app, step, 100.0, 0.0);
        assert!(
            (u - 18.0).abs() < 0.6,
            "step {step} of a static macro should be the stroke, got {u}"
        );
    }
}

/// A macro reaches as far as the frames it holds and no further (M33).
///
/// Past its last frame a macro draws nothing — spec.md 8.7's rule, and why a
/// patch is the one that holds (D65) — so a range running to the end of the
/// timeline claimed steps it paints nothing at, and the timeline drew the
/// macro across all of them.
#[test]
fn a_macro_reaches_only_as_far_as_its_frames() {
    let root = TempRoot::new("frames-long");
    let app = app(&root);
    travelling_stroke(&app, 18.0);
    macros::capture_start(&app, region(0.0, 0.0), 0, false, None).expect("start");
    macros::capture_place(&app, 1, 10.0, 0.0).expect("place");
    macros::capture_place(&app, 2, 20.0, 0.0).expect("place");
    let library = macros::capture_finish(&app, "Three".to_owned(), 2).expect("finish");
    let entry = &library.entries[0];
    assert_eq!(entry.frames, 3);

    // Three frames of a three-hourly project placed at step 1: steps 1, 2
    // and 3, in a timeline of six.
    macros::macro_insert(&app, &entry.id, 100.0, 0.0, 1, None).expect("insert");
    let project = {
        let mut session = app.session.lock().expect("lock");
        session.require_open().expect("open").project.clone()
    };
    let placed = project.layers[0]
        .objects
        .last()
        .expect("the macro is the newest object");
    assert_eq!(
        (placed.active_range.start, placed.active_range.end),
        (1, 3),
        "a three-frame macro covers three steps"
    );
    // And what it draws agrees with what it claims.
    assert!(
        field(&app, 3, 100.0, 0.0).0.abs() > 0.6,
        "the last frame is drawn"
    );
    assert!(
        field(&app, 4, 100.0, 0.0).0.abs() < 0.6,
        "and nothing past it"
    );
    // And the timeline is drawn from the tree, so the tree has to say so too.
    let node = ve_app::document::tree(&app, 0).expect("tree").layers[0]
        .objects
        .last()
        .cloned()
        .expect("the macro is in the tree");
    assert_eq!(
        (node.start_step, node.end_step),
        (1, 3),
        "the timeline's own start and end indicators"
    );
}

/// A macro begins at the step it was placed (M23).
///
/// Its frames run from the object's first active step, and a new object's
/// range began at 0 whatever step the click was made at — so a three-frame
/// macro placed at step 3 had already ended, and the click made an object
/// that showed nothing.
#[test]
fn a_macro_placed_at_a_later_step_begins_there() {
    let root = TempRoot::new("later-step");
    let app = app(&root);
    travelling_stroke(&app, 18.0);
    macros::capture_start(&app, region(0.0, 0.0), 0, false, None).expect("start");
    macros::capture_place(&app, 1, 10.0, 0.0).expect("place");
    macros::capture_place(&app, 2, 20.0, 0.0).expect("place");
    let library = macros::capture_finish(&app, "Later".to_owned(), 2).expect("finish");
    let entry = &library.entries[0];

    // Into a second layer, to check the layer is honoured too (D66).
    ve_app::document::layer_add(&app, "Upper".to_owned()).expect("layer");
    let upper = ve_app::document::tree(&app, 0).expect("tree").layers[1].id;
    macros::macro_insert(&app, &entry.id, 100.0, 0.0, 3, Some(upper)).expect("insert");

    assert!(
        field(&app, 2, 100.0, 0.0).0.abs() < 0.6,
        "nothing before the step it was placed at"
    );
    for step in 3..6 {
        let (u, _) = field(&app, step, 100.0, 0.0);
        assert!(
            (u - 18.0).abs() < 0.6,
            "step {step} should show the macro, got {u}"
        );
    }
    let tree = ve_app::document::tree(&app, 3).expect("tree");
    assert_eq!(
        tree.layers[1].objects.len(),
        1,
        "it joined the layer it was aimed at"
    );
    assert_eq!(tree.layers[1].objects[0].start_step, 3);
}

/// And the other half: with **record movement** and the region left alone, the
/// insert moves the way the original did.
#[test]
fn a_recorded_capture_moves_the_way_the_original_did() {
    let root = TempRoot::new("record");
    let app = app(&root);
    travelling_stroke(&app, 18.0);

    macros::capture_start(&app, region(0.0, 0.0), 0, true, None).expect("start");
    macros::capture_place(&app, 1, 10.0, 0.0).expect("place");
    macros::capture_place(&app, 2, 20.0, 0.0).expect("place");
    let library = macros::capture_finish(&app, "Moving".to_owned(), 2).expect("finish");
    let entry = &library.entries[0];
    assert!(
        entry.moves,
        "record movement stores each frame's displacement"
    );

    macros::macro_insert(&app, &entry.id, 100.0, 0.0, 0, None).expect("insert");
    // Step 0 at the insert point; step 2 twenty degrees east of it, because
    // the recorded region moved twenty degrees.
    assert!((field(&app, 0, 100.0, 0.0).0 - 18.0).abs() < 0.6);
    assert!(
        (field(&app, 2, 120.0, 0.0).0 - 18.0).abs() < 0.6,
        "the macro should have travelled with its recorded displacement"
    );
}

/// A moving macro is grabbed where it *is*, not where its keys put it.
///
/// The field is drawn at the anchor plus the displacement the capture
/// recorded (M33), so everything that points at the object — the hit test,
/// the outline, the handles a drag starts from — has to be measured there
/// too, or the box is in one place and the macro in another.
#[test]
fn a_moving_macro_is_pointed_at_where_it_is_drawn() {
    let root = TempRoot::new("point");
    let app = app(&root);
    travelling_stroke(&app, 18.0);
    macros::capture_start(&app, region(0.0, 0.0), 0, true, None).expect("start");
    macros::capture_place(&app, 1, 10.0, 0.0).expect("place");
    macros::capture_place(&app, 2, 20.0, 0.0).expect("place");
    let library = macros::capture_finish(&app, "Moving".to_owned(), 2).expect("finish");
    macros::macro_insert(&app, &library.entries[0].id, 100.0, 0.0, 0, None).expect("insert");
    let object = {
        let session = app.session.lock().expect("lock");
        session.open.as_ref().expect("open").project.layers[0]
            .objects
            .last()
            .expect("the macro")
            .id
            .raw()
    };

    // Where the field is at step 2: twenty degrees east of the insert.
    assert!(
        (field(&app, 2, 120.0, 0.0).0 - 18.0).abs() < 0.6,
        "the field moved"
    );

    assert_eq!(
        document::hit_test(&app, 120.0, 0.0, 2).expect("hit test"),
        Some(object),
        "a click on the macro selects it"
    );
    assert_eq!(
        document::hit_test(&app, 100.0, 0.0, 2).expect("hit test"),
        None,
        "and a click where it no longer is selects nothing"
    );

    let outlines =
        ve_app::transform::outlines_at(&app, 2, None, &[object], None, false).expect("outlines");
    let anchor = outlines.first().expect("an outline").anchor;
    assert!(
        (anchor[0] - 120.0).abs() < 0.5 && anchor[1].abs() < 0.5,
        "the outline is drawn around the macro, not around its keys: {anchor:?}"
    );

    let handles = ve_app::transform::start_transform(
        &app,
        &[object],
        2,
        ve_app::transform::TransformKind::Move,
        120.0,
        0.0,
        false,
    )
    .expect("begin")
    .expect("handles");
    assert!(
        (handles.lon - 120.0).abs() < 0.5 && handles.lat.abs() < 0.5,
        "and the handles are on it: {}, {}",
        handles.lon,
        handles.lat
    );
}

/// Capture mode is a **backend** lockout: while it runs, every document write
/// is refused, and cancelling writes nothing.
#[test]
fn capture_mode_refuses_every_write_and_cancel_restores() {
    let root = TempRoot::new("lockout");
    let app = app(&root);
    travelling_stroke(&app, 12.0);
    let before = {
        let session = app.session.lock().expect("lock");
        session.open.as_ref().expect("open").project.object_count()
    };

    macros::capture_start(&app, region(0.0, 0.0), 0, false, None).expect("start");
    assert!(macros::mode(&app, None).expect("mode").active);
    // Each frame holds its own position, and the map draws the region while
    // the capture runs — so asking about a step has to give *that* step's
    // place, and a step never visited gives the one it was drawn at.
    let at_first = macros::mode(&app, Some(0)).expect("mode");
    assert!(
        at_first.position.is_some(),
        "a running capture has a position"
    );
    // The timeline marks the frames the region has been placed at; before any
    // placement that is the one the capture began on, and nothing else.
    assert_eq!(
        at_first.visited,
        vec![0],
        "only the first step is visited yet"
    );
    let unvisited = macros::mode(&app, Some(1)).expect("mode");
    assert_eq!(
        unvisited.position, at_first.position,
        "a step never visited keeps the position the region was drawn at"
    );

    // Every kind of write: a new object, a property edit, a keyframe, an undo.
    assert!(
        create::create(
            &app,
            NewObject {
                tool: Tool::Brush,
                gesture: Gesture::Stroke {
                    points: vec![[5.0, 5.0]]
                },
                options: vec![number("SizeKm", 100.0)],
                layer: None,
            },
        )
        .is_err(),
        "drawing during a capture must be refused"
    );
    assert!(
        document::layer_add(&app, "Nope".to_owned()).is_err(),
        "adding a layer during a capture must be refused"
    );
    assert!(
        ve_app::edit::undo_for_test(&app).is_err(),
        "undo during a capture must be refused"
    );

    macros::capture_cancel(&app).expect("cancel");
    assert!(!macros::mode(&app, None).expect("mode").active);
    let after = {
        let session = app.session.lock().expect("lock");
        session.open.as_ref().expect("open").project.object_count()
    };
    assert_eq!(after, before, "cancelling wrote nothing");
    assert_eq!(
        macros::library(&app).expect("library").entries.len(),
        0,
        "cancelling left no library file"
    );
}

/// Finishing creates exactly one library file and **no object**, and writes
/// are allowed again.
#[test]
fn finishing_makes_one_file_and_no_object() {
    let root = TempRoot::new("finish");
    let app = app(&root);
    travelling_stroke(&app, 12.0);
    let before = {
        let session = app.session.lock().expect("lock");
        session.open.as_ref().expect("open").project.object_count()
    };

    macros::capture_start(&app, region(0.0, 0.0), 0, false, None).expect("start");
    let library = macros::capture_finish(&app, "One".to_owned(), 1).expect("finish");
    assert_eq!(library.entries.len(), 1);
    assert!(library.total_bytes > 0);
    let session_count = {
        let session = app.session.lock().expect("lock");
        session.open.as_ref().expect("open").project.object_count()
    };
    assert_eq!(session_count, before, "a capture creates no object");
    // Writes work again.
    assert!(document::layer_add(&app, "Fine".to_owned()).is_ok());
}

/// The library is not project data: a project that used a macro keeps its own
/// copy of the frames, so clearing the library breaks nothing (D52).
#[test]
fn deleting_the_library_leaves_inserted_macros_working() {
    let root = TempRoot::new("library");
    let app = app(&root);
    travelling_stroke(&app, 15.0);
    macros::capture_start(&app, region(0.0, 0.0), 0, false, None).expect("start");
    let library = macros::capture_finish(&app, "Kept".to_owned(), 1).expect("finish");
    macros::macro_insert(&app, &library.entries[0].id, 60.0, 0.0, 0, None).expect("insert");
    let before = field(&app, 0, 60.0, 0.0);
    assert!((before.0 - 15.0).abs() < 0.6);

    let emptied = macros::macros_delete(&app, None).expect("delete all");
    assert!(emptied.entries.is_empty());
    assert_eq!(
        field(&app, 0, 60.0, 0.0),
        before,
        "an inserted macro carries its own frames"
    );

    // And the object is a macro, in the panels.
    let tree = document::tree(&app, 0).expect("tree");
    let object = tree.layers[0].objects.last().expect("the macro");
    assert_eq!(object.tool, "macro");
    let session = app.session.lock().expect("lock");
    let project = &session.open.as_ref().expect("open").project;
    assert_eq!(
        project.layers[0].objects.last().expect("object").tool,
        ToolKind::Macro
    );
}

/// Two inserts of one macro share one archive entry, by content hash.
#[test]
fn two_inserts_share_one_entry() {
    let root = TempRoot::new("share");
    let app = app(&root);
    travelling_stroke(&app, 15.0);
    macros::capture_start(&app, region(0.0, 0.0), 0, false, None).expect("start");
    let library = macros::capture_finish(&app, "Twice".to_owned(), 1).expect("finish");
    let id = &library.entries[0].id;
    macros::macro_insert(&app, id, 60.0, 0.0, 0, None).expect("insert");
    macros::macro_insert(&app, id, 80.0, 0.0, 0, None).expect("insert");
    let session = app.session.lock().expect("lock");
    let project = &session.open.as_ref().expect("open").project;
    assert_eq!(project.captures.len(), 1, "one capture, two objects");
    assert_eq!(project.object_count(), 3);
}

// --- M26: keys, interpolation, and the preview ------------------------------

/// A capture's positions are keys (D72): between two keys the region is on
/// the great circle between them, a removed key hands its step back to the
/// interpolation, and the bake reads the same rule.
#[test]
fn positions_are_keys_and_the_gaps_interpolate() {
    let root = TempRoot::new("keys");
    let app = app(&root);
    travelling_stroke(&app, 18.0);

    macros::capture_start(&app, region(0.0, 0.0), 0, true, None).expect("start");
    // Visit 1 and 2 without dragging: keys where the region stands.
    macros::capture_visit(&app, 1).expect("visit");
    macros::capture_visit(&app, 2).expect("visit");
    assert_eq!(macros::mode(&app, None).expect("mode").keys, vec![0, 1, 2]);
    // Drag at 4: a key at 40° east; 3 is now between 2 (at 0°) and 4.
    macros::capture_place(&app, 4, 40.0, 0.0).expect("place");
    let at_three = macros::mode(&app, Some(3))
        .expect("mode")
        .position
        .expect("pos");
    assert!(
        (at_three[0] - 20.0).abs() < 0.05 && at_three[1].abs() < 0.05,
        "halfway between the keys at 2 and 4, got {at_three:?}"
    );
    // Remove the key at 2 and the interpolation spans 1..4 instead.
    macros::capture_unplace(&app, 2).expect("unplace");
    let mode = macros::mode(&app, Some(2)).expect("mode");
    assert_eq!(mode.keys, vec![0, 1, 4]);
    let at_two = mode.position.expect("pos");
    assert!(
        (at_two[0] - 40.0 / 3.0).abs() < 0.05,
        "a third of the way from 1 to 4, got {at_two:?}"
    );
    // The first step's key cannot go.
    macros::capture_unplace(&app, 0).expect("unplace");
    assert!(macros::mode(&app, None).expect("mode").keys.contains(&0));
    // The bake stores the interpolated displacement, since movement is on.
    let library = macros::capture_finish(&app, "Keys".to_owned(), 4).expect("finish");
    let entry = &library.entries[0];
    assert_eq!(entry.frames, 5);
    assert!(
        (entry.track[2][0] - 40.0 / 3.0).abs() < 0.05,
        "{:?}",
        entry.track
    );
    assert!((entry.track[4][0] - 40.0).abs() < 1e-9);
}

/// The preview bakes into the session and shows on an empty map under its
/// own revision (D71): the document is never written, the history lock
/// stands, edit goes back to recording with the keys kept, and save keeps
/// what was looked at.
#[test]
fn a_preview_writes_nothing_and_is_served_apart_from_the_document() {
    let root = TempRoot::new("preview");
    let app = app(&root);
    travelling_stroke(&app, 18.0);
    let (revision_before, entries_before) = {
        let session = app.session.lock().expect("lock");
        let open = session.open.as_ref().expect("open");
        (open.revision, open.history.entries().len())
    };

    macros::capture_start(&app, region(0.0, 0.0), 0, false, None).expect("start");
    macros::capture_place(&app, 2, 20.0, 0.0).expect("place");
    let mode = macros::capture_preview(&app, 2).expect("preview");
    assert_eq!(mode.phase, macros::CapturePhase::Previewing);
    let preview_revision = mode.preview_revision.expect("a preview revision");
    assert_ne!(preview_revision, revision_before, "never the document's");
    assert_eq!(mode.stamp, Some([0.0, 0.0]), "stamped where it was drawn");

    // The preview opens empty (M34): what is on it is what the user has
    // placed, and nothing of the document or of the recording is there.
    {
        let session = app.session.lock().expect("lock");
        let scene = &session.preview.as_ref().expect("preview").project;
        let at = |step: u32, lon: f64| {
            sample_scene(&flatten(scene, step), LonLat::new(lon, 0.0).unwrap()).u
        };
        assert!(at(0, 0.0).abs() < 1e-6, "nothing where it was recorded");
        assert!(at(0, 60.0).abs() < 1e-6, "and nothing else on the map");
        assert_eq!(scene.layers.len(), 1, "a layer for the kind captured");
        assert!(scene.layers[0].objects.is_empty(), "and nothing in it");
    }
    // A click puts a copy there, under a new revision.
    let placed = macros::preview_place(&app, 90.0, 0.0).expect("place");
    assert_ne!(placed.preview_revision, Some(preview_revision));
    {
        let session = app.session.lock().expect("lock");
        let scene = &session.preview.as_ref().expect("preview").project;
        let u = sample_scene(&flatten(scene, 0), LonLat::new(90.0, 0.0).unwrap()).u;
        assert!((u - 18.0).abs() < 0.6);
    }
    // Every write is still refused, and the document has not moved.
    assert!(
        edit::paint(
            &app,
            BrushStroke {
                points: vec![[50.0, 0.0]],
                size_km: 400.0,
                speed_mps: 5.0,
                direction_toward_deg: 0.0,
                feather: 0.0,
                ..Default::default()
            },
        )
        .is_err()
    );
    {
        let session = app.session.lock().expect("lock");
        let open = session.open.as_ref().expect("open");
        assert_eq!(open.revision, revision_before);
        assert_eq!(open.history.entries().len(), entries_before);
    }
    // Edit: back to recording, keys intact, preview gone.
    let edited = macros::capture_edit(&app).expect("edit");
    assert_eq!(edited.phase, macros::CapturePhase::Recording);
    assert_eq!(edited.keys, vec![0, 2]);
    assert!(edited.preview_revision.is_none());
    assert!(app.session.lock().expect("lock").preview.is_none());
    // Preview again and save: the library has the bake, the lock is lifted.
    macros::capture_preview(&app, 2).expect("preview");
    let library = macros::capture_finish(&app, "Kept".to_owned(), 2).expect("finish");
    assert_eq!(library.entries.len(), 1);
    assert_eq!(library.entries[0].frames, 3);
    assert!(app.session.lock().expect("lock").preview.is_none());
    assert!(!macros::mode(&app, None).expect("mode").active);
    let after = edit::paint(
        &app,
        BrushStroke {
            points: vec![[50.0, 0.0]],
            size_km: 400.0,
            speed_mps: 5.0,
            direction_toward_deg: 0.0,
            feather: 0.0,
            ..Default::default()
        },
    );
    assert!(after.is_ok(), "the lock is lifted with the save");
}

/// Cancelling from the preview clears it too.
/// A click in the preview places a copy of the macro in the preview's own
/// scene (M29): the document gains nothing, the lock stands, the scene
/// shows the macro at the original and at the click, and Cancel leaves no
/// trace of either.
#[test]
fn a_click_in_the_preview_places_a_copy_in_the_preview_scene_alone() {
    let root = TempRoot::new("preview-place");
    let app = app(&root);
    travelling_stroke(&app, 18.0);
    macros::capture_start(&app, region(0.0, 0.0), 0, false, None).expect("start");
    macros::capture_place(&app, 2, 20.0, 0.0).expect("place");
    let mode = macros::capture_preview(&app, 2).expect("preview");
    let (entries_before, objects_before) = {
        let session = app.session.lock().expect("lock");
        let open = session.open.as_ref().expect("open");
        (open.history.entries().len(), open.project.object_count())
    };

    let placed = macros::preview_place(&app, 90.0, 0.0).expect("place in preview");
    assert_eq!(
        placed.phase,
        macros::CapturePhase::Previewing,
        "still previewing"
    );
    assert_ne!(
        placed.preview_revision, mode.preview_revision,
        "a new scene"
    );
    {
        let session = app.session.lock().expect("lock");
        let open = session.open.as_ref().expect("open");
        assert_eq!(
            open.history.entries().len(),
            entries_before,
            "nothing written"
        );
        assert_eq!(
            open.project.object_count(),
            objects_before,
            "no object in any layer"
        );
        assert!(open.history.is_locked(), "the lock stands");
        let scene = &session.preview.as_ref().expect("preview").project;
        assert_eq!(
            scene.layers[0].objects.len(),
            1,
            "the copy the click placed, and only that (M34)"
        );
        let at = |lon: f64| sample_scene(&flatten(scene, 0), LonLat::new(lon, 0.0).unwrap()).u;
        assert!(at(0.0).abs() < 1e-6, "nothing where the macro was recorded");
        assert!((at(90.0) - 18.0).abs() < 0.6, "and the copy at the click");
    }
    // Back to recording drops the copies; the next preview starts clean.
    macros::capture_edit(&app).expect("edit");
    let again = macros::capture_preview(&app, 2).expect("preview again");
    assert!(again.preview_revision.is_some());
    {
        let session = app.session.lock().expect("lock");
        let scene = &session.preview.as_ref().expect("preview").project;
        assert!(
            scene.layers[0].objects.is_empty(),
            "the copies went with the edit, and a fresh preview is empty (M34)"
        );
    }
    macros::capture_cancel(&app).expect("cancel");
    let session = app.session.lock().expect("lock");
    let open = session.open.as_ref().expect("open");
    assert_eq!(open.project.object_count(), objects_before);
    assert!(session.preview.is_none());
}

#[test]
fn cancel_from_the_preview_clears_the_preview() {
    let root = TempRoot::new("preview-cancel");
    let app = app(&root);
    travelling_stroke(&app, 18.0);
    macros::capture_start(&app, region(0.0, 0.0), 0, false, None).expect("start");
    macros::capture_preview(&app, 1).expect("preview");
    assert!(app.session.lock().expect("lock").preview.is_some());
    macros::capture_cancel(&app).expect("cancel");
    assert!(app.session.lock().expect("lock").preview.is_none());
    assert!(!macros::mode(&app, None).expect("mode").active);
}

/// Moving a macro moves the field it paints, not only its outline.
#[test]
fn moving_a_macro_moves_the_field_it_paints() {
    use ve_app::transform::{self, TransformKind};

    let root = TempRoot::new("move");
    let app = app(&root);
    travelling_stroke(&app, 18.0);
    macros::capture_start(&app, region(0.0, 0.0), 0, false, None).expect("start");
    let library = macros::capture_finish(&app, "Still".to_owned(), 0).expect("finish");
    let entry = &library.entries[0];
    macros::macro_insert(&app, &entry.id, 100.0, 0.0, 0, None).expect("insert");
    let object = {
        let session = app.session.lock().expect("lock");
        session.open.as_ref().expect("open").project.layers[0]
            .objects
            .last()
            .expect("the macro")
            .id
            .raw()
    };
    assert!(
        (field(&app, 0, 100.0, 0.0).0 - 18.0).abs() < 0.6,
        "placed: {:?}",
        field(&app, 0, 100.0, 0.0)
    );

    let before_keys = tile_keys(&app);
    transform::start_transform(&app, &[object], 0, TransformKind::Move, 100.0, 0.0, false)
        .expect("begin");
    transform::update_transform(&app, 100.0, 30.0).expect("drag");

    assert!(
        (field(&app, 0, 100.0, 30.0).0 - 18.0).abs() < 0.6,
        "the macro should have moved with it: {:?}",
        field(&app, 0, 100.0, 30.0)
    );
    assert!(
        field(&app, 0, 100.0, 0.0).0.abs() < 0.6,
        "and left where it was: {:?}",
        field(&app, 0, 100.0, 0.0)
    );

    // And the tiles it reaches are re-keyed, at both ends of the move: the map
    // keeps a texture by its key, so a key that did not change is a macro that
    // does not move on screen however right the document is.
    assert_ne!(before_keys, tile_keys(&app), "the tiles the macro reaches");
}

/// The content keys of the tiles over the macro's two positions.
fn tile_keys(state: &AppState) -> Vec<[u8; 32]> {
    let session = state.session.lock().expect("lock");
    let project = &session.open.as_ref().expect("open").project;
    let scene = flatten(project, 0);
    let digests = ve_render::cull::digests_of(&scene);
    // Level 4: 11.25 degrees a side. (100, 0) and (100, 30).
    [(0.0, 0.0), (100.0, 0.0), (100.0, 30.0)]
        .into_iter()
        .map(|(lon, lat): (f64, f64)| {
            let x = ((lon + 180.0) / 11.25).floor() as u32;
            let y = ((90.0 - lat) / 11.25).floor() as u32;
            let tile = ve_render::tile::TileId::new(4, x, y).expect("tile");
            ve_render::cull::tile_scene(&scene, &digests, tile).hash
        })
        .collect()
}

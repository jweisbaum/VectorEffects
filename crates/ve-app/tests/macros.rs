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

    macros::capture_start(&app, region(0.0, 0.0), 0, false).expect("start");
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
    macros::capture_start(&app, region(0.0, 0.0), 0, false).expect("start");
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

    macros::capture_start(&app, region(0.0, 0.0), 0, true).expect("start");
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

    macros::capture_start(&app, region(0.0, 0.0), 0, false).expect("start");
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

    macros::capture_start(&app, region(0.0, 0.0), 0, false).expect("start");
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
    macros::capture_start(&app, region(0.0, 0.0), 0, false).expect("start");
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
    macros::capture_start(&app, region(0.0, 0.0), 0, false).expect("start");
    let library = macros::capture_finish(&app, "Twice".to_owned(), 1).expect("finish");
    let id = &library.entries[0].id;
    macros::macro_insert(&app, id, 60.0, 0.0, 0, None).expect("insert");
    macros::macro_insert(&app, id, 80.0, 0.0, 0, None).expect("insert");
    let session = app.session.lock().expect("lock");
    let project = &session.open.as_ref().expect("open").project;
    assert_eq!(project.captures.len(), 1, "one capture, two objects");
    assert_eq!(project.object_count(), 3);
}

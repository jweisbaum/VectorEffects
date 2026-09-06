#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test setup: a panic naming the failed step is the right outcome, and clippy's allow-expect-in-tests does not reach helpers in tests/"
)]
//! Capturing a region and pasting it as a patch (spec.md 8.5, M14).
//!
//! The acceptance the plan asks for, and the one rule everything else hangs
//! off: **zero and undefined stay distinct** through the composite, the
//! container, the kernel and a save. A patch pasted over another field paints
//! what its source painted and leaves alone what its source never covered.

use std::path::PathBuf;

use ve_app::capture::{self, RegionShape};
use ve_app::commands::AppState;
use ve_app::create::{self, Gesture, NewObject, Tool, ToolOption};
use ve_app::document::{self, PropertyValue};
use ve_app::edit;
use ve_app::paths::AppPaths;
use ve_app::projects::{self, NewProjectRequest};
use ve_core::LonLat;
use ve_core::schema::ToolKind;
use ve_render::cpu::sample_scene;
use ve_render::scene::flatten;

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ve-capture-{}-{label}-{}",
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
            name: "Capture".to_owned(),
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

fn number(property: &str, value: f64) -> ToolOption {
    ToolOption {
        property: property.to_owned(),
        value: PropertyValue::Number { value },
    }
}

/// A hard-edged brush stamp at a position, painting eastward.
fn stroke(state: &AppState, lon: f64, lat: f64, size_km: f64, speed: f64) -> u64 {
    create::create(
        state,
        NewObject {
            tool: Tool::Brush,
            gesture: Gesture::Stroke {
                points: vec![[lon, lat]],
            },
            options: vec![
                number("SizeKm", size_km),
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
    // A new object joins the end of its layer.
    let session = state.session.lock().expect("lock");
    session.open.as_ref().expect("open").project.layers[0]
        .objects
        .last()
        .expect("an object")
        .id
        .raw()
}

/// The field at a position, through the same path the tiles take.
fn field(state: &AppState, lon: f64, lat: f64) -> (f32, f32) {
    let session = state.session.lock().expect("lock");
    let project = &session.open.as_ref().expect("open").project;
    let uv = sample_scene(&flatten(project, 0), LonLat::new(lon, lat).unwrap());
    (uv.u, uv.v)
}

fn rect(lon: f64, lat: f64, half_w: f64, half_h: f64) -> RegionShape {
    RegionShape::Rect {
        centre: [lon, lat],
        half_width_deg: half_w,
        half_height_deg: half_h,
    }
}

/// A region copy carries the animation (D65, M23).
///
/// A stroke travels east through a fixed region — at 10° on step 1, 20° on
/// step 2, 30° on step 3. Copied at step 1 and pasted at step 0 somewhere
/// else, the patch shows at *its* step 0 what the source showed at step 1,
/// carries on from there, and holds its last frame past the end.
#[test]
fn a_region_copy_carries_the_animation_from_the_copy_step() {
    let (_root, app) = project("animated-copy");
    let object = stroke(&app, 0.0, 0.0, 600.0, 18.0);
    for (step, lon) in [(0u32, 0.0f64), (3, 30.0)] {
        ve_app::animation::key_at(
            &app,
            object,
            "Position",
            step,
            Some(PropertyValue::Position { lon, lat: 0.0 }),
        )
        .expect("key");
    }
    let at = |step: u32, lon: f64| {
        let session = app.session.lock().expect("lock");
        let project = &session.open.as_ref().expect("open").project;
        sample_scene(&flatten(project, step), LonLat::new(lon, 0.0).unwrap()).u
    };
    assert!(
        (at(1, 10.0) - 18.0).abs() < 0.6,
        "the source is at 10° on step 1"
    );

    // The region spans 0°..30° about 15°, so every step's stroke is inside.
    let held = capture::region_capture(&app, rect(15.0, 0.0, 20.0, 6.0), 1, None).expect("capture");
    assert_eq!(held.frames, 3, "steps 1, 2 and 3 differ, so three frames");

    // Pasted at step 0, centred at 115°: the source's 15° lands on 115°.
    capture::capture_paste(&app, Some(115.0), Some(0.0), 0, None, false).expect("paste");
    assert!(
        (at(0, 110.0) - 18.0).abs() < 0.6,
        "at its step 0 the patch shows the source's step 1 (stroke at 10°)"
    );
    assert!(
        at(0, 130.0).abs() < 0.6,
        "and nothing yet where the stroke will be on the last frame"
    );
    assert!(
        (at(2, 130.0) - 18.0).abs() < 0.6,
        "at step 2 the source's step 3 (stroke at 30°)"
    );
    assert!(
        (at(3, 130.0) - 18.0).abs() < 0.6,
        "past its last frame a patch holds it"
    );
}

/// A still paste of the same copy keeps the copied frame alone (M27): at
/// every step the patch shows what the source showed on the copy step, and
/// the later frames of the run are not in it.
#[test]
fn a_still_paste_of_an_animated_copy_keeps_the_copied_frame_alone() {
    let (_root, app) = project("still-paste");
    let object = stroke(&app, 0.0, 0.0, 600.0, 18.0);
    for (step, lon) in [(0u32, 0.0f64), (3, 30.0)] {
        ve_app::animation::key_at(
            &app,
            object,
            "Position",
            step,
            Some(PropertyValue::Position { lon, lat: 0.0 }),
        )
        .expect("key");
    }
    let at = |step: u32, lon: f64| {
        let session = app.session.lock().expect("lock");
        let project = &session.open.as_ref().expect("open").project;
        sample_scene(&flatten(project, step), LonLat::new(lon, 0.0).unwrap()).u
    };
    let held = capture::region_capture(&app, rect(15.0, 0.0, 20.0, 6.0), 1, None).expect("capture");
    assert_eq!(held.frames, 3, "the clipboard holds the run");

    capture::capture_paste(&app, Some(115.0), Some(0.0), 0, None, true).expect("still paste");
    for step in [0u32, 2, 3] {
        assert!(
            (at(step, 110.0) - 18.0).abs() < 0.6,
            "at step {step} the still shows the copy step's stroke at 10°"
        );
        assert!(
            at(step, 130.0).abs() < 0.6,
            "and never the run's last frame at 30° (step {step})"
        );
    }
    let session = app.session.lock().expect("lock");
    let project = &session.open.as_ref().expect("open").project;
    let patch = project.layers[0].objects.last().expect("the patch");
    let hash = patch.capture.as_ref().expect("its samples");
    let kept = project.captures.get(hash).expect("in the project");
    assert_eq!(kept.frames.len(), 1, "one frame, its own archive entry");
}

/// A still scene bakes one frame, however long the timeline is.
#[test]
fn a_still_region_copy_is_one_frame() {
    let (_root, app) = project("still-copy");
    stroke(&app, 0.0, 0.0, 600.0, 18.0);
    let held = capture::region_capture(&app, rect(0.0, 0.0, 6.0, 6.0), 0, None).expect("capture");
    assert_eq!(held.frames, 1);
}

/// The plan's acceptance, in one test.
///
/// A region over a 20 m/s eastward stroke and open water, pasted somewhere
/// else over another field: the stroke's half reads 20 m/s east, and the
/// water's half reads *the other field* rather than a hole of calm.
#[test]
fn a_pasted_patch_paints_its_source_and_leaves_the_rest_alone() {
    let (_root, app) = project("acceptance");
    // A stroke covering the western half of the region only.
    stroke(&app, -6.0, 0.0, 800.0, 20.0);
    assert!((field(&app, -6.0, 0.0).0 - 20.0).abs() < 0.5, "the stroke");
    assert!(field(&app, 6.0, 0.0).0.abs() < 1e-4, "open water");

    // Capture a box spanning both.
    let taken = capture::region_capture(&app, rect(0.0, 0.0, 10.0, 6.0), 0, None).expect("capture");
    assert!(taken.has_capture);

    // Somewhere else entirely, a southward field to paste over.
    let elsewhere = create::create(
        &app,
        NewObject {
            tool: Tool::Brush,
            gesture: Gesture::Stroke {
                points: vec![[100.0, 0.0]],
            },
            options: vec![
                number("SizeKm", 4_000.0),
                number("Speed", 7.0),
                number("Feather", 0.0),
                ToolOption {
                    property: "Direction".to_owned(),
                    value: PropertyValue::Angle { degrees: 180.0 },
                },
            ],
            layer: None,
        },
    )
    .expect("a second stroke");
    let _ = elsewhere;
    assert!((field(&app, 100.0, 0.0).1 + 7.0).abs() < 0.5, "southward");

    capture::capture_paste(&app, Some(100.0), Some(0.0), 0, None, false).expect("paste");

    // Where the stroke was, the patch paints the stroke.
    let (u, v) = field(&app, 94.0, 0.0);
    assert!(
        (u - 20.0).abs() < 0.6 && v.abs() < 0.6,
        "under the captured stroke the patch reads ({u}, {v})"
    );
    // Where the water was, the patch is transparent and the field beneath
    // shows through — not a hole of calm (D58).
    let (u, v) = field(&app, 106.0, 0.0);
    assert!(
        u.abs() < 0.6 && (v + 7.0).abs() < 0.6,
        "under the captured water the patch reads ({u}, {v}), not the field beneath"
    );
}

/// A patch is an object: it undoes, it is a patch and not a shape fill, and
/// it creates exactly one.
#[test]
fn a_paste_makes_one_patch_and_undoes() {
    let (_root, app) = project("undo");
    stroke(&app, 0.0, 0.0, 800.0, 15.0);
    capture::region_capture(&app, rect(0.0, 0.0, 6.0, 6.0), 0, None).expect("capture");
    let before = {
        let session = app.session.lock().expect("lock");
        session.open.as_ref().expect("open").project.object_count()
    };
    capture::capture_paste(&app, Some(40.0), Some(0.0), 0, None, false).expect("paste");
    {
        let session = app.session.lock().expect("lock");
        let project = &session.open.as_ref().expect("open").project;
        assert_eq!(project.object_count(), before + 1);
        let patch = project.layers[0].objects.last().expect("the patch");
        assert_eq!(patch.tool, ToolKind::Patch);
        assert!(patch.capture.is_some(), "the patch names its samples");
        assert_eq!(project.captures.len(), 1);
    }
    assert!((field(&app, 40.0, 0.0).0 - 15.0).abs() < 0.6, "it paints");

    edit::undo_for_test(&app).expect("undo");
    let session = app.session.lock().expect("lock");
    assert_eq!(
        session.open.as_ref().expect("open").project.object_count(),
        before
    );
}

/// The samples travel with the project, as their own archive entry, and the
/// JSON keeps the hash and nothing else (D52).
#[test]
fn a_patch_survives_a_save_and_load_with_its_samples() {
    let (root, app) = project("roundtrip");
    stroke(&app, 0.0, 0.0, 800.0, 18.0);
    capture::region_capture(&app, rect(0.0, 0.0, 6.0, 6.0), 0, None).expect("capture");
    capture::capture_paste(&app, Some(50.0), Some(0.0), 0, None, false).expect("paste");
    let before = field(&app, 50.0, 0.0);
    assert!((before.0 - 18.0).abs() < 0.6);

    let path = root.0.join("patch.veproj");
    projects::save_as(&app, path.to_string_lossy().into_owned()).expect("save");

    // The archive holds the capture as its own entry, and the JSON does not
    // hold samples: invariants 1 and 2 as reworded (D52).
    let mut archive =
        zip::ZipArchive::new(std::fs::File::open(&path).expect("open archive")).expect("zip");
    let names: Vec<String> = archive.file_names().map(str::to_owned).collect();
    assert_eq!(
        names
            .iter()
            .filter(|n| n.starts_with("captures/") && n.ends_with(".vecap"))
            .count(),
        1,
        "one capture entry, got {names:?}"
    );
    let mut json = String::new();
    {
        use std::io::Read;
        archive
            .by_name("project.json")
            .expect("project.json")
            .read_to_string(&mut json)
            .expect("read");
    }
    assert!(json.contains("\"capture\""), "the object names its capture");
    assert!(
        !json.contains("\"uv\"") && !json.contains("\"samples\""),
        "the json must hold no samples"
    );

    projects::open(&app, path.to_string_lossy().into_owned(), false).expect("open");
    let after = field(&app, 50.0, 0.0);
    assert!(
        (after.0 - before.0).abs() < 1e-4 && (after.1 - before.1).abs() < 1e-4,
        "the patch read {after:?} after reopening, not {before:?}"
    );
}

/// A cell a mask removed is undefined in the capture, so a patch taken over
/// one is transparent there rather than painting a hole of dead air (D58).
#[test]
fn a_mask_leaves_a_hole_the_patch_does_not_fill() {
    let (_root, app) = project("mask");
    stroke(&app, 0.0, 0.0, 3_000.0, 22.0);
    create::create(
        &app,
        NewObject {
            tool: Tool::Mask,
            gesture: Gesture::Stroke {
                points: vec![[0.0, 0.0]],
            },
            options: vec![number("SizeKm", 600.0), number("Feather", 0.0)],
            layer: None,
        },
    )
    .expect("a mask");
    assert!(
        field(&app, 0.0, 0.0).0.abs() < 1e-4,
        "the mask took it away"
    );

    capture::region_capture(&app, rect(0.0, 0.0, 12.0, 12.0), 0, None).expect("capture");

    // Somewhere else, a westward field.
    create::create(
        &app,
        NewObject {
            tool: Tool::Brush,
            gesture: Gesture::Stroke {
                points: vec![[100.0, 0.0]],
            },
            options: vec![
                number("SizeKm", 4_000.0),
                number("Speed", 9.0),
                number("Feather", 0.0),
                ToolOption {
                    property: "Direction".to_owned(),
                    value: PropertyValue::Angle { degrees: 270.0 },
                },
            ],
            layer: None,
        },
    )
    .expect("a second stroke");
    capture::capture_paste(&app, Some(100.0), Some(0.0), 0, None, false).expect("paste");

    // Under the masked hole the field beneath shows through, unchanged.
    let (u, v) = field(&app, 100.0, 0.0);
    assert!(
        (u + 9.0).abs() < 0.6 && v.abs() < 0.6,
        "the mask's hole should be transparent, got ({u}, {v})"
    );
    // Away from the hole, the patch paints what it captured.
    let (u, _) = field(&app, 108.0, 0.0);
    assert!(
        (u - 22.0).abs() < 0.6,
        "outside the hole the patch reads {u}"
    );
}

/// A patch is not offered in the palette: there is no gesture that makes one.
#[test]
fn the_patch_is_not_in_the_palette() {
    let (_root, app) = project("palette");
    let _ = &app;
    let palette = ve_app::palette::tool_palette().expect("palette");
    assert!(
        palette.iter().all(|entry| entry.tool != Tool::Patch),
        "a patch is pasted, not drawn"
    );
    assert!(!palette.is_empty());
}

/// The inspector and the timeline still have to name it.
#[test]
fn a_patch_appears_in_the_document_tree() {
    let (_root, app) = project("tree");
    stroke(&app, 0.0, 0.0, 800.0, 11.0);
    capture::region_capture(&app, rect(0.0, 0.0, 6.0, 6.0), 0, None).expect("capture");
    capture::capture_paste(&app, Some(30.0), Some(0.0), 0, None, false).expect("paste");
    let tree = document::tree(&app, 0).expect("tree");
    let object = tree.layers[0].objects.last().expect("the patch");
    assert_eq!(object.tool, "patch");
    assert_eq!(object.tool_label, "Patch");
}

/// The cost of the whole-map animated copy the plan bounds (M23, D65):
/// `Cmd`-`Shift`-`A` at 0.25° over 24 steps of a scene that changes every
/// step. Ignored because it is a measurement, not an assertion; run it in
/// release with `--nocapture` and put the number in the plan.
#[test]
#[ignore = "a measurement: cargo test -p ve-app --release --test capture -- --ignored --nocapture"]
fn whole_map_animated_copy_cost() {
    let root = TempRoot::new("copy-cost");
    let app = AppState::new(AppPaths::in_directory(&root.0).expect("paths"));
    projects::create(
        &app,
        NewProjectRequest {
            name: "Cost".to_owned(),
            field_kind: "wind".to_owned(),
            resolution: "0.25".to_owned(),
            step_hours: 3,
            step_count: 24,
        },
        false,
    )
    .expect("create");
    let object = stroke(&app, 0.0, 0.0, 2_000.0, 18.0);
    for (step, lon) in [(0u32, -60.0f64), (23, 60.0)] {
        ve_app::animation::key_at(
            &app,
            object,
            "Position",
            step,
            Some(PropertyValue::Position { lon, lat: 0.0 }),
        )
        .expect("key");
    }
    let started = std::time::Instant::now();
    let held =
        capture::region_capture(&app, rect(0.0, 0.0, 180.0, 90.0), 0, None).expect("capture");
    let elapsed = started.elapsed();
    println!(
        "whole-map copy at 0.25° over 24 animated steps: {} frames of {}x{} in {:.2} s",
        held.frames,
        held.ni,
        held.nj,
        elapsed.as_secs_f64()
    );
    let started = std::time::Instant::now();
    let still =
        capture::region_capture(&app, rect(0.0, 0.0, 180.0, 90.0), 23, None).expect("capture");
    println!(
        "the same region at the last step (one frame): {} frame in {:.2} s",
        still.frames,
        started.elapsed().as_secs_f64()
    );
}

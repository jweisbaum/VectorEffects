#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test setup: a panic naming the failed step is the right outcome, and clippy's allow-expect-in-tests does not reach helpers in tests/"
)]
//! Copying an imported layer's frames between steps (spec.md 4.8, M20).
//!
//! A forecast file rarely lines up with a timeline, and §4.8's rule that a
//! step the file says nothing about shows nothing leaves the user no way to
//! say otherwise. An override is that way. What these check is that it is an
//! *instruction* and not a copy: the frame served is one the file already
//! holds, the project gains a step number and no samples, and two steps
//! showing the same message hash alike so the render cache holds one frame
//! for the pair.
//!
//! The generated file carries `u` equal to the forecast hour, so a step's
//! sample says which message it is showing.

use std::path::PathBuf;

use ve_app::commands::AppState;
use ve_app::paths::AppPaths;
use ve_app::projects::{self, NewProjectRequest};
use ve_app::{document, frames, import};
use ve_core::LonLat;
use ve_grib::writer::{GridSpec, MessageSpec, Parameter, ReferenceTime, message};
use ve_render::cpu::sample_scene;
use ve_render::scene::flatten;

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ve-grib-frames-{}-{}-{label}",
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

fn state(root: &TempRoot, step_hours: u32, step_count: u32) -> AppState {
    let state = AppState::new(AppPaths::in_directory(&root.0).expect("paths"));
    projects::create(
        &state,
        NewProjectRequest {
            name: "Frames".to_owned(),
            field_kind: "wind".to_owned(),
            resolution: "1.0".to_owned(),
            step_hours,
            step_count,
        },
        false,
    )
    .expect("create");
    state
}

/// A 1° global wind file whose `u` is the forecast hour and whose `v` is 0.
fn write_file(root: &TempRoot, name: &str, hours: &[u32]) -> String {
    let grid = GridSpec {
        ni: 360,
        nj: 181,
        micro_degrees: 1_000_000,
    };
    let count = grid.point_count() as usize;
    let mut bytes = Vec::new();
    for &hour in hours {
        for (parameter, value) in [(Parameter::WindU, hour as f32), (Parameter::WindV, 0.0f32)] {
            let spec = MessageSpec {
                parameter,
                grid,
                reference_time: ReferenceTime {
                    year: 2026,
                    month: 9,
                    day: 4,
                    hour: 0,
                    minute: 0,
                    second: 0,
                },
                forecast_hour: hour,
                centre: 255,
            };
            bytes.extend(message(&spec, &vec![value; count]).unwrap());
        }
    }
    let path = root.0.join(name);
    std::fs::write(&path, bytes).expect("write grib");
    path.to_string_lossy().into_owned()
}

/// The field at a fixed position, through the path the tiles take.
fn u_at(state: &AppState, step: u32) -> f32 {
    let session = state.session.lock().expect("lock");
    let project = &session.open.as_ref().expect("open").project;
    sample_scene(&flatten(project, step), LonLat::new(-30.0, 45.0).unwrap()).u
}

/// What every step shows, in order.
fn seen(state: &AppState, steps: u32) -> Vec<f32> {
    (0..steps).map(|s| u_at(state, s)).collect()
}

/// The imported layer's id.
fn grib_layer(state: &AppState) -> u64 {
    let session = state.session.lock().expect("lock");
    let project = &session.open.as_ref().expect("open").project;
    project
        .layers
        .iter()
        .find(|l| l.is_grib())
        .expect("a grib layer")
        .id
        .raw()
}

/// A project with a 6-hourly file on a 3-hourly timeline: a message on every
/// other step, which is the case the feature exists for.
fn sparse(root: &TempRoot, steps: u32) -> AppState {
    let app = state(root, 3, steps);
    let path = write_file(root, "wind.grib2", &[0, 6, 12, 18]);
    import::grib_import(&app, path).expect("import");
    app
}

#[test]
fn a_pasted_step_shows_the_message_its_source_shows() {
    let root = TempRoot::new("paste-one");
    let app = sparse(&root, 8);
    let layer = grib_layer(&app);

    // Steps 0, 2, 4, 6 carry the file's 0, 6, 12 and 18 h messages; the odd
    // steps fall between them and show nothing.
    assert_eq!(
        seen(&app, 8),
        vec![0.0, 0.0, 6.0, 0.0, 12.0, 0.0, 18.0, 0.0]
    );

    let state = frames::frames_copy(&app, layer, vec![2]).expect("copy");
    assert_eq!(state.count, 1);
    assert_eq!(state.layer, Some(layer));
    frames::frames_paste(&app, 3).expect("paste");

    assert_eq!(
        seen(&app, 8),
        vec![0.0, 0.0, 6.0, 6.0, 12.0, 0.0, 18.0, 0.0],
        "step 3 shows step 2's message and nothing else moved"
    );
}

/// The point of storing a step number rather than a lattice: two steps
/// showing the same message flatten to the same raster, so the render cache
/// holds one frame for the pair rather than two identical ones (D59).
#[test]
fn two_steps_showing_one_message_hash_alike() {
    let root = TempRoot::new("hash");
    let app = sparse(&root, 8);
    let layer = grib_layer(&app);
    frames::frames_copy(&app, layer, vec![2]).expect("copy");
    frames::frames_paste(&app, 3).expect("paste");

    let session = app.session.lock().expect("lock");
    let project = &session.open.as_ref().expect("open").project;
    let hash_at = |step: u32| {
        let scene = flatten(project, step);
        assert_eq!(scene.rasters.len(), 1, "one imported field at step {step}");
        scene.rasters[0].grid.hash
    };
    assert_eq!(hash_at(2), hash_at(3));
    assert_ne!(hash_at(2), hash_at(4));
}

#[test]
fn a_run_keeps_its_spacing_and_undoes_as_one() {
    let root = TempRoot::new("run");
    let app = sparse(&root, 12);
    let layer = grib_layer(&app);

    // Steps 0, 2 and 4 carry 0, 6 and 12 h. Copy the three and paste at 6:
    // the spacing is kept, so 6, 8 and 10 take them.
    frames::frames_copy(&app, layer, vec![0, 2, 4]).expect("copy");
    let summary = frames::frames_paste(&app, 6).expect("paste");
    assert!(summary.can_undo);
    assert_eq!(
        seen(&app, 12),
        vec![0.0, 0.0, 6.0, 0.0, 12.0, 0.0, 0.0, 0.0, 6.0, 0.0, 12.0, 0.0],
        "steps 6, 8 and 10 took the run"
    );

    // One undo returns all three.
    ve_app::edit::undo_for_test(&app).expect("undo");
    assert_eq!(
        seen(&app, 12),
        vec![0.0, 0.0, 6.0, 0.0, 12.0, 0.0, 18.0, 0.0, 0.0, 0.0, 0.0, 0.0]
    );
}

/// A step past the end is dropped, not clamped. Clamping would pile the tail
/// of a run onto the last step, each overwriting the one before, and leave
/// one frame where the user asked for three.
#[test]
fn a_paste_that_runs_off_the_end_drops_what_falls_off() {
    let root = TempRoot::new("overflow");
    let app = sparse(&root, 8);
    let layer = grib_layer(&app);
    frames::frames_copy(&app, layer, vec![0, 2, 4]).expect("copy");
    // Steps 7, 9 and 11 — only the first is on the timeline.
    frames::frames_paste(&app, 7).expect("paste");
    assert_eq!(
        seen(&app, 8),
        vec![0.0, 0.0, 6.0, 0.0, 12.0, 0.0, 18.0, 0.0],
        "the 0 h message is calm, so step 7 shows calm — but it is covered"
    );
    let tree = document::tree(&app, 0).expect("tree");
    let grib = tree.layers[1].grib.as_ref().expect("a grib layer");
    assert_eq!(grib.steps[7].source, Some(0), "step 7 took the first frame");
    assert!(grib.steps[7].shown);
    assert_eq!(
        grib.steps.iter().filter(|s| s.source.is_some()).count(),
        1,
        "the two that fell off the end were dropped, not clamped onto step 7"
    );
}

/// `Delete` means two things, and which applies is a property of the step:
/// on a pasted frame it restores the file's own message, and on the file's
/// own it hides that message.
#[test]
fn delete_restores_a_pasted_step_and_hides_a_file_one() {
    let root = TempRoot::new("delete");
    let app = sparse(&root, 8);
    let layer = grib_layer(&app);

    // Paste over a step the file already covers, then take it away again.
    frames::frames_copy(&app, layer, vec![4]).expect("copy");
    frames::frames_paste(&app, 2).expect("paste");
    assert_eq!(u_at(&app, 2), 12.0, "the paste wins over the file");
    frames::frames_delete(&app, layer, vec![2]).expect("delete");
    assert_eq!(u_at(&app, 2), 6.0, "the file's own message is back");

    // Delete on the file's own message hides it, and what was painted on the
    // layer — nothing, here — stands alone.
    frames::frames_delete(&app, layer, vec![2]).expect("delete");
    assert_eq!(u_at(&app, 2), 0.0);
    let tree = document::tree(&app, 0).expect("tree");
    let grib = tree.layers[1].grib.as_ref().expect("a grib layer");
    assert!(grib.steps[2].in_file, "the file still has the message");
    assert!(grib.steps[2].hidden);
    assert!(!grib.steps[2].shown);

    ve_app::edit::undo_for_test(&app).expect("undo");
    assert_eq!(u_at(&app, 2), 6.0);
}

/// The paste goes back to the layer the copy came from, whatever layer is
/// active — a GRIB frame pasted into another layer would be a copy of samples
/// by another route (D59) — and it creates no object anywhere.
#[test]
fn a_paste_lands_on_its_own_layer_and_makes_no_object() {
    let root = TempRoot::new("layer");
    let app = sparse(&root, 8);
    let layer = grib_layer(&app);
    frames::frames_copy(&app, layer, vec![2]).expect("copy");
    // Add a painted layer and leave it on top; the paste must ignore it.
    document::layer_add(&app, "Painted".to_owned()).expect("add layer");
    frames::frames_paste(&app, 3).expect("paste");

    assert_eq!(u_at(&app, 3), 6.0);
    let session = app.session.lock().expect("lock");
    let project = &session.open.as_ref().expect("open").project;
    assert!(
        project.layers.iter().all(|l| l.objects.is_empty()),
        "a pasted frame is not an object"
    );
    assert_eq!(
        project
            .layers
            .iter()
            .filter(|l| !l.frame_overrides.is_empty())
            .count(),
        1,
        "only the source layer took the paste"
    );
}

/// Overrides survive a save and load, and the `.veproj` gains no samples: it
/// keeps a step number, which is what invariants 1 and 2 require.
#[test]
fn overrides_round_trip_and_the_file_gains_no_samples() {
    let root = TempRoot::new("roundtrip");
    let app = sparse(&root, 8);
    let layer = grib_layer(&app);
    frames::frames_copy(&app, layer, vec![2, 4]).expect("copy");
    frames::frames_paste(&app, 3).expect("paste");
    frames::frames_delete(&app, layer, vec![0]).expect("hide");
    let before = seen(&app, 8);

    let project_path = root.0.join("frames.veproj");
    projects::save_as(&app, project_path.to_string_lossy().into_owned()).expect("save");
    // Small enough to be geometry and parameters and nothing else: a global
    // 1° lattice is 65,160 nodes, which no amount of deflate brings near.
    let size = std::fs::metadata(&project_path).expect("metadata").len();
    assert!(size < 64 * 1024, "the project file is {size} bytes");

    projects::open(&app, project_path.to_string_lossy().into_owned(), false).expect("open");
    assert_eq!(seen(&app, 8), before, "the overrides came back");
    let tree = document::tree(&app, 0).expect("tree");
    let grib = tree.layers[1].grib.as_ref().expect("a grib layer");
    // Copied 2 and 4, pasted at 3: the spacing is kept, so 3 and 5 took them.
    assert_eq!(grib.steps[3].source, Some(2));
    assert_eq!(grib.steps[5].source, Some(4));
    assert!(grib.steps[0].hidden);
}

/// The file can change under an override. It references a step, and which
/// message a step gets is decided by time alignment on open, so a file that
/// no longer has a message at the source leaves the pasted step empty rather
/// than showing something else.
#[test]
fn a_replaced_file_leaves_a_pasted_step_empty() {
    let root = TempRoot::new("replaced");
    let app = sparse(&root, 8);
    let layer = grib_layer(&app);
    frames::frames_copy(&app, layer, vec![4]).expect("copy");
    frames::frames_paste(&app, 5).expect("paste");
    assert_eq!(u_at(&app, 5), 12.0);

    let project_path = root.0.join("replaced.veproj");
    projects::save_as(&app, project_path.to_string_lossy().into_owned()).expect("save");
    // Same path, a file that stops before the source step's hour.
    write_file(&root, "wind.grib2", &[0, 6]);
    projects::open(&app, project_path.to_string_lossy().into_owned(), false).expect("open");

    assert_eq!(u_at(&app, 5), 0.0, "the source message is gone");
    let tree = document::tree(&app, 0).expect("tree");
    let grib = tree.layers[1].grib.as_ref().expect("a grib layer");
    assert_eq!(grib.steps[5].source, Some(4), "the override is still there");
    assert!(!grib.steps[5].shown, "but there is nothing to show");
    assert_eq!(
        u_at(&app, 2),
        6.0,
        "the messages it still has are unaffected"
    );
}

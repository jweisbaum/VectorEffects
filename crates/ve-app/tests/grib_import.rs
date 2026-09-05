#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test setup: a panic naming the failed step is the right outcome, and clippy's allow-expect-in-tests does not reach helpers in tests/"
)]
//! Importing a GRIB2 file as layers, end to end (spec.md 4.8).
//!
//! A file the app's own writer produced is imported into a project, and the
//! field is read back through the same flatten-and-evaluate path the tiles
//! and the export use. What matters: the layers that appear, which is shown,
//! which message each step shows, that one undo removes the import, and
//! that the project reopens with the field read back from the file rather
//! than from the project.

use std::path::PathBuf;

use ve_app::commands::AppState;
use ve_app::document;
use ve_app::edit;
use ve_app::import;
use ve_app::paths::AppPaths;
use ve_app::projects::{self, NewProjectRequest};
use ve_core::LonLat;
use ve_core::project::FieldKind;
use ve_grib::writer::{GridSpec, MessageSpec, Parameter, ReferenceTime, message};
use ve_render::cpu::sample_scene;
use ve_render::scene::flatten;

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ve-grib-import-{}-{}-{label}",
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
            name: "Import".to_owned(),
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

/// A 1° global file: `u` equals the forecast hour everywhere, `v` is 0.
fn write_file(root: &TempRoot, name: &str, kinds: &[FieldKind], hours: &[u32]) -> String {
    let grid = GridSpec {
        ni: 360,
        nj: 181,
        micro_degrees: 1_000_000,
    };
    let mut bytes = Vec::new();
    for &kind in kinds {
        let (pu, pv) = match kind {
            FieldKind::Wind => (Parameter::WindU, Parameter::WindV),
            FieldKind::Current => (Parameter::CurrentU, Parameter::CurrentV),
        };
        for &hour in hours {
            let count = grid.point_count() as usize;
            for (parameter, value) in [(pu, hour as f32), (pv, 0.0)] {
                let spec = MessageSpec {
                    parameter,
                    grid,
                    reference_time: ReferenceTime {
                        year: 2026,
                        month: 9,
                        day: 3,
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
    }
    let path = root.0.join(name);
    std::fs::write(&path, bytes).expect("write grib");
    path.to_string_lossy().into_owned()
}

/// The field at a position, through the same path the tiles take.
fn u_at(state: &AppState, step: u32) -> f32 {
    let session = state.session.lock().expect("lock");
    let project = &session.open.as_ref().expect("open").project;
    sample_scene(&flatten(project, step), LonLat::new(-30.0, 45.0).unwrap()).u
}

#[test]
fn a_grib_becomes_a_layer_whose_messages_follow_the_projects_hours() {
    let root = TempRoot::new("hours");
    // Hourly project, 3-hourly file.
    let app = state(&root, 1, 12);
    let path = write_file(&root, "wind.grib2", &[FieldKind::Wind], &[0, 3, 6]);

    let summary = import::grib_import(&app, path.clone()).expect("import");
    assert_eq!(summary.layer_count, 2);
    assert!(summary.can_undo);

    let tree = document::tree(&app, 0).expect("tree");
    let layer = &tree.layers[1];
    assert_eq!(layer.name, "Wind (wind.grib2)");
    assert!(layer.visible, "the project's own kind is shown");
    let grib = layer.grib.as_ref().expect("a grib layer");
    assert_eq!(grib.path, path);
    assert_eq!(grib.field_kind, "wind");
    assert!(grib.loaded);
    assert_eq!(grib.frame_count, 3);
    assert_eq!(grib.span_hours, 6.0);
    // Spec 4.8: the timeline marks which steps the file has a message for, so
    // a field that comes and goes is legible rather than mysterious. Hourly
    // project, 3-hourly file: hours 0, 3 and 6 and no others.
    assert_eq!(
        grib.covered_steps,
        vec![
            true, false, false, true, false, false, true, false, false, false, false, false
        ]
    );
    assert!(
        tree.layers[0].grib.is_none(),
        "the painted layer is unchanged"
    );

    // Spec 4.8: a step the file has no message for shows no imported field.
    // Hours 1, 2, 4, 5 fall between messages and 7 onward are past the last
    // one; all of them are calm, because nothing was painted on the layer.
    let seen: Vec<f32> = (0..12).map(|s| u_at(&app, s)).collect();
    assert_eq!(
        seen,
        vec![0.0, 0.0, 0.0, 3.0, 0.0, 0.0, 6.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        "an hour the file says nothing about shows nothing"
    );
}

#[test]
fn an_hourly_file_in_a_three_hourly_project_is_read_every_third_hour() {
    let root = TempRoot::new("coarse");
    let app = state(&root, 3, 4);
    let path = write_file(
        &root,
        "hourly.grib2",
        &[FieldKind::Wind],
        &[0, 1, 2, 3, 4, 5, 6],
    );
    import::grib_import(&app, path).expect("import");
    // 0, 3 and 6 are messages the file has; the fourth step is at 9 h, which it
    // does not, so there is nothing to show there.
    let seen: Vec<f32> = (0..4).map(|s| u_at(&app, s)).collect();
    assert_eq!(seen, vec![0.0, 3.0, 6.0, 0.0]);
}

/// Spec 4.8: a GRIB layer can be limited to a band of speeds, and a sample
/// outside it is dropped exactly as a missing one is — what is beneath shows
/// through, the layer's own painted objects included.
#[test]
fn a_speed_filter_keeps_only_the_band_it_names() {
    let root = TempRoot::new("filter");
    let app = state(&root, 3, 2);
    // The generator's messages carry `u` equal to the forecast hour, so the
    // 0 h message is calm and the 3 h one blows at 3 m/s.
    let path = write_file(&root, "band.grib2", &[FieldKind::Wind], &[0, 3]);
    import::grib_import(&app, path).expect("import");
    let layer = {
        let session = app.session.lock().expect("lock");
        session.open.as_ref().expect("open").project.layers[1]
            .id
            .raw()
    };

    assert!((u_at(&app, 1) - 3.0).abs() < 1e-3, "unfiltered");

    // A band above the file's speed keeps nothing.
    ve_app::document::layer_speed_range(&app, layer, Some(10.0), Some(20.0)).expect("filter");
    assert!(
        u_at(&app, 1).abs() < 1e-6,
        "a sample outside the band must be dropped: {}",
        u_at(&app, 1)
    );

    // A band around it keeps it.
    ve_app::document::layer_speed_range(&app, layer, Some(2.0), Some(4.0)).expect("filter");
    assert!((u_at(&app, 1) - 3.0).abs() < 1e-3);

    // Ends the wrong way round are ordered rather than refused: dragging the
    // low end past the high one is a gesture, not a mistake.
    ve_app::document::layer_speed_range(&app, layer, Some(4.0), Some(2.0)).expect("filter");
    assert!((u_at(&app, 1) - 3.0).abs() < 1e-3);

    // Clearing it brings everything back, and undoes like any other edit —
    // back to the band that was dropping the sample.
    ve_app::document::layer_speed_range(&app, layer, Some(10.0), Some(20.0)).expect("filter");
    ve_app::document::layer_speed_range(&app, layer, None, None).expect("clear");
    assert!((u_at(&app, 1) - 3.0).abs() < 1e-3);
    ve_app::edit::undo_for_test(&app).expect("undo");
    assert!(
        u_at(&app, 1).abs() < 1e-6,
        "undoing the clear brings the band back"
    );
}

/// Spec 4.8: past the file's last message the layer is simply not there, and
/// what the user painted on it is left standing alone. This is the case the
/// hold rule got wrong most visibly — a six-hour file stood in for a ten-day
/// timeline.
#[test]
fn past_the_last_message_the_imported_field_is_gone_and_the_painting_stays() {
    let root = TempRoot::new("past-the-end");
    let app = state(&root, 3, 6);
    let path = write_file(&root, "short.grib2", &[FieldKind::Wind], &[0, 3]);
    import::grib_import(&app, path).expect("import");

    // A stroke on the layer above the import, alive for the whole timeline.
    ve_app::edit::paint(
        &app,
        ve_app::edit::BrushStroke {
            points: vec![[-30.0, 45.0]],
            size_km: 900.0,
            speed_mps: 11.0,
            direction_toward_deg: 90.0,
            feather: 0.0,
            ..Default::default()
        },
    )
    .expect("paint");

    // Where the file has a message the painting sits over it; where it has
    // none the painting is all there is — and it is unchanged, which is the
    // point: the import going away must not take anything else with it.
    let seen: Vec<f32> = (0..6).map(|s| u_at(&app, s)).collect();
    for (step, u) in seen.iter().enumerate() {
        assert!(
            (u - 11.0).abs() < 0.5,
            "step {step}: the stroke reads {u} m/s"
        );
    }
}

#[test]
fn a_file_with_both_kinds_makes_two_layers_and_hides_the_other_kind() {
    let root = TempRoot::new("both");
    let app = state(&root, 3, 4);
    let path = write_file(
        &root,
        "both.grib2",
        &[FieldKind::Current, FieldKind::Wind],
        &[0, 3],
    );
    let summary = import::grib_import(&app, path).expect("import");
    assert_eq!(summary.layer_count, 3);

    let tree = document::tree(&app, 0).expect("tree");
    let names: Vec<(String, bool, String)> = tree.layers[1..]
        .iter()
        .map(|l| {
            (
                l.name.clone(),
                l.visible,
                l.grib.as_ref().expect("grib").field_kind.clone(),
            )
        })
        .collect();
    assert_eq!(
        names,
        vec![
            ("Wind (both.grib2)".to_owned(), true, "wind".to_owned()),
            (
                "Currents (both.grib2)".to_owned(),
                false,
                "current".to_owned()
            ),
        ]
    );

    // Wind is the field the map shows; the hidden currents contribute nothing.
    assert_eq!(u_at(&app, 1), 3.0);

    // One undo removes both layers.
    let undone = edit::undo_for_test(&app).expect("undo");
    assert_eq!(undone.layer_count, 1);
    assert_eq!(u_at(&app, 1), 0.0);
    let redone = edit::redo_for_test(&app).expect("redo");
    assert_eq!(redone.layer_count, 3);
    assert_eq!(
        u_at(&app, 1),
        3.0,
        "redo brings the field back with the layer"
    );
}

#[test]
fn the_field_is_read_from_the_file_again_when_the_project_reopens() {
    let root = TempRoot::new("reopen");
    let app = state(&root, 3, 4);
    let grib = write_file(&root, "wind.grib2", &[FieldKind::Wind], &[0, 3]);
    import::grib_import(&app, grib.clone()).expect("import");

    let project_path = root.0.join("imported.veproj");
    projects::save_as(&app, project_path.to_string_lossy().into_owned()).expect("save");
    // The project file is small: it holds the path, not the 65,160 samples.
    let size = std::fs::metadata(&project_path).expect("saved").len();
    assert!(
        size < 20_000,
        "project file is {size} bytes; the field must not be in it"
    );
    projects::close_open(&app, true).expect("close");

    let reopened =
        projects::open(&app, project_path.to_string_lossy().into_owned(), false).expect("open");
    assert_eq!(reopened.layer_count, 2);
    assert_eq!(u_at(&app, 1), 3.0, "the field is back from the file");
    let tree = document::tree(&app, 0).expect("tree");
    assert!(tree.layers[1].grib.as_ref().expect("grib").loaded);

    // With the file gone the layer opens empty and says so.
    projects::close_open(&app, true).expect("close");
    std::fs::remove_file(&grib).expect("remove grib");
    projects::open(&app, project_path.to_string_lossy().into_owned(), false).expect("open");
    let tree = document::tree(&app, 0).expect("tree");
    let info = tree.layers[1].grib.as_ref().expect("grib");
    assert!(!info.loaded);
    assert_eq!(info.frame_count, 0);
    assert_eq!(u_at(&app, 1), 0.0, "a missing file contributes nothing");
}

#[test]
fn a_file_without_vector_fields_is_refused_by_name() {
    let root = TempRoot::new("refused");
    let app = state(&root, 3, 4);
    let path = root.0.join("notes.grib2");
    std::fs::write(&path, b"not a grib").expect("write");
    let err = import::grib_import(&app, path.to_string_lossy().into_owned()).unwrap_err();
    assert_eq!(err.kind(), "grib", "{err}");
    let summary = projects::current(&app).expect("current").expect("open");
    assert_eq!(summary.layer_count, 1, "nothing was added");
    assert!(!summary.can_undo, "nothing to undo");
}

// --- A project from a file ---------------------------------------------------

fn fresh(root: &TempRoot) -> AppState {
    AppState::new(AppPaths::in_directory(&root.0).expect("paths"))
}

#[test]
fn a_project_from_a_grib_takes_its_shape_from_the_file() {
    let root = TempRoot::new("from-grib");
    let app = fresh(&root);
    let path = write_file(&root, "forecast.grib2", &[FieldKind::Wind], &[0, 3, 6, 9]);

    let summary = import::grib_project(&app, path, false).expect("create");
    assert_eq!(summary.name, "forecast");
    assert_eq!(summary.field_kind, "wind");
    assert_eq!(summary.resolution_label, "1°", "the file's 1° grid");
    assert_eq!(summary.step_hours, 3, "the file's own spacing");
    assert_eq!(summary.step_count, 4, "0, 3, 6 and 9 h");
    // 2026-09-03T00:00Z, the first message's valid time.
    assert_eq!(summary.start_unix_s, Some(1_788_393_600));
    assert_eq!(summary.layer_count, 2, "the default layer and the import");
    assert!(
        !summary.can_undo,
        "the import is the project, not an edit to it"
    );
    assert!(summary.dirty, "never saved");

    let seen: Vec<f32> = (0..4).map(|s| u_at(&app, s)).collect();
    assert_eq!(seen, vec![0.0, 3.0, 6.0, 9.0]);
}

#[test]
fn a_current_only_file_makes_a_current_project() {
    let root = TempRoot::new("from-current");
    let app = fresh(&root);
    let path = write_file(&root, "rtofs.grib2", &[FieldKind::Current], &[0, 24, 48]);
    let summary = import::grib_project(&app, path, false).expect("create");
    assert_eq!(summary.field_kind, "current");
    assert_eq!(summary.step_hours, 24);
    assert_eq!(summary.step_count, 3);
    let tree = document::tree(&app, 0).expect("tree");
    assert!(
        tree.layers[1].visible,
        "the current layer is the project's kind"
    );
}

#[test]
fn a_file_with_one_message_makes_a_one_step_project() {
    let root = TempRoot::new("from-single");
    let app = fresh(&root);
    let path = write_file(&root, "analysis.grib2", &[FieldKind::Wind], &[0]);
    let summary = import::grib_project(&app, path, false).expect("create");
    assert_eq!(summary.step_count, 1);
}

#[test]
fn creating_from_a_grib_refuses_to_discard_unsaved_work() {
    let root = TempRoot::new("from-guarded");
    let app = state(&root, 3, 4);
    let path = write_file(&root, "wind.grib2", &[FieldKind::Wind], &[0, 3]);
    let err = import::grib_project(&app, path.clone(), false).unwrap_err();
    assert_eq!(err.kind(), "unsaved-changes", "{err}");
    let summary = import::grib_project(&app, path, true).expect("discard and create");
    assert_eq!(summary.name, "wind");
}

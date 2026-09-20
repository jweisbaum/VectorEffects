#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::Path;
use std::sync::Arc;

use ve_app::commands::AppState;
use ve_app::paths::AppPaths;
use ve_app::{document, edit, frames, projects, zarr};
use ve_core::LonLat;
use ve_core::document::LayerSource;
use ve_core::project::FieldKind;
use ve_render::cpu::sample_scene;
use ve_render::scene::flatten_kind;
use ve_zarr::export::{Layout, Writer, f16};

fn write_store(path: &Path) {
    let layout = Layout::new(1_000_000, 2, 3).unwrap();
    let mut writer =
        Writer::create(path, layout, "routing", "test", "2012-01-01T00:00:00").unwrap();
    let mut values = vec![f16::NAN; layout.chunk_len()];
    for t in 0..2 {
        for p in 0..4 {
            for r in 0..10 {
                for c in 0..10 {
                    // Wind varies in time and space; currents are constant.
                    let value = match p {
                        0 => 4 + t * 4 + r,
                        1 => c,
                        2 => 1,
                        _ => 2,
                    };
                    values[((t * 4 + p) * 10 + r) * 10 + c] = f16::from_f32(value as f32);
                }
            }
        }
    }
    writer.write_chunk(0, 4, 15, &values).unwrap();
    writer.finish_time().unwrap();
}

fn app(path: &Path) -> AppState {
    AppState::new(AppPaths::in_directory(path).unwrap())
}

fn sample(app: &AppState, step: u32, kind: FieldKind) -> (f32, f32) {
    let session = app.session.lock().unwrap();
    let project = &session.open.as_ref().unwrap().project;
    let uv = sample_scene(
        &flatten_kind(project, step, kind),
        LonLat::new(-28.0, 49.0).unwrap(),
    );
    (uv.u, uv.v)
}

#[test]
fn opens_as_wind_and_current_layers_with_the_native_grid_and_clock() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("routing_test");
    write_store(&path);
    let app = app(root.path());
    let summary = zarr::zarr_project(&app, path.display().to_string(), false).unwrap();
    assert_eq!(summary.name, "routing_test");
    assert_eq!(summary.layer_count, 3);
    assert_eq!(summary.field_kind, "wind");
    assert_eq!(summary.step_hours, 3);
    assert_eq!(summary.step_count, 2);
    assert_eq!(summary.start_unix_s, Some(1_325_376_000));
    assert_eq!(summary.resolution_label, "1°");
    assert!(!summary.can_undo);
    assert_eq!(sample(&app, 0, FieldKind::Wind), (5.0, 2.0));
    assert_eq!(sample(&app, 1, FieldKind::Wind), (9.0, 2.0));
    assert_eq!(sample(&app, 0, FieldKind::Current), (1.0, 2.0));

    let tree = document::tree(&app, 0).unwrap();
    for layer in &tree.layers[1..] {
        assert_eq!(layer.source, "zarr");
        assert!(layer.visible);
        let raster = layer.grib.as_ref().unwrap();
        assert!(raster.loaded);
        assert!(raster.history.is_none());
        assert_eq!(raster.frame_count, 2);
        assert_eq!(raster.covered_steps, [true, true]);
    }
    let session = app.session.lock().unwrap();
    let current = session.open.as_ref().unwrap().project.layers[2]
        .raster
        .as_ref()
        .unwrap();
    assert!(Arc::ptr_eq(
        &current.frames[0].grid,
        &current.frames[1].grid
    ));
    let wind = session.open.as_ref().unwrap().project.layers[1]
        .raster
        .as_ref()
        .unwrap();
    assert!(
        wind.frames[0].grid.sample(0.0, 0.0).is_none(),
        "absent shards remain uncovered"
    );
    assert_eq!(
        (wind.frames[0].grid.lon0, wind.frames[0].grid.nj),
        (-180.0, 180)
    );

    let exported = root.path().join("exported.zarr");
    ve_app::export::run_zarr(
        &session.open.as_ref().unwrap().project,
        &ve_app::export::ExportZarrRequest {
            path: exported.display().to_string(),
            year: 2012,
            month: 1,
            day: 1,
            hour: 0,
        },
        &std::sync::atomic::AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    let roundtrip = zarr::read(&exported).unwrap();
    let uv = roundtrip[0].frames[1].grid.sample(-28.0, 49.0).unwrap();
    assert_eq!((uv.u, uv.v), (9.0, 2.0), "imported vectors reach export");
}

#[test]
fn component_names_determine_the_vector_order() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("reordered.zarr");
    write_store(&path);
    let store = Arc::new(zarrs::filesystem::FilesystemStore::new(&path).unwrap());
    let param = zarrs::array::Array::open(store, "/param").unwrap();
    let names: Vec<Vec<char>> = ["v10", "u10", "vcur", "ucur"]
        .into_iter()
        .map(|name| name.chars().collect())
        .collect();
    param.store_chunk(&[0], names).unwrap();
    let sequences = zarr::read(&path).unwrap();
    let wind = sequences[0].frames[0].grid.sample(-28.0, 49.0).unwrap();
    let current = sequences[1].frames[0].grid.sample(-28.0, 49.0).unwrap();
    assert_eq!((wind.u, wind.v), (2.0, 5.0));
    assert_eq!((current.u, current.v), (2.0, 1.0));
}

#[test]
fn import_undo_frame_edits_filters_and_reopen_use_the_existing_raster_path() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("field.zarr");
    write_store(&path);
    let app = app(root.path());
    projects::create(
        &app,
        projects::NewProjectRequest {
            name: "Import".into(),
            field_kind: "wind".into(),
            resolution: "1.0".into(),
            step_hours: 1,
            step_count: 6,
        },
        false,
    )
    .unwrap();
    zarr::zarr_import(&app, path.display().to_string()).unwrap();
    let tree = document::tree(&app, 0).unwrap();
    let layer = tree.layers[1].id;
    assert_eq!(
        tree.layers[1].grib.as_ref().unwrap().covered_steps,
        [true, false, false, true, false, false]
    );
    edit::undo_for_test(&app).unwrap();
    assert_eq!(document::tree(&app, 0).unwrap().layers.len(), 1);
    edit::redo_for_test(&app).unwrap();
    assert_eq!(sample(&app, 3, FieldKind::Wind), (9.0, 2.0));
    frames::frames_copy(&app, layer, vec![3]).unwrap();
    frames::frames_paste(&app, 1).unwrap();
    assert_eq!(sample(&app, 1, FieldKind::Wind), (9.0, 2.0));
    document::layer_speed_range(&app, layer, Some(20.0), Some(30.0), None).unwrap();
    assert_eq!(sample(&app, 1, FieldKind::Wind), (0.0, 0.0));
    edit::undo_for_test(&app).unwrap();

    let saved = root.path().join("roundtrip.veproj").display().to_string();
    projects::save_as(&app, saved.clone()).unwrap();
    projects::close_open(&app, false).unwrap();
    projects::open(&app, saved.clone(), false).unwrap();
    assert_eq!(sample(&app, 1, FieldKind::Wind), (9.0, 2.0));
    {
        let session = app.session.lock().unwrap();
        assert!(matches!(
            session.open.as_ref().unwrap().project.layers[1].source,
            LayerSource::ZarrFile { .. }
        ));
    }
    // The project survives a missing source and reports unloaded rasters.
    projects::close_open(&app, false).unwrap();
    std::fs::rename(&path, root.path().join("moved.zarr")).unwrap();
    projects::open(&app, saved, false).unwrap();
    let tree = document::tree(&app, 0).unwrap();
    assert_eq!(tree.layers.len(), 3);
    assert!(
        tree.layers[1..]
            .iter()
            .all(|l| !l.grib.as_ref().unwrap().loaded)
    );
}

#[test]
fn invalid_imports_and_unsaved_replacement_leave_the_project_intact() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("routing_test");
    write_store(&path);
    let app = app(root.path());
    zarr::zarr_project(&app, path.display().to_string(), false).unwrap();
    let error = zarr::zarr_project(&app, path.display().to_string(), false).unwrap_err();
    assert_eq!(error.kind(), "unsaved-changes");
    let bad = root.path().join("not-a-store").display().to_string();
    let error = zarr::zarr_import(&app, bad.clone()).unwrap_err();
    assert!(error.to_string().contains(&bad));
    assert_eq!(projects::current(&app).unwrap().unwrap().layer_count, 3);
    zarr::zarr_project(&app, path.display().to_string(), true).unwrap();
}

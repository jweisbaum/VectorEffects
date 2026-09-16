#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! M4's end-to-end acceptance: paint, export, decode, and find the paint.
//!
//! This is the walking skeleton the whole plan is built around — a project, a
//! brush stroke, a GRIB2 file, and an independent parse of what came out.

use std::sync::atomic::AtomicBool;

use ve_app::commands::AppState;
use ve_app::document;
use ve_app::edit::{self, BrushStroke};
use ve_app::error::AppError;
use ve_app::export::{self, ExportRequest, ExportZarrRequest};
use ve_app::paths::AppPaths;
use ve_app::projects::{self, NewProjectRequest};
use ve_grib::reader::decode;

struct TempRoot(std::path::PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ve-export-{}-{label}-{}",
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
    AppState::new(AppPaths::in_directory(&root.0).expect("paths"))
}

/// A one-degree wind project: small enough to export in a test, real enough to
/// exercise every code path.
fn new_project(field_kind: &str) -> NewProjectRequest {
    NewProjectRequest {
        name: "Export".to_owned(),
        field_kind: field_kind.to_owned(),
        resolution: "1.0".to_owned(),
        step_hours: 3,
        step_count: 2,
    }
}

fn request(path: &std::path::Path) -> ExportRequest {
    ExportRequest {
        path: path.to_string_lossy().into_owned(),
        year: 2026,
        month: 9,
        day: 2,
        hour: 0,
    }
}

fn zarr_request(path: &std::path::Path) -> ExportZarrRequest {
    ExportZarrRequest {
        path: path.to_string_lossy().into_owned(),
        year: 2026,
        month: 9,
        day: 2,
        hour: 0,
    }
}

/// Splits a file into its messages the way a decoder would.
fn messages(bytes: &[u8]) -> Vec<ve_grib::reader::Decoded> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 16 <= bytes.len() {
        assert_eq!(&bytes[at..at + 4], b"GRIB", "message {} magic", out.len());
        let length =
            u64::from_be_bytes(bytes[at + 8..at + 16].try_into().expect("eight bytes")) as usize;
        out.push(decode(&bytes[at..at + length]));
        at += length;
    }
    out
}

/// The whole point of the application, in one test.
#[test]
fn a_painted_stroke_reaches_the_exported_file() {
    let root = TempRoot::new("skeleton");
    let state = app(&root);
    projects::create(&state, new_project("wind"), false).expect("create");

    // A stroke straight across the equator, blowing due east at 20 m/s.
    edit::paint(
        &state,
        BrushStroke {
            points: vec![[-20.0, 0.0], [0.0, 0.0], [20.0, 0.0]],
            size_km: 1000.0,
            speed_mps: 20.0,
            direction_toward_deg: 90.0,
            feather: 0.0,
            layer: None,
            ..Default::default()
        },
    )
    .expect("paint");

    let path = root.0.join("out.grib2");
    let project = {
        let mut session = state.session.lock().expect("lock");
        session.require_open().expect("open").project.clone()
    };
    let result =
        export::run(&project, &request(&path), &AtomicBool::new(false), |_| {}).expect("export");

    assert_eq!(result.messages, 4, "two components for each of two steps");
    assert!(path.exists());

    let decoded = messages(&std::fs::read(&path).expect("read"));
    assert_eq!(decoded.len(), 4);

    // Message order is u then v, step by step.
    let u = &decoded[0];
    let v = &decoded[1];
    assert_eq!((u.category, u.number), (2, 2), "UGRD");
    assert_eq!((v.category, v.number), (2, 3), "VGRD");
    assert_eq!(u.forecast_hour, 0);
    assert_eq!(
        decoded[2].forecast_hour, 3,
        "the second step is three hours on"
    );

    // The stroke sits on the equator at longitude 0, which in scanning order is
    // row 90 (90 degrees south of the first row), column 0.
    let index = 90 * 360;
    assert!(
        (u.values[index] - 20.0).abs() < 0.01,
        "expected 20 m/s eastward at the stroke, got u={}",
        u.values[index]
    );
    assert!(
        v.values[index].abs() < 0.01,
        "a due-east flow has no northward component, got v={}",
        v.values[index]
    );

    // Away from the stroke the field is undefined, distinct from a painted calm.
    let far = 10 * 360 + 180;
    assert!(
        u.values[far].is_nan() && v.values[far].is_nan(),
        "should be undefined"
    );
}

/// A due-east flow must land in `u`, not `v`. This is the check that catches
/// the single most likely silent bug in the codebase.
/// A project with a wind layer and a current layer exports both (M29): per
/// step, one u/v pair of each, the wind pair meteorological and the current
/// pair oceanographic, each baked from its own layers alone.
#[test]
fn wind_and_current_layers_export_as_two_message_pairs_per_step() {
    let root = TempRoot::new("two-kinds");
    let state = app(&root);
    projects::create(&state, new_project("wind"), false).expect("create");
    edit::paint(
        &state,
        BrushStroke {
            points: vec![[-20.0, 0.0], [20.0, 0.0]],
            size_km: 1000.0,
            speed_mps: 20.0,
            direction_toward_deg: 90.0,
            feather: 0.0,
            layer: None,
            ..Default::default()
        },
    )
    .expect("wind stroke");
    let currents = {
        document::layer_add(&state, "Currents".to_owned()).expect("add layer");
        let tree = document::tree(&state, 0).expect("tree");
        tree.layers.last().expect("the new layer").id
    };
    document::layer_parameter(&state, currents, "current").expect("current layer");
    edit::paint(
        &state,
        BrushStroke {
            points: vec![[-20.0, 0.0], [20.0, 0.0]],
            size_km: 1000.0,
            speed_mps: 1.0,
            direction_toward_deg: 180.0,
            feather: 0.0,
            layer: Some(currents),
            ..Default::default()
        },
    )
    .expect("current stroke");

    let path = root.0.join("both.grib2");
    let project = {
        let mut session = state.session.lock().expect("lock");
        session.require_open().expect("open").project.clone()
    };
    let result =
        export::run(&project, &request(&path), &AtomicBool::new(false), |_| {}).expect("export");
    assert_eq!(
        result.messages, 8,
        "u and v of each kind, for each of two steps"
    );

    let decoded = messages(&std::fs::read(&path).expect("read"));
    let disciplines: Vec<u8> = decoded.iter().map(|m| m.discipline).collect();
    assert_eq!(disciplines, vec![0, 0, 10, 10, 0, 0, 10, 10]);
    // The wind messages hold the wind alone — 20 m/s east at most, and no
    // southward component anywhere — and the current messages the current
    // alone: nothing eastward, 1 m/s south at most.
    let strongest = |message: &ve_grib::reader::Decoded| {
        message
            .values
            .iter()
            .copied()
            .fold(0.0f32, |a, b| if b.abs() > a.abs() { b } else { a })
    };
    assert!((strongest(&decoded[0]) - 20.0).abs() < 0.5, "wind u");
    assert!(
        strongest(&decoded[1]).abs() < 0.5,
        "wind v holds no current"
    );
    assert!(
        strongest(&decoded[2]).abs() < 0.1,
        "current u holds no wind"
    );
    assert!((strongest(&decoded[3]) + 1.0).abs() < 0.1, "current v");
}

#[test]
fn components_are_not_transposed() {
    let root = TempRoot::new("uv");
    let state = app(&root);
    projects::create(&state, new_project("wind"), false).expect("create");

    // Due north this time: it must appear in v, and not at all in u.
    edit::paint(
        &state,
        BrushStroke {
            points: vec![[0.0, 0.0]],
            size_km: 2000.0,
            speed_mps: 15.0,
            direction_toward_deg: 0.0,
            feather: 0.0,
            layer: None,
            ..Default::default()
        },
    )
    .expect("paint");

    let path = root.0.join("north.grib2");
    let project = {
        let mut session = state.session.lock().expect("lock");
        session.require_open().expect("open").project.clone()
    };
    export::run(&project, &request(&path), &AtomicBool::new(false), |_| {}).expect("export");

    let decoded = messages(&std::fs::read(&path).expect("read"));
    let index = 90 * 360;
    assert!(
        decoded[0].values[index].abs() < 0.01,
        "northward flow has no u"
    );
    assert!(
        (decoded[1].values[index] - 15.0).abs() < 0.01,
        "northward flow is all v, got {}",
        decoded[1].values[index]
    );
}

#[test]
fn a_current_project_exports_oceanographic_parameters() {
    let root = TempRoot::new("current");
    let state = app(&root);
    projects::create(&state, new_project("current"), false).expect("create");

    let path = root.0.join("current.grib2");
    let project = {
        let mut session = state.session.lock().expect("lock");
        session.require_open().expect("open").project.clone()
    };
    export::run(&project, &request(&path), &AtomicBool::new(false), |_| {}).expect("export");

    let decoded = messages(&std::fs::read(&path).expect("read"));
    assert_eq!(decoded[0].discipline, 10, "oceanographic");
    assert_eq!((decoded[0].category, decoded[0].number), (1, 2), "UOGRD");
    assert_eq!(decoded[0].surface_type, 160, "depth below sea surface");
}

/// Invariant 4: the same project exports the same bytes.
#[test]
fn export_is_byte_reproducible() {
    let root = TempRoot::new("reproducible");
    let state = app(&root);
    projects::create(&state, new_project("wind"), false).expect("create");
    edit::paint(
        &state,
        BrushStroke {
            points: vec![[10.0, 20.0], [30.0, 25.0]],
            size_km: 1500.0,
            speed_mps: 12.5,
            direction_toward_deg: 137.0,
            feather: 0.4,
            layer: None,
            ..Default::default()
        },
    )
    .expect("paint");

    let project = {
        let mut session = state.session.lock().expect("lock");
        session.require_open().expect("open").project.clone()
    };
    let (a, b) = (root.0.join("a.grib2"), root.0.join("b.grib2"));
    export::run(&project, &request(&a), &AtomicBool::new(false), |_| {}).expect("first");
    export::run(&project, &request(&b), &AtomicBool::new(false), |_| {}).expect("second");

    assert_eq!(
        std::fs::read(&a).expect("read"),
        std::fs::read(&b).expect("read"),
        "two exports of one project must be identical"
    );
}

/// A cancelled export must leave nothing behind — not a partial file, and not
/// the temporary it was writing to.
#[test]
fn cancelling_leaves_no_file() {
    let root = TempRoot::new("cancel");
    let state = app(&root);
    projects::create(&state, new_project("wind"), false).expect("create");

    let project = {
        let mut session = state.session.lock().expect("lock");
        session.require_open().expect("open").project.clone()
    };
    let path = root.0.join("cancelled.grib2");
    let cancel = AtomicBool::new(true);

    let outcome = export::run(&project, &request(&path), &cancel, |_| {});
    assert!(matches!(outcome, Err(AppError::ExportCancelled)));
    assert!(!path.exists(), "no output file");
    assert!(
        !path.with_extension("grib2.partial").exists(),
        "no temporary either"
    );
}

#[test]
fn progress_is_reported_for_every_step() {
    let root = TempRoot::new("progress");
    let state = app(&root);
    projects::create(&state, new_project("wind"), false).expect("create");

    let project = {
        let mut session = state.session.lock().expect("lock");
        session.require_open().expect("open").project.clone()
    };
    let mut seen = Vec::new();
    export::run(
        &project,
        &request(&root.0.join("p.grib2")),
        &AtomicBool::new(false),
        |progress| seen.push((progress.step, progress.total)),
    )
    .expect("export");

    assert_eq!(seen, vec![(1, 2), (2, 2)]);
}

/// A field with no content packs to nothing: every value is identical, so the
/// message needs no data section. Worth pinning, because it is the state every
/// project starts in and it makes the size estimate an upper bound.
#[test]
fn an_empty_project_exports_almost_nothing() {
    let root = TempRoot::new("empty");
    let state = app(&root);
    projects::create(&state, new_project("wind"), false).expect("create");

    let project = {
        let mut session = state.session.lock().expect("lock");
        session.require_open().expect("open").project.clone()
    };
    let path = root.0.join("empty.grib2");
    let result =
        export::run(&project, &request(&path), &AtomicBool::new(false), |_| {}).expect("export");

    assert!(
        result.bytes < 35_000,
        "an empty field needs only bitmaps and headers, got {}",
        result.bytes
    );

    // And it must still be a valid, decodable file of calm values.
    let decoded = messages(&std::fs::read(&path).expect("read"));
    assert_eq!(decoded.len(), 4);
    assert_eq!(decoded[0].values.len(), 360 * 181);
    assert!(decoded[0].values.iter().all(|v| v.is_nan()));
}

#[test]
fn an_invalid_reference_time_is_refused_before_writing() {
    let root = TempRoot::new("baddate");
    let state = app(&root);
    projects::create(&state, new_project("wind"), false).expect("create");

    let project = {
        let mut session = state.session.lock().expect("lock");
        session.require_open().expect("open").project.clone()
    };
    let path = root.0.join("bad.grib2");
    let mut spec = request(&path);
    spec.month = 13;

    assert!(export::run(&project, &spec, &AtomicBool::new(false), |_| {}).is_err());
    assert!(!path.exists());
}

#[test]
fn the_size_estimate_bounds_a_sparse_export() {
    let root = TempRoot::new("estimate");
    let state = app(&root);
    projects::create(&state, new_project("wind"), false).expect("create");
    // A project with content: an entirely constant field packs to no data
    // section at all, which the estimate deliberately does not model.
    edit::paint(
        &state,
        BrushStroke {
            points: vec![[0.0, 0.0]],
            size_km: 3000.0,
            speed_mps: 18.0,
            direction_toward_deg: 45.0,
            feather: 0.5,
            layer: None,
            ..Default::default()
        },
    )
    .expect("paint");

    let project = {
        let mut session = state.session.lock().expect("lock");
        session.require_open().expect("open").project.clone()
    };
    let estimate = export::estimate(&project, 16);
    let path = root.0.join("e.grib2");
    let actual =
        export::run(&project, &request(&path), &AtomicBool::new(false), |_| {}).expect("export");

    assert_eq!(estimate.messages, actual.messages);
    assert!(
        actual.bytes < estimate.bytes,
        "missing cells need no packed value"
    );
    assert!(
        actual.bytes > u64::from(actual.messages) * (360_u64 * 181).div_ceil(8),
        "each sparse message includes a bitmap"
    );
}

#[test]
fn painted_calm_remains_defined_beside_missing_cells() {
    let root = TempRoot::new("defined-calm");
    let state = app(&root);
    projects::create(&state, new_project("wind"), false).unwrap();
    edit::paint(
        &state,
        BrushStroke {
            points: vec![[0.0, 0.0]],
            size_km: 500.0,
            speed_mps: 0.0,
            feather: 0.0,
            ..Default::default()
        },
    )
    .unwrap();
    let project = state
        .session
        .lock()
        .unwrap()
        .require_open()
        .unwrap()
        .project
        .clone();
    let path = root.0.join("calm.grib2");
    export::run(&project, &request(&path), &AtomicBool::new(false), |_| {}).unwrap();
    for message in messages(&std::fs::read(path).unwrap()) {
        assert_eq!(message.values[90 * 360], 0.0);
        assert!(message.values[10 * 360 + 180].is_nan());
    }
}

/// The Zarr export is the GRIB export in another container: the same stroke
/// lands at the same cell, read back through zarrs rather than our own writer.
#[test]
fn a_painted_stroke_reaches_the_zarr_store() {
    use ve_zarr::export::f16;
    use zarrs::array::Array;
    use zarrs::filesystem::FilesystemStore;

    let root = TempRoot::new("zarr");
    let state = app(&root);
    projects::create(&state, new_project("wind"), false).expect("create");
    edit::paint(
        &state,
        BrushStroke {
            points: vec![[-20.0, 0.0], [0.0, 0.0], [20.0, 0.0]],
            size_km: 1000.0,
            speed_mps: 20.0,
            direction_toward_deg: 90.0,
            feather: 0.0,
            layer: None,
            ..Default::default()
        },
    )
    .expect("paint");

    let path = root.0.join("out.zarr");
    let project = {
        let mut session = state.session.lock().expect("lock");
        session.require_open().expect("open").project.clone()
    };
    let mut seen = Vec::new();
    let result = export::run_zarr(
        &project,
        &zarr_request(&path),
        &AtomicBool::new(false),
        |progress| seen.push((progress.step, progress.total)),
    )
    .expect("export");

    assert!(path.is_dir(), "the store is a directory");
    assert!(
        !root.0.join("out.zarr.partial").exists(),
        "no temporary left"
    );
    // A 1° grid is 360 x 181; ten-degree chunks tile it 36 x 19, and two
    // 3-hourly steps fit one 72-hour time chunk.
    assert_eq!(result.chunks, 36 * 19);
    assert_eq!(seen, vec![(1, 2), (2, 2)], "progress per evaluated step");

    let store = std::sync::Arc::new(FilesystemStore::new(&path).expect("store"));
    let array = Array::open(store, "/data").expect("open");
    assert_eq!(array.shape(), &[2, 4, 181, 360]);
    let regular: Vec<u64> = array
        .chunk_shape(&[0, 0, 0, 0])
        .expect("chunk shape")
        .iter()
        .map(|n| n.get())
        .collect();
    assert_eq!(regular, vec![24, 4, 10, 10]);

    // What the dialog asked for and what a reader needs to place a cell.
    let attributes = array.attributes();
    assert_eq!(attributes["reference_time"], "2026-09-02T00:00:00Z");
    assert_eq!(attributes["step_hours"], 3);
    assert_eq!(attributes["latitude_start"], 90.0);
    assert_eq!(attributes["latitude_step"], -1.0);
    assert_eq!(attributes["longitude_start"], 0.0);
    assert_eq!(attributes["longitude_step"], 1.0);
    assert_eq!(
        attributes["parameter_order"],
        serde_json::json!([
            "u10m_wind",
            "v10m_wind",
            "u_total_surface_current",
            "v_total_surface_current"
        ])
    );

    // The equator at longitude 0 is row 90, column 0: chunk [0, 0, 9, 0],
    // local row 0, local column 0. Within a chunk the layout is
    // [time][parameter][row][column], 10 x 10 cells per parameter.
    let chunk: Vec<f32> = array
        .retrieve_chunk::<Vec<f16>>(&[0, 0, 9, 0])
        .expect("chunk")
        .into_iter()
        .map(f16::to_f32)
        .collect();
    assert_eq!(chunk.len(), 24 * 4 * 10 * 10);
    let (u, v, cu, cv) = (chunk[0], chunk[100], chunk[200], chunk[300]);
    assert!(
        (u - 20.0).abs() < 0.05,
        "20 m/s eastward at the stroke, got u={u}"
    );
    assert!(
        v.abs() < 0.05,
        "a due-east flow has no northward component, got v={v}"
    );
    assert!(
        cu.is_nan() && cv.is_nan(),
        "a wind project writes no current, only the mask"
    );
    // The second step holds the same still field, one frame stride on.
    assert!((chunk[400] - 20.0).abs() < 0.05, "step 1 u");
    // Beyond the project's two steps the chunk is padding: fill value only.
    assert!(chunk[800].is_nan(), "time padding is the fill value");

    // Away from the stroke the field is undefined, distinct from a painted calm.
    let far: Vec<f16> = array
        .retrieve_chunk::<Vec<f16>>(&[0, 0, 1, 18])
        .expect("far chunk");
    assert!(
        far.iter().all(|value| value.is_nan()),
        "should be undefined"
    );
}

/// A cancelled Zarr export leaves nothing behind, and an existing destination
/// is refused rather than overwritten.
#[test]
fn a_zarr_export_never_leaves_a_partial_store() {
    let root = TempRoot::new("zarr-cancel");
    let state = app(&root);
    projects::create(&state, new_project("wind"), false).expect("create");

    let project = {
        let mut session = state.session.lock().expect("lock");
        session.require_open().expect("open").project.clone()
    };
    let path = root.0.join("cancelled.zarr");
    let outcome = export::run_zarr(
        &project,
        &zarr_request(&path),
        &AtomicBool::new(true),
        |_| {},
    );
    assert!(matches!(outcome, Err(AppError::ExportCancelled)));
    assert!(!path.exists(), "no store");
    assert!(
        !root.0.join("cancelled.zarr.partial").exists(),
        "no temporary either"
    );
    assert!(!root.0.join("cancelled.zarr.spool").exists(), "no spool");

    std::fs::create_dir_all(&path).expect("an existing directory");
    let outcome = export::run_zarr(
        &project,
        &zarr_request(&path),
        &AtomicBool::new(false),
        |_| {},
    );
    assert!(matches!(outcome, Err(AppError::Doing { .. })), "refused");
    assert!(
        std::fs::read_dir(&path).expect("read").next().is_none(),
        "the existing directory is untouched"
    );

    let mut bad = zarr_request(&root.0.join("bad.zarr"));
    bad.month = 13;
    assert!(export::run_zarr(&project, &bad, &AtomicBool::new(false), |_| {}).is_err());
    assert!(!root.0.join("bad.zarr").exists());
}

/// A project longer than 72 hours spans two time chunks; the second must hold
/// its steps from its own start, not the array's.
#[test]
fn a_zarr_export_crosses_a_time_chunk_boundary() {
    use ve_zarr::export::f16;
    use zarrs::array::Array;
    use zarrs::filesystem::FilesystemStore;

    let root = TempRoot::new("zarr-chunks");
    let state = app(&root);
    // Daily steps: 72 hours is three steps, so five steps are two chunks.
    projects::create(
        &state,
        NewProjectRequest {
            step_hours: 24,
            step_count: 5,
            ..new_project("wind")
        },
        false,
    )
    .expect("create");
    edit::paint(
        &state,
        BrushStroke {
            points: vec![[0.0, 0.0]],
            size_km: 2000.0,
            speed_mps: 15.0,
            direction_toward_deg: 0.0,
            feather: 0.0,
            layer: None,
            ..Default::default()
        },
    )
    .expect("paint");

    let path = root.0.join("long.zarr");
    let project = {
        let mut session = state.session.lock().expect("lock");
        session.require_open().expect("open").project.clone()
    };
    let result = export::run_zarr(
        &project,
        &zarr_request(&path),
        &AtomicBool::new(false),
        |_| {},
    )
    .expect("export");
    assert_eq!(result.chunks, 36 * 19 * 2);

    let store = std::sync::Arc::new(FilesystemStore::new(&path).expect("store"));
    let array = Array::open(store, "/data").expect("open");
    assert_eq!(array.shape(), &[5, 4, 181, 360]);
    let second: Vec<f32> = array
        .retrieve_chunk::<Vec<f16>>(&[1, 0, 9, 0])
        .expect("chunk")
        .into_iter()
        .map(f16::to_f32)
        .collect();
    // Steps 3 and 4 are local 0 and 1 of the second chunk: 3 x 4 x 100 per
    // step, v is the second plane.
    assert!(
        (second[100] - 15.0).abs() < 0.05,
        "step 3 v, got {}",
        second[100]
    );
    assert!(
        (second[500] - 15.0).abs() < 0.05,
        "step 4 v, got {}",
        second[500]
    );
    assert!(second[900].is_nan(), "the third slot is padding");
    assert!(second[0].abs() < 0.05, "northward flow has no u");
}

/// The cost the export panel sees: a 0.25° project, two steps, evaluated
/// once per step. Printed, not asserted; `--nocapture` shows it.
#[test]
fn zarr_export_cost_at_quarter_degree() {
    let root = TempRoot::new("zarr-cost");
    let state = app(&root);
    projects::create(
        &state,
        NewProjectRequest {
            resolution: "0.25".to_owned(),
            ..new_project("wind")
        },
        false,
    )
    .expect("create");
    edit::paint(
        &state,
        BrushStroke {
            points: vec![[-20.0, 0.0], [20.0, 10.0]],
            size_km: 1500.0,
            speed_mps: 12.0,
            direction_toward_deg: 45.0,
            feather: 0.5,
            layer: None,
            ..Default::default()
        },
    )
    .expect("paint");
    let project = {
        let mut session = state.session.lock().expect("lock");
        session.require_open().expect("open").project.clone()
    };
    let path = root.0.join("cost.zarr");
    let result = export::run_zarr(
        &project,
        &zarr_request(&path),
        &AtomicBool::new(false),
        |_| {},
    )
    .expect("export");
    println!(
        "0.25 degree, 2 steps: {} chunks, {} bytes, {} ms",
        result.chunks, result.bytes, result.elapsed_ms
    );
}

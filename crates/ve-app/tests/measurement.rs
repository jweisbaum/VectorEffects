#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! M8's acceptance: the measurements measure, survive a save, and stay out of
//! the export (spec.md 10).

use ve_app::commands::AppState;
use ve_app::edit;
use ve_app::measure::{self, MeasurementKind, NewMeasurement};
use ve_app::paths::AppPaths;
use ve_app::projects::{self, NewProjectRequest};

struct TempRoot(std::path::PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ve-measure-{}-{label}-{}",
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

fn open(state: &AppState) {
    projects::create(
        state,
        NewProjectRequest {
            name: "Measure".to_owned(),
            field_kind: "wind".to_owned(),
            resolution: "1.0".to_owned(),
            step_hours: 3,
            step_count: 2,
        },
        true,
    )
    .expect("create");
}

fn dividers(points: Vec<[f64; 2]>) -> NewMeasurement {
    NewMeasurement {
        kind: MeasurementKind::Dividers,
        points,
        interval_km: 0.0,
        count: 0,
    }
}

/// A chain measures, extends, and totals what it measures.
#[test]
fn a_chain_measures_its_legs_and_extends() {
    let root = TempRoot::new("chain");
    let state = app(&root);
    open(&state);

    let views = measure::measurement_added(&state, dividers(vec![[0.0, 0.0], [10.0, 0.0]]))
        .expect("place a chain");
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].paths.len(), 1);
    assert!(views[0].total.is_some(), "a chain has a running total");
    // Ten degrees along the equator: 600 nm to the nearest mile, which is a
    // number a navigator can check without a computer.
    assert!(
        views[0].paths[0].label.starts_with("600 nm"),
        "{:?}",
        views[0].paths[0].label
    );
    // Due east, so the course is 090.
    assert!(
        views[0].paths[0].label.ends_with("090°"),
        "{:?}",
        views[0].paths[0].label
    );

    let id = views[0].id;
    let views = measure::measurement_extended(&state, id, [20.0, 0.0]).expect("extend");
    assert_eq!(views[0].paths.len(), 2, "a third point makes a second leg");
    assert_eq!(views[0].handles.len(), 3);

    // A passage cannot be extended: it has two ends, and a caller doing this
    // has confused itself about which measurement it holds.
    let passage = measure::measurement_added(
        &state,
        NewMeasurement {
            kind: MeasurementKind::Passage,
            points: vec![[0.0, 40.0], [30.0, 50.0]],
            interval_km: 0.0,
            count: 0,
        },
    )
    .expect("place a passage");
    let passage_id = passage[1].id;
    assert!(measure::measurement_extended(&state, passage_id, [1.0, 1.0]).is_err());
}

/// The acceptance case: the two paths of a passage visibly part company, and
/// the rhumb line is the longer one.
#[test]
fn a_passage_draws_both_paths_and_names_them() {
    let root = TempRoot::new("passage");
    let state = app(&root);
    open(&state);

    let views = measure::measurement_added(
        &state,
        NewMeasurement {
            kind: MeasurementKind::Passage,
            // New York to Amsterdam: far enough north and long enough that a
            // navigator would care which line was sailed.
            points: vec![[-73.78, 40.64], [4.9, 52.3]],
            interval_km: 0.0,
            count: 0,
        },
    )
    .expect("place a passage");

    let paths = &views[0].paths;
    assert_eq!(paths.len(), 2);
    assert!(paths[0].label.starts_with("GC "), "{:?}", paths[0].label);
    assert!(paths[1].label.starts_with("RL "), "{:?}", paths[1].label);
    assert_eq!(
        views[0].total, None,
        "two answers to one question do not add up"
    );

    // They are drawn as different curves, not as one line twice.
    let apart = paths[0]
        .points
        .iter()
        .zip(&paths[1].points)
        .map(|(a, b)| (a[1] - b[1]).abs())
        .fold(0.0_f64, f64::max);
    assert!(apart > 1.0, "the paths differ by {apart} degrees at most");
}

/// Rings are placed in kilometres, edited, and capped.
#[test]
fn rings_take_an_interval_and_a_count() {
    let root = TempRoot::new("rings");
    let state = app(&root);
    open(&state);

    let views = measure::measurement_added(
        &state,
        NewMeasurement {
            kind: MeasurementKind::Rings,
            points: vec![[-30.0, 45.0]],
            interval_km: 100.0,
            count: 3,
        },
    )
    .expect("place rings");
    assert_eq!(views[0].paths.len(), 3);
    assert!(
        views[0].paths[0].label.starts_with("54.0 nm"),
        "{:?}",
        views[0].paths[0].label
    );
    assert!(views[0].total.as_deref().unwrap().starts_with("outer "));

    let id = views[0].id;
    let views = measure::rings_set(&state, id, 250.0, 5).expect("edit rings");
    assert_eq!(views[0].paths.len(), 5);

    // An interval that would draw nothing is refused rather than accepted and
    // silently ignored.
    assert!(measure::rings_set(&state, id, 0.0, 3).is_err());
    assert!(measure::rings_set(&state, id, f64::NAN, 3).is_err());
}

/// Dragging a point is one history entry, and undo puts it back where the drag
/// began rather than one pointer report back.
#[test]
fn a_drag_is_one_undo() {
    let root = TempRoot::new("drag");
    let state = app(&root);
    open(&state);

    let views = measure::measurement_added(&state, dividers(vec![[0.0, 0.0], [10.0, 0.0]]))
        .expect("place a chain");
    let id = views[0].id;

    for lat in [1.0, 2.0, 3.0, 4.0] {
        measure::handle_moved(&state, id, 1, [10.0, lat]).expect("drag");
    }
    let dragged = measure::measurements_of(&state).expect("read");
    assert_eq!(dragged[0].handles[1], [10.0, 4.0]);

    edit::undo_for_test(&state).expect("undo");
    let back = measure::measurements_of(&state).expect("read");
    assert_eq!(
        back[0].handles[1],
        [10.0, 0.0],
        "one undo returns the whole drag"
    );

    edit::redo_for_test(&state).expect("redo");
    let again = measure::measurements_of(&state).expect("read");
    assert_eq!(again[0].handles[1], [10.0, 4.0]);
}

/// Per-tool clear takes that tool's measurements and leaves the others.
#[test]
fn clearing_is_per_tool_and_global() {
    let root = TempRoot::new("clear");
    let state = app(&root);
    open(&state);

    measure::measurement_added(&state, dividers(vec![[0.0, 0.0], [5.0, 0.0]])).expect("chain");
    measure::measurement_added(
        &state,
        NewMeasurement {
            kind: MeasurementKind::Passage,
            points: vec![[0.0, 10.0], [20.0, 20.0]],
            interval_km: 0.0,
            count: 0,
        },
    )
    .expect("passage");
    let rings = measure::measurement_added(
        &state,
        NewMeasurement {
            kind: MeasurementKind::Rings,
            points: vec![[40.0, 40.0]],
            interval_km: 50.0,
            count: 2,
        },
    )
    .expect("rings");
    assert_eq!(rings.len(), 3);

    let left = measure::measurements_cleared(&state, Some(MeasurementKind::Passage))
        .expect("clear passages");
    assert_eq!(left.len(), 2);
    assert!(left.iter().all(|m| m.kind != MeasurementKind::Passage));

    // Removing one by id leaves the rest.
    let id = left[0].id;
    let after = measure::measurement_removed(&state, id).expect("remove one");
    assert_eq!(after.len(), 1);

    let none = measure::measurements_cleared(&state, None).expect("clear all");
    assert!(none.is_empty());

    // And clearing is undoable like everything else.
    edit::undo_for_test(&state).expect("undo");
    assert_eq!(measure::measurements_of(&state).expect("read").len(), 1);
}

/// Measurements survive a save and reopen — the point of storing them at all.
#[test]
fn measurements_survive_a_round_trip() {
    let root = TempRoot::new("roundtrip");
    let state = app(&root);
    open(&state);

    measure::measurement_added(&state, dividers(vec![[-73.78, 40.64], [4.9, 52.3]]))
        .expect("chain");
    measure::measurement_added(
        &state,
        NewMeasurement {
            kind: MeasurementKind::Rings,
            points: vec![[10.0, -20.0]],
            interval_km: 125.5,
            count: 4,
        },
    )
    .expect("rings");
    let before = measure::measurements_of(&state).expect("read");

    let path = root.0.join("measured.veproj");
    let file = path.to_string_lossy().into_owned();
    projects::save_as(&state, file.clone()).expect("save");
    projects::close_open(&state, true).expect("close");
    projects::open(&state, file, true).expect("reopen");

    let after = measure::measurements_of(&state).expect("read");
    assert_eq!(after.len(), before.len());
    for (a, b) in after.iter().zip(&before) {
        assert_eq!(a.kind, b.kind);
        // To the file's own precision, not bit for bit: a coordinate is
        // quantised to nine decimal places on the way out and on the way back
        // (`canonical`), so a value that reached memory through a normalising
        // subtraction comes back *cleaner* than it left. Nine places is a tenth
        // of a millimetre.
        for (p, q) in a.handles.iter().zip(&b.handles) {
            assert!((p[0] - q[0]).abs() < 1e-9, "{p:?} != {q:?}");
            assert!((p[1] - q[1]).abs() < 1e-9, "{p:?} != {q:?}");
        }
        // The labels are the measurements: if the numbers came back the same,
        // so did everything they were computed from.
        assert_eq!(
            a.paths.iter().map(|p| p.label.clone()).collect::<Vec<_>>(),
            b.paths.iter().map(|p| p.label.clone()).collect::<Vec<_>>(),
        );
    }
}

/// Measurements never reach an export.
///
/// Asserted on the bytes, not on the structure: the same project exports
/// identically with a chain, a passage and four range rings laid over it as it
/// does with none. That is invariant 4 doing the work — the export is
/// deterministic — so a byte for byte match is a complete statement that the
/// measurements were not consulted, and it stays true if someone later gives
/// `FlatScene` a field it should not have.
#[test]
fn measurements_never_reach_an_export() {
    use std::sync::atomic::AtomicBool;
    use ve_app::edit::BrushStroke;
    use ve_app::export::{self, ExportRequest};

    let root = TempRoot::new("export");
    let state = app(&root);
    open(&state);

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
    .expect("paint");

    let export_now = |name: &str| {
        let path = root.0.join(name);
        let project = {
            let mut session = state.session.lock().expect("lock");
            session.require_open().expect("open").project.clone()
        };
        export::run(
            &project,
            &ExportRequest {
                path: path.to_string_lossy().into_owned(),
                year: 2026,
                month: 9,
                day: 2,
                hour: 0,
                centre: 255,
                bits: 16,
            },
            &AtomicBool::new(false),
            |_| {},
        )
        .expect("export");
        std::fs::read(&path).expect("read")
    };

    let clean = export_now("clean.grib2");

    measure::measurement_added(&state, dividers(vec![[-20.0, 0.0], [20.0, 0.0]])).expect("chain");
    measure::measurement_added(
        &state,
        NewMeasurement {
            kind: MeasurementKind::Passage,
            points: vec![[-73.78, 40.64], [4.9, 52.3]],
            interval_km: 0.0,
            count: 0,
        },
    )
    .expect("passage");
    measure::measurement_added(
        &state,
        NewMeasurement {
            kind: MeasurementKind::Rings,
            // Centred on the painted stroke, so a measurement that did leak
            // into the field would leak into the values being compared.
            points: vec![[0.0, 0.0]],
            interval_km: 500.0,
            count: 4,
        },
    )
    .expect("rings");
    assert_eq!(measure::measurements_of(&state).expect("read").len(), 3);

    let measured = export_now("measured.grib2");
    assert_eq!(
        clean, measured,
        "a measurement changed the exported bytes, which it must never do"
    );
}

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! M10: crash recovery. A dirty project is snapshotted, an unchanged one is
//! not written again, a snapshot comes back as the project it was taken from,
//! and a clean save or a deliberate close forgets it (spec.md 4.2).

use ve_app::autosave;
use ve_app::commands::AppState;
use ve_app::edit::{self, BrushStroke};
use ve_app::paths::AppPaths;
use ve_app::projects::{self, NewProjectRequest};

struct TempRoot(std::path::PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ve-autosave-{}-{label}-{}",
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

fn create(state: &AppState) {
    projects::create(
        state,
        NewProjectRequest {
            name: "Recoverable".to_owned(),
            field_kind: "wind".to_owned(),
            resolution: "1.0".to_owned(),
            step_hours: 3,
            step_count: 4,
        },
        true,
    )
    .expect("create");
}

fn paint(state: &AppState, lon: f64) {
    edit::paint(
        state,
        BrushStroke {
            points: vec![[lon, 0.0], [lon + 5.0, 0.0]],
            size_km: 600.0,
            speed_mps: 12.0,
            direction_toward_deg: 90.0,
            ..Default::default()
        },
    )
    .expect("paint");
}

/// A dirty project is snapshotted; an unchanged one is not written twice.
#[test]
fn a_dirty_project_is_snapshotted_once_per_change() {
    let root = TempRoot::new("snapshot");
    let state = app(&root);
    create(&state);
    // A new project is dirty from the start (it exists only in memory), so
    // the very first tick writes.
    assert!(
        autosave::snapshot(&state, true).expect("snapshot"),
        "first tick writes"
    );
    assert_eq!(autosave::list(&state).len(), 1);
    assert!(
        !autosave::snapshot(&state, true).expect("snapshot"),
        "an unchanged project is not written again, even when forced"
    );
    paint(&state, 0.0);
    assert!(
        !autosave::snapshot(&state, false).expect("snapshot"),
        "one edit, seconds later: neither the minute nor fifty entries has passed"
    );
    assert!(
        autosave::snapshot(&state, true).expect("snapshot"),
        "forced, an edit is written"
    );
    assert_eq!(
        autosave::list(&state).len(),
        1,
        "one snapshot per project, replaced"
    );
}

/// Spec 4.2: fifty history entries trigger a snapshot before the minute does.
#[test]
fn fifty_entries_trigger_a_snapshot_before_the_minute() {
    let root = TempRoot::new("burst");
    let state = app(&root);
    create(&state);
    assert!(autosave::snapshot(&state, true).expect("snapshot"));
    for i in 0..49 {
        paint(&state, f64::from(i) - 90.0);
        // Strokes that touch merge into one entry; spread them so they do not.
    }
    assert!(
        !autosave::snapshot(&state, false).expect("snapshot"),
        "forty-nine entries is not fifty"
    );
    paint(&state, 120.0);
    assert!(
        autosave::snapshot(&state, false).expect("snapshot"),
        "the fiftieth entry is a snapshot, minute or no minute"
    );
}

/// With nothing open, or nothing dirty, nothing is written.
#[test]
fn a_clean_or_absent_project_writes_nothing() {
    let root = TempRoot::new("clean");
    let state = app(&root);
    assert!(
        !autosave::snapshot(&state, true).expect("snapshot"),
        "nothing open"
    );
    create(&state);
    let path = root.0.join("saved.veproj").to_string_lossy().into_owned();
    projects::save_as(&state, path).expect("save");
    assert!(
        !autosave::snapshot(&state, true).expect("snapshot"),
        "just saved: not dirty"
    );
    assert!(autosave::list(&state).is_empty());
}

/// The acceptance case: the process dies mid-edit, and the work comes back.
///
/// "Dies" here is a second `AppState` over the same directories — a fresh
/// process sees the same disk — with everything the first one held in memory
/// gone. The recovered project has the painted stroke, is dirty, and carries
/// the path it was last saved to so the next Save goes where it should.
#[test]
fn a_snapshot_comes_back_as_the_project_it_was_taken_from() {
    let root = TempRoot::new("recover");
    let original = root.0.join("mine.veproj");
    let id = {
        let state = app(&root);
        create(&state);
        projects::save_as(&state, original.to_string_lossy().into_owned()).expect("save");
        paint(&state, 20.0); // unsaved work
        assert!(autosave::snapshot(&state, true).expect("snapshot"));
        autosave::list(&state)[0].id
        // The process "dies" here: the state is dropped without a save.
    };

    let state = app(&root);
    let offered = autosave::list(&state);
    assert_eq!(offered.len(), 1, "the snapshot survived the process");
    assert_eq!(offered[0].id, id);
    assert_eq!(offered[0].name, "Recoverable");
    assert_eq!(
        offered[0].original_path.as_deref(),
        Some(original.to_string_lossy().as_ref())
    );

    let summary = autosave::recover(&state, id, false).expect("recover");
    assert!(summary.dirty, "recovered work is unsaved work");
    assert_eq!(summary.object_count, 1, "the painted stroke is there");
    assert_eq!(
        summary.path.as_deref(),
        Some(original.to_string_lossy().as_ref()),
        "Save will write where the project lived"
    );

    // Saving cleanly forgets the snapshot: nothing is left to offer back.
    projects::save(&state).expect("save");
    assert!(
        autosave::list(&state).is_empty(),
        "a clean save forgets the snapshot"
    );
}

/// A project that was never saved comes back with no path, so Save asks.
#[test]
fn a_never_saved_project_recovers_without_a_path() {
    let root = TempRoot::new("unsaved");
    let id = {
        let state = app(&root);
        create(&state);
        paint(&state, -30.0);
        assert!(autosave::snapshot(&state, true).expect("snapshot"));
        autosave::list(&state)[0].id
    };
    let state = app(&root);
    let summary = autosave::recover(&state, id, false).expect("recover");
    assert!(summary.dirty);
    assert_eq!(summary.path, None, "never saved: Save must ask where");
    assert_eq!(summary.object_count, 1);
}

/// A deliberate close is not a crash: the snapshot goes with the project.
#[test]
fn closing_on_purpose_forgets_the_snapshot() {
    let root = TempRoot::new("close");
    let state = app(&root);
    create(&state);
    assert!(autosave::snapshot(&state, true).expect("snapshot"));
    projects::close_open(&state, true).expect("close, discarding");
    assert!(
        autosave::list(&state).is_empty(),
        "what the user chose to drop is not offered back"
    );
}

/// Recovering refuses to drop unsaved work in the project already open.
#[test]
fn recovering_will_not_silently_discard_an_open_dirty_project() {
    let root = TempRoot::new("guard");
    let id = {
        let state = app(&root);
        create(&state);
        assert!(autosave::snapshot(&state, true).expect("snapshot"));
        autosave::list(&state)[0].id
    };
    let state = app(&root);
    create(&state); // dirty, unsaved
    assert!(autosave::recover(&state, id, false).is_err());
    assert!(autosave::recover(&state, id, true).is_ok());
}

/// Autosave is a setting (D70, M25): `save` writes the project file itself in
/// place when it has one, on the same cadence, and leaves nothing to recover.
#[test]
fn save_mode_writes_the_project_in_place() {
    let root = TempRoot::new("save-mode");
    let state = app(&root);
    create(&state);
    let path = root.0.join("kept.veproj");
    projects::save_as(&state, path.to_string_lossy().into_owned()).expect("save as");
    let before = std::fs::metadata(&path).expect("saved").len();
    ve_app::settings::autosave_mode_set(&state, ve_app::settings::AutosaveMode::Save)
        .expect("mode");

    paint(&state, 0.0);
    assert!(
        projects::current(&state)
            .expect("current")
            .expect("open")
            .dirty,
        "an edit dirties the project"
    );
    assert!(
        autosave::snapshot(&state, true).expect("tick"),
        "the tick writes"
    );
    let summary = projects::current(&state).expect("current").expect("open");
    assert!(!summary.dirty, "written in place, so the project is clean");
    assert_ne!(
        std::fs::metadata(&path).expect("saved").len(),
        before,
        "the file at the project's own path changed"
    );
    assert!(
        autosave::list(&state).is_empty(),
        "nothing to recover: the file is the save"
    );
    assert!(
        !autosave::snapshot(&state, true).expect("tick"),
        "and an unchanged project is not written again"
    );
}

/// A project that has never been saved has nowhere to be written in place,
/// so `save` mode takes a recovery snapshot for it like everyone else.
#[test]
fn save_mode_snapshots_a_project_with_no_path() {
    let root = TempRoot::new("save-mode-no-path");
    let state = app(&root);
    create(&state);
    ve_app::settings::autosave_mode_set(&state, ve_app::settings::AutosaveMode::Save)
        .expect("mode");
    assert!(autosave::snapshot(&state, true).expect("tick"));
    assert_eq!(autosave::list(&state).len(), 1);
}

/// `off` writes nothing, however dirty the project is.
#[test]
fn off_writes_nothing() {
    let root = TempRoot::new("off");
    let state = app(&root);
    create(&state);
    ve_app::settings::autosave_mode_set(&state, ve_app::settings::AutosaveMode::Off).expect("mode");
    for lon in 0..60 {
        paint(&state, f64::from(lon) * 2.0 - 60.0);
    }
    assert!(!autosave::snapshot(&state, true).expect("tick"));
    assert!(autosave::list(&state).is_empty());
}

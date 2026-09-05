#![allow(
    clippy::expect_used,
    reason = "test setup: a panic naming the failed step is the right outcome, and clippy's allow-expect-in-tests does not reach helpers in tests/"
)]

//! End-to-end project lifecycle: create, save, close, reopen.
//!
//! Drives the same functions the Tauri commands call, so the flow is covered
//! without a webview. What matters here is that a project survives the whole
//! round trip and that the recent-files list tracks it.

use std::path::PathBuf;

use ve_app::commands::AppState;
use ve_app::error::AppError;
use ve_app::paths::AppPaths;
use ve_app::projects::{self, NewProjectRequest};

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ve-lifecycle-{}-{}-{label}",
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

fn state(root: &TempRoot) -> AppState {
    AppState::new(AppPaths::in_directory(&root.0).expect("paths"))
}

fn request(name: &str) -> NewProjectRequest {
    NewProjectRequest {
        name: name.to_owned(),
        field_kind: "wind".to_owned(),
        resolution: "0.25".to_owned(),
        step_hours: 3,
        step_count: 24,
    }
}

#[test]
fn a_project_survives_create_save_close_and_reopen() {
    let root = TempRoot::new("round-trip");
    let app = state(&root);
    let path = root.0.join("demo.veproj");

    let created = projects::create(&app, request("Demo"), false).expect("create");
    assert_eq!(created.name, "Demo");
    assert!(created.dirty, "a new project has never been saved");
    assert!(created.path.is_none());
    assert_eq!((created.grid_ni, created.grid_nj), (1440, 721));

    let saved = projects::save_as(&app, path.to_string_lossy().into_owned()).expect("save as");
    assert!(!saved.dirty, "saving clears the dirty flag");
    assert_eq!(saved.path.as_deref(), Some(path.to_string_lossy().as_ref()));
    assert!(path.exists(), "the file must actually be on disk");

    // Closing is guarded like every other path that replaces the open
    // project: the backend knows whether the document is dirty, so it is what
    // refuses. A frontend that forgot to ask still cannot drop unsaved work.
    projects::close_open(&app, true).expect("close");
    assert!(projects::current(&app).expect("current").is_none());

    let reopened = projects::open(&app, path.to_string_lossy().into_owned(), false).expect("open");
    assert_eq!(reopened.name, "Demo");
    assert!(!reopened.dirty);
    assert_eq!(reopened.step_count, 24);
    assert_eq!(reopened.resolution_label, "0.25°");
    assert_eq!(reopened.field_kind, "wind");
    assert_eq!(reopened.direction_convention, "from");
}

/// Save As must add the extension when the user omits it, or the file cannot be
/// found again by the same filter that saved it.
#[test]
fn save_as_supplies_the_extension() {
    let root = TempRoot::new("extension");
    let app = state(&root);

    projects::create(&app, request("NoExt"), false).expect("create");
    let bare = root.0.join("noext");
    let saved = projects::save_as(&app, bare.to_string_lossy().into_owned()).expect("save as");

    assert!(
        saved
            .path
            .as_deref()
            .unwrap_or_default()
            .ends_with(".veproj"),
        "got {:?}",
        saved.path
    );
    assert!(bare.with_extension("veproj").exists());
}

#[test]
fn saving_twice_writes_to_the_same_place() {
    let root = TempRoot::new("resave");
    let app = state(&root);
    let path = root.0.join("again.veproj");

    projects::create(&app, request("Again"), false).expect("create");
    projects::save_as(&app, path.to_string_lossy().into_owned()).expect("save as");
    let saved = projects::save(&app).expect("save");

    assert_eq!(saved.path.as_deref(), Some(path.to_string_lossy().as_ref()));
    assert!(!saved.dirty);
}

#[test]
fn saving_a_never_saved_project_asks_for_a_location() {
    let root = TempRoot::new("never-saved");
    let app = state(&root);

    projects::create(&app, request("Fresh"), false).expect("create");
    assert!(matches!(
        projects::save(&app),
        Err(AppError::ProjectNeverSaved)
    ));
}

#[test]
fn operations_without_a_project_report_it_rather_than_panicking() {
    let root = TempRoot::new("no-project");
    let app = state(&root);

    assert!(matches!(projects::save(&app), Err(AppError::NoProjectOpen)));
    assert!(matches!(
        projects::save_as(&app, "/tmp/x.veproj".to_owned()),
        Err(AppError::NoProjectOpen)
    ));
    assert!(projects::current(&app).expect("current").is_none());
}

/// The recent list is the only navigation aid on the start screen, so it has to
/// survive a restart.
#[test]
fn recent_projects_persist_across_sessions() {
    let root = TempRoot::new("recent");
    let first = root.0.join("one.veproj");
    let second = root.0.join("two.veproj");

    {
        let app = state(&root);
        projects::create(&app, request("One"), false).expect("create");
        projects::save_as(&app, first.to_string_lossy().into_owned()).expect("save");
        projects::create(&app, request("Two"), false).expect("create");
        projects::save_as(&app, second.to_string_lossy().into_owned()).expect("save");
    }

    // A fresh state, as if the application had been restarted.
    let restarted = state(&root);
    let recent = projects::recent(&restarted).expect("recent");

    assert_eq!(recent.len(), 2, "both saves should be remembered");
    assert_eq!(recent[0].path, second.to_string_lossy());
    assert_eq!(recent[0].name, "two");
    assert!(recent.iter().all(|entry| entry.exists));
}

/// A recent entry whose file has been moved or deleted must be reported as
/// missing rather than offered as if it would open.
#[test]
fn a_deleted_recent_entry_is_marked_missing() {
    let root = TempRoot::new("missing");
    let path = root.0.join("gone.veproj");

    let app = state(&root);
    projects::create(&app, request("Gone"), false).expect("create");
    projects::save_as(&app, path.to_string_lossy().into_owned()).expect("save");
    std::fs::remove_file(&path).expect("remove");

    let recent = projects::recent(&app).expect("recent");
    assert_eq!(recent.len(), 1);
    assert!(!recent[0].exists);
}

#[test]
fn opening_a_file_that_is_not_a_project_fails_cleanly() {
    let root = TempRoot::new("junk");
    let app = state(&root);
    let path = root.0.join("junk.veproj");
    std::fs::write(&path, b"not a project").expect("write");

    assert!(projects::open(&app, path.to_string_lossy().into_owned(), false).is_err());
    assert!(
        projects::current(&app).expect("current").is_none(),
        "a failed open must not leave a half-open project"
    );
}

/// A current project must not open showing wind's "from" convention.
#[test]
fn a_current_project_uses_the_oceanographic_convention() {
    let root = TempRoot::new("current");
    let app = state(&root);

    let mut req = request("Gulf Stream");
    req.field_kind = "current".to_owned();
    let created = projects::create(&app, req, false).expect("create");

    assert_eq!(created.field_kind, "current");
    assert_eq!(created.direction_convention, "toward");
}

/// The prompt that offers to save lives in the frontend. The refusal lives in
/// the backend, so no caller can drop unsaved work by forgetting to ask.
#[test]
fn a_new_project_will_not_silently_discard_unsaved_changes() {
    let root = TempRoot::new("guard-new");
    let app = state(&root);

    projects::create(&app, request("First"), false).expect("create");
    let err = projects::create(&app, request("Second"), false);
    assert!(
        matches!(&err, Err(AppError::UnsavedChanges { name }) if name == "First"),
        "expected a refusal naming the project, got {err:?}"
    );
    assert_eq!(
        projects::current(&app)
            .expect("current")
            .expect("open")
            .name,
        "First",
        "and the open project is untouched"
    );
}

#[test]
fn opening_a_project_will_not_silently_discard_unsaved_changes() {
    let root = TempRoot::new("guard-open");
    let app = state(&root);
    let path = root.0.join("saved.veproj");

    projects::create(&app, request("Saved"), false).expect("create");
    projects::save_as(&app, path.to_string_lossy().into_owned()).expect("save as");
    projects::close_open(&app, true).expect("close");

    projects::create(&app, request("Unsaved"), false).expect("create");
    let err = projects::open(&app, path.to_string_lossy().into_owned(), false);
    assert!(
        matches!(err, Err(AppError::UnsavedChanges { .. })),
        "got {err:?}"
    );
}

/// A project with nothing to lose is replaced without ceremony — the guard is
/// about unsaved work, not about making the user close things.
#[test]
fn a_saved_project_is_replaced_without_a_refusal() {
    let root = TempRoot::new("guard-clean");
    let app = state(&root);
    let path = root.0.join("clean.veproj");

    projects::create(&app, request("Clean"), false).expect("create");
    projects::save_as(&app, path.to_string_lossy().into_owned()).expect("save as");

    let next = projects::create(&app, request("Next"), false).expect("create over a saved project");
    assert_eq!(next.name, "Next");
}

/// "Don't save" has to reach the command that replaces the project, or the
/// guard turns a deliberate choice into a dead end.
#[test]
fn an_explicit_discard_replaces_the_project() {
    let root = TempRoot::new("discard");
    let app = state(&root);

    projects::create(&app, request("Doomed"), false).expect("create");
    let next = projects::create(&app, request("Replacement"), true).expect("discard and create");
    assert_eq!(next.name, "Replacement");
}

/// Closing a dirty project is refused unless the caller says to discard.
///
/// The guard is the backend's, not the dialog's: the frontend asks first and
/// passes the answer, and a frontend that forgot to ask must still not be able
/// to drop somebody's work.
#[test]
fn closing_a_dirty_project_is_refused_without_a_decision() {
    let root = TempRoot::new("close-dirty");
    let app = state(&root);
    projects::create(&app, request("Dirty"), false).expect("create");
    ve_app::edit::paint(
        &app,
        ve_app::edit::BrushStroke {
            points: vec![[0.0, 0.0], [5.0, 0.0]],
            size_km: 500.0,
            speed_mps: 10.0,
            direction_toward_deg: 90.0,
            ..Default::default()
        },
    )
    .expect("paint");

    assert!(
        projects::close_open(&app, false).is_err(),
        "a dirty project must not close silently"
    );
    assert!(
        projects::current(&app).expect("current").is_some(),
        "and it must still be open afterwards"
    );

    projects::close_open(&app, true).expect("close with a decision");
    assert!(projects::current(&app).expect("current").is_none());
}

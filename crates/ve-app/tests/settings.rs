#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test setup: a panic naming the failed step is the right outcome, and clippy's allow-expect-in-tests does not reach helpers in tests/"
)]
//! Application settings, end to end (spec.md 8.6, M15).
//!
//! What the milestone asks for: a rebound key sticks and survives a restart, a
//! collision is refused, a broken file falls back to defaults, and a
//! colour-scale edit changes the map **without re-rendering a tile**.

use std::path::PathBuf;

use ve_app::commands::AppState;
use ve_app::paths::AppPaths;
use ve_app::projects::{self, NewProjectRequest};
use ve_app::settings::{self, Shortcut, ShortcutAction};

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ve-settings-{}-{label}-{}",
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

fn with_project(state: &AppState, kind: &str) {
    // `true`: replacing an unsaved project is refused unless the caller says
    // otherwise, and every one of these is a scratch project.
    projects::create(
        state,
        NewProjectRequest {
            name: "Settings".to_owned(),
            field_kind: kind.to_owned(),
            resolution: "1.0".to_owned(),
            step_hours: 3,
            step_count: 4,
        },
        true,
    )
    .expect("create");
}

fn tool(name: &str, key: &str) -> Shortcut {
    Shortcut {
        action: ShortcutAction::Tool,
        tool: name.to_owned(),
        key: key.to_owned(),
        shift: false,
    }
}

/// A rebound key sticks, is refused when it collides, and survives a restart.
#[test]
fn a_rebound_key_survives_a_restart_and_a_collision_is_refused() {
    let root = TempRoot::new("rebind");
    let state = app(&root);

    let after = settings::shortcut_set(&state, tool("brush", "q")).expect("q is free");
    assert_eq!(
        after
            .binding(ShortcutAction::Tool, "brush")
            .map(Shortcut::chord),
        Some("q".to_owned())
    );

    // `c` is the circle's: giving it to the brush is refused, and says whose.
    let refused = settings::shortcut_set(&state, tool("brush", "c")).expect_err("a collision");
    assert!(format!("{refused}").contains("circle"), "{refused}");
    // ...and the refusal changed nothing.
    let now = settings::settings_of(&state).expect("settings");
    assert_eq!(
        now.binding(ShortcutAction::Tool, "brush")
            .map(Shortcut::chord),
        Some("q".to_owned())
    );

    // A fresh app over the same directory reads it back.
    let restarted = app(&root);
    let loaded = settings::settings_of(&restarted).expect("settings");
    assert_eq!(
        loaded
            .binding(ShortcutAction::Tool, "brush")
            .map(Shortcut::chord),
        Some("q".to_owned()),
        "the rebinding did not survive a restart"
    );

    // And reset puts it back.
    let reset = settings::shortcuts_reset(&restarted).expect("reset");
    assert_eq!(
        reset
            .binding(ShortcutAction::Tool, "brush")
            .map(Shortcut::chord),
        Some("p".to_owned())
    );
}

/// A settings file that will not parse costs the preferences and not the
/// launch.
#[test]
fn a_broken_settings_file_falls_back_to_defaults() {
    let root = TempRoot::new("broken");
    let paths = AppPaths::in_directory(&root.0).expect("paths");
    let file = paths.settings_file();
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).expect("config dir");
    }
    std::fs::write(&file, "{ this is not json").expect("write");

    let state = app(&root);
    let loaded = settings::settings_of(&state).expect("settings");
    assert_eq!(
        loaded
            .binding(ShortcutAction::Tool, "brush")
            .map(Shortcut::chord),
        Some("p".to_owned()),
        "a broken file should read as the defaults"
    );
}

/// The colour scale is the project's, is undoable, and — the point of the
/// acceptance — **costs no tile**: tiles carry speed, not colour, so the
/// render cache is untouched by a scale change.
#[test]
fn a_colour_scale_edit_changes_the_project_without_a_render() {
    let root = TempRoot::new("scale");
    let state = app(&root);
    with_project(&state, "wind");

    let before = projects::current(&state).expect("summary").expect("open");
    assert!(
        (before.colour_scale_knots - 60.0).abs() < 1e-9,
        "wind default"
    );

    // What the map is drawn from is the scene, and a scale is not in it: the
    // flattened scene before and after must hash alike, which is exactly what
    // the render cache keys on (spec.md 7.10).
    let key_of = |state: &AppState| {
        let session = state.session.lock().expect("lock");
        let project = &session.open.as_ref().expect("open").project;
        ve_render::cache::scene_hash(&ve_render::scene::flatten(project, 0))
    };
    let key_before = key_of(&state);

    let after = settings::colour_scale_set(&state, 25.0).expect("set");
    assert!((after.colour_scale_knots - 25.0).abs() < 1e-9);
    assert!(after.can_undo, "a scale change is a document edit");
    assert_eq!(
        key_of(&state),
        key_before,
        "a colour scale must not change what a tile holds"
    );

    ve_app::edit::undo_for_test(&state).expect("undo");
    let undone = projects::current(&state).expect("summary").expect("open");
    assert!((undone.colour_scale_knots - 60.0).abs() < 1e-9);
}

/// The preference is the default for *new* projects; the project keeps its own.
#[test]
fn the_preference_is_the_default_for_new_projects_only() {
    let root = TempRoot::new("default");
    let state = app(&root);
    settings::default_scales_set(&state, 35.0, 4.0).expect("defaults");

    with_project(&state, "wind");
    let wind = projects::current(&state).expect("summary").expect("open");
    assert!((wind.colour_scale_knots - 35.0).abs() < 1e-9);

    // A current project takes the other default, since currents run an order
    // of magnitude slower.
    with_project(&state, "current");
    let current = projects::current(&state).expect("summary").expect("open");
    assert!((current.colour_scale_knots - 4.0).abs() < 1e-9);

    // Changing the preference afterwards leaves the open project alone.
    settings::default_scales_set(&state, 90.0, 9.0).expect("defaults");
    let unchanged = projects::current(&state).expect("summary").expect("open");
    assert!((unchanged.colour_scale_knots - 4.0).abs() < 1e-9);
}

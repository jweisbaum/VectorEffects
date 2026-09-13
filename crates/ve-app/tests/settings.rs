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
        alt: false,
        accel: false,
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
        (before.wind_scale_knots - 60.0).abs() < 1e-9,
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

    let after = settings::colour_scale_set(&state, "wind", 25.0).expect("set");
    assert!((after.wind_scale_knots - 25.0).abs() < 1e-9);
    assert!(after.can_undo, "a scale change is a document edit");
    assert_eq!(
        key_of(&state),
        key_before,
        "a colour scale must not change what a tile holds"
    );

    ve_app::edit::undo_for_test(&state).expect("undo");
    let undone = projects::current(&state).expect("summary").expect("open");
    assert!((undone.wind_scale_knots - 60.0).abs() < 1e-9);
}

/// The preference is the default for *new* projects; the project keeps its own.
#[test]
fn the_preference_is_the_default_for_new_projects_only() {
    let root = TempRoot::new("default");
    let state = app(&root);
    settings::default_scales_set(&state, 35.0, 4.0).expect("defaults");

    with_project(&state, "wind");
    let wind = projects::current(&state).expect("summary").expect("open");
    assert!((wind.wind_scale_knots - 35.0).abs() < 1e-9);

    // Every project takes both defaults (M29): a project may hold wind and
    // current layers together, and currents run an order of magnitude
    // slower, so each kind has a scale of its own.
    assert!((wind.current_scale_knots - 4.0).abs() < 1e-9);
    with_project(&state, "current");
    let current = projects::current(&state).expect("summary").expect("open");
    assert!((current.current_scale_knots - 4.0).abs() < 1e-9);
    assert!((current.wind_scale_knots - 35.0).abs() < 1e-9);

    // Changing the preference afterwards leaves the open project alone.
    settings::default_scales_set(&state, 90.0, 9.0).expect("defaults");
    let unchanged = projects::current(&state).expect("summary").expect("open");
    assert!((unchanged.wind_scale_knots - 35.0).abs() < 1e-9);
    assert!((unchanged.current_scale_knots - 4.0).abs() < 1e-9);
}

#[test]
fn display_units_persist_without_changing_the_open_project() {
    use settings::{DistanceUnit, SpeedUnit};
    let root = TempRoot::new("display-units");
    let state = app(&root);
    with_project(&state, "wind");
    let before = projects::current(&state).expect("summary").expect("open");
    for speed in [SpeedUnit::Kt, SpeedUnit::Mph, SpeedUnit::Kmh] {
        let saved = settings::display_units_set(&state, DistanceUnit::Nm, speed).expect("units");
        assert_eq!(saved.speed_unit, speed);
        assert_eq!(saved.distance_unit, DistanceUnit::Nm);
        let restarted = app(&root);
        let session = restarted.session.lock().expect("settings");
        assert_eq!(session.settings.speed_unit, speed);
        assert_eq!(session.settings.distance_unit, DistanceUnit::Nm);
    }
    let after = projects::current(&state).expect("summary").expect("open");
    assert_eq!(before.revision, after.revision);
    assert_eq!(before.wind_scale_knots, after.wind_scale_knots);
    assert_eq!(before.dirty, after.dirty);
    let older: settings::AppSettings = serde_json::from_str("{}").expect("old preferences");
    assert_eq!(older.distance_unit, DistanceUnit::Km);
    assert_eq!(older.speed_unit, SpeedUnit::Kt);
}

#[test]
fn glyph_appearance_is_independent_persistent_and_does_not_change_the_field() {
    use settings::{GlyphAppearance, GlyphSetting, GlyphStyle};
    let root = TempRoot::new("glyph-appearance");
    let state = app(&root);
    with_project(&state, "wind");
    let before = projects::current(&state).expect("summary").expect("open");
    for setting in [
        GlyphSetting::SizePercent(160),
        GlyphSetting::StrokeWidthPx(2.5),
        GlyphSetting::Color("#12ABef".to_owned()),
        GlyphSetting::OpacityPercent(45),
        GlyphSetting::DensityPercent(200),
        GlyphSetting::FadeWithSpeed(false),
        GlyphSetting::ShadowEnabled(true),
        GlyphSetting::ShadowColor("#123456".to_owned()),
        GlyphSetting::ShadowOpacityPercent(75),
        GlyphSetting::ShadowOffsetXPx(-3.5),
        GlyphSetting::ShadowOffsetYPx(4.5),
    ] {
        settings::glyph_appearance_set(&state, GlyphStyle::Barb, setting).expect("appearance");
    }
    settings::glyph_appearance_set(
        &state,
        GlyphStyle::Arrow,
        GlyphSetting::Color("#ff0000".to_owned()),
    )
    .expect("arrow colour");
    let saved = settings::settings_of(&state).expect("settings");
    assert_eq!(saved.glyphs.barb.size_percent, 160);
    assert_eq!(saved.glyphs.barb.density_percent, 200);
    assert_eq!(saved.glyphs.barb.color, "#12abef");
    assert_eq!(saved.glyphs.barb.shadow.offset_x_px, -3.5);
    assert_eq!(saved.glyphs.arrow.size_percent, 100);
    assert_eq!(saved.glyphs.arrow.color, "#ff0000");
    let loaded = settings::settings_of(&app(&root)).expect("restarted");
    assert_eq!(loaded.glyphs, saved.glyphs);
    let after = projects::current(&state).expect("summary").expect("open");
    assert_eq!(before.revision, after.revision);
    assert_eq!(before.dirty, after.dirty);
    let reset = settings::glyph_appearance_set(&state, GlyphStyle::Barb, GlyphSetting::Reset)
        .expect("reset");
    assert_eq!(reset.glyphs.barb, GlyphAppearance::default());
    assert_eq!(reset.glyphs.arrow, saved.glyphs.arrow);
}

#[test]
fn invalid_glyph_edits_are_atomic_and_old_preferences_gain_defaults() {
    use settings::{AppSettings, GlyphAppearance, GlyphSetting, GlyphStyle};
    let root = TempRoot::new("glyph-invalid");
    let state = app(&root);
    let before = settings::settings_of(&state).expect("settings");
    for invalid in [
        GlyphSetting::SizePercent(0),
        GlyphSetting::DensityPercent(301),
        GlyphSetting::OpacityPercent(101),
        GlyphSetting::Color("red".to_owned()),
        GlyphSetting::StrokeWidthPx(f32::NAN),
        GlyphSetting::ShadowOffsetXPx(f32::INFINITY),
        GlyphSetting::ShadowOffsetYPx(-13.0),
        GlyphSetting::ShadowOpacityPercent(101),
        GlyphSetting::ShadowColor("#gg0000".to_owned()),
    ] {
        assert!(settings::glyph_appearance_set(&state, GlyphStyle::Arrow, invalid).is_err());
        assert_eq!(settings::settings_of(&state).expect("unchanged"), before);
    }
    let older: AppSettings = serde_json::from_str("{}").expect("old preferences");
    assert_eq!(older.glyphs.arrow, GlyphAppearance::default());
    let partial: AppSettings = serde_json::from_str(r##"{"glyphs":{"barb":{"color":"#123456"}}}"##)
        .expect("partial preferences");
    assert_eq!(partial.glyphs.barb.color, "#123456");
    assert_eq!(partial.glyphs.barb.size_percent, 100);
    assert!(!partial.glyphs.barb.shadow.enabled);
    let mut invalid = partial;
    invalid.glyphs.arrow.size_percent = 0;
    let fixed = invalid.normalised();
    assert_eq!(fixed.glyphs.arrow, GlyphAppearance::default());
    assert_eq!(fixed.glyphs.barb.color, "#123456");
}

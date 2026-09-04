//! Application settings: shortcuts, display defaults, and the macro library
//! (spec.md 8.6, M15).
//!
//! One bindings table, read by the palette's tooltips, the timeline's keys and
//! the map's handlers alike. Before this each of those wired its own keys by
//! hand, which meant a key could be bound twice with nothing to say so and a
//! rebind was not a thing the application could have.
//!
//! The file lives beside the recent list, in the config directory, and **grows
//! rather than being replaced**: a settings file written by an older build is
//! missing fields, not wrong, so every field defaults and a file that will not
//! parse at all falls back to defaults rather than refusing to start. Losing a
//! preference is a far better outcome than not launching.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::commands::AppState;
use crate::error::{AppError, Result};
use crate::projects::with_session;

/// Everything a shortcut can be bound to (spec.md 8.1, M15).
///
/// A closed set rather than free text: a binding names an *action*, and an
/// action that is not in this list is one nothing would perform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export, export_to = "ShortcutAction.ts")]
#[serde(rename_all = "snake_case")]
pub enum ShortcutAction {
    /// Start or stop playback.
    PlayPause,
    /// Move the playhead one step earlier.
    StepBack,
    /// And one step later.
    StepForward,
    /// Pan the map, by a screenful fraction.
    PanLeft,
    /// And the other three ways.
    PanRight,
    /// Up.
    PanUp,
    /// Down.
    PanDown,
    /// Zoom the map in.
    ZoomIn,
    /// And out.
    ZoomOut,
    /// Select a tool. The payload is the tool's wire name.
    Tool,
}

/// One binding: an action, the key that performs it, and its modifiers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "Shortcut.ts")]
pub struct Shortcut {
    /// What it does.
    pub action: ShortcutAction,
    /// For [`ShortcutAction::Tool`], the tool's wire name; empty otherwise.
    #[serde(default)]
    pub tool: String,
    /// The key, as the browser's `KeyboardEvent.key` lowercased: a letter, or
    /// a name like `arrowleft` or `" "`.
    pub key: String,
    /// Whether the binding wants shift held.
    #[serde(default)]
    pub shift: bool,
}

impl Shortcut {
    /// The chord, for comparing two bindings and for showing one.
    pub fn chord(&self) -> String {
        if self.shift {
            format!("shift+{}", self.key)
        } else {
            self.key.clone()
        }
    }

    /// How the chord reads in a tooltip.
    pub fn label(&self) -> String {
        let key = match self.key.as_str() {
            " " => "Space".to_owned(),
            "arrowleft" => "←".to_owned(),
            "arrowright" => "→".to_owned(),
            "arrowup" => "↑".to_owned(),
            "arrowdown" => "↓".to_owned(),
            other => other.to_uppercase(),
        };
        if self.shift {
            format!("Shift-{key}")
        } else {
            key
        }
    }

    /// Which slot this binding fills: one per action, and one per *tool* for
    /// the tool action.
    fn slot(&self) -> (ShortcutAction, String) {
        (self.action, self.tool.clone())
    }
}

/// Keys the webview or the operating system owns.
///
/// A binding on one of these silently does nothing, or does something the
/// application cannot see — which looks exactly like a broken shortcut. Better
/// to refuse it on entry and say why.
const RESERVED: &[&str] = &[
    "tab",
    "escape",
    "enter",
    "backspace",
    "delete",
    "f5",
    "f11",
    "f12",
    "contextmenu",
];

/// The defaults, which are the keys the app had wired by hand before this.
pub fn default_shortcuts() -> Vec<Shortcut> {
    let tool = |name: &str, key: &str| Shortcut {
        action: ShortcutAction::Tool,
        tool: name.to_owned(),
        key: key.to_owned(),
        shift: false,
    };
    let plain = |action: ShortcutAction, key: &str| Shortcut {
        action,
        tool: String::new(),
        key: key.to_owned(),
        shift: false,
    };
    let shifted = |action: ShortcutAction, key: &str| Shortcut {
        action,
        tool: String::new(),
        key: key.to_owned(),
        shift: true,
    };
    vec![
        plain(ShortcutAction::PlayPause, " "),
        plain(ShortcutAction::StepBack, "arrowleft"),
        plain(ShortcutAction::StepForward, "arrowright"),
        // The bare arrows are the timeline's, so the map's pan takes shift.
        shifted(ShortcutAction::PanLeft, "arrowleft"),
        shifted(ShortcutAction::PanRight, "arrowright"),
        shifted(ShortcutAction::PanUp, "arrowup"),
        shifted(ShortcutAction::PanDown, "arrowdown"),
        plain(ShortcutAction::ZoomIn, "="),
        plain(ShortcutAction::ZoomOut, "-"),
        tool("hand", "v"),
        tool("select", "m"),
        tool("fill", "g"),
        tool("brush", "p"),
        tool("circle", "c"),
        tool("shape_fill", "f"),
        tool("mask", "e"),
        tool("clone_stamp", "s"),
        tool("curve", "b"),
        tool("intensity", "i"),
        tool("divergence", "d"),
        tool("turn", "r"),
        tool("warp", "w"),
    ]
}

/// The application's persisted preferences.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "AppSettings.ts")]
#[serde(default)]
pub struct AppSettings {
    /// Every binding, in the order the dialog lists them.
    pub shortcuts: Vec<Shortcut>,
    /// The colour-ramp top a *new* wind project gets, in knots.
    pub default_wind_scale_knots: f64,
    /// And a new current project.
    pub default_current_scale_knots: f64,
    /// Where the macro library lives. Empty means the default under the app
    /// data directory (M16).
    pub macro_directory: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            shortcuts: default_shortcuts(),
            default_wind_scale_knots: 60.0,
            default_current_scale_knots: 6.0,
            macro_directory: String::new(),
        }
    }
}

impl AppSettings {
    /// The binding for an action, if one is set.
    pub fn binding(&self, action: ShortcutAction, tool: &str) -> Option<&Shortcut> {
        self.shortcuts
            .iter()
            .find(|s| s.action == action && s.tool == tool)
    }

    /// Replaces one binding, refusing a chord that is reserved or taken.
    ///
    /// Refused **on entry** rather than allowed and resolved later: two
    /// bindings on one chord means one of them silently stops working, and
    /// which one would depend on the order of a list nobody can see.
    pub fn rebind(&mut self, binding: Shortcut) -> Result<()> {
        let refuse = |why: String| AppError::BadOption {
            field: "shortcut",
            value: why,
        };
        if binding.key.trim().is_empty() {
            return Err(refuse("a binding needs a key".to_owned()));
        }
        let key = binding.key.to_lowercase();
        if RESERVED.contains(&key.as_str()) {
            return Err(refuse(format!(
                "{} is reserved by the window and cannot be bound",
                binding.label()
            )));
        }
        let binding = Shortcut { key, ..binding };
        let slot = binding.slot();
        if let Some(clash) = self
            .shortcuts
            .iter()
            .find(|s| s.slot() != slot && s.chord() == binding.chord())
        {
            return Err(refuse(format!(
                "{} is already {}",
                binding.label(),
                describe(clash)
            )));
        }
        match self.shortcuts.iter_mut().find(|s| s.slot() == slot) {
            Some(existing) => *existing = binding,
            None => self.shortcuts.push(binding),
        }
        Ok(())
    }

    /// Drops anything a hand-edited file got wrong, and fills what it lacks.
    ///
    /// A settings file is a file like any other: it can be edited, truncated
    /// or written by a different build. Anything unusable is replaced by its
    /// default rather than refused, so a bad line costs one preference and
    /// not the launch.
    pub fn normalised(mut self) -> Self {
        let defaults = default_shortcuts();
        let mut seen: BTreeMap<String, ()> = BTreeMap::new();
        self.shortcuts.retain(|binding| {
            let key = binding.key.to_lowercase();
            !key.trim().is_empty()
                && !RESERVED.contains(&key.as_str())
                && seen.insert(binding.chord(), ()).is_none()
        });
        for fallback in defaults {
            let slot = fallback.slot();
            if !self.shortcuts.iter().any(|s| s.slot() == slot)
                && !self.shortcuts.iter().any(|s| s.chord() == fallback.chord())
            {
                self.shortcuts.push(fallback);
            }
        }
        self.default_wind_scale_knots = self.default_wind_scale_knots.clamp(1.0, 400.0);
        self.default_current_scale_knots = self.default_current_scale_knots.clamp(1.0, 400.0);
        self
    }
}

fn describe(binding: &Shortcut) -> String {
    match binding.action {
        ShortcutAction::Tool => format!("the {} tool", binding.tool.replace('_', " ")),
        ShortcutAction::PlayPause => "play/pause".to_owned(),
        ShortcutAction::StepBack => "step back".to_owned(),
        ShortcutAction::StepForward => "step forward".to_owned(),
        ShortcutAction::PanLeft => "pan left".to_owned(),
        ShortcutAction::PanRight => "pan right".to_owned(),
        ShortcutAction::PanUp => "pan up".to_owned(),
        ShortcutAction::PanDown => "pan down".to_owned(),
        ShortcutAction::ZoomIn => "zoom in".to_owned(),
        ShortcutAction::ZoomOut => "zoom out".to_owned(),
    }
}

/// The current settings.
#[tauri::command]
pub fn app_settings(state: tauri::State<'_, AppState>) -> Result<AppSettings> {
    settings_of(&state)
}

/// Implementation of [`app_settings`].
pub fn settings_of(state: &AppState) -> Result<AppSettings> {
    with_session(state, |session| Ok(session.settings.clone()))
}

/// Rebinds one shortcut, refusing a collision, and saves.
#[tauri::command]
pub fn set_shortcut(state: tauri::State<'_, AppState>, binding: Shortcut) -> Result<AppSettings> {
    shortcut_set(&state, binding)
}

/// Implementation of [`set_shortcut`].
pub fn shortcut_set(state: &AppState, binding: Shortcut) -> Result<AppSettings> {
    let file = state.paths.settings_file();
    with_session(state, |session| {
        session.settings.rebind(binding)?;
        session.save_settings(&file)?;
        Ok(session.settings.clone())
    })
}

/// Puts every shortcut back to its default.
#[tauri::command]
pub fn reset_shortcuts(state: tauri::State<'_, AppState>) -> Result<AppSettings> {
    shortcuts_reset(&state)
}

/// Implementation of [`reset_shortcuts`].
pub fn shortcuts_reset(state: &AppState) -> Result<AppSettings> {
    let file = state.paths.settings_file();
    with_session(state, |session| {
        session.settings.shortcuts = default_shortcuts();
        session.save_settings(&file)?;
        Ok(session.settings.clone())
    })
}

/// Sets the colour-ramp top new projects of each kind get.
#[tauri::command]
pub fn set_default_scales(
    state: tauri::State<'_, AppState>,
    wind_knots: f64,
    current_knots: f64,
) -> Result<AppSettings> {
    default_scales_set(&state, wind_knots, current_knots)
}

/// Implementation of [`set_default_scales`].
pub fn default_scales_set(
    state: &AppState,
    wind_knots: f64,
    current_knots: f64,
) -> Result<AppSettings> {
    let file = state.paths.settings_file();
    with_session(state, |session| {
        session.settings.default_wind_scale_knots = wind_knots.clamp(1.0, 400.0);
        session.settings.default_current_scale_knots = current_knots.clamp(1.0, 400.0);
        session.save_settings(&file)?;
        Ok(session.settings.clone())
    })
}

/// Sets the open project's colour scale — a document write, undoable.
#[tauri::command]
pub fn set_colour_scale(
    state: tauri::State<'_, AppState>,
    max_knots: f64,
) -> Result<crate::projects::ProjectSummary> {
    colour_scale_set(&state, max_knots)
}

/// Implementation of [`set_colour_scale`].
///
/// The scale is what the project is *drawn with*, so it is the project's and
/// not the application's: two people opening one file should see the same map.
/// The application's preference is the default for new projects, and lives in
/// [`AppSettings`].
pub fn colour_scale_set(
    state: &AppState,
    max_knots: f64,
) -> Result<crate::projects::ProjectSummary> {
    with_session(state, |session| {
        let open = session.require_open()?;
        let before = open.project.settings.colour_scale;
        let after = Some(ve_core::project::ColourScale { max_knots }.clamped());
        if after == before {
            return Ok(crate::projects::ProjectSummary::of(open));
        }
        let command = ve_core::Command::SetColourScale { before, after };
        let (project, history) = (&mut open.project, &mut open.history);
        history.push(project, command)?;
        open.touch();
        Ok(crate::projects::ProjectSummary::of(session.require_open()?))
    })
}

/// Sets where the macro library lives (M16 fills it).
#[tauri::command]
pub fn set_macro_directory(
    state: tauri::State<'_, AppState>,
    directory: String,
) -> Result<AppSettings> {
    macro_directory_set(&state, directory)
}

/// Implementation of [`set_macro_directory`].
pub fn macro_directory_set(state: &AppState, directory: String) -> Result<AppSettings> {
    let file = state.paths.settings_file();
    with_session(state, |session| {
        session.settings.macro_directory = directory.trim().to_owned();
        session.save_settings(&file)?;
        Ok(session.settings.clone())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_bind_every_action_once() {
        let settings = AppSettings::default();
        let mut chords: Vec<String> = settings.shortcuts.iter().map(Shortcut::chord).collect();
        chords.sort();
        let count = chords.len();
        chords.dedup();
        assert_eq!(chords.len(), count, "two defaults share a chord");
    }

    #[test]
    fn a_colliding_binding_is_refused() {
        let mut settings = AppSettings::default();
        // `p` is the brush; giving it to the circle must be refused, and the
        // brush must keep it.
        let clash = Shortcut {
            action: ShortcutAction::Tool,
            tool: "circle".to_owned(),
            key: "p".to_owned(),
            shift: false,
        };
        let refused = settings.rebind(clash).expect_err("a collision");
        assert!(format!("{refused}").contains("brush"), "{refused}");
        assert_eq!(
            settings
                .binding(ShortcutAction::Tool, "brush")
                .map(Shortcut::chord),
            Some("p".to_owned())
        );
    }

    #[test]
    fn a_reserved_key_is_refused() {
        let mut settings = AppSettings::default();
        for key in ["escape", "tab", "f12"] {
            assert!(
                settings
                    .rebind(Shortcut {
                        action: ShortcutAction::Tool,
                        tool: "brush".to_owned(),
                        key: key.to_owned(),
                        shift: false,
                    })
                    .is_err(),
                "{key} should be reserved"
            );
        }
    }

    #[test]
    fn a_free_chord_is_taken() {
        let mut settings = AppSettings::default();
        settings
            .rebind(Shortcut {
                action: ShortcutAction::Tool,
                tool: "brush".to_owned(),
                key: "q".to_owned(),
                shift: false,
            })
            .expect("q is free");
        assert_eq!(
            settings
                .binding(ShortcutAction::Tool, "brush")
                .map(Shortcut::chord),
            Some("q".to_owned())
        );
        // ...and the chord it gave up is free again.
        settings
            .rebind(Shortcut {
                action: ShortcutAction::Tool,
                tool: "circle".to_owned(),
                key: "p".to_owned(),
                shift: false,
            })
            .expect("p is free now");
    }

    /// A shifted binding is a different chord from the bare key, which is what
    /// lets the map's pan share the arrows with the timeline's steps.
    #[test]
    fn shift_makes_a_different_chord() {
        let settings = AppSettings::default();
        let back = settings
            .binding(ShortcutAction::StepBack, "")
            .expect("step back");
        let pan = settings
            .binding(ShortcutAction::PanLeft, "")
            .expect("pan left");
        assert_eq!(back.key, pan.key);
        assert_ne!(back.chord(), pan.chord());
    }

    /// A hand-edited file loses only what it got wrong.
    #[test]
    fn a_broken_file_normalises_to_something_usable() {
        let mut settings = AppSettings::default();
        settings.shortcuts.truncate(3);
        settings.shortcuts.push(Shortcut {
            action: ShortcutAction::Tool,
            tool: "brush".to_owned(),
            key: "escape".to_owned(),
            shift: false,
        });
        settings.default_wind_scale_knots = -5.0;
        let fixed = settings.normalised();
        assert!(fixed.default_wind_scale_knots >= 1.0);
        assert!(
            fixed
                .binding(ShortcutAction::Tool, "brush")
                .is_some_and(|b| b.key != "escape"),
            "a reserved binding is dropped and the default comes back"
        );
        // Every action has a binding again.
        for fallback in default_shortcuts() {
            assert!(
                fixed
                    .shortcuts
                    .iter()
                    .any(|s| s.action == fallback.action && s.tool == fallback.tool),
                "{:?} {} lost its binding",
                fallback.action,
                fallback.tool
            );
        }
    }
}

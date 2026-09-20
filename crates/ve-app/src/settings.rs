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
    /// Move the selected objects, or the selected region, a step west on
    /// the screen (spec.md 8.2, M23).
    NudgeLeft,
    /// And the other three ways.
    NudgeRight,
    /// Up.
    NudgeUp,
    /// Down.
    NudgeDown,
    /// Drop the selection (spec.md 8.2, M47).
    Deselect,
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
    /// Whether the binding wants alt (option) held.
    ///
    /// Added with the nudge (D67): the arrows are the timeline's bare, the
    /// nudge's shifted, and the map's pan needed a third modifier that the
    /// window does not own. The chord spelling grows one word and the table
    /// stays one table with one collision rule.
    #[serde(default)]
    pub alt: bool,
    /// Whether the binding wants the command key held — `Cmd` on a Mac,
    /// `Ctrl` elsewhere (M47).
    ///
    /// Added with the deselect, which the drawing applications settled on as
    /// accel-D long enough ago that it is what a hand reaches for. Absent
    /// from a settings file written before it, which then reads as false —
    /// exactly the bindings that file already had.
    #[serde(default)]
    pub accel: bool,
}

impl Shortcut {
    /// The chord, for comparing two bindings and for showing one.
    ///
    /// Modifiers in a fixed order — `accel+alt+shift+key` — so a chord
    /// spelled by the frontend and one spelled here compare equal.
    pub fn chord(&self) -> String {
        let mut chord = String::new();
        if self.accel {
            chord.push_str("accel+");
        }
        if self.alt {
            chord.push_str("alt+");
        }
        if self.shift {
            chord.push_str("shift+");
        }
        chord.push_str(&self.key);
        chord
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
        let mut label = String::new();
        if self.accel {
            // The symbol on a Mac and the word elsewhere would need the
            // platform here; the frontend already labels chords for display,
            // so this stays the neutral spelling.
            label.push_str("Cmd-");
        }
        if self.alt {
            label.push_str("Alt-");
        }
        if self.shift {
            label.push_str("Shift-");
        }
        label.push_str(&key);
        label
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

/// Bindings a previous build shipped as defaults and this one does not.
///
/// A settings file holds whatever the defaults were when it was written, so
/// a user who never rebound pan still has it on `Shift`+arrows — the chord
/// the nudge now takes (D67). A stored binding that equals an old default is
/// the old default, not a choice, and gives way; one the user set stands.
const SUPERSEDED: &[(ShortcutAction, &str)] = &[
    (ShortcutAction::PanLeft, "shift+arrowleft"),
    (ShortcutAction::PanRight, "shift+arrowright"),
    (ShortcutAction::PanUp, "shift+arrowup"),
    (ShortcutAction::PanDown, "shift+arrowdown"),
];

/// The defaults, which are the keys the app had wired by hand before this.
pub fn default_shortcuts() -> Vec<Shortcut> {
    let tool = |name: &str, key: &str| Shortcut {
        action: ShortcutAction::Tool,
        tool: name.to_owned(),
        key: key.to_owned(),
        shift: false,
        alt: false,
        accel: false,
    };
    let plain = |action: ShortcutAction, key: &str| Shortcut {
        action,
        tool: String::new(),
        key: key.to_owned(),
        shift: false,
        alt: false,
        accel: false,
    };
    let shifted = |action: ShortcutAction, key: &str| Shortcut {
        action,
        tool: String::new(),
        key: key.to_owned(),
        shift: true,
        alt: false,
        accel: false,
    };
    let alted = |action: ShortcutAction, key: &str| Shortcut {
        action,
        tool: String::new(),
        key: key.to_owned(),
        shift: false,
        alt: true,
        accel: false,
    };
    vec![
        plain(ShortcutAction::PlayPause, " "),
        plain(ShortcutAction::StepBack, "arrowleft"),
        plain(ShortcutAction::StepForward, "arrowright"),
        // The bare arrows are the timeline's, the shifted ones nudge the
        // selection, and the map's pan takes alt (D67).
        shifted(ShortcutAction::NudgeLeft, "arrowleft"),
        shifted(ShortcutAction::NudgeRight, "arrowright"),
        shifted(ShortcutAction::NudgeUp, "arrowup"),
        shifted(ShortcutAction::NudgeDown, "arrowdown"),
        alted(ShortcutAction::PanLeft, "arrowleft"),
        alted(ShortcutAction::PanRight, "arrowright"),
        alted(ShortcutAction::PanUp, "arrowup"),
        alted(ShortcutAction::PanDown, "arrowdown"),
        plain(ShortcutAction::ZoomIn, "="),
        plain(ShortcutAction::ZoomOut, "-"),
        // Accel-D, the shape the drawing applications settled on for
        // "deselect" long enough ago that it is what a hand reaches for.
        Shortcut {
            action: ShortcutAction::Deselect,
            tool: String::new(),
            key: "d".to_owned(),
            shift: false,
            alt: false,
            accel: true,
        },
        tool("hand", "v"),
        tool("select", "m"),
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
        tool("liquify", "l"),
        // The measurement tools (spec.md 10, M8). `M` is the marquee's key in
        // every paint program, so measure takes `T` (D54).
        tool("measure", "t"),
        // The macro tools (spec.md 8.7, M16): capture a run of frames, and
        // put one back down (D54).
        tool("capture", "k"),
        tool("insert", "n"),
        // The eraser (spec.md 8.1, M28). `E` is the mask's, which came first
        // and is the eraser that keeps what it erased; `X` is free.
        tool("erase", "x"),
    ]
}

/// What the autosave thread does with a dirty project (D70, M25).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "AutosaveMode.ts")]
#[serde(rename_all = "snake_case")]
pub enum AutosaveMode {
    /// Nothing is written until the user saves.
    Off,
    /// A crash-recovery snapshot beside the app data, offered back on the
    /// start screen (spec.md 4.2). The default, and what every install did
    /// before the setting existed.
    #[default]
    Recovery,
    /// The project file itself, written in place on the same cadence when
    /// the project has a path — and a recovery snapshot when it does not.
    Save,
}

/// Preferred display unit for ground distances; stored geometry remains km.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DistanceUnit {
    /// Kilometres.
    #[default]
    Km,
    /// Nautical miles.
    Nm,
}

/// Preferred display unit for speeds; fields remain metres per second.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SpeedUnit {
    /// Knots.
    #[default]
    Kt,
    /// Miles per hour.
    Mph,
    /// Kilometres per hour.
    Kmh,
}

/// Which direction glyph's appearance is being edited.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum GlyphStyle {
    /// Ocean-current arrow.
    Arrow,
    /// Meteorological wind barb.
    Barb,
}

/// A glyph's drop shadow, in display pixels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct GlyphShadow {
    /// Draw the shadow behind the glyph.
    pub enabled: bool,
    /// Six-digit sRGB hex colour.
    pub color: String,
    /// Opacity, from zero to one hundred percent.
    pub opacity_percent: u8,
    /// Horizontal offset in CSS pixels, positive rightward.
    pub offset_x_px: f32,
    /// Vertical offset in CSS pixels, positive downward.
    pub offset_y_px: f32,
}

impl Default for GlyphShadow {
    fn default() -> Self {
        Self {
            enabled: false,
            color: "#000000".to_owned(),
            opacity_percent: 65,
            offset_x_px: 1.5,
            offset_y_px: 1.5,
        }
    }
}

/// Appearance of one glyph style. These preferences never reach field evaluation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct GlyphAppearance {
    /// Size relative to the standard glyph, 25–300 percent.
    pub size_percent: u16,
    /// Stroke width in CSS pixels, 0.5–6.
    pub stroke_width_px: f32,
    /// Six-digit sRGB hex colour.
    pub color: String,
    /// Opacity, from zero to one hundred percent.
    pub opacity_percent: u8,
    /// Number of glyphs per area relative to normal, 25–300 percent.
    pub density_percent: u16,
    /// Preserve the familiar fading of glyphs in slower flow.
    pub fade_with_speed: bool,
    /// Shadow appearance.
    pub shadow: GlyphShadow,
}

impl Default for GlyphAppearance {
    fn default() -> Self {
        Self {
            size_percent: 100,
            stroke_width_px: 1.8,
            color: "#f0f7ff".to_owned(),
            opacity_percent: 90,
            density_percent: 100,
            fade_with_speed: true,
            shadow: GlyphShadow::default(),
        }
    }
}

/// Independent arrow and wind-barb preferences.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct GlyphSettings {
    /// Ocean-current arrows.
    pub arrow: GlyphAppearance,
    /// Wind barbs.
    pub barb: GlyphAppearance,
}

/// One atomic appearance edit; simultaneous controls cannot overwrite each other.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "property", content = "value", rename_all = "snake_case")]
#[ts(export)]
pub enum GlyphSetting {
    /// Relative glyph size.
    SizePercent(u16),
    /// Line width in CSS pixels.
    StrokeWidthPx(f32),
    /// Glyph colour.
    Color(String),
    /// Glyph opacity.
    OpacityPercent(u8),
    /// Relative glyph density.
    DensityPercent(u16),
    /// Fade slow flow.
    FadeWithSpeed(bool),
    /// Enable the shadow.
    ShadowEnabled(bool),
    /// Shadow colour.
    ShadowColor(String),
    /// Shadow opacity.
    ShadowOpacityPercent(u8),
    /// Shadow horizontal offset.
    ShadowOffsetXPx(f32),
    /// Shadow vertical offset.
    ShadowOffsetYPx(f32),
    /// Restore this style's defaults.
    Reset,
}

fn glyph_color_valid(color: &str) -> bool {
    color.len() == 7
        && color.starts_with('#')
        && color.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
}

impl GlyphAppearance {
    fn valid(&self) -> bool {
        (25..=300).contains(&self.size_percent)
            && (0.5..=6.0).contains(&self.stroke_width_px)
            && glyph_color_valid(&self.color)
            && self.opacity_percent <= 100
            && (25..=300).contains(&self.density_percent)
            && glyph_color_valid(&self.shadow.color)
            && self.shadow.opacity_percent <= 100
            && (-12.0..=12.0).contains(&self.shadow.offset_x_px)
            && (-12.0..=12.0).contains(&self.shadow.offset_y_px)
    }

    fn apply(&mut self, setting: GlyphSetting) {
        match setting {
            GlyphSetting::SizePercent(value) => self.size_percent = value,
            GlyphSetting::StrokeWidthPx(value) => self.stroke_width_px = value,
            GlyphSetting::Color(value) => self.color = value.to_ascii_lowercase(),
            GlyphSetting::OpacityPercent(value) => self.opacity_percent = value,
            GlyphSetting::DensityPercent(value) => self.density_percent = value,
            GlyphSetting::FadeWithSpeed(value) => self.fade_with_speed = value,
            GlyphSetting::ShadowEnabled(value) => self.shadow.enabled = value,
            GlyphSetting::ShadowColor(value) => self.shadow.color = value.to_ascii_lowercase(),
            GlyphSetting::ShadowOpacityPercent(value) => self.shadow.opacity_percent = value,
            GlyphSetting::ShadowOffsetXPx(value) => self.shadow.offset_x_px = value,
            GlyphSetting::ShadowOffsetYPx(value) => self.shadow.offset_y_px = value,
            GlyphSetting::Reset => *self = Self::default(),
        }
    }
}

/// The MCP service's switch, port and token (spec.md 8.8).
///
/// The token is a plain string in the file the person already owns: it
/// grants a local process what sitting at the keyboard grants, nothing more.
/// Empty means no token, and `token::matches` refuses everything then.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "McpSettings.ts")]
pub struct McpSettings {
    pub enabled: bool,
    pub port: u16,
    pub token: String,
}

/// The port a fresh install listens on when the service is first enabled.
pub const DEFAULT_MCP_PORT: u16 = 47391;

impl Default for McpSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            port: DEFAULT_MCP_PORT,
            token: String::new(),
        }
    }
}

/// The application's persisted preferences.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "AppSettings.ts")]
#[serde(default)]
pub struct AppSettings {
    /// Global arrow and wind-barb appearance preferences.
    pub glyphs: GlyphSettings,
    /// Global ground-distance display preference.
    pub distance_unit: DistanceUnit,
    /// Global speed display preference.
    pub speed_unit: SpeedUnit,
    /// What happens to unsaved work while the user is not saving (D70).
    pub autosave: AutosaveMode,
    /// Every binding, in the order the dialog lists them.
    pub shortcuts: Vec<Shortcut>,
    /// The colour-ramp top a *new* wind project gets, in knots.
    pub default_wind_scale_knots: f64,
    /// And a new current project.
    pub default_current_scale_knots: f64,
    /// Where the macro library lives. Empty means the default under the app
    /// data directory (M16).
    pub macro_directory: String,
    /// How the map lays the world out (M11): one of [`PROJECTIONS`].
    ///
    /// A view preference and nothing else. Nothing below the view reads it —
    /// the document is geodesic, the evaluator works in lat/lon and the export
    /// has its own grid — so changing it cannot change a saved project or an
    /// exported file (invariant 3). It lives in the application's settings
    /// rather than the project's for exactly that reason: it says how *this*
    /// person likes to look at a map, not what the map is.
    pub projection: String,
    /// Whether the colour ramp follows the field in view (M27, spec.md 5.3).
    ///
    /// On, the ramp runs from the slowest to the fastest speed among the
    /// tiles on screen, across every layer and object, and the legend says
    /// so; off, it runs from calm to the project's own scale. A view
    /// preference like the projection: it changes no stored or exported
    /// value, only which colour a speed is drawn in.
    pub auto_scale: bool,
    /// The MCP service (spec.md 8.8). Absent from older files: off.
    #[serde(default)]
    pub mcp: McpSettings,
    /// Where the S-57 electronic charts live (spec.md 4.11). Empty means
    /// none is chosen, and *Display charts* has nothing to show.
    ///
    /// A view preference like the projection: it says what this person has
    /// on their disk to look at, and changes no project and no export.
    #[serde(default)]
    pub chart_directory: String,
}

/// The map projections the view offers, in the order the menu lists them.
///
/// Named here because the setting is validated against them, and a hand-edited
/// settings file naming something else must cost the preference and not the
/// launch. The view formulas live in `ui/src/map/projection.ts`; frozen pixel
/// geometry uses the corresponding spaces in `ve_render::aeqd`.
pub const PROJECTIONS: [&str; 24] = [
    "equirectangular",
    "mercator",
    "miller",
    "lambert",
    "behrmann",
    "gall_peters",
    "hobo_dyer",
    "gall_stereographic",
    "braun",
    "central_cylindrical",
    "patterson",
    "compact_miller",
    "equidistant_30",
    "equidistant_45",
    "orthographic",
    "robinson",
    "mollweide",
    "winkel_tripel",
    "equal_earth",
    "sinusoidal",
    "azimuthal_equidistant",
    "azimuthal_equal_area",
    "stereographic",
    "gnomonic",
];

/// The frontend and backend read the same offline national/polar CRS catalogue.
fn supported_projection(id: &str) -> bool {
    if PROJECTIONS.contains(&id) {
        return true;
    }
    if let Some(definition) = id.strip_prefix("custom:") {
        // The frontend validates PROJ/WKT. Persist bounded encoded definitions;
        // they are data, never commands or resource URLs.
        return !definition.is_empty()
            && definition.len() <= 24576
            && definition.is_ascii()
            && !definition.chars().any(char::is_control);
    }
    static CATALOGUE: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    CATALOGUE
        .get_or_init(|| {
            let rows: Vec<serde_json::Value> =
                serde_json::from_str(include_str!("../../../ui/src/map/projections/crs.json"))
                    .unwrap_or_default();
            rows.iter()
                .filter_map(|row| row["id"].as_str().map(str::to_owned))
                .collect()
        })
        .iter()
        .any(|known| known == id)
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            glyphs: GlyphSettings::default(),
            distance_unit: DistanceUnit::Km,
            speed_unit: SpeedUnit::Kt,
            autosave: AutosaveMode::Recovery,
            shortcuts: default_shortcuts(),
            default_wind_scale_knots: 60.0,
            default_current_scale_knots: 6.0,
            macro_directory: String::new(),
            chart_directory: String::new(),
            projection: PROJECTIONS[0].to_owned(),
            auto_scale: false,
            mcp: McpSettings::default(),
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
        for appearance in [&mut self.glyphs.arrow, &mut self.glyphs.barb] {
            if !appearance.valid() {
                *appearance = GlyphAppearance::default();
            }
        }
        let defaults = default_shortcuts();
        let mut seen: BTreeMap<String, ()> = BTreeMap::new();
        self.shortcuts.retain(|binding| {
            let key = binding.key.to_lowercase();
            !key.trim().is_empty()
                && !RESERVED.contains(&key.as_str())
                && !SUPERSEDED
                    .iter()
                    .any(|(action, chord)| *action == binding.action && *chord == binding.chord())
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
        if !supported_projection(&self.projection) {
            self.projection = PROJECTIONS[0].to_owned();
        }
        self
    }
}

fn describe(binding: &Shortcut) -> String {
    match binding.action {
        ShortcutAction::Deselect => "deselect".to_owned(),
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
        ShortcutAction::NudgeLeft => "nudge left".to_owned(),
        ShortcutAction::NudgeRight => "nudge right".to_owned(),
        ShortcutAction::NudgeUp => "nudge up".to_owned(),
        ShortcutAction::NudgeDown => "nudge down".to_owned(),
    }
}

/// Sets one arrow or wind-barb appearance preference.
#[tauri::command]
pub fn set_glyph_appearance(
    state: tauri::State<'_, AppState>,
    style: GlyphStyle,
    setting: GlyphSetting,
) -> Result<AppSettings> {
    glyph_appearance_set(&state, style, setting)
}

/// Changes and persists one glyph preference without invalidating rendered tiles.
pub fn glyph_appearance_set(
    state: &AppState,
    style: GlyphStyle,
    setting: GlyphSetting,
) -> Result<AppSettings> {
    let file = state.paths.settings_file();
    with_session(state, |session| {
        let mut next = session.settings.clone();
        let appearance = match style {
            GlyphStyle::Arrow => &mut next.glyphs.arrow,
            GlyphStyle::Barb => &mut next.glyphs.barb,
        };
        appearance.apply(setting);
        if !appearance.valid() {
            return Err(AppError::BadOption {
                field: "glyph appearance",
                value: "use valid colours and values within the displayed ranges".to_owned(),
            });
        }
        let before = std::mem::replace(&mut session.settings, next);
        if let Err(error) = session.save_settings(&file) {
            session.settings = before;
            return Err(error);
        }
        Ok(session.settings.clone())
    })
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

/// Changes global display units without modifying any document values.
#[tauri::command]
pub fn set_display_units(
    state: tauri::State<'_, AppState>,
    distance_unit: DistanceUnit,
    speed_unit: SpeedUnit,
) -> Result<AppSettings> {
    display_units_set(&state, distance_unit, speed_unit)
}

/// Persists the display preferences.
pub fn display_units_set(
    state: &AppState,
    distance_unit: DistanceUnit,
    speed_unit: SpeedUnit,
) -> Result<AppSettings> {
    let file = state.paths.settings_file();
    with_session(state, |session| {
        session.settings.distance_unit = distance_unit;
        session.settings.speed_unit = speed_unit;
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
    kind: String,
    max_knots: f64,
) -> Result<crate::projects::ProjectSummary> {
    colour_scale_set(&state, &kind, max_knots)
}

/// Implementation of [`set_colour_scale`]: the top of the ramp for one kind
/// of field (M29), an undoable document write.
pub fn colour_scale_set(
    state: &AppState,
    kind: &str,
    max_knots: f64,
) -> Result<crate::projects::ProjectSummary> {
    let kind = crate::projects::parse_field_kind(kind)?;
    with_session(state, |session| {
        let open = session.require_open()?;
        let before = open.project.settings.colour_scale;
        let after = Some(
            open.project
                .settings
                .scale()
                .with_knots(kind, max_knots)
                .clamped(),
        );
        if before == after {
            return Ok(crate::projects::ProjectSummary::of(session.require_open()?));
        }
        let command = ve_core::Command::SetColourScale { before, after };
        let (project, history) = (&mut open.project, &mut open.history);
        history.push(project, command)?;
        open.touch();
        Ok(crate::projects::ProjectSummary::of(session.require_open()?))
    })
}

/// Which gradients the map may be drawn with (spec.md 5.3, M42).
///
/// Served rather than written into the frontend, so the names, the colours
/// and the notes are one table: a list of names here and a list of colours
/// there is the arrangement that drifts, and the drift would be silent —
/// the map painting one palette while the legend drew another.
#[tauri::command]
pub fn colour_gradients() -> Vec<GradientView> {
    ve_core::colour::GRADIENTS
        .iter()
        .map(GradientView::of)
        .collect()
}

/// One gradient on the wire.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "GradientView.ts")]
pub struct GradientView {
    /// What the document stores, e.g. `"viridis"`.
    pub id: String,
    /// What the settings offer.
    pub label: String,
    /// A sentence on what it is for.
    pub note: String,
    /// Stops as `0.0`–`1.0` RGB, calm first, evenly spaced across the range.
    pub stops: Vec<[f32; 3]>,
}

impl GradientView {
    fn of(gradient: &ve_core::colour::Gradient) -> Self {
        Self {
            id: gradient.id.to_owned(),
            label: gradient.label.to_owned(),
            note: gradient.note.to_owned(),
            stops: gradient.stops.to_vec(),
        }
    }
}

/// Sets the gradient one kind of field is painted with (spec.md 5.3, M42).
#[tauri::command]
pub fn set_colour_gradient(
    state: tauri::State<'_, AppState>,
    kind: String,
    gradient: String,
) -> Result<crate::projects::ProjectSummary> {
    colour_gradient_set(&state, &kind, &gradient)
}

/// Implementation of [`set_colour_gradient`]: an undoable document write, as
/// the scale beside it is.
///
/// An unknown identifier is refused here rather than stored and drawn with
/// the default. A file *arriving* with a name this build does not know is a
/// different case — that one is carried through untouched, since the version
/// that wrote it can still read it — but a name this build is being *asked*
/// to write can only be a caller's mistake.
pub fn colour_gradient_set(
    state: &AppState,
    kind: &str,
    gradient: &str,
) -> Result<crate::projects::ProjectSummary> {
    let kind = crate::projects::parse_field_kind(kind)?;
    if ve_core::colour::gradient(gradient).is_none() {
        return Err(AppError::BadOption {
            field: "gradient",
            value: format!("{gradient} is not a gradient this draws"),
        });
    }
    with_session(state, |session| {
        let open = session.require_open()?;
        let before = open.project.settings.colour_gradients.clone();
        let after = Some(open.project.settings.gradients().with_id(kind, gradient));
        if before == after {
            return Ok(crate::projects::ProjectSummary::of(session.require_open()?));
        }
        let command = ve_core::Command::SetColourGradients { before, after };
        let (project, history) = (&mut open.project, &mut open.history);
        history.push(project, command)?;
        open.touch();
        Ok(crate::projects::ProjectSummary::of(session.require_open()?))
    })
}

/// Sets how the map lays the world out (M11).
#[tauri::command]
pub fn set_projection(
    state: tauri::State<'_, AppState>,
    projection: String,
) -> Result<AppSettings> {
    projection_set(&state, projection)
}

/// Implementation of [`set_projection`].
pub fn projection_set(state: &AppState, projection: String) -> Result<AppSettings> {
    if !supported_projection(&projection) {
        return Err(AppError::BadOption {
            field: "projection",
            value: projection,
        });
    }
    let file = state.paths.settings_file();
    with_session(state, |session| {
        session.settings.projection = projection.clone();
        session.save_settings(&file)?;
        Ok(session.settings.clone())
    })
}

/// Sets whether the colour ramp follows the field in view (M27).
#[tauri::command]
pub fn set_auto_scale(state: tauri::State<'_, AppState>, on: bool) -> Result<AppSettings> {
    auto_scale_set(&state, on)
}

/// Implementation of [`set_auto_scale`].
pub fn auto_scale_set(state: &AppState, on: bool) -> Result<AppSettings> {
    let file = state.paths.settings_file();
    with_session(state, |session| {
        session.settings.auto_scale = on;
        session.save_settings(&file)?;
        Ok(session.settings.clone())
    })
}

/// Sets what the autosave thread does with unsaved work (D70).
#[tauri::command]
pub fn set_autosave_mode(
    state: tauri::State<'_, AppState>,
    mode: AutosaveMode,
) -> Result<AppSettings> {
    autosave_mode_set(&state, mode)
}

/// Implementation of [`set_autosave_mode`].
pub fn autosave_mode_set(state: &AppState, mode: AutosaveMode) -> Result<AppSettings> {
    let file = state.paths.settings_file();
    with_session(state, |session| {
        session.settings.autosave = mode;
        session.save_settings(&file)?;
        Ok(session.settings.clone())
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

/// Sets where the S-57 charts live, and says what was found there.
///
/// The directory is indexed straight away rather than at the first tile, so
/// the dialog can say "144 cells" or say what is wrong with the choice — a
/// directory that holds no chart is the most likely mistake, and finding
/// out at the first tile means finding out as an empty map.
#[tauri::command(async)]
pub fn set_chart_directory(
    state: tauri::State<'_, AppState>,
    directory: String,
) -> Result<crate::charts::ChartStatus> {
    chart_directory_set(&state, directory)
}

/// Implementation of [`set_chart_directory`].
pub fn chart_directory_set(
    state: &AppState,
    directory: String,
) -> Result<crate::charts::ChartStatus> {
    let file = state.paths.settings_file();
    let directory = directory.trim().to_owned();
    with_session(state, |session| {
        session.settings.chart_directory = directory.clone();
        session.save_settings(&file)?;
        Ok(())
    })?;
    Ok(state.backdrops.chart_status(&directory))
}

/// What the chart directory currently holds.
#[tauri::command(async)]
pub fn chart_status(state: tauri::State<'_, AppState>) -> Result<crate::charts::ChartStatus> {
    let directory = state.chart_directory();
    Ok(state.backdrops.chart_status(&directory))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regional_and_custom_projections_survive_settings_round_trips() {
        let rows: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../ui/src/map/projections/crs.json")).unwrap();
        assert_eq!(rows.len(), 246);
        for row in rows {
            let projection = row["id"].as_str().unwrap().to_owned();
            let settings = AppSettings {
                projection: projection.clone(),
                ..AppSettings::default()
            };
            let saved = serde_json::to_string(&settings).unwrap();
            let restored: AppSettings = serde_json::from_str(&saved).unwrap();
            assert_eq!(restored.normalised().projection, projection);
        }
        assert!(supported_projection("custom:%2Bproj%3Dlcc%20%2Blat_1%3D33"));
        assert!(!supported_projection("epsg_999999"));
        assert!(!supported_projection("custom:"));
        assert!(!supported_projection(&format!(
            "custom:{}",
            "x".repeat(24577)
        )));
    }

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
    fn a_settings_file_without_mcp_loads_with_it_off() {
        let json = r#"{"glyphs":{},"distance_unit":"km","speed_unit":"kt","autosave":"recovery","shortcuts":[],"default_wind_scale_knots":60.0,"default_current_scale_knots":6.0,"macro_directory":"","projection":"equirectangular","auto_scale":false}"#;
        let settings: AppSettings = serde_json::from_str(json).expect("older settings load");
        assert_eq!(settings.mcp, McpSettings::default());
        assert!(!settings.mcp.enabled);
        assert_eq!(settings.mcp.port, 47391);
    }

    #[test]
    fn a_settings_file_naming_an_unknown_projection_costs_the_preference() {
        // A settings file is a file: it can be hand-edited or written by a
        // different build. An unusable projection must fall back to the flat
        // map rather than reach the renderer, which indexes a list with it.
        let settings = AppSettings {
            projection: "gall-peters".to_owned(),
            ..AppSettings::default()
        }
        .normalised();
        assert_eq!(settings.projection, "equirectangular");

        for projection in PROJECTIONS {
            let settings = AppSettings {
                projection: projection.to_owned(),
                ..AppSettings::default()
            };
            let json = serde_json::to_string(&settings).unwrap();
            let kept = serde_json::from_str::<AppSettings>(&json)
                .unwrap()
                .normalised();
            assert_eq!(kept.projection, projection);
        }
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
            alt: false,
            accel: false,
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
                        alt: false,
                        accel: false,
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
                alt: false,
                accel: false,
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
                alt: false,
                accel: false,
            })
            .expect("p is free now");
    }

    /// A modifier makes a different chord from the bare key, which is what
    /// lets the timeline's steps, the nudge and the map's pan share one row
    /// of arrows (D67).
    #[test]
    fn modifiers_make_different_chords() {
        let settings = AppSettings::default();
        let back = settings
            .binding(ShortcutAction::StepBack, "")
            .expect("step back");
        let nudge = settings
            .binding(ShortcutAction::NudgeLeft, "")
            .expect("nudge left");
        let pan = settings
            .binding(ShortcutAction::PanLeft, "")
            .expect("pan left");
        assert_eq!(back.key, pan.key);
        assert_eq!(back.key, nudge.key);
        assert_eq!(nudge.chord(), "shift+arrowleft");
        assert_eq!(pan.chord(), "alt+arrowleft");
        assert_eq!(pan.label(), "Alt-←");
    }

    /// A settings file from before the nudge has pan on `Shift`+arrows as
    /// its stored default. That is not a choice the user made, so it gives
    /// way to the new defaults — and a chord the user *did* set stands.
    #[test]
    fn an_old_default_gives_way_and_a_chosen_chord_stands() {
        let mut stored = AppSettings::default();
        stored.shortcuts.retain(|s| {
            !matches!(
                s.action,
                ShortcutAction::NudgeLeft
                    | ShortcutAction::NudgeRight
                    | ShortcutAction::NudgeUp
                    | ShortcutAction::NudgeDown
            )
        });
        for binding in &mut stored.shortcuts {
            if matches!(
                binding.action,
                ShortcutAction::PanLeft | ShortcutAction::PanUp
            ) {
                binding.alt = false;
                binding.shift = true;
            }
            if binding.action == ShortcutAction::PanRight {
                // A chord the user chose: alt-shift-right.
                binding.alt = true;
                binding.shift = true;
            }
        }
        let fixed = stored.normalised();
        assert_eq!(
            fixed
                .binding(ShortcutAction::PanLeft, "")
                .map(Shortcut::chord),
            Some("alt+arrowleft".to_owned()),
            "the old default gives way"
        );
        assert_eq!(
            fixed
                .binding(ShortcutAction::NudgeLeft, "")
                .map(Shortcut::chord),
            Some("shift+arrowleft".to_owned()),
            "and the nudge takes the chord"
        );
        assert_eq!(
            fixed
                .binding(ShortcutAction::PanRight, "")
                .map(Shortcut::chord),
            Some("alt+shift+arrowright".to_owned()),
            "a chosen chord stands"
        );
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
            alt: false,
            accel: false,
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

/// What the Settings dialog shows about the service (spec.md 8.8).
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "McpStatus.ts")]
pub struct McpStatus {
    pub enabled: bool,
    pub port: u16,
    /// The token, so the dialog can build a client configuration.
    pub token: String,
    /// The port actually bound, or null while off or if binding failed.
    pub bound_port: Option<u16>,
    /// Why the listener is not up although the setting is on.
    pub bind_error: Option<String>,
    /// Open client sessions.
    pub sessions: u32,
    /// The last tool a client called, if any.
    pub last_tool: Option<String>,
    /// The clients the dialog can add the service to on this platform.
    pub clients: Vec<crate::mcp::clients::McpClient>,
}

/// Reads the service's settings and live state.
#[tauri::command]
pub fn mcp_status(
    state: tauri::State<'_, AppState>,
    service: tauri::State<'_, crate::mcp::McpService>,
) -> Result<McpStatus> {
    let mcp = with_session(&state, |session| Ok(session.settings.mcp.clone()))?;
    Ok(service.status(&mcp))
}

/// Turns the service on or off and sets its port. Enabling issues a fresh
/// token; disabling clears it and drops the listener.
///
/// Generic over the Tauri runtime so the integration tests can drive it
/// through a mock application. `async`: `McpService::apply` calls
/// `Running::stop`, which can block briefly waiting for the listener's
/// accept loop to confirm its socket closed, so this must run on Tauri's
/// thread pool and never the main (webview) thread. Not in `LONG_RUNNING`
/// (`ui/src/ipc.ts`) — a settings toggle must not spin the status bar.
#[tauri::command(async)]
pub fn mcp_set<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: tauri::State<'_, AppState>,
    service: tauri::State<'_, crate::mcp::McpService>,
    enabled: bool,
    port: u16,
) -> Result<McpStatus> {
    if port == 0 {
        return Err(AppError::BadOption {
            field: "port",
            value: port.to_string(),
        });
    }
    let file = state.paths.settings_file();
    let mcp = with_session(&state, |session| {
        let turning_on = enabled && !session.settings.mcp.enabled;
        session.settings.mcp.enabled = enabled;
        session.settings.mcp.port = port;
        if turning_on || (enabled && session.settings.mcp.token.is_empty()) {
            session.settings.mcp.token = crate::mcp::token::fresh();
        }
        if !enabled {
            session.settings.mcp.token.clear();
        }
        session.save_settings(&file)?;
        Ok(session.settings.mcp.clone())
    })?;
    service.apply(&app, &mcp);
    Ok(service.status(&mcp))
}

/// Issues a new token and restarts the listener with it.
///
/// Generic over the Tauri runtime so the integration tests can drive it
/// through a mock application. `async` for the same reason as `mcp_set`:
/// `McpService::apply` can block briefly on `Running::stop`.
#[tauri::command(async)]
pub fn mcp_rotate_token<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: tauri::State<'_, AppState>,
    service: tauri::State<'_, crate::mcp::McpService>,
) -> Result<McpStatus> {
    let file = state.paths.settings_file();
    let mcp = with_session(&state, |session| {
        if !session.settings.mcp.enabled {
            return Err(AppError::BadOption {
                field: "mcp",
                value: "is not turned on".to_owned(),
            });
        }
        session.settings.mcp.token = crate::mcp::token::fresh();
        session.save_settings(&file)?;
        Ok(session.settings.mcp.clone())
    })?;
    service.apply(&app, &mcp);
    Ok(service.status(&mcp))
}

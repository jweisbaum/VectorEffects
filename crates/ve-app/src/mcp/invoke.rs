//! `invoke`: any IPC command by name, for what no curated tool covers.
//!
//! One entry per command, calling the command function with the state the
//! handle gives — so a call through the escape hatch is the same call the
//! interface makes, undo and all. The test at the bottom holds the table
//! equal to `generate_handler!` minus [`EXCLUDED`], which is what makes a
//! command added later reachable the day it lands.

use serde_json::Value;
use tauri::Manager;

use crate::commands::AppState;
use crate::error::{AppError, Result};

/// A command, reduced to the one shape the table can hold.
///
/// Not generic over the runtime, unlike the rest of this module: a `const`
/// of function pointers cannot be. Every entry only ever needs the managed
/// [`AppState`], and `tauri::State` is not parameterised by the runtime, so
/// [`call`] does the one `app.state()` and hands the result on.
type Handler = for<'a> fn(tauri::State<'a, AppState>, Value) -> Result<Value>;

/// A refusal that names the command, since the client sees one `invoke`.
fn bad(name: &str, why: impl std::fmt::Display) -> AppError {
    AppError::BadOption {
        field: "command",
        value: format!("{name}: {why}"),
    }
}

/// Builds one entry: the command name, a struct of its arguments, the call.
///
/// The arguments are named and typed exactly as the command declares them,
/// so a signature that changes under the table is a compile error rather
/// than a refusal at run time.
///
/// Unknown keys are refused rather than ignored: a client that misspells an
/// argument hears about it instead of watching the default apply.
macro_rules! command {
    ($name:ident, $path:path, { $($field:ident : $ty:ty),* $(,)? }) => {{
        fn handler(state: tauri::State<'_, AppState>, args: Value) -> Result<Value> {
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args { $($field: $ty),* }
            let Args { $($field),* } =
                serde_json::from_value(args).map_err(|e| bad(stringify!($name), e))?;
            let out = $path(state, $($field),*)?;
            serde_json::to_value(out).map_err(|e| bad(stringify!($name), e))
        }
        (stringify!($name), handler as Handler)
    }};
}

/// Commands the escape hatch will not run.
///
/// Three kinds: the interface's own plumbing — the beta gate, the tile and
/// log paths, the static catalogues, the service itself and the frontend's
/// answers to a screenshot request — which edits no document and tells a
/// client nothing; the two that answer with bytes rather than JSON; and the
/// four that take a handle or the cancel flag, which are curated tools so
/// that a client gets the progress the relay carries.
///
/// Every one of them also fails to fit the table's shape — no `AppState`, a
/// different managed state, or `async` — so the exclusions and the mechanics
/// agree rather than the list papering over a signature.
pub const EXCLUDED: &[&str] = &[
    // The beta gate and the facts the shell reads about itself.
    "beta_status",
    "app_info",
    "tile_base_url",
    // Answered with bytes, not JSON.
    "basemap",
    "selection_preview",
    // The webview's own plumbing: a screenshot to the log directory, its log
    // lines, the tile addresses and the render-ahead pool.
    "save_debug_capture",
    "frontend_log",
    "tile_keys",
    "render_ahead",
    "frame_readiness",
    // Static catalogues, already in the tool schemas.
    "colour_gradients",
    "tool_palette",
    // The service itself, which a client reached through it could turn off
    // under its own feet, and the frontend's two answers to a screenshot
    // request (`deliver_capture`, `refuse_capture`). Registering the service
    // with a client is asked for by a person in Settings: a client that is
    // connected has no use for it, and one that is not cannot call it.
    "mcp_status",
    "mcp_set",
    "mcp_rotate_token",
    "mcp_register_client",
    "deliver_capture",
    "refuse_capture",
    // A handle or the cancel flag rather than the state alone. The curated
    // tools carry these, with the progress a client needs.
    "cancel_export",
    "export_grib",
    "export_zarr",
    "import_history",
    // No state to hand it, which is the one shape the table holds; the
    // curated tool of the same name carries it.
    "history_archives",
];

/// Every command reachable by name.
pub const TABLE: &[(&str, Handler)] = &[
    command!(new_project, crate::projects::new_project, { request: crate::projects::NewProjectRequest, discard_unsaved: bool }),
    command!(open_project, crate::projects::open_project, { path: String, discard_unsaved: bool }),
    command!(save_project, crate::projects::save_project, {}),
    command!(save_project_as, crate::projects::save_project_as, { path: String }),
    command!(close_project, crate::projects::close_project, { discard_unsaved: bool }),
    command!(current_project, crate::projects::current_project, {}),
    command!(recent_projects, crate::projects::recent_projects, {}),
    command!(
        clear_recent_projects,
        crate::projects::clear_recent_projects,
        {}
    ),
    command!(sample_field, crate::commands::sample_field, { lon: f64, lat: f64, step: u32, kind: Option<String> }),
    command!(create_object, crate::create::create_object, { object: crate::create::NewObject }),
    command!(add_brush_stroke, crate::edit::add_brush_stroke, { stroke: crate::edit::BrushStroke }),
    command!(undo, crate::edit::undo, {}),
    command!(redo, crate::edit::redo, {}),
    command!(export_estimate, crate::export::export_estimate, {}),
    command!(import_grib, crate::import::import_grib, { path: String }),
    command!(new_project_from_grib, crate::import::new_project_from_grib, { path: String, discard_unsaved: bool }),
    command!(import_zarr, crate::zarr::import_zarr, { path: String }),
    command!(new_project_from_zarr, crate::zarr::new_project_from_zarr, { path: String, discard_unsaved: bool }),
    command!(import_gis, crate::charts::import_gis, { path: String }),
    command!(set_gis_style, crate::charts::set_gis_style, { layer: u64, colour: Option<String>, width_px: Option<f64>, fill_opacity: Option<f64> }),
    command!(set_chart_directory, crate::settings::set_chart_directory, { directory: String }),
    command!(chart_status, crate::settings::chart_status, {}),
    command!(document_tree, crate::document::document_tree, { step: u32 }),
    command!(object_properties, crate::document::object_properties, { object: u64, step: u32 }),
    command!(set_liquify_destination, crate::document::set_liquify_destination, { object: u64, to: [f64; 2], step: u32, auto_key: bool, from: Option<[f64; 2]> }),
    command!(set_object_property, crate::document::set_object_property, { object: u64, property: String, value: crate::document::PropertyValue, step: u32, auto_key: bool, gesture: Option<String> }),
    command!(rename_project, crate::document::rename_project, { name: String }),
    command!(add_layer, crate::document::add_layer, { name: String }),
    command!(remove_layer, crate::document::remove_layer, { layer: u64 }),
    command!(rename_layer, crate::document::rename_layer, { layer: u64, name: String }),
    command!(set_layer_visible, crate::document::set_layer_visible, { layer: u64, visible: bool }),
    command!(set_layer_parameter, crate::document::set_layer_parameter, { layer: u64, parameter: String }),
    command!(set_layer_speed_range, crate::document::set_layer_speed_range, { layer: u64, min_mps: Option<f32>, max_mps: Option<f32>, gesture: Option<String> }),
    command!(set_layer_locked, crate::document::set_layer_locked, { layer: u64, locked: bool }),
    command!(move_layer, crate::document::move_layer, { from: usize, to: usize }),
    command!(rename_object, crate::document::rename_object, { object: u64, name: String }),
    command!(remove_object, crate::document::remove_object, { object: u64 }),
    command!(remove_objects, crate::document::remove_objects, { objects: Vec<u64> }),
    command!(move_object, crate::document::move_object, { object: u64, layer: u64, index: usize }),
    command!(duplicate_object, crate::document::duplicate_object, { object: u64 }),
    command!(set_active_range, crate::document::set_active_range, { object: u64, start: u32, end: u32 }),
    command!(object_at, crate::document::object_at, { lon: f64, lat: f64, step: u32 }),
    command!(selection_transform, crate::transform::selection_transform, { objects: Vec<u64>, step: u32 }),
    command!(begin_transform, crate::transform::begin_transform, { objects: Vec<u64>, step: u32, kind: crate::transform::TransformKind, lon: f64, lat: f64, auto_key: Option<bool> }),
    command!(preview_transform, crate::transform::preview_transform, { lon: f64, lat: f64 }),
    command!(drag_transform, crate::transform::drag_transform, { lon: f64, lat: f64 }),
    command!(objects_in_region, crate::transform::objects_in_region, { west: f64, south: f64, east: f64, north: f64, step: u32, layer: Option<u64> }),
    command!(object_outlines, crate::transform::object_outlines, { step: u32, tool: Option<crate::create::Tool>, objects: Vec<u64>, layer: Option<u64>, all_in_layer: bool }),
    command!(end_gesture, crate::document::end_gesture, {}),
    command!(copy_objects, crate::document::copy_objects, { objects: Vec<u64>, step: u32 }),
    command!(cut_objects, crate::document::cut_objects, { objects: Vec<u64>, step: u32 }),
    command!(paste_objects, crate::document::paste_objects, { layer: Option<u64>, step: u32, absolute_timing: bool, still: bool }),
    command!(erase_stroke, crate::document::erase_stroke, { stroke: crate::document::EraseStroke }),
    command!(clipboard_state, crate::document::clipboard_state, {}),
    command!(clipboard_kind, crate::document::clipboard_kind, {}),
    command!(history_view, crate::document::history_view, {}),
    command!(jump_to_history, crate::document::jump_to_history, { target: usize }),
    command!(app_settings, crate::settings::app_settings, {}),
    command!(set_theme, crate::settings::set_theme, { theme: String }),
    command!(set_custom_theme, crate::settings::set_custom_theme, { custom: crate::theme::CustomTheme }),
    command!(set_shortcut, crate::settings::set_shortcut, { binding: crate::settings::Shortcut }),
    command!(reset_shortcuts, crate::settings::reset_shortcuts, {}),
    command!(set_default_scales, crate::settings::set_default_scales, { wind_knots: f64, current_knots: f64 }),
    command!(set_macro_directory, crate::settings::set_macro_directory, { directory: String }),
    command!(set_autosave_mode, crate::settings::set_autosave_mode, { mode: crate::settings::AutosaveMode }),
    command!(set_projection, crate::settings::set_projection, { projection: String }),
    command!(set_auto_scale, crate::settings::set_auto_scale, { on: bool }),
    command!(set_display_units, crate::settings::set_display_units, { distance_unit: crate::settings::DistanceUnit, speed_unit: crate::settings::SpeedUnit }),
    command!(set_glyph_appearance, crate::settings::set_glyph_appearance, { style: crate::settings::GlyphStyle, setting: crate::settings::GlyphSetting }),
    command!(set_colour_scale, crate::settings::set_colour_scale, { kind: String, max_knots: f64 }),
    command!(set_colour_gradient, crate::settings::set_colour_gradient, { kind: String, gradient: String }),
    command!(autosaves, crate::autosave::autosaves, {}),
    command!(recover_autosave, crate::autosave::recover_autosave, { id: u64, discard_unsaved: bool }),
    command!(discard_autosave, crate::autosave::discard_autosave, { id: u64 }),
    command!(import_image, crate::image::import_image, { path: String, view: Option<[f64; 4]> }),
    command!(resize_image, crate::image::resize_image, { layer: u64, handle: u8, from: [f64; 2], to: [f64; 2], gesture: String }),
    command!(move_image, crate::image::move_image, { layer: u64, lon: f64, lat: f64, gesture: Option<String> }),
    command!(set_image_control_points, crate::image::set_image_control_points, { layer: u64, points: Vec<[f64; 4]> }),
    command!(set_image_opacity, crate::image::set_image_opacity, { layer: u64, opacity: f64 }),
    command!(reset_image_placement, crate::image::reset_image_placement, { layer: u64 }),
    command!(measurements, crate::measure::measurements, {}),
    command!(add_measurement, crate::measure::add_measurement, { measurement: crate::measure::NewMeasurement }),
    command!(preview_measurement, crate::measure::preview_measurement, { measurement: crate::measure::NewMeasurement }),
    command!(move_measurement_handle, crate::measure::move_measurement_handle, { id: u64, index: usize, at: [f64; 2] }),
    command!(extend_measurement, crate::measure::extend_measurement, { id: u64, at: [f64; 2] }),
    command!(set_measurement_rings, crate::measure::set_measurement_rings, { id: u64, interval_km: f64, count: u32 }),
    command!(remove_measurement, crate::measure::remove_measurement, { id: u64 }),
    command!(clear_measurements, crate::measure::clear_measurements, { kind: Option<crate::measure::MeasurementKind> }),
    command!(macro_library, crate::macros::macro_library, {}),
    command!(delete_macros, crate::macros::delete_macros, { id: Option<String> }),
    command!(start_capture, crate::macros::start_capture, { region: crate::capture::RegionShape, step: u32, record_movement: bool, kind: Option<String> }),
    command!(place_capture, crate::macros::place_capture, { step: u32, lon: f64, lat: f64 }),
    command!(visit_capture, crate::macros::visit_capture, { step: u32 }),
    command!(unplace_capture, crate::macros::unplace_capture, { step: u32 }),
    command!(preview_capture, crate::macros::preview_capture, { last_step: u32 }),
    command!(place_preview, crate::macros::place_preview, { lon: f64, lat: f64 }),
    command!(edit_capture, crate::macros::edit_capture, {}),
    command!(cancel_capture, crate::macros::cancel_capture, {}),
    command!(capture_mode, crate::macros::capture_mode, { step: Option<u32> }),
    command!(finish_capture, crate::macros::finish_capture, { name: String, last_step: u32 }),
    command!(insert_macro, crate::macros::insert_macro, { id: String, lon: f64, lat: f64, step: u32, layer: Option<u64> }),
    command!(capture_region, crate::capture::capture_region, { region: crate::capture::RegionShape, step: u32, kind: Option<String> }),
    command!(paste_capture, crate::capture::paste_capture, { lon: Option<f64>, lat: Option<f64>, step: u32, layer: Option<u64>, still: bool }),
    command!(capture_state, crate::capture::capture_state, {}),
    command!(copy_grib_frames, crate::frames::copy_grib_frames, { layer: u64, steps: Vec<u32> }),
    command!(paste_grib_frames, crate::frames::paste_grib_frames, { at: u32 }),
    command!(delete_grib_frames, crate::frames::delete_grib_frames, { layer: u64, steps: Vec<u32> }),
    command!(
        frame_clipboard_state,
        crate::frames::frame_clipboard_state,
        {}
    ),
    command!(shape_controls, crate::shape_animation::shape_controls, { object: u64, step: u32 }),
    command!(move_shape_point, crate::shape_animation::move_shape_point, { object: u64, step: u32, ring: usize, point: usize, lon: f64, lat: f64, revision: u64 }),
    command!(object_tracks, crate::animation::object_tracks, { object: u64, step: u32 }),
    command!(track_samples, crate::animation::track_samples, { object: u64, property: String }),
    command!(set_keyframe, crate::animation::set_keyframe, { object: u64, property: String, step: u32, value: Option<crate::document::PropertyValue> }),
    command!(add_constant_motion, crate::animation::add_constant_motion, { object: u64, step: u32, direction: f64, speed_mps: f64, overwrite: bool }),
    command!(set_motion, crate::animation::set_motion, { object: u64, property: String, on: bool }),
    command!(set_follow, crate::animation::set_follow, { object: u64, property: String, primary: Option<u64>, step: u32 }),
    command!(remove_keyframe, crate::animation::remove_keyframe, { object: u64, property: String, step: u32 }),
    command!(move_keyframe, crate::animation::move_keyframe, { object: u64, property: String, from: u32, to: u32, gesture: Option<String> }),
    command!(set_interpolation, crate::animation::set_interpolation, { object: u64, property: String, step: u32, interp: crate::animation::InterpolationView }),
    command!(step_count_impact, crate::animation::step_count_impact, { step_count: u32 }),
    command!(set_step_count, crate::animation::set_step_count, { step_count: u32 }),
    command!(set_start_time, crate::animation::set_start_time, { start_unix_s: Option<i64> }),
];

/// Runs a command by name.
pub fn call<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    name: &str,
    args: Value,
) -> Result<Value> {
    if EXCLUDED.contains(&name) {
        return Err(AppError::BadOption {
            field: "command",
            value: format!("{name} is not available through invoke"),
        });
    }
    match TABLE.iter().find(|(n, _)| *n == name) {
        Some((_, handler)) => handler(app.state::<AppState>(), args),
        None => Err(AppError::BadOption {
            field: "command",
            value: format!("unknown command {name}"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every command in `generate_handler!` is either in the table or
    /// deliberately excluded, so a command added later is reachable the day
    /// it lands or someone has said why not.
    #[test]
    fn the_table_covers_every_registered_command() {
        let lib = include_str!("../lib.rs");
        let start = lib.find("generate_handler![").expect("handler list");
        let end = lib[start..].find("];").expect("end of list") + start;
        let names: Vec<&str> = lib[start..end]
            .split(',')
            .filter_map(|entry| entry.trim().rsplit("::").next())
            .filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
            .collect();
        assert!(names.len() > 100, "parsed only {} names", names.len());
        let missing: Vec<&str> = names
            .iter()
            .copied()
            .filter(|n| !EXCLUDED.contains(n) && !TABLE.iter().any(|(t, _)| t == n))
            .collect();
        assert!(
            missing.is_empty(),
            "commands neither in TABLE nor EXCLUDED: {missing:?}"
        );
        let stale: Vec<&str> = TABLE
            .iter()
            .map(|(t, _)| *t)
            .chain(EXCLUDED.iter().copied())
            .filter(|t| !names.contains(t))
            .collect();
        assert!(stale.is_empty(), "table names no command has: {stale:?}");
    }
}

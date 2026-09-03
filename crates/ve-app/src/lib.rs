//! VectorEffects desktop application.
//!
//! Rust owns the entire domain; the webview is a view layer. Everything the
//! frontend can reach goes through [`commands`].

pub mod animation;
pub mod commands;
pub mod create;
pub mod document;
pub mod edit;
pub mod error;
pub mod export;
pub mod logging;
pub mod merge;
pub mod palette;
pub mod paths;
pub mod projects;
pub mod protocol;
pub mod render_pool;
pub mod session;
pub mod transform;

use commands::AppState;
use paths::AppPaths;

/// Starts the application. Returns only when the last window closes.
pub fn run() -> anyhow::Result<()> {
    let paths = AppPaths::resolve()?;
    // Held for the process lifetime; dropping it loses buffered log lines.
    let _log_guard = logging::init(&paths.log_dir);

    let state = AppState::new(paths);
    tracing::info!(
        backend = %state.evaluators.selection.backend,
        reason = ?state.evaluators.selection.fallback_reason,
        "starting VectorEffects"
    );

    // Fail loudly at startup rather than showing a blank map later.
    let basemap = ve_render::basemap::inspect(ve_render::basemap::EMBEDDED)?;
    tracing::info!(lods = basemap.lods.len(), "basemap asset validated");

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .manage(protocol::SceneCache::default())
        .manage(export::ExportCancel::default())
        .manage(std::sync::Arc::new(render_pool::RenderPool::new()))
        .setup(|app| {
            use tauri::{Emitter, Manager};
            // The pool renders ahead of the playhead for the life of the
            // process, and tells the frontend as each tile lands so the
            // readiness strip can catch up (spec.md 9.5).
            let pool = app.state::<std::sync::Arc<render_pool::RenderPool>>();
            let handle = app.handle().clone();
            pool.on_progress(move |progress| {
                let _ = handle.emit("render://progress", progress);
            });
            pool.start(app.handle().clone());
            Ok(())
        })
        .register_uri_scheme_protocol(protocol::SCHEME, |ctx, request| {
            protocol::handle(ctx.app_handle(), &request)
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::basemap,
            commands::tile_base_url,
            commands::sample_field,
            commands::save_debug_capture,
            commands::frontend_log,
            projects::new_project,
            projects::open_project,
            projects::save_project,
            projects::save_project_as,
            projects::close_project,
            projects::current_project,
            projects::recent_projects,
            create::create_object,
            palette::tool_palette,
            edit::add_brush_stroke,
            edit::undo,
            edit::redo,
            export::export_grib,
            export::export_estimate,
            export::cancel_export,
            document::document_tree,
            document::object_properties,
            document::set_object_property,
            document::add_layer,
            document::remove_layer,
            document::rename_layer,
            document::set_layer_visible,
            document::set_layer_locked,
            document::move_layer,
            document::rename_object,
            document::remove_object,
            document::move_object,
            document::duplicate_object,
            document::set_active_range,
            document::object_at,
            transform::selection_transform,
            transform::begin_transform,
            transform::preview_transform,
            transform::drag_transform,
            transform::objects_in_region,
            document::end_gesture,
            document::copy_objects,
            document::cut_objects,
            document::paste_objects,
            document::clipboard_state,
            document::history_view,
            document::jump_to_history,
            animation::object_tracks,
            animation::set_keyframe,
            animation::remove_keyframe,
            animation::move_keyframe,
            animation::set_interpolation,
            animation::step_count_impact,
            animation::set_step_count,
            animation::set_start_time,
            render_pool::render_ahead,
            render_pool::frame_readiness
        ])
        .run(tauri::generate_context!())?;

    Ok(())
}

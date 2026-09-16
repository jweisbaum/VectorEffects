//! VectorEffects desktop application.
//!
//! Rust owns the entire domain; the webview is a view layer. Everything the
//! frontend can reach goes through [`commands`].

pub mod animation;
pub mod autosave;
pub mod beta;
pub mod capture;
pub mod commands;
pub mod create;
pub mod document;
pub mod edit;
pub mod error;
pub mod export;
pub mod frames;
pub mod history;
pub mod image;
pub mod import;
pub mod logging;
pub mod macros;
pub mod mcp;
pub mod measure;
pub mod merge;
pub mod palette;
pub mod paths;
pub mod projects;
pub mod protocol;
pub mod render_pool;
mod screen_erasure;
pub mod selection_preview;
pub mod session;
pub mod settings;
pub mod shape_animation;
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

    let builder = tauri::Builder::default().plugin(tauri_plugin_dialog::init());
    // End-to-end automation, compiled in only when asked for (see the crate's
    // `[features]`). It binds a WebDriver endpoint on loopback, which is a way
    // into the application rather than out of it — invariant 5 in the other
    // direction — so a shipped build must not carry it, and the feature being
    // off by default is what makes sure of that.
    #[cfg(feature = "webdriver")]
    let builder = {
        tracing::warn!(
            "the WebDriver automation endpoint is compiled in; this build is for \
             testing and must not be shipped"
        );
        builder.plugin(tauri_plugin_webdriver_automation::init())
    };
    builder
        .manage(state)
        .manage(export::ExportCancel::default())
        .manage(std::sync::Arc::new(render_pool::RenderPool::new()))
        .manage(mcp::McpService::default())
        .setup(|app| {
            use tauri::{Emitter, Manager};
            if beta::expired_now() {
                return Ok(());
            }
            let menu = tauri::menu::Menu::default(app.handle())?;
            if let Some(item) = menu.get(tauri::menu::HELP_SUBMENU_ID)
                && let Some(help) = item.as_submenu()
            {
                help.append(&tauri::menu::MenuItem::with_id(
                    app,
                    "vector-help",
                    "VectorEffects Help",
                    true,
                    Some("F1"),
                )?)?;
            }
            app.set_menu(menu)?;
            // The pool renders ahead of the playhead for the life of the
            // process, and tells the frontend as each tile lands so the
            // readiness strip can catch up (spec.md 9.5).
            let pool = app.state::<std::sync::Arc<render_pool::RenderPool>>();
            let handle = app.handle().clone();
            pool.on_progress(move |progress| {
                let _ = handle.emit("render://progress", progress);
            });
            pool.start(app.handle().clone());
            // Crash recovery: a snapshot of unsaved work every minute or fifty
            // edits, offered back on the start screen (spec.md 4.2, M10).
            autosave::start(app.handle().clone());
            // The MCP service, only if the person left it on (spec.md 8.8).
            let app_state = app.state::<AppState>();
            let mcp_settings = app_state.session.lock().map(|s| s.settings.mcp.clone());
            if let Ok(mcp_settings) = mcp_settings {
                app.state::<mcp::McpService>()
                    .apply(app.handle(), &mcp_settings);
            }
            Ok(())
        })
        .on_menu_event(|app, event| {
            if event.id().as_ref() == "vector-help" && !beta::expired_now() {
                use tauri::Emitter;
                let _ = app.emit("help://open", ());
            }
        })
        // **Asynchronous, deliberately.** The synchronous variant runs the
        // handler on the main thread — the webview's own — and a tile of an
        // imported field costs tens of milliseconds to evaluate on the CPU, a
        // full viewport of them a second or more. Every one of those blocked
        // the interface for as long as it took: a slider dragged across a
        // GRIB layer stood still while the tiles its last tick asked for
        // rendered under it. The work now runs on the runtime's blocking
        // pool and the main thread is handed the response when it is ready
        // (spec.md 13: zero evaluation on the UI thread).
        .register_asynchronous_uri_scheme_protocol(protocol::SCHEME, |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                responder.respond(protocol::handle(&app, &request));
            });
        })
        .invoke_handler(|invoke: tauri::ipc::Invoke| {
            if beta::expired_now() && invoke.message.command() != "beta_status" {
                invoke.resolver.reject(beta::EXPIRED_MESSAGE);
                return true;
            }
            let handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![
                beta::beta_status,
                commands::app_info,
                commands::basemap,
                commands::tile_base_url,
                commands::sample_field,
                selection_preview::selection_preview,
                commands::save_debug_capture,
                commands::frontend_log,
                projects::new_project,
                projects::open_project,
                projects::save_project,
                projects::save_project_as,
                projects::close_project,
                projects::current_project,
                projects::recent_projects,
                projects::clear_recent_projects,
                create::create_object,
                palette::tool_palette,
                edit::add_brush_stroke,
                edit::undo,
                edit::redo,
                export::export_grib,
                export::export_zarr,
                export::export_estimate,
                export::cancel_export,
                import::import_grib,
                import::new_project_from_grib,
                history::import_history,
                document::document_tree,
                document::object_properties,
                document::set_object_property,
                document::rename_project,
                document::add_layer,
                document::remove_layer,
                document::rename_layer,
                document::set_layer_visible,
                document::set_layer_parameter,
                document::set_layer_speed_range,
                document::set_layer_locked,
                document::move_layer,
                document::rename_object,
                document::remove_object,
                document::remove_objects,
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
                document::erase_stroke,
                document::clipboard_state,
                document::clipboard_kind,
                settings::app_settings,
                settings::set_shortcut,
                settings::reset_shortcuts,
                settings::set_default_scales,
                settings::set_macro_directory,
                settings::mcp_status,
                settings::mcp_set,
                settings::mcp_rotate_token,
                settings::set_autosave_mode,
                settings::set_projection,
                settings::set_auto_scale,
                settings::set_display_units,
                settings::set_glyph_appearance,
                autosave::autosaves,
                autosave::recover_autosave,
                autosave::discard_autosave,
                image::import_image,
                image::set_image_corners,
                image::set_image_opacity,
                image::reset_image_placement,
                measure::measurements,
                measure::add_measurement,
                measure::preview_measurement,
                measure::move_measurement_handle,
                measure::extend_measurement,
                measure::set_measurement_rings,
                measure::remove_measurement,
                measure::clear_measurements,
                settings::set_colour_scale,
                settings::colour_gradients,
                settings::set_colour_gradient,
                macros::macro_library,
                macros::delete_macros,
                macros::start_capture,
                macros::place_capture,
                macros::visit_capture,
                macros::unplace_capture,
                macros::preview_capture,
                macros::place_preview,
                macros::edit_capture,
                macros::cancel_capture,
                macros::capture_mode,
                macros::finish_capture,
                macros::insert_macro,
                capture::capture_region,
                capture::paste_capture,
                capture::capture_state,
                frames::copy_grib_frames,
                frames::paste_grib_frames,
                frames::delete_grib_frames,
                frames::frame_clipboard_state,
                document::history_view,
                document::jump_to_history,
                transform::object_outlines,
                shape_animation::shape_controls,
                shape_animation::move_shape_point,
                animation::object_tracks,
                animation::track_samples,
                animation::set_keyframe,
                animation::add_constant_motion,
                animation::set_motion,
                animation::set_follow,
                animation::remove_keyframe,
                animation::move_keyframe,
                animation::set_interpolation,
                animation::step_count_impact,
                animation::set_step_count,
                animation::set_start_time,
                render_pool::render_ahead,
                render_pool::frame_readiness,
                protocol::tile_keys
            ];
            handler(invoke)
        })
        .run(tauri::generate_context!())?;

    Ok(())
}

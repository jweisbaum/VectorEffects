//! What the service tells the frontend, so the interface follows the client
//! (spec.md 8.8, M76).
//!
//! **Emitted by the service only.** A command the interface invoked returns
//! its summary to the caller; emitting here as well would refresh every
//! panel twice per edit.

use serde::Serialize;
use tauri::{Emitter, Manager};
use ts_rs::TS;

use crate::commands::AppState;
use crate::error::Result;
use crate::projects::ProjectSummary;

pub const CHANGED: &str = "document://changed";
pub const FOCUS: &str = "view://focus";
pub const STEP: &str = "view://step";
pub const SELECTION: &str = "view://selection";
pub const ACTIVITY: &str = "mcp://activity";

/// The document after a tool wrote to it. `project` null means closed.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "DocumentChanged.ts")]
pub struct DocumentChanged {
    pub project: Option<ProjectSummary>,
    /// A different project than before: the frontend resets step, selection
    /// and the active layer, as its own open path does.
    pub opened: bool,
}

/// Where the map should look.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "ViewFocus.ts")]
pub struct ViewFocus {
    pub lon: f64,
    pub lat: f64,
    /// Screen pixels per degree; null keeps the current zoom.
    pub px_per_deg: Option<f64>,
}

/// What a client is doing, for the status bar.
#[derive(Debug, Clone, Default, Serialize, TS)]
#[ts(export, export_to = "McpActivity.ts")]
pub struct McpActivity {
    pub sessions: u32,
    pub last_tool: Option<String>,
}

/// Reads the summary and tells the frontend.
///
/// Generic over the Tauri runtime so the mock application used by the
/// integration tests can drive the same path as the shipped `tauri::Wry`
/// build.
pub fn changed<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    opened: bool,
) -> Result<Option<ProjectSummary>> {
    let project = crate::projects::current(app.state::<AppState>().inner())?;
    let _ = app.emit(
        CHANGED,
        DocumentChanged {
            project: project.clone(),
            opened,
        },
    );
    Ok(project)
}

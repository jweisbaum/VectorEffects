//! The view group (spec.md 8.8): the map's camera, the timeline step, the
//! selection, and a screenshot of the map.

use base64::Engine;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{CallToolResult, ContentBlock, ErrorData as McpError};
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use tauri::Manager;

use super::{Done, StepParams, ToolError, VectorEffects, events};
use crate::commands::AppState;
use crate::error::AppError;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FocusParams {
    pub lon: f64,
    pub lat: f64,
    /// Screen pixels per degree (3 is the whole world, 60 is a bay). Null
    /// keeps the current zoom.
    pub px_per_deg: Option<f64>,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SelectionParams {
    pub objects: Vec<u64>,
}

#[tool_router(router = tool_router_view, vis = "pub(crate)")]
impl<R: tauri::Runtime> VectorEffects<R> {
    #[tool(description = "Pans the map to lon/lat, optionally at a zoom.")]
    async fn view_focus(
        &self,
        Parameters(p): Parameters<FocusParams>,
    ) -> std::result::Result<Json<Done>, ToolError> {
        ve_core::LonLat::new(p.lon, p.lat).map_err(AppError::from)?;
        self.app
            .state::<crate::mcp::McpService>()
            .note_tool(&self.app, "view_focus");
        let _ = tauri::Emitter::emit(
            &self.app,
            events::FOCUS,
            events::ViewFocus {
                lon: p.lon,
                lat: p.lat,
                px_per_deg: p.px_per_deg,
            },
        );
        Ok(Json(Done { ok: true }))
    }

    #[tool(description = "Moves the timeline to a step.")]
    async fn step_set(
        &self,
        Parameters(p): Parameters<StepParams>,
    ) -> std::result::Result<Json<Done>, ToolError> {
        let step = p.step;
        self.run("step_set", move |app| {
            let summary =
                crate::projects::current_project(app.state())?.ok_or(AppError::NoProjectOpen)?;
            if step >= summary.step_count {
                return Err(AppError::BadOption {
                    field: "step",
                    value: format!("{step} is past the last step {}", summary.step_count - 1),
                });
            }
            let _ = tauri::Emitter::emit(app, events::STEP, step);
            Ok(())
        })
        .await?;
        Ok(Json(Done { ok: true }))
    }

    #[tool(description = "Selects objects in the interface; an empty list clears the selection.")]
    async fn selection_set(
        &self,
        Parameters(p): Parameters<SelectionParams>,
    ) -> std::result::Result<Json<Done>, ToolError> {
        self.app
            .state::<crate::mcp::McpService>()
            .note_tool(&self.app, "selection_set");
        let _ = tauri::Emitter::emit(&self.app, events::SELECTION, p.objects);
        Ok(Json(Done { ok: true }))
    }

    #[tool(
        description = "The map as the interface shows it, once its tiles have settled: a PNG. Needs an open project."
    )]
    async fn screenshot(&self) -> std::result::Result<CallToolResult, ToolError> {
        // Neither `run` nor `write`: the work is the frontend's, so there is
        // no closure to put on `spawn_blocking` and no document to report a
        // change to. The activity note is what the other two would have done.
        let service = self.app.state::<crate::mcp::McpService>();
        service.note_tool(&self.app, "screenshot");
        // With no project the map is not mounted and nothing would answer;
        // refuse now rather than after the timeout.
        if crate::projects::current(&self.app.state::<AppState>())?.is_none() {
            return Err(ToolError::from(AppError::NoProjectOpen));
        }
        // The guard forgets the request on every way out of here, the
        // dropped future included, so nothing has to be cleaned up by hand.
        let mut pending = service.captures.request(&self.app);
        // The frontend waits up to 10 s for tiles and 20 s for a frame (M78);
        // a little longer than both, then give up rather than hang the client.
        let answer =
            tokio::time::timeout(std::time::Duration::from_secs(35), pending.receiver()).await;
        match answer {
            Ok(Ok(Ok(png))) => {
                let data = base64::engine::general_purpose::STANDARD.encode(png);
                Ok(CallToolResult::success(vec![ContentBlock::image(
                    data,
                    "image/png",
                )]))
            }
            Ok(Ok(Err(reason))) => Err(ToolError::Refused(format!(
                "the map could not take the picture: {reason}"
            ))),
            _ => Err(ToolError::Internal(McpError::internal_error(
                "the map did not answer the capture: is the window shown?",
                None,
            ))),
        }
    }
}

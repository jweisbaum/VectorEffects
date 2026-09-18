//! The history group (spec.md 8.8): undo, redo and jumping through it.

use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use tauri::Manager;

use super::{ToolError, VectorEffects};
use crate::projects::ProjectSummary;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HistoryJumpParams {
    pub target: usize,
}

#[tool_router(router = tool_router_history, vis = "pub(crate)")]
impl<R: tauri::Runtime> VectorEffects<R> {
    #[tool(description = "Undoes the last edit.")]
    async fn undo(&self) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("undo", false, |app| crate::edit::undo(app.state()))
            .await
            .map(Json)
    }

    #[tool(description = "Redoes the last undone edit.")]
    async fn redo(&self) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("redo", false, |app| crate::edit::redo(app.state()))
            .await
            .map(Json)
    }

    #[tool(
        description = "The UNDO history: the edits made to this project, with the current position. Nothing to do with past weather, which is import_history."
    )]
    async fn history_list(
        &self,
    ) -> std::result::Result<Json<crate::document::HistoryView>, ToolError> {
        self.run("history_list", |app| {
            crate::document::history_view(app.state())
        })
        .await
        .map(Json)
    }

    #[tool(description = "Jumps to an entry of the undo history that history_list returns.")]
    async fn history_jump(
        &self,
        Parameters(p): Parameters<HistoryJumpParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("history_jump", false, move |app| {
            crate::document::jump_to_history(app.state(), p.target)
        })
        .await
        .map(Json)
    }
}

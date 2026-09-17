//! The project group (spec.md 8.8): opening, closing, saving, and the tool
//! catalogue.

use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tauri::Manager;

use super::{ToolError, VectorEffects};
use crate::projects::ProjectSummary;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProjectNewParams {
    /// Project name.
    pub name: String,
    /// "wind" or "current".
    pub field_kind: String,
    /// Grid resolution in degrees as a string: "1.0", "0.5", "0.25", "0.1".
    pub resolution: String,
    /// Hours between steps: 1, 3 or 6.
    pub step_hours: u32,
    /// Number of steps.
    pub step_count: u32,
    /// Discard unsaved changes in the open project. Refused without it.
    #[serde(default)]
    pub discard_unsaved: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProjectOpenParams {
    /// Path to a .veproj file.
    pub path: String,
    #[serde(default)]
    pub discard_unsaved: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProjectSaveParams {
    /// Save here instead of the project's own path (a Save As).
    pub path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DiscardParams {
    #[serde(default)]
    pub discard_unsaved: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ProjectStatus {
    /// The open project, or null.
    pub project: Option<ProjectSummary>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Closed {
    pub closed: bool,
}

#[tool_router(router = tool_router_project, vis = "pub(crate)")]
impl<R: tauri::Runtime> VectorEffects<R> {
    #[tool(
        description = "The open project's summary (name, path, dirty, grid, steps, revision, undo state), or null when none is open."
    )]
    async fn project_status(&self) -> std::result::Result<Json<ProjectStatus>, ToolError> {
        let project = self
            .run("project_status", |app| {
                crate::projects::current_project(app.state())
            })
            .await?;
        Ok(Json(ProjectStatus { project }))
    }

    #[tool(
        description = "Creates a new project and opens it in the interface. Refused while the open project has unsaved changes unless discard_unsaved is true."
    )]
    async fn project_new(
        &self,
        Parameters(p): Parameters<ProjectNewParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        let request = crate::projects::NewProjectRequest {
            name: p.name,
            field_kind: p.field_kind,
            resolution: p.resolution,
            step_hours: p.step_hours,
            step_count: p.step_count,
        };
        let discard = p.discard_unsaved;
        self.write("project_new", true, move |app| {
            crate::projects::new_project(app.state(), request, discard)
        })
        .await
        .map(Json)
    }

    #[tool(description = "Opens a .veproj file in the interface.")]
    async fn project_open(
        &self,
        Parameters(p): Parameters<ProjectOpenParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("project_open", true, move |app| {
            crate::projects::open_project(app.state(), p.path, p.discard_unsaved)
        })
        .await
        .map(Json)
    }

    #[tool(description = "Saves the project to its own path, or to `path` as a Save As.")]
    async fn project_save(
        &self,
        Parameters(p): Parameters<ProjectSaveParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("project_save", false, move |app| match p.path {
            Some(path) => crate::projects::save_project_as(app.state(), path),
            None => crate::projects::save_project(app.state()),
        })
        .await
        .map(Json)
    }

    #[tool(
        description = "Closes the project and returns the interface to the start screen. Refused with unsaved changes unless discard_unsaved is true."
    )]
    async fn project_close(
        &self,
        Parameters(p): Parameters<DiscardParams>,
    ) -> std::result::Result<Json<Closed>, ToolError> {
        self.write("project_close", true, move |app| {
            crate::projects::close_project(app.state(), p.discard_unsaved)
        })
        .await?;
        Ok(Json(Closed { closed: true }))
    }

    #[tool(description = "Recently opened projects, newest first.")]
    async fn recent_projects(
        &self,
    ) -> std::result::Result<Json<Vec<crate::projects::RecentProject>>, ToolError> {
        self.run("recent_projects", |app| {
            crate::projects::recent_projects(app.state())
        })
        .await
        .map(Json)
    }

    #[tool(
        description = "Every drawing tool with its options, defaults, ranges and gesture shape. Read this before object_create."
    )]
    async fn tool_catalogue(
        &self,
    ) -> std::result::Result<Json<Vec<crate::palette::ToolSchema>>, ToolError> {
        self.run("tool_catalogue", |_| crate::palette::tool_palette())
            .await
            .map(Json)
    }
}

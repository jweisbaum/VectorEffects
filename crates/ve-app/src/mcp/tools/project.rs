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
    /// Hours between steps: 1, 3, 6 or 24. Fixed for the life of the
    /// project. 6 suits an event of several days, 1 or 3 a day or two.
    pub step_hours: u32,
    /// Number of steps, 1 to 240; the timeline spans
    /// (step_count - 1) x step_hours hours. `import_history` resizes it to
    /// the range it downloads, so any value does before one.
    pub step_count: u32,
    /// Discard unsaved changes in the open project. Refused without it.
    #[serde(default)]
    pub discard_unsaved: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProjectOpenParams {
    /// Path to a .veproj file: absolute, or beginning with `~/`.
    pub path: String,
    #[serde(default)]
    pub discard_unsaved: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProjectSaveParams {
    /// Save here instead of the project's own path (a Save As). Absolute, or
    /// beginning with `~/`.
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

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct CatalogueParams {
    /// One tool's name ("circle", "brush", …) for that entry alone; absent
    /// or null for every tool, which is long.
    #[serde(default)]
    pub tool: Option<String>,
}
/// A tool's structured result is an object (MCP, `structuredContent`), so a
/// list travels inside one.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Catalogue {
    pub tools: Vec<crate::palette::ToolSchema>,
}
/// See [`Catalogue`].
#[derive(Debug, Serialize, JsonSchema)]
pub struct RecentProjects {
    pub projects: Vec<crate::projects::RecentProject>,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct Closed {
    pub closed: bool,
}

#[tool_router(router = tool_router_project, vis = "pub(crate)")]
impl<R: tauri::Runtime> VectorEffects<R> {
    #[tool(
        description = "What VectorEffects — the wind and ocean-current field application open on this computer — has open: the project's summary (name, path, dirty, grid, steps, revision, undo state), or null when none is open. A good first call after vectoreffects_guide: an open project that is dirty holds the user's unsaved work."
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
        description = "Starts a new VectorEffects project and opens it in the interface: the first step of making any wind or ocean-current field, GRIB2 or Zarr, whether the weather is then downloaded (import_history) or drawn (storm_create, object_create). Refused while the open project has unsaved changes unless discard_unsaved is true."
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
        let path = super::files::absolute(&p.path)?;
        self.write("project_open", true, move |app| {
            crate::projects::open_project(app.state(), path, p.discard_unsaved)
        })
        .await
        .map(Json)
    }

    #[tool(
        description = "Saves the project to its own path, or to `path` as a Save As: absolute or beginning with ~/, ending in .veproj. A project that has never been saved needs `path`. Saving is not needed before an export."
    )]
    async fn project_save(
        &self,
        Parameters(p): Parameters<ProjectSaveParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        let path = p.path.as_deref().map(super::files::absolute).transpose()?;
        self.write("project_save", false, move |app| match path {
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
    async fn recent_projects(&self) -> std::result::Result<Json<RecentProjects>, ToolError> {
        self.run("recent_projects", |app| {
            crate::projects::recent_projects(app.state())
        })
        .await
        .map(|projects| Json(RecentProjects { projects }))
    }

    #[tool(
        description = "The drawing tools with their options, defaults, ranges and gesture shape. Read a tool's entry before object_create: pass `tool` (\"circle\", \"brush\", \"curve\", \"shape_fill\", \"mask\", …) for that one entry, or nothing for all of them. A choice option's value is {\"kind\":\"choice\",\"index\":i}, i indexing the option's `variants`; `depends_on` says which choice makes an option live."
    )]
    async fn tool_catalogue(
        &self,
        Parameters(p): Parameters<CatalogueParams>,
    ) -> std::result::Result<Json<Catalogue>, ToolError> {
        let mut tools = self
            .run("tool_catalogue", |_| crate::palette::tool_palette())
            .await?;
        if let Some(wanted) = p.tool {
            // A tool's name as the catalogue spells it, which is how a
            // client names it back.
            let name_of = |tool: &crate::palette::ToolSchema| {
                serde_json::to_value(tool.tool)
                    .ok()
                    .and_then(|name| name.as_str().map(str::to_owned))
            };
            let known: Vec<String> = tools.iter().filter_map(name_of).collect();
            tools.retain(|tool| name_of(tool).as_deref() == Some(wanted.as_str()));
            if tools.is_empty() {
                return Err(ToolError::Refused(format!(
                    "there is no tool named {wanted:?}; the tools are {}",
                    known.join(", ")
                )));
            }
        }
        Ok(Json(Catalogue { tools }))
    }
}

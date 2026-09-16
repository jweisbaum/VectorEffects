//! The tools a client sees (spec.md 8.8).
//!
//! Every tool calls the command the interface calls, with the `State` the
//! handle gives, so there is one implementation of each feature. A tool that
//! writes ends with `write`, which emits `document://changed`.

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::IntoCallToolResult;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{
    CallToolResponse, CallToolResult, ContentBlock, ErrorData as McpError, Implementation,
    ServerCapabilities, ServerConfig,
};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::Manager;

use super::events;
use crate::commands::AppState;
use crate::error::AppError;
use crate::projects::ProjectSummary;

/// One handler per client session, holding the application it drives.
///
/// Generic over the Tauri runtime so the mock application used by the
/// integration tests (`tauri::test::MockRuntime`) can drive the same code
/// path as the shipped `tauri::Wry` build.
///
/// Deliberately not `Clone`: the service counts live sessions in `Drop`
/// (`session_delta`), and a clone would make that count wrong.
pub struct VectorEffects<R: tauri::Runtime> {
    pub(crate) app: tauri::AppHandle<R>,
    tool_router: ToolRouter<Self>,
}

impl<R: tauri::Runtime> std::fmt::Debug for VectorEffects<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VectorEffects").finish_non_exhaustive()
    }
}

impl<R: tauri::Runtime> Drop for VectorEffects<R> {
    fn drop(&mut self) {
        self.app
            .state::<super::McpService>()
            .session_delta(&self.app, -1);
    }
}

/// A tool's failure, in the two shapes MCP distinguishes.
///
/// The controller's ruling (task 3): a refused command reaches the client as
/// a **tool result** with `is_error: true` and the `AppError` text as its
/// content, never as a JSON-RPC protocol error — a model reading the result
/// needs to see *why* `project_new` was refused. A protocol error stays for
/// what is not the request's fault: a panic in `spawn_blocking`, or (handled
/// by `rmcp` itself, never constructed here) a malformed request or an
/// unknown tool name.
pub(crate) enum ToolError {
    /// An `AppError`'s message, and a bad `values`/`gesture` payload's parse
    /// error alongside it — both are things the caller should read and can
    /// act on, so both are refusals rather than protocol errors.
    Refused(String),
    /// The tool machinery's own failure, not the request's.
    Internal(McpError),
}

impl From<AppError> for ToolError {
    fn from(err: AppError) -> Self {
        ToolError::Refused(err.to_string())
    }
}

impl IntoCallToolResult for ToolError {
    fn into_call_tool_result(self) -> std::result::Result<CallToolResponse, McpError> {
        match self {
            ToolError::Refused(message) => {
                Ok(CallToolResult::error(vec![ContentBlock::text(message)]).into())
            }
            ToolError::Internal(err) => Err(err),
        }
    }
}

impl<R: tauri::Runtime> VectorEffects<R> {
    /// Runs a command off the async thread, since commands lock the session.
    pub(crate) async fn run<T: Send + 'static>(
        &self,
        name: &'static str,
        f: impl FnOnce(&tauri::AppHandle<R>) -> crate::error::Result<T> + Send + 'static,
    ) -> std::result::Result<T, ToolError> {
        let app = self.app.clone();
        app.state::<super::McpService>().note_tool(&app, name);
        tokio::task::spawn_blocking(move || f(&app))
            .await
            .map_err(|e| {
                ToolError::Internal(McpError::internal_error(
                    format!("tool panicked: {e}"),
                    None,
                ))
            })?
            .map_err(ToolError::from)
    }

    /// `run`, then tell the frontend the document changed.
    ///
    /// Emits whether the closure succeeded or refused partway through: a
    /// multi-command tool (`layer_set`, `object_set`) applies one command per
    /// field, so a later field refusing still leaves the earlier ones
    /// committed, and the frontend must not miss that the document changed
    /// (task 3 review, fix round 1, finding 2). The closure's own error is
    /// what the caller needs to see, so a failure to emit alongside it is
    /// swallowed rather than replacing that error.
    pub(crate) async fn write<T: Send + 'static>(
        &self,
        name: &'static str,
        opened: bool,
        f: impl FnOnce(&tauri::AppHandle<R>) -> crate::error::Result<T> + Send + 'static,
    ) -> std::result::Result<T, ToolError> {
        let result = self.run(name, f).await;
        let emitted = events::changed(&self.app, opened);
        match result {
            Ok(out) => {
                emitted.map_err(ToolError::from)?;
                Ok(out)
            }
            Err(err) => Err(err),
        }
    }
}

// ---------------------------------------------------------------- project

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

#[tool_router]
impl<R: tauri::Runtime> VectorEffects<R> {
    pub fn new(app: tauri::AppHandle<R>) -> Self {
        Self {
            app,
            tool_router: Self::tool_router(),
        }
    }

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

    // ------------------------------------------------------------- structure

    #[tool(
        description = "Layers and their objects at a step: ids, names, kinds, visibility, locks. Ids are what every other tool takes."
    )]
    async fn layers_list(
        &self,
        Parameters(p): Parameters<StepParams>,
    ) -> std::result::Result<Json<crate::document::DocumentTree>, ToolError> {
        self.run("layers_list", move |app| {
            crate::document::document_tree(app.state(), p.step)
        })
        .await
        .map(Json)
    }

    #[tool(description = "Adds an empty painted layer on top and returns the summary.")]
    async fn layer_add(
        &self,
        Parameters(p): Parameters<NameParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("layer_add", false, move |app| {
            crate::document::add_layer(app.state(), p.name)
        })
        .await
        .map(Json)
    }

    #[tool(
        description = "Sets any of a layer's name, visibility, lock, parameter (\"wind\"/\"current\") or speed range in m/s; each field set is its own undo step. Giving only one of min_mps/max_mps keeps the layer's other current bound; refused (writing nothing) if the layer has no existing band to fill it from. Omitted fields are left alone."
    )]
    async fn layer_set(
        &self,
        Parameters(p): Parameters<LayerSetParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("layer_set", false, move |app| {
            let state = app.state::<AppState>();

            // Resolved and validated before any sub-write runs (ruling,
            // task 3 review, fix round 1, findings 1 and 2c): a lone bound
            // fills its partner from the layer's *current* band rather than
            // clearing it, since `set_layer_speed_range` treats anything but
            // (Some, Some) as "no filter". A lone bound with no existing band
            // to fill from is refused before `name`/`visible`/`locked`/
            // `parameter` ever write, so a bad call leaves nothing committed.
            let bounds = if p.min_mps.is_some() || p.max_mps.is_some() {
                let tree = crate::document::tree(state.inner(), 0)?;
                let layer = tree
                    .layers
                    .iter()
                    .find(|l| l.id == p.layer)
                    .ok_or(AppError::Core(ve_core::CoreError::MissingLayer(p.layer)))?;
                let current = layer.speed_filter.as_ref();
                let min = p
                    .min_mps
                    .or_else(|| current.and_then(|f| f.speed_min_mps));
                let max = p
                    .max_mps
                    .or_else(|| current.and_then(|f| f.speed_max_mps));
                match (min, max) {
                    (Some(min), Some(max)) => Some((min, max)),
                    _ => {
                        return Err(AppError::BadOption {
                            field: "min_mps/max_mps",
                            value: "one bound was given but the layer has no band to fill the other end from".to_owned(),
                        });
                    }
                }
            } else {
                None
            };

            let mut last = None;
            if let Some(name) = p.name {
                last = Some(crate::document::rename_layer(state.clone(), p.layer, name)?);
            }
            if let Some(visible) = p.visible {
                last = Some(crate::document::set_layer_visible(
                    state.clone(),
                    p.layer,
                    visible,
                )?);
            }
            if let Some(locked) = p.locked {
                last = Some(crate::document::set_layer_locked(
                    state.clone(),
                    p.layer,
                    locked,
                )?);
            }
            if let Some(parameter) = p.parameter {
                last = Some(crate::document::set_layer_parameter(
                    state.clone(),
                    p.layer,
                    parameter,
                )?);
            }
            if let Some((min, max)) = bounds {
                last = Some(crate::document::set_layer_speed_range(
                    state.clone(),
                    p.layer,
                    Some(min),
                    Some(max),
                    None,
                )?);
            }
            match last {
                Some(summary) => Ok(summary),
                None => crate::projects::current_project(state)?.ok_or(AppError::NoProjectOpen),
            }
        })
        .await
        .map(Json)
    }

    #[tool(description = "Moves a layer from one index to another. Index 0 is the bottom.")]
    async fn layer_move(
        &self,
        Parameters(p): Parameters<MoveParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("layer_move", false, move |app| {
            crate::document::move_layer(app.state(), p.from, p.to)
        })
        .await
        .map(Json)
    }

    #[tool(description = "Removes a layer and everything on it. Undoable.")]
    async fn layer_remove(
        &self,
        Parameters(p): Parameters<LayerParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("layer_remove", false, move |app| {
            crate::document::remove_layer(app.state(), p.layer)
        })
        .await
        .map(Json)
    }

    #[tool(description = "Objects on one layer, or on every layer, at a step.")]
    async fn objects_list(
        &self,
        Parameters(p): Parameters<ObjectsListParams>,
    ) -> std::result::Result<Json<ObjectsList>, ToolError> {
        let layer = p.layer;
        let tree = self
            .run("objects_list", move |app| {
                crate::document::document_tree(app.state(), p.step)
            })
            .await?;
        let objects = tree
            .layers
            .into_iter()
            .filter(|l| layer.is_none_or(|id| id == l.id))
            .flat_map(|l| l.objects)
            .collect();
        Ok(Json(ObjectsList { objects }))
    }

    #[tool(
        description = "An object's properties at a step, with kinds, units, ranges and whether each is keyed."
    )]
    async fn object_get(
        &self,
        Parameters(p): Parameters<ObjectStepParams>,
    ) -> std::result::Result<Json<ObjectProperties>, ToolError> {
        let properties = self
            .run("object_get", move |app| {
                crate::document::object_properties(app.state(), p.object, p.step)
            })
            .await?;
        Ok(Json(ObjectProperties { properties }))
    }

    #[tool(
        description = "Draws an object with a tool. `tool` is a name from tool_catalogue; `gesture` is that tool's gesture ({\"kind\":\"point\",\"at\":[lon,lat]}, {\"kind\":\"stroke\",\"points\":[[lon,lat],...]}, or the catalogue's drag form); `options` is a list of {\"property\",\"value\"} pairs (value tagged as object_set's are), omitted ones take defaults; `layer` null means the top layer. Returns the summary and the new object's id, which becomes the selection."
    )]
    async fn object_create(
        &self,
        Parameters(p): Parameters<ObjectCreateParams>,
    ) -> std::result::Result<Json<crate::create::Created>, ToolError> {
        let object: crate::create::NewObject = serde_json::from_value(serde_json::json!({
            "tool": p.tool,
            "gesture": p.gesture,
            "options": p.options,
            "layer": p.layer
        }))
        .map_err(|e| ToolError::Refused(format!("object_create: {e}")))?;
        let created = self
            .write("object_create", false, move |app| {
                crate::create::create_object(app.state(), object)
            })
            .await?;
        let _ = tauri::Emitter::emit(&self.app, events::SELECTION, vec![created.object]);
        Ok(Json(created))
    }

    #[tool(
        description = "Sets one or more properties of an object at a step; each is applied as its own command and its own undo step. `values` maps property name (from object_get's `id`) to a tagged value: {\"kind\":\"number\",\"value\":n}, {\"kind\":\"bool\",\"value\":b}, {\"kind\":\"angle\",\"degrees\":d}, {\"kind\":\"position\",\"lon\":x,\"lat\":y} or {\"kind\":\"choice\",\"index\":i}. Every value is parsed and validated before any is applied, so a malformed one writes nothing. With auto_key true a change on an animated property adds a keyframe at that step."
    )]
    async fn object_set(
        &self,
        Parameters(p): Parameters<ObjectSetParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        let ObjectSetParams {
            object,
            step,
            values,
            auto_key,
        } = p;
        // Parsed before any sub-write runs (ruling, task 3 review, fix
        // round 1, finding 2b): a malformed value used to be caught only
        // when its own turn in the loop came around, after any values ahead
        // of it in the map had already written.
        let parsed = values
            .into_iter()
            .map(|(property, raw)| {
                let value: crate::document::PropertyValue = serde_json::from_value(raw)
                    .map_err(|e| ToolError::Refused(format!("values.{property}: {e}")))?;
                Ok((property, value))
            })
            .collect::<std::result::Result<Vec<_>, ToolError>>()?;
        if parsed.is_empty() {
            return Err(ToolError::Refused("values is empty".to_owned()));
        }
        self.write("object_set", false, move |app| {
            let state = app.state::<AppState>();
            let mut last = None;
            for (property, value) in parsed {
                last = Some(crate::document::set_object_property(
                    state.clone(),
                    object,
                    property,
                    value,
                    step,
                    auto_key,
                    None,
                )?);
            }
            crate::document::end_gesture(state.clone())?;
            last.ok_or(AppError::BadOption {
                field: "values",
                value: "empty".to_owned(),
            })
        })
        .await
        .map(Json)
    }

    #[tool(description = "Moves an object to a layer at an index (0 = bottom of that layer).")]
    async fn object_move(
        &self,
        Parameters(p): Parameters<ObjectMoveParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("object_move", false, move |app| {
            crate::document::move_object(app.state(), p.object, p.layer, p.index)
        })
        .await
        .map(Json)
    }

    #[tool(description = "Duplicates an object in place.")]
    async fn object_duplicate(
        &self,
        Parameters(p): Parameters<ObjectParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("object_duplicate", false, move |app| {
            crate::document::duplicate_object(app.state(), p.object)
        })
        .await
        .map(Json)
    }

    #[tool(description = "Removes objects. One undo.")]
    async fn object_remove(
        &self,
        Parameters(p): Parameters<ObjectsParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("object_remove", false, move |app| {
            crate::document::remove_objects(app.state(), p.objects)
        })
        .await
        .map(Json)
    }

    #[tool(
        description = "Ids of the objects whose footprint touches a lon/lat box at a step, optionally on one layer."
    )]
    async fn objects_in_region(
        &self,
        Parameters(p): Parameters<RegionParams>,
    ) -> std::result::Result<Json<ObjectIds>, ToolError> {
        let objects = self
            .run("objects_in_region", move |app| {
                crate::transform::objects_in_region(
                    app.state(),
                    p.west,
                    p.south,
                    p.east,
                    p.north,
                    p.step,
                    p.layer,
                )
            })
            .await?;
        Ok(Json(ObjectIds { objects }))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StepParams {
    pub step: u32,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct NameParams {
    pub name: String,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct LayerParams {
    pub layer: u64,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ObjectParams {
    pub object: u64,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ObjectsParams {
    pub objects: Vec<u64>,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ObjectStepParams {
    pub object: u64,
    pub step: u32,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MoveParams {
    pub from: usize,
    pub to: usize,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct LayerSetParams {
    pub layer: u64,
    pub name: Option<String>,
    pub visible: Option<bool>,
    pub locked: Option<bool>,
    pub parameter: Option<String>,
    pub min_mps: Option<f32>,
    pub max_mps: Option<f32>,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ObjectsListParams {
    pub step: u32,
    pub layer: Option<u64>,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ObjectCreateParams {
    pub tool: String,
    pub gesture: Value,
    #[serde(default)]
    pub options: Vec<Value>,
    pub layer: Option<u64>,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ObjectSetParams {
    pub object: u64,
    pub step: u32,
    pub values: serde_json::Map<String, Value>,
    #[serde(default)]
    pub auto_key: bool,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ObjectMoveParams {
    pub object: u64,
    pub layer: u64,
    pub index: usize,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RegionParams {
    pub west: f64,
    pub south: f64,
    pub east: f64,
    pub north: f64,
    pub step: u32,
    pub layer: Option<u64>,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct ObjectsList {
    pub objects: Vec<crate::document::ObjectNode>,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct ObjectProperties {
    pub properties: Vec<crate::document::PropertyView>,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct ObjectIds {
    pub objects: Vec<u64>,
}

#[tool_handler(router = self.tool_router.clone())]
impl<R: tauri::Runtime> ServerHandler for VectorEffects<R> {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "VectorEffects",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "Paints global wind and current fields. Open or create a project first; every \
                 edit is undoable and shows on the map.",
            )
    }

    /// Counts the session once the client has finished initialising, paired
    /// with the decrement in `Drop` when the session ends.
    fn on_initialized(
        &self,
        _context: rmcp::service::NotificationContext<rmcp::RoleServer>,
    ) -> impl std::future::Future<Output = ()> + Send + '_ {
        self.app
            .state::<super::McpService>()
            .session_delta(&self.app, 1);
        std::future::ready(())
    }
}

//! The tools a client sees (spec.md 8.8).
//!
//! Every tool calls the command the interface calls, with the `State` the
//! handle gives, so there is one implementation of each feature. A tool that
//! writes ends with `write`, which emits `document://changed`.

use base64::Engine;
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

    /// Forwards a Tauri progress event to the client's progress token for
    /// the life of one tool call.
    ///
    /// The command emits as it always did; the interface's own bar and the
    /// client's both see it, so there is no second progress path to keep in
    /// step with the first. `map` turns one event into the notification's
    /// (progress, total, message).
    ///
    /// A client that asked for no progress token gets a relay that listens
    /// to nothing: the events would have nowhere to go.
    pub(crate) fn relay_progress<P: serde::de::DeserializeOwned + Send + 'static>(
        &self,
        event: &'static str,
        ctx: rmcp::service::RequestContext<rmcp::RoleServer>,
        map: impl Fn(&P) -> (f64, Option<f64>, Option<String>) + Send + Sync + 'static,
    ) -> ProgressRelay<R> {
        use tauri::Listener;
        let Some(token) = ctx.meta.get_progress_token() else {
            return ProgressRelay {
                app: self.app.clone(),
                id: None,
            };
        };
        let peer = ctx.peer.clone();
        let id = self.app.listen(event, move |e| {
            if let Ok(payload) = serde_json::from_str::<P>(e.payload()) {
                let (progress, total, message) = map(&payload);
                let peer = peer.clone();
                let token = token.clone();
                // Spawned rather than awaited: a Tauri listener is a
                // synchronous callback, and it runs on whichever thread the
                // export is emitting from.
                tauri::async_runtime::spawn(async move {
                    let mut param = rmcp::model::ProgressNotificationParam::new(token, progress);
                    param.total = total;
                    param.message = message;
                    let _ = peer.notify_progress(param).await;
                });
            }
        });
        ProgressRelay {
            app: self.app.clone(),
            id: Some(id),
        }
    }
}

/// Stops the relay when the call ends.
///
/// A listener outliving its call would forward another caller's export to a
/// progress token nobody is reading any more.
pub(crate) struct ProgressRelay<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
    id: Option<tauri::EventId>,
}

impl<R: tauri::Runtime> ProgressRelay<R> {
    /// Ends the relay. Dropping it does exactly this; the method is here so
    /// the call site says when the call is over rather than relying on where
    /// the binding happens to fall.
    pub(crate) fn stop(self) {
        drop(self);
    }
}

impl<R: tauri::Runtime> Drop for ProgressRelay<R> {
    /// Unlistens however the call ended, including the one way `stop` cannot
    /// cover: the tool *future* dropped at its `.await`, which is what an MCP
    /// client disconnecting or cancelling mid-export does. A listener left
    /// behind would deserialise every later `export://progress` and spawn a
    /// notification into a dead peer, once per leak, for the life of the
    /// process.
    fn drop(&mut self) {
        use tauri::Listener;
        if let Some(id) = self.id.take() {
            self.app.unlisten(id);
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

    // ------------------------------------------------------------------ time

    #[tool(
        description = "Sets a keyframe on an animated property at a step. value null keys the value the property has there now."
    )]
    async fn keyframe_set(
        &self,
        Parameters(p): Parameters<KeyframeSetParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        let value = p
            .value
            .map(serde_json::from_value::<crate::document::PropertyValue>)
            .transpose()
            .map_err(|e| ToolError::Refused(format!("value: {e}")))?;
        self.write("keyframe_set", false, move |app| {
            crate::animation::set_keyframe(app.state(), p.object, p.property, p.step, value)
        })
        .await
        .map(Json)
    }

    #[tool(description = "Removes the keyframe at a step.")]
    async fn keyframe_remove(
        &self,
        Parameters(p): Parameters<KeyframeParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("keyframe_remove", false, move |app| {
            crate::animation::remove_keyframe(app.state(), p.object, p.property, p.step)
        })
        .await
        .map(Json)
    }

    #[tool(description = "Moves a keyframe from one step to another.")]
    async fn keyframe_move(
        &self,
        Parameters(p): Parameters<KeyframeMoveParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("keyframe_move", false, move |app| {
            let summary = crate::animation::move_keyframe(
                app.state(),
                p.object,
                p.property,
                p.from,
                p.to,
                None,
            )?;
            crate::document::end_gesture(app.state())?;
            Ok(summary)
        })
        .await
        .map(Json)
    }

    #[tool(description = "Sets how a property interpolates out of the keyframe at a step.")]
    async fn interpolation_set(
        &self,
        Parameters(p): Parameters<InterpolationParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        let interp: crate::animation::InterpolationView =
            serde_json::from_value(p.interpolation)
                .map_err(|e| ToolError::Refused(format!("interpolation: {e}")))?;
        self.write("interpolation_set", false, move |app| {
            crate::animation::set_interpolation(app.state(), p.object, p.property, p.step, interp)
        })
        .await
        .map(Json)
    }

    #[tool(
        description = "Constant motion from a step to the next position key: a rhumb line at a bearing and speed. Existing keys in between need overwrite true."
    )]
    async fn motion_add(
        &self,
        Parameters(p): Parameters<MotionParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("motion_add", false, move |app| {
            crate::animation::add_constant_motion(
                app.state(),
                p.object,
                p.step,
                p.direction,
                p.speed_mps,
                p.overwrite,
            )
        })
        .await
        .map(Json)
    }

    #[tool(description = "Links a property to another object's, or unlinks it with primary null.")]
    async fn follow_set(
        &self,
        Parameters(p): Parameters<FollowParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("follow_set", false, move |app| {
            crate::animation::set_follow(app.state(), p.object, p.property, p.primary, p.step)
        })
        .await
        .map(Json)
    }

    #[tool(
        description = "Changes the step count and/or the start time (unix seconds). Shrinking drops keys beyond the end; read step_count_impact through invoke first if that matters."
    )]
    async fn timeline_set(
        &self,
        Parameters(p): Parameters<TimelineParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("timeline_set", false, move |app| {
            let mut last = None;
            if let Some(count) = p.step_count {
                last = Some(crate::animation::set_step_count(app.state(), count)?);
            }
            if let Some(start) = p.start_unix_s {
                last = Some(crate::animation::set_start_time(app.state(), start)?);
            }
            last.ok_or_else(|| AppError::BadOption {
                field: "step_count/start_unix_s",
                value: "nothing to set".to_owned(),
            })
        })
        .await
        .map(Json)
    }

    #[tool(description = "An object's animated properties with their keyframes and interpolation.")]
    async fn object_tracks(
        &self,
        Parameters(p): Parameters<ObjectStepParams>,
    ) -> std::result::Result<Json<crate::animation::ObjectTracks>, ToolError> {
        self.run("object_tracks", move |app| {
            crate::animation::object_tracks(app.state(), p.object, p.step)
        })
        .await
        .map(Json)
    }

    // ----------------------------------------------------------------- field

    #[tool(
        description = "The evaluated field at points and steps: speed m/s, azimuth toward (degrees clockwise from north), u east, v north, and whether the cell is defined. Refused above 4096 points*steps."
    )]
    async fn field_sample(
        &self,
        Parameters(p): Parameters<SampleParams>,
    ) -> std::result::Result<Json<Samples>, ToolError> {
        // Refused before any allocation or session lock: each step flattens
        // the whole project (`ve_render::scene::flatten`), so an unbounded
        // points*steps grid is minutes of a starved interface from one call
        // (invariant 6).
        let total = p.points.len().saturating_mul(p.steps.len());
        if total > MAX_FIELD_SAMPLES {
            return Err(ToolError::Refused(format!(
                "field_sample: {total} points*steps requested, over the {MAX_FIELD_SAMPLES} limit; ask for fewer points or steps"
            )));
        }
        let samples = self
            .run("field_sample", move |app| {
                let state = app.state::<AppState>();
                let points: Vec<(f64, f64)> =
                    p.points.iter().map(|&[lon, lat]| (lon, lat)).collect();
                let mut out = Vec::with_capacity(total);
                // One flatten per step, not per (point, step) pair: a flatten
                // is a whole-scene walk, so this is the difference between
                // `steps.len()` flattens and `points.len() * steps.len()`.
                for &step in &p.steps {
                    let at_step = crate::commands::sample_points_at_step(
                        state.inner(),
                        &points,
                        step,
                        p.kind.clone(),
                    )?;
                    for (&[lon, lat], s) in p.points.iter().zip(at_step) {
                        let az = s.azimuth_toward_deg.to_radians();
                        out.push(Sample {
                            lon,
                            lat,
                            step,
                            speed_mps: s.speed_mps,
                            azimuth_toward_deg: s.azimuth_toward_deg,
                            u_mps: s.speed_mps * az.sin(),
                            v_mps: s.speed_mps * az.cos(),
                            defined: s.defined,
                        });
                    }
                }
                Ok(out)
            })
            .await?;
        Ok(Json(Samples { samples }))
    }

    #[tool(
        description = "Captures the field inside a region at a step onto the clipboard, for field_paste. Drops any copied objects."
    )]
    async fn field_capture(
        &self,
        Parameters(p): Parameters<CaptureParams>,
    ) -> std::result::Result<Json<crate::capture::CaptureState>, ToolError> {
        let region: crate::capture::RegionShape = serde_json::from_value(p.region)
            .map_err(|e| ToolError::Refused(format!("region: {e}")))?;
        self.run("field_capture", move |app| {
            crate::capture::capture_region(app.state(), region, p.step, p.kind)
        })
        .await
        .map(Json)
    }

    #[tool(
        description = "Pastes the captured field at lon/lat (or where it was captured), from a step, onto a layer. still true pastes one frame."
    )]
    async fn field_paste(
        &self,
        Parameters(p): Parameters<PasteParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("field_paste", false, move |app| {
            crate::capture::paste_capture(app.state(), p.lon, p.lat, p.step, p.layer, p.still)
        })
        .await
        .map(Json)
    }

    #[tool(description = "The macro library: id, name, kind, frames, footprint.")]
    async fn macro_list(
        &self,
    ) -> std::result::Result<Json<crate::macros::MacroLibrary>, ToolError> {
        self.run("macro_list", |app| {
            crate::macros::macro_library(app.state())
        })
        .await
        .map(Json)
    }

    #[tool(description = "Inserts a macro from the library at lon/lat starting at a step.")]
    async fn macro_insert(
        &self,
        Parameters(p): Parameters<MacroInsertParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("macro_insert", false, move |app| {
            crate::macros::insert_macro(app.state(), p.id, p.lon, p.lat, p.step, p.layer)
        })
        .await
        .map(Json)
    }

    // --------------------------------------------------------------- history

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

    #[tool(description = "The history list with the current position.")]
    async fn history_list(
        &self,
    ) -> std::result::Result<Json<crate::document::HistoryView>, ToolError> {
        self.run("history_list", |app| {
            crate::document::history_view(app.state())
        })
        .await
        .map(Json)
    }

    #[tool(description = "Jumps to an entry of history_list.")]
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

    // ------------------------------------------------------------------ view

    #[tool(description = "Pans the map to lon/lat, optionally at a zoom.")]
    async fn view_focus(
        &self,
        Parameters(p): Parameters<FocusParams>,
    ) -> std::result::Result<Json<Done>, ToolError> {
        ve_core::LonLat::new(p.lon, p.lat).map_err(AppError::from)?;
        self.app
            .state::<super::McpService>()
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
            .state::<super::McpService>()
            .note_tool(&self.app, "selection_set");
        let _ = tauri::Emitter::emit(&self.app, events::SELECTION, p.objects);
        Ok(Json(Done { ok: true }))
    }

    // ----------------------------------------------------------------- files

    #[tool(
        description = "Imports a GRIB2 file as a new layer of the open project. Slow for large files; progress is reported."
    )]
    async fn import_grib(
        &self,
        Parameters(p): Parameters<PathParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("import_grib", false, move |app| {
            crate::import::import_grib(app.state(), p.path)
        })
        .await
        .map(Json)
    }

    #[tool(description = "Adds an image layer (PNG, JPEG, GeoTIFF). Display only; never exported.")]
    async fn import_image(
        &self,
        Parameters(p): Parameters<ImageParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("import_image", false, move |app| {
            crate::image::import_image(app.state(), p.path, p.view)
        })
        .await
        .map(Json)
    }

    #[tool(
        description = "Downloads historical wind or current (ERA5, GlobCurrent) over HTTPS into a layer. The one tool that reaches the network, and only when called."
    )]
    async fn import_history(
        &self,
        Parameters(p): Parameters<HistoryParams>,
        ctx: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        let relay = self.relay_progress::<crate::history::HistoryProgress>(
            "history://progress",
            ctx,
            |h| {
                (
                    f64::from(h.done),
                    Some(f64::from(h.total)),
                    Some(h.archive.clone()),
                )
            },
        );
        let out = self
            .write("import_history", false, move |app| {
                crate::history::import_history(
                    app.clone(),
                    p.archives,
                    p.start_unix_s,
                    p.end_unix_s,
                    p.set_start_time,
                )
            })
            .await;
        relay.stop();
        out.map(Json)
    }

    #[tool(
        description = "Exports the project as GRIB2 to path. Refuses an existing file. Progress is reported; export_cancel stops it."
    )]
    async fn export_grib(
        &self,
        Parameters(p): Parameters<ExportParams>,
        ctx: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> std::result::Result<Json<crate::export::ExportResult>, ToolError> {
        let request = crate::export::ExportRequest {
            path: p.path,
            year: p.year,
            month: p.month,
            day: p.day,
            hour: p.hour,
        };
        let relay =
            self.relay_progress::<crate::export::ExportProgress>("export://progress", ctx, |e| {
                (f64::from(e.step), Some(f64::from(e.total)), None)
            });
        let out = self
            .run("export_grib", move |app| {
                crate::export::export_grib(app.clone(), app.state(), app.state(), request)
            })
            .await;
        relay.stop();
        out.map(Json)
    }

    #[tool(
        description = "Exports the project as a Zarr V3 directory at path (Float16, 72 h by 10 degree chunks, Zstd, NaN where uncovered). Refuses an existing directory."
    )]
    async fn export_zarr(
        &self,
        Parameters(p): Parameters<ExportParams>,
        ctx: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> std::result::Result<Json<crate::export::ExportZarrResult>, ToolError> {
        let request = crate::export::ExportZarrRequest {
            path: p.path,
            year: p.year,
            month: p.month,
            day: p.day,
            hour: p.hour,
        };
        let relay =
            self.relay_progress::<crate::export::ExportProgress>("export://progress", ctx, |e| {
                (f64::from(e.step), Some(f64::from(e.total)), None)
            });
        let out = self
            .run("export_zarr", move |app| {
                crate::export::export_zarr(app.clone(), app.state(), app.state(), request)
            })
            .await;
        relay.stop();
        out.map(Json)
    }

    #[tool(description = "Asks a running export to stop.")]
    async fn export_cancel(&self) -> std::result::Result<Json<Done>, ToolError> {
        self.run("export_cancel", |app| {
            crate::export::cancel_export(app.state());
            Ok(())
        })
        .await?;
        Ok(Json(Done { ok: true }))
    }

    #[tool(
        description = "The map as the interface shows it, once its tiles have settled: a PNG. Needs an open project."
    )]
    async fn screenshot(&self) -> std::result::Result<CallToolResult, ToolError> {
        // Neither `run` nor `write`: the work is the frontend's, so there is
        // no closure to put on `spawn_blocking` and no document to report a
        // change to. The activity note is what the other two would have done.
        let service = self.app.state::<super::McpService>();
        service.note_tool(&self.app, "screenshot");
        // The guard forgets the request on every way out of here, the
        // dropped future included, so nothing has to be cleaned up by hand.
        let mut pending = service.captures.request(&self.app);
        // The frontend waits up to 10 s for tiles and 20 s for a frame (M78);
        // a little longer than both, then give up rather than hang the client.
        let answer =
            tokio::time::timeout(std::time::Duration::from_secs(35), pending.receiver()).await;
        match answer {
            Ok(Ok(png)) => {
                let data = base64::engine::general_purpose::STANDARD.encode(png);
                Ok(CallToolResult::success(vec![ContentBlock::image(
                    data,
                    "image/png",
                )]))
            }
            _ => Err(ToolError::Internal(McpError::internal_error(
                "the map did not answer the capture: is a project open and the window shown?",
                None,
            ))),
        }
    }

    // ---------------------------------------------------------------- invoke

    #[tool(
        description = "Runs any IPC command by name with JSON arguments — the escape hatch for what no other tool covers, including the transform gesture (begin_transform, drag_transform, end_gesture). Writes are undoable and the interface follows."
    )]
    async fn invoke(
        &self,
        Parameters(p): Parameters<InvokeParams>,
    ) -> std::result::Result<Json<Invoked>, ToolError> {
        // A command with no arguments takes an empty `Args {}`, which
        // deserialises from `{}` and not from null, so a missing `args` is
        // the empty object rather than what `Option` left behind.
        let args = p.args.unwrap_or_else(|| Value::Object(Default::default()));
        let name = p.command;
        // The five that leave a different project open, or none. `opened` is
        // what tells the frontend to reset its step, selection and layer.
        let opened = matches!(
            name.as_str(),
            "new_project"
                | "open_project"
                | "close_project"
                | "new_project_from_grib"
                | "recover_autosave"
        );
        let result = self
            .write("invoke", opened, move |app| {
                super::invoke::call(app, &name, args)
            })
            .await?;
        Ok(Json(Invoked { result }))
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

// -------------------------------------------------------------------- time

#[derive(Debug, Deserialize, JsonSchema)]
pub struct KeyframeSetParams {
    pub object: u64,
    /// A property id (`object_get`'s `id`, e.g. `"Position"`, `"Speed"`).
    pub property: String,
    pub step: u32,
    /// A tagged `PropertyValue` (object_set's shape), or null to key the
    /// value the property has at this step right now.
    pub value: Option<Value>,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct KeyframeParams {
    pub object: u64,
    pub property: String,
    pub step: u32,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct KeyframeMoveParams {
    pub object: u64,
    pub property: String,
    pub from: u32,
    pub to: u32,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct InterpolationParams {
    pub object: u64,
    pub property: String,
    pub step: u32,
    /// A tagged `InterpolationView`: `{"kind":"step"}`, `{"kind":"linear"}`,
    /// `{"kind":"ease_in"}`, `{"kind":"ease_out"}`, `{"kind":"ease_in_out"}`,
    /// or `{"kind":"bezier","x1":..,"y1":..,"x2":..,"y2":..}` — the names
    /// `object_tracks` returns in a track's `interpolations`.
    pub interpolation: Value,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MotionParams {
    pub object: u64,
    pub step: u32,
    /// True bearing toward, degrees clockwise from north.
    pub direction: f64,
    pub speed_mps: f64,
    #[serde(default)]
    pub overwrite: bool,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FollowParams {
    pub object: u64,
    pub property: String,
    /// The object to follow, or null to stop following.
    pub primary: Option<u64>,
    pub step: u32,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TimelineParams {
    pub step_count: Option<u32>,
    /// Unix seconds, `null` to clear it, or left out entirely to leave it
    /// alone.
    ///
    /// Plain `Option<Option<i64>>` cannot tell an explicit `null` from an
    /// absent field apart: `serde_json`'s `Option` deserialisation collapses
    /// both to the outer `None` (verified: `{}"` and `{"x":null}` both
    /// deserialise `x: Option<Option<i64>>` to `None`), which would make
    /// clearing the start time unreachable through this tool.
    /// `deserialize_present` runs only when the key is in the request, so a
    /// present `null` reaches it and becomes `Some(None)`.
    #[serde(default, deserialize_with = "deserialize_present")]
    pub start_unix_s: Option<Option<i64>>,
}

/// Deserialises a field only when its key is present, distinguishing an
/// explicit `null` (`Some(None)`) from an absent key (the `default` this is
/// paired with). See [`TimelineParams::start_unix_s`].
fn deserialize_present<'de, D, T>(deserializer: D) -> std::result::Result<Option<T>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    T::deserialize(deserializer).map(Some)
}

// ------------------------------------------------------------------- field

/// The largest `points.len() * steps.len()` `field_sample` accepts.
///
/// Each step flattens the whole project once (`sample_points_at_step`); at
/// this bound a request is at most a few thousand samples plus however many
/// flattens `steps` asks for, not the minutes of whole-scene work an
/// unbounded grid would be (invariant 6).
const MAX_FIELD_SAMPLES: usize = 4096;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SampleParams {
    /// `[lon, lat]` pairs.
    pub points: Vec<[f64; 2]>,
    pub steps: Vec<u32>,
    /// "wind" or "current"; null samples the composite.
    pub kind: Option<String>,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct Sample {
    pub lon: f64,
    pub lat: f64,
    pub step: u32,
    pub speed_mps: f64,
    pub azimuth_toward_deg: f64,
    pub u_mps: f64,
    pub v_mps: f64,
    pub defined: bool,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct Samples {
    pub samples: Vec<Sample>,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CaptureParams {
    /// A region as the interface draws it: `{"kind":"rect","centre":[lon,lat],"half_width_deg":..,"half_height_deg":..}`,
    /// `{"kind":"disc","centre":[lon,lat],"radius_deg":..}`, or
    /// `{"kind":"polygon","points":[[lon,lat],...]}` (`RegionShape` in
    /// capture.rs).
    pub region: Value,
    pub step: u32,
    /// "wind" or "current"; used only as a fallback when the project has no
    /// visible field layer at all.
    pub kind: Option<String>,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct PasteParams {
    pub lon: Option<f64>,
    pub lat: Option<f64>,
    pub step: u32,
    pub layer: Option<u64>,
    #[serde(default)]
    pub still: bool,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MacroInsertParams {
    pub id: String,
    pub lon: f64,
    pub lat: f64,
    pub step: u32,
    pub layer: Option<u64>,
}

// ----------------------------------------------------------------- history

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HistoryJumpParams {
    pub target: usize,
}

// -------------------------------------------------------------------- view

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
#[derive(Debug, Serialize, JsonSchema)]
pub struct Done {
    pub ok: bool,
}

// ------------------------------------------------------------------- files

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PathParams {
    pub path: String,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ImageParams {
    pub path: String,
    /// `[west, north, east, south]` to place an image with no georeference
    /// of its own; null uses the file's.
    pub view: Option<[f64; 4]>,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct HistoryParams {
    /// Archive names as the History panel lists them: "era5",
    /// "globcurrent".
    pub archives: Vec<String>,
    pub start_unix_s: i64,
    pub end_unix_s: i64,
    #[serde(default)]
    pub set_start_time: bool,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExportParams {
    pub path: String,
    /// Reference time, UTC.
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
}

// ------------------------------------------------------------------ invoke

#[derive(Debug, Deserialize, JsonSchema)]
pub struct InvokeParams {
    /// A command name from the application's IPC surface, e.g.
    /// "rename_project".
    pub command: String,
    /// The command's arguments, snake_case as in Rust. Omitted is `{}`.
    #[serde(default)]
    pub args: Option<Value>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Invoked {
    /// Whatever the command returned, as JSON.
    pub result: Value,
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tauri::{Emitter, Listener};

    use super::ProgressRelay;

    /// A relay stops listening when it is dropped, not only when `stop` is
    /// called. The tool future dropped at its `.await` — a client that
    /// disconnects or cancels mid-export — never reaches `stop`, and a
    /// listener left behind would forward every later export to a peer
    /// nobody is reading, once per leak, for the life of the process.
    #[test]
    fn a_dropped_relay_stops_listening() {
        let app = tauri::test::mock_app();
        let seen = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&seen);
        let relay = ProgressRelay {
            app: app.handle().clone(),
            id: Some(app.listen("export://progress", move |_| {
                counted.fetch_add(1, Ordering::Relaxed);
            })),
        };

        let _ = app.emit("export://progress", 1_u32);
        assert_eq!(
            seen.load(Ordering::Relaxed),
            1,
            "a live relay should hear the event"
        );

        // Dropped, never stopped: the one path `stop` cannot cover.
        drop(relay);
        let _ = app.emit("export://progress", 2_u32);
        assert_eq!(
            seen.load(Ordering::Relaxed),
            1,
            "a dropped relay must not still be listening"
        );
    }
}

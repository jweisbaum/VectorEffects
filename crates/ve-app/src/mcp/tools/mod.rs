//! The tools a client sees (spec.md 8.8).
//!
//! Every tool calls the command the interface calls, with the `State` the
//! handle gives, so there is one implementation of each feature. A tool that
//! writes ends with `write`, which emits `document://changed`.

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::IntoCallToolResult;
use rmcp::model::{
    CallToolResponse, CallToolResult, ContentBlock, ErrorData as McpError, Implementation,
    ServerCapabilities, ServerConfig,
};
use rmcp::{ServerHandler, tool_handler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tauri::Manager;

use super::events;
use crate::error::AppError;

mod escape;
mod field;
mod files;
mod history;
mod project;
mod structure;
mod time;
mod view;

pub use escape::{InvokeParams, Invoked};
pub use field::*;
pub use files::*;
pub use history::*;
pub use project::*;
pub use structure::*;
pub use time::*;
pub use view::*;

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
    ///
    /// `opened` is only ever true when the closure actually succeeded
    /// (final review, finding 1): a refused `project_open`/`project_new`/
    /// `project_close` leaves the previous project open, and telling the
    /// frontend `opened: true` anyway makes it reset step, selection and the
    /// active layer for a document that never changed.
    pub(crate) async fn write<T: Send + 'static>(
        &self,
        name: &'static str,
        opened: bool,
        f: impl FnOnce(&tauri::AppHandle<R>) -> crate::error::Result<T> + Send + 'static,
    ) -> std::result::Result<T, ToolError> {
        let result = self.run(name, f).await;
        let emitted = events::changed(&self.app, opened && result.is_ok());
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

// Shared by more than one group's tools, so it stays here rather than being
// re-exported from two files: `StepParams` (structure's `layers_list`, view's
// `step_set`), `ObjectStepParams` (structure's `object_get`, time's
// `object_tracks`), `Done` (view's `view_focus`/`step_set`/`selection_set`,
// files' `export_cancel`).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct StepParams {
    pub step: u32,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ObjectStepParams {
    pub object: u64,
    pub step: u32,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct Done {
    pub ok: bool,
}

impl<R: tauri::Runtime> VectorEffects<R> {
    pub fn new(app: tauri::AppHandle<R>) -> Self {
        // One router per group, each generated by its file's `#[tool_router]`;
        // `ToolRouter` adds. A group left out of this sum is a group of tools
        // no client can list, which the integration tests catch by name.
        let tool_router = Self::tool_router_project()
            + Self::tool_router_structure()
            + Self::tool_router_time()
            + Self::tool_router_field()
            + Self::tool_router_files()
            + Self::tool_router_view()
            + Self::tool_router_history()
            + Self::tool_router_escape();
        Self { app, tool_router }
    }
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

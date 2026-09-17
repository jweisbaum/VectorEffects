//! The escape hatch (spec.md 8.8): `invoke`, which runs any IPC command by
//! name for what no dedicated tool covers.

use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{ToolError, VectorEffects};

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

#[tool_router(router = tool_router_escape, vis = "pub(crate)")]
impl<R: tauri::Runtime> VectorEffects<R> {
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
                crate::mcp::invoke::call(app, &name, args)
            })
            .await?;
        Ok(Json(Invoked { result }))
    }
}

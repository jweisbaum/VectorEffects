//! The tools a client sees (spec.md 8.8).

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ServerHandler, tool_handler, tool_router};

/// One handler per client session, holding the application it drives.
///
/// Generic over the Tauri runtime so the mock application used by the
/// integration tests (`tauri::test::MockRuntime`) can drive the same code
/// path as the shipped `tauri::Wry` build.
// The fields are read from Task 3 on, once the router carries real tools and
// a call reaches into the application through `app`.
#[allow(dead_code, reason = "read by Task 3's tool implementations")]
pub struct VectorEffects<R: tauri::Runtime> {
    pub(crate) app: tauri::AppHandle<R>,
    tool_router: ToolRouter<Self>,
}

impl<R: tauri::Runtime> Clone for VectorEffects<R> {
    fn clone(&self) -> Self {
        Self {
            app: self.app.clone(),
            tool_router: self.tool_router.clone(),
        }
    }
}

impl<R: tauri::Runtime> std::fmt::Debug for VectorEffects<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VectorEffects").finish_non_exhaustive()
    }
}

// `allow_empty`: this task's handler declares no `#[tool]` fn yet; Task 3
// grows the router and the attribute comes off.
#[tool_router(allow_empty)]
impl<R: tauri::Runtime> VectorEffects<R> {
    pub fn new(app: tauri::AppHandle<R>) -> Self {
        Self {
            app,
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_handler]
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
}

//! The MCP service (spec.md 8.8): a loopback endpoint through which a client
//! drives the application, on only while the setting says so.

pub mod token;

use std::sync::Mutex;

use crate::settings::{McpSettings, McpStatus};

/// The service's live state, managed by Tauri beside `AppState`.
#[derive(Debug, Default)]
pub struct McpService {
    /// Why the last bind failed, shown in Settings.
    pub(crate) bind_error: Mutex<Option<String>>,
}

impl McpService {
    /// Reconciles the listener with the settings: start, restart or stop.
    pub fn apply(&self, _app: &tauri::AppHandle, _mcp: &McpSettings) {}

    /// What Settings shows.
    pub fn status(&self, mcp: &McpSettings) -> McpStatus {
        McpStatus {
            enabled: mcp.enabled,
            port: mcp.port,
            token: mcp.token.clone(),
            bound_port: None,
            bind_error: self.bind_error.lock().ok().and_then(|e| e.clone()),
            sessions: 0,
            last_tool: None,
        }
    }
}

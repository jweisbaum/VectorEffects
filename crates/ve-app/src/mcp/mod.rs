//! The MCP service (spec.md 8.8): a loopback endpoint through which a client
//! drives the application, on only while the setting says so.

pub mod server;
pub mod token;
pub mod tools;

use std::sync::Mutex;

use crate::settings::{McpSettings, McpStatus};

/// The service's live state, managed by Tauri beside `AppState`.
#[derive(Debug, Default)]
pub struct McpService {
    running: Mutex<Option<server::Running>>,
    /// Why the last bind failed, shown in Settings.
    pub(crate) bind_error: Mutex<Option<String>>,
}

impl McpService {
    /// Reconciles the listener with the settings: start, restart or stop.
    ///
    /// Always restarts when on, because the token or the port may have
    /// changed and both are baked into the running listener. Generic over
    /// the Tauri runtime so the mock application used by the integration
    /// tests can drive the same path as the shipped `tauri::Wry` build.
    pub fn apply<R: tauri::Runtime>(&self, app: &tauri::AppHandle<R>, mcp: &McpSettings) {
        if let Ok(mut slot) = self.running.lock()
            && let Some(running) = slot.take()
        {
            running.stop();
        }
        if !mcp.enabled {
            self.set_bind_error(None);
            return;
        }
        match server::start(app.clone(), mcp.port, mcp.token.clone()) {
            Ok(running) => {
                self.set_bind_error(None);
                self.adopt(running);
            }
            Err(message) => {
                tracing::warn!(%message, "mcp listener did not start");
                self.set_bind_error(Some(message));
            }
        }
    }

    /// Holds a listener started elsewhere (the tests start one on port 0).
    pub fn adopt(&self, running: server::Running) {
        if let Ok(mut slot) = self.running.lock()
            && let Some(old) = slot.replace(running)
        {
            old.stop();
        }
    }

    /// The bound port while the listener is up.
    pub fn running_port(&self) -> Option<u16> {
        self.running
            .lock()
            .ok()
            .and_then(|r| r.as_ref().map(|r| r.port))
    }

    fn set_bind_error(&self, error: Option<String>) {
        if let Ok(mut slot) = self.bind_error.lock() {
            *slot = error;
        }
    }

    /// What Settings shows.
    pub fn status(&self, mcp: &McpSettings) -> McpStatus {
        McpStatus {
            enabled: mcp.enabled,
            port: mcp.port,
            token: mcp.token.clone(),
            bound_port: self.running_port(),
            bind_error: self.bind_error.lock().ok().and_then(|e| e.clone()),
            sessions: 0,
            last_tool: None,
        }
    }
}

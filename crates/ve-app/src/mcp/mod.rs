//! The MCP service (spec.md 8.8): a loopback endpoint through which a client
//! drives the application, on only while the setting says so.

pub mod capture;
pub mod events;
pub mod invoke;
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
    /// Open client sessions and the last tool called (status bar, Settings).
    activity: Mutex<events::McpActivity>,
    /// Screenshots asked of the frontend and not yet answered.
    pub captures: capture::Captures,
}

impl McpService {
    /// Reconciles the listener with the settings: start, restart or stop.
    ///
    /// Always restarts when on, because the token or the port may have
    /// changed and both are baked into the running listener. Generic over
    /// the Tauri runtime so the mock application used by the integration
    /// tests can drive the same path as the shipped `tauri::Wry` build.
    pub fn apply<R: tauri::Runtime>(&self, app: &tauri::AppHandle<R>, mcp: &McpSettings) {
        // `take()` under the lock, then drop the guard before `stop()`:
        // `stop` can block briefly waiting for the accept loop, and nothing
        // else that touches `self.running` (`running_port`, `status`) must
        // wait on that.
        let previous = self.running.lock().ok().and_then(|mut slot| slot.take());
        if let Some(previous) = previous {
            previous.stop();
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
        // `replace()` under the lock, then drop the guard before `stop()`,
        // for the same reason as `apply`: a concurrent `running_port` or
        // `status` read must not wait on the old listener's shutdown.
        let previous = self
            .running
            .lock()
            .ok()
            .and_then(|mut slot| slot.replace(running));
        if let Some(previous) = previous {
            previous.stop();
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

    /// Records a tool call and tells the status bar.
    pub fn note_tool<R: tauri::Runtime>(&self, app: &tauri::AppHandle<R>, name: &str) {
        use tauri::Emitter;
        if let Ok(mut activity) = self.activity.lock() {
            activity.last_tool = Some(name.to_owned());
            let _ = app.emit(events::ACTIVITY, activity.clone());
        }
    }

    /// A client session opened (+1) or closed (-1).
    pub fn session_delta<R: tauri::Runtime>(&self, app: &tauri::AppHandle<R>, delta: i32) {
        use tauri::Emitter;
        if let Ok(mut activity) = self.activity.lock() {
            activity.sessions = activity.sessions.saturating_add_signed(delta);
            if activity.sessions == 0 {
                activity.last_tool = None;
            }
            let _ = app.emit(events::ACTIVITY, activity.clone());
        }
    }

    /// What Settings shows.
    pub fn status(&self, mcp: &McpSettings) -> McpStatus {
        let activity = self.activity.lock().ok();
        McpStatus {
            enabled: mcp.enabled,
            port: mcp.port,
            token: mcp.token.clone(),
            bound_port: self.running_port(),
            bind_error: self.bind_error.lock().ok().and_then(|e| e.clone()),
            sessions: activity.as_ref().map_or(0, |a| a.sessions),
            last_tool: activity.and_then(|a| a.last_tool.clone()),
        }
    }
}

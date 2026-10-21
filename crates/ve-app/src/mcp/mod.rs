//! The MCP service (spec.md 8.8): a loopback endpoint through which a client
//! drives the application, on only while the setting says so.

pub mod capture;
pub mod claude_settings;
pub mod clients;
pub mod desktop;
pub mod events;
pub mod invoke;
pub mod native;
pub mod server;
pub mod skill;
pub mod token;
pub mod tools;

use std::sync::Mutex;

use crate::settings::{McpSettings, McpStatus};

/// The service's live state, managed by Tauri beside `AppState`.
#[derive(Debug, Default)]
pub struct McpService {
    running: Mutex<Option<server::Running>>,
    app_lock: Mutex<Option<std::fs::File>>,
    /// Why the last bind failed, shown in Settings.
    pub(crate) bind_error: Mutex<Option<String>>,
    /// Open client sessions and the last tool called (status bar, Settings).
    activity: Mutex<events::McpActivity>,
    activity_generation: std::sync::atomic::AtomicU64,
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
        use tauri::Manager;
        let dir = app
            .state::<crate::commands::AppState>()
            .paths
            .config_dir
            .join("mcp");
        let ready = self
            .app_lock
            .lock()
            .map(|mut lock| {
                if lock.is_none() {
                    *lock = whirlwind_mcp::endpoint::try_lock(&dir).ok().flatten();
                }
                lock.is_some()
            })
            .unwrap_or(false);
        if !ready {
            self.set_bind_error(Some("Another app instance owns AI assistant access".into()));
            return;
        }
        whirlwind_mcp::endpoint::remove(&dir);
        self.activity_generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Ok(mut activity) = self.activity.lock() {
            *activity = events::McpActivity::default();
        }

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
                let endpoint = whirlwind_mcp::endpoint::Endpoint {
                    version: 1,
                    port: running.port,
                    token: mcp.token.clone(),
                    pid: std::process::id(),
                };
                if let Err(error) = whirlwind_mcp::endpoint::write(&dir, &endpoint) {
                    running.stop();
                    self.set_bind_error(Some(error.to_string()));
                    return;
                }

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

    /// Stateless HTTP has no persistent session. Show recent activity for one minute;
    /// installed-host connection verification is tracked separately by the native bridge.
    pub fn note_tool<R: tauri::Runtime>(&self, app: &tauri::AppHandle<R>, name: &str) {
        use tauri::Emitter;
        let generation = self
            .activity_generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        if let Ok(mut activity) = self.activity.lock() {
            activity.sessions = 1;
            activity.last_tool = Some(name.to_owned());
            let _ = app.emit(events::ACTIVITY, activity.clone());
        }
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            use tauri::Manager;
            let service = app.state::<McpService>();
            if service
                .activity_generation
                .load(std::sync::atomic::Ordering::SeqCst)
                == generation
                && let Ok(mut activity) = service.activity.lock()
            {
                *activity = events::McpActivity::default();
                let _ = app.emit(events::ACTIVITY, activity.clone());
            }
        });
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
            clients: clients::McpClient::available(),
            ask: mcp.ask,
        }
    }
}

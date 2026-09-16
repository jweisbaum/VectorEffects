//! The screenshot's round trip: the service asks, `MapView` reads its
//! framebuffer, and `deliver_capture` brings the bytes back.
//!
//! The map is a canvas, so only the application can say what is on it — the
//! same reason `window.__veCapture` exists for the WebDriver tooling (M71).
//! The picture never becomes document or session state: it is held in one
//! channel for the length of one tool call and nothing else.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use base64::Engine;
use serde::Serialize;
use tauri::Emitter;
use ts_rs::TS;

use crate::error::{AppError, Context, Result};

/// The event `MapView` answers.
pub const CAPTURE: &str = "view://capture";

/// What the frontend is asked for, and the id it answers with.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "CaptureRequest.ts")]
pub struct CaptureRequest {
    pub id: u64,
}

/// Outstanding requests, keyed by id.
#[derive(Debug, Default)]
pub struct Captures {
    next: AtomicU64,
    pending: Mutex<HashMap<u64, tokio::sync::oneshot::Sender<Vec<u8>>>>,
}

impl Captures {
    /// Asks the frontend for a picture. The receiver yields PNG bytes.
    ///
    /// Generic over the Tauri runtime, like everything else in `mcp`, so the
    /// mock application the integration tests drive takes the same path as
    /// the shipped `tauri::Wry` build.
    pub fn request<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
    ) -> (u64, tokio::sync::oneshot::Receiver<Vec<u8>>) {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = tokio::sync::oneshot::channel();
        // The guard is dropped before the emit: a listener registered in the
        // same process answers synchronously, and `deliver` would then wait
        // on a lock this call still held.
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(id, tx);
        }
        let _ = app.emit(CAPTURE, CaptureRequest { id });
        (id, rx)
    }

    /// Forgets a request the tool stopped waiting for.
    pub fn forget(&self, id: u64) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(&id);
        }
    }

    fn deliver(&self, id: u64, png: Vec<u8>) -> Result<()> {
        let sender = self.pending.lock().ok().and_then(|mut p| p.remove(&id));
        match sender {
            Some(tx) => {
                // A closed receiver means the tool gave up first; the picture
                // is simply dropped, which is not the frontend's failure.
                let _ = tx.send(png);
                Ok(())
            }
            None => Err(AppError::BadOption {
                field: "capture id",
                value: id.to_string(),
            }),
        }
    }
}

/// The frontend's answer to `view://capture`.
#[tauri::command]
pub fn deliver_capture(
    service: tauri::State<'_, super::McpService>,
    id: u64,
    png_base64: String,
) -> Result<()> {
    let png = base64::engine::general_purpose::STANDARD
        .decode(png_base64.trim())
        .doing("decode", format!("the capture answering request {id}"))?;
    service.captures.deliver(id, png)
}

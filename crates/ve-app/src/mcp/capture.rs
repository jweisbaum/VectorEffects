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
    /// Asks the frontend for a picture.
    ///
    /// The returned guard forgets the request when it is dropped, so a tool
    /// that stops waiting — timed out, or its future dropped at the `.await`
    /// because the client disconnected — leaves nothing behind. Only the
    /// frontend actually answering removes an entry any other way.
    ///
    /// Generic over the Tauri runtime, like everything else in `mcp`, so the
    /// mock application the integration tests drive takes the same path as
    /// the shipped `tauri::Wry` build.
    pub fn request<R: tauri::Runtime>(&self, app: &tauri::AppHandle<R>) -> PendingCapture<'_> {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = tokio::sync::oneshot::channel();
        // The guard is dropped before the emit: a listener registered in the
        // same process answers synchronously, and `deliver` would then wait
        // on a lock this call still held.
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(id, tx);
        }
        let _ = app.emit(CAPTURE, CaptureRequest { id });
        PendingCapture {
            captures: self,
            id,
            rx,
        }
    }

    /// Forgets a request nothing is waiting on any more.
    fn forget(&self, id: u64) {
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

/// One outstanding request, which the map is answering right now.
///
/// Holding the guard is what keeps the request alive: drop it and the entry
/// is gone, so a picture that arrives afterwards is refused rather than
/// filling a channel nobody holds.
#[derive(Debug)]
pub struct PendingCapture<'a> {
    captures: &'a Captures,
    id: u64,
    rx: tokio::sync::oneshot::Receiver<Vec<u8>>,
}

impl PendingCapture<'_> {
    /// The id the frontend answers with.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The channel the PNG bytes arrive on.
    ///
    /// Borrowed rather than taken so the guard outlives the await: a
    /// receiver moved out would drop the guard with it, and the entry would
    /// go before the answer could arrive.
    pub fn receiver(&mut self) -> &mut tokio::sync::oneshot::Receiver<Vec<u8>> {
        &mut self.rx
    }
}

impl Drop for PendingCapture<'_> {
    fn drop(&mut self) {
        // Unconditional: on the answered path the entry is already gone and
        // this is a no-op, and on every other path — timeout, or the tool's
        // future dropped mid-await — it is the only thing that removes it.
        self.captures.forget(self.id);
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

#[cfg(test)]
mod tests {
    use tauri::Manager;

    use super::*;

    /// A request lives exactly as long as its guard. The guard is what the
    /// `screenshot` tool holds across its `.await`, so a future dropped
    /// there — an MCP client disconnecting mid-screenshot — leaves nothing
    /// behind for a late answer to fill.
    #[test]
    fn a_dropped_request_is_forgotten() {
        let app = tauri::test::mock_app();
        let captures = Captures::default();

        // Held: the frontend's answer is accepted and arrives on the channel.
        let mut pending = captures.request(app.handle());
        assert!(captures.deliver(pending.id(), vec![1, 2, 3]).is_ok());
        assert_eq!(pending.receiver().try_recv().ok(), Some(vec![1, 2, 3]));
        drop(pending);

        // Asked for and then abandoned without ever being answered: the
        // temporary guard is dropped at the end of this statement, and that
        // is the only thing that removes the entry.
        let abandoned = captures.request(app.handle()).id();
        let refused = captures
            .deliver(abandoned, vec![4, 5, 6])
            .expect_err("an answer to an abandoned request must be refused");
        assert!(
            matches!(refused, AppError::BadOption { field, .. } if field == "capture id"),
            "{refused}"
        );
    }

    #[test]
    fn a_capture_that_is_not_base64_is_refused_by_name() {
        let app = tauri::test::mock_app();
        app.manage(super::super::McpService::default());
        let service = app.state::<super::super::McpService>();
        let pending = service.captures.request(app.handle());
        let refused = deliver_capture(app.state(), pending.id(), "not base64!".to_owned())
            .expect_err("a body that is not base64 must be refused");
        assert!(refused.to_string().contains("decode"), "{refused}");
    }
}

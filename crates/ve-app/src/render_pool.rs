//! Rendering ahead of the playhead, and saying which frames are ready
//! (spec.md 9.5).
//!
//! A worker pool renders the tiles of the current viewport for the steps
//! around the playhead, through [`crate::protocol::serve`] — the same function
//! the map's own tile requests go through. That is the whole design: a tile
//! rendered here is keyed, evaluated and encoded exactly as the map will later
//! ask for it, so "ready" means the request will be a cache hit, and nothing
//! else has to be tracked.
//!
//! Invalidation is not done here at all. Tiles are keyed by the content hash
//! of the scene that produced them (spec.md 7.10), so an edit that changes what
//! a step looks like changes that step's key and leaves every other step's
//! entries where they were. The pool only has to look again. A unit in flight
//! when an edit lands is not cancelled: it finishes into a key that is now
//! simply unreachable, which costs one tile of work and nothing in correctness.
//!
//! **No `HashMap` is iterated anywhere on this path.** Scenes are hashed once
//! per step into a `Vec` indexed by step, and the queue is a `Vec` in priority
//! order.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use ve_core::project::Project;
use ve_render::cache::TileKey;
use ve_render::scene::{Scene, flatten};
use ve_render::tile::TileId;

use crate::commands::AppState;
use crate::error::{AppError, Result};
use crate::protocol::{self, Frame};

/// A tile address on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "TileAddress.ts")]
pub struct TileAddress {
    /// Zoom level.
    pub z: u32,
    /// Column.
    pub x: u32,
    /// Row.
    pub y: u32,
}

/// How much of one step's viewport is cached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "StepReadiness.ts")]
pub struct StepReadiness {
    /// Which step.
    pub step: u32,
    /// Tiles of the viewport already in the cache.
    pub ready: u32,
    /// Tiles of the viewport.
    pub total: u32,
}

/// Readiness of every step, for one viewport at one revision.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "TimelineReadiness.ts")]
pub struct TimelineReadiness {
    /// The revision the answer describes. A frontend holding an older one
    /// knows a frame it saw as solid may now be stale (spec.md 9.5).
    pub revision: u64,
    /// One entry per step, in step order.
    pub steps: Vec<StepReadiness>,
}

/// Progress, emitted as the `render://progress` event when a tile lands.
#[derive(Debug, Clone, Copy, Serialize, TS)]
#[ts(export, export_to = "RenderProgress.ts")]
pub struct RenderProgress {
    /// The revision being rendered.
    pub revision: u64,
    /// The step the tile belongs to.
    pub step: u32,
}

/// One step, flattened, hashed and planned — everything a tile of it needs
/// that does not depend on which tile.
struct Prepared {
    frame: Frame,
    /// Each tile's key, once asked for: a key is the hash of the objects
    /// that reach the tile (M31), and the readiness probe asks after the
    /// same viewport several times a second.
    keys: Mutex<HashMap<TileId, TileKey>>,
}

impl Prepared {
    /// The tile's key, computed the first time.
    fn key(&self, state: &AppState, tile: TileId) -> TileKey {
        if let Ok(keys) = self.keys.lock()
            && let Some(key) = keys.get(&tile)
        {
            return *key;
        }
        let key = self.frame.tile(state, tile).1;
        if let Ok(mut keys) = self.keys.lock() {
            keys.insert(tile, key);
        }
        key
    }

    /// The tile's sub-scene and its key, for rendering it.
    fn tile(&self, state: &AppState, tile: TileId) -> (Scene, TileKey) {
        let (scene, key) = self.frame.tile(state, tile);
        if let Ok(mut keys) = self.keys.lock() {
            keys.insert(tile, key);
        }
        (scene, key)
    }
}

/// The document as the pool sees it: one revision, its steps prepared on
/// demand.
///
/// Held as an `Arc` so a worker can flatten a step without holding the session
/// lock — a render is milliseconds, and the map must never wait on one. Each
/// step is flattened and hashed once per snapshot, not once per tile: a
/// viewport is a hundred tiles, and the readiness probe asks after every step
/// several times a second.
struct Snapshot {
    revision: u64,
    project: Arc<Project>,
    /// Indexed by step: no map is iterated on this path.
    steps: Mutex<Vec<Option<Arc<Prepared>>>>,
}

impl Snapshot {
    fn of(revision: u64, project: Project) -> Self {
        let steps = project.settings.step_count as usize;
        Self {
            revision,
            project: Arc::new(project),
            steps: Mutex::new(vec![None; steps]),
        }
    }

    /// The step's scene — every layer of every kind, as the map draws it
    /// (M31) — prepared the first time it is asked for.
    fn prepared(&self, step: u32) -> Arc<Prepared> {
        if let Ok(steps) = self.steps.lock()
            && let Some(Some(prepared)) = steps.get(step as usize)
        {
            return Arc::clone(prepared);
        }
        let prepared = Arc::new(Prepared {
            frame: Frame::of(flatten(&self.project, step)),
            keys: Mutex::new(HashMap::new()),
        });
        if let Ok(mut steps) = self.steps.lock()
            && let Some(slot) = steps.get_mut(step as usize)
        {
            *slot = Some(Arc::clone(&prepared));
        }
        prepared
    }
}

/// One tile of one step, waiting to be rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Unit {
    step: u32,
    tile: TileId,
}

/// What the workers are working through.
struct Queue {
    /// Units in priority order; the front is next.
    units: Vec<Unit>,
    /// The snapshot the units belong to.
    snapshot: Option<Arc<Snapshot>>,
}

/// What the pool calls when a tile lands.
type Notify = Box<dyn Fn(RenderProgress) + Send + Sync>;

/// The pool: its queue, and the workers waiting on it.
pub struct RenderPool {
    queue: Mutex<Queue>,
    wake: Condvar,
    /// Told when a tile lands, so the frontend can refresh its readiness.
    notify: Mutex<Option<Notify>>,
}

impl std::fmt::Debug for RenderPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let queued = self.queue.lock().map(|q| q.units.len()).unwrap_or(0);
        f.debug_struct("RenderPool")
            .field("queued", &queued)
            .finish()
    }
}

impl Default for RenderPool {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderPool {
    /// An idle pool with no workers. [`Self::start`] adds them.
    pub fn new() -> Self {
        Self {
            queue: Mutex::new(Queue {
                units: Vec::new(),
                snapshot: None,
            }),
            wake: Condvar::new(),
            notify: Mutex::new(None),
        }
    }

    /// How many workers to run.
    ///
    /// Two, whatever the machine. The CPU evaluator already parallelises each
    /// tile across every core through rayon, so more workers would only
    /// oversubscribe it; and one GPU serialises its queue however many threads
    /// submit to it. Two is enough to overlap one tile's flatten-and-encode
    /// with another's evaluation, which is the gain on offer.
    pub const WORKERS: usize = 2;

    /// Installs the progress callback.
    pub fn on_progress(&self, notify: impl Fn(RenderProgress) + Send + Sync + 'static) {
        if let Ok(mut slot) = self.notify.lock() {
            *slot = Some(Box::new(notify));
        }
    }

    /// Replaces the queue with the viewport's tiles for the steps around
    /// `current`, in priority order (spec.md 9.5): the current step, then the
    /// steps adjacent to it, then the rest of the range outward.
    ///
    /// A snapshot of the document is taken here, under the session lock, and
    /// the workers render from it without the lock. The old queue is simply
    /// dropped — a unit in flight finishes into a key that is either still
    /// wanted or harmlessly unreachable.
    pub fn request(&self, state: &AppState, current: u32, tiles: &[TileAddress]) -> Result<()> {
        let snapshot = {
            let session = state
                .session
                .lock()
                .map_err(|_| AppError::Internal("session lock was poisoned".to_owned()))?;
            let Some(open) = session.open.as_ref() else {
                self.clear();
                return Ok(());
            };
            // Reuse the snapshot if the document has not moved: the hashes it
            // has already computed are still right.
            let existing = self
                .queue
                .lock()
                .ok()
                .and_then(|queue| queue.snapshot.clone())
                .filter(|snapshot| snapshot.revision == open.revision);
            match existing {
                Some(snapshot) => snapshot,
                None => Arc::new(Snapshot::of(open.revision, open.project.clone())),
            }
        };

        let ids: Vec<TileId> = tiles
            .iter()
            .filter_map(|t| TileId::new(t.z, t.x, t.y).ok())
            .collect();
        let last = snapshot.project.last_step();
        let mut units = Vec::with_capacity(ids.len() * (last as usize + 1));
        for step in priority_order(current.min(last), last) {
            for tile in &ids {
                units.push(Unit { step, tile: *tile });
            }
        }
        // Reversed so the workers can pop from the back in priority order.
        units.reverse();

        if let Ok(mut queue) = self.queue.lock() {
            queue.units = units;
            queue.snapshot = Some(snapshot);
        }
        self.wake.notify_all();
        Ok(())
    }

    /// Drops the queue.
    pub fn clear(&self) {
        if let Ok(mut queue) = self.queue.lock() {
            queue.units.clear();
            queue.snapshot = None;
        }
    }

    /// Renders the next unit, if there is one. Returns whether it did any work.
    ///
    /// A unit already in the cache is skipped without rendering. That is what
    /// makes a re-request after an edit cheap: the steps the edit did not
    /// touch still hash to entries that exist.
    pub fn process_next(&self, state: &AppState) -> bool {
        let (unit, snapshot) = {
            let Ok(mut queue) = self.queue.lock() else {
                return false;
            };
            let Some(unit) = queue.units.pop() else {
                return false;
            };
            let Some(snapshot) = queue.snapshot.clone() else {
                return false;
            };
            (unit, snapshot)
        };

        let prepared = snapshot.prepared(unit.step);
        if state.tiles.contains(&prepared.key(state, unit.tile)) {
            return true;
        }
        let (scene, key) = prepared.tile(state, unit.tile);
        if let Err(err) = protocol::serve_keyed(state, &scene, key) {
            tracing::warn!(%err, step = unit.step, "render ahead failed");
            return true;
        }
        if let Ok(notify) = self.notify.lock()
            && let Some(notify) = notify.as_ref()
        {
            notify(RenderProgress {
                revision: snapshot.revision,
                step: unit.step,
            });
        }
        true
    }

    /// Renders everything queued, on the calling thread. For tests.
    pub fn drain(&self, state: &AppState) {
        while self.process_next(state) {}
    }

    /// Spawns the workers. Each loops for the life of the process, sleeping on
    /// the condition variable while the queue is empty, and borrows the
    /// application state through the handle on every iteration — Tauri hands
    /// out borrows, not owned references, and a worker outlives any of them.
    pub fn start(self: &Arc<Self>, app: tauri::AppHandle) {
        use tauri::Manager;
        for index in 0..Self::WORKERS {
            let pool = Arc::clone(self);
            let app = app.clone();
            let spawned = std::thread::Builder::new()
                .name(format!("render-{index}"))
                .spawn(move || {
                    loop {
                        let worked = {
                            let state = app.state::<AppState>();
                            pool.process_next(&state)
                        };
                        if !worked
                            && let Ok(queue) = pool.queue.lock()
                            && let Ok(guard) = pool.wake.wait_while(queue, |q| q.units.is_empty())
                        {
                            // Nothing to do: wait until a request arrives, then
                            // release the queue before looking again.
                            drop(guard);
                        }
                    }
                });
            // A worker that cannot start costs render-ahead, not the
            // application: the map still renders on demand as it always has.
            if let Err(err) = spawned {
                tracing::error!(%err, index, "could not start a render worker");
            }
        }
    }

    /// How ready every step is, for a viewport, at the current revision.
    ///
    /// Answered from the cache index alone — no tile is read or rendered — so
    /// the timeline can ask often. The snapshot is taken or reused exactly as
    /// [`Self::request`] takes it, so the hashes the answer is built from are
    /// the hashes the pool is rendering under.
    pub fn readiness(&self, state: &AppState, tiles: &[TileAddress]) -> Result<TimelineReadiness> {
        let snapshot = {
            let session = state
                .session
                .lock()
                .map_err(|_| AppError::Internal("session lock was poisoned".to_owned()))?;
            let open = session.open.as_ref().ok_or(AppError::NoProjectOpen)?;
            let existing = self
                .queue
                .lock()
                .ok()
                .and_then(|queue| queue.snapshot.clone())
                .filter(|snapshot| snapshot.revision == open.revision);
            match existing {
                Some(snapshot) => snapshot,
                None => {
                    let snapshot = Arc::new(Snapshot::of(open.revision, open.project.clone()));
                    if let Ok(mut queue) = self.queue.lock() {
                        queue.snapshot = Some(Arc::clone(&snapshot));
                    }
                    snapshot
                }
            }
        };

        let ids: Vec<TileId> = tiles
            .iter()
            .filter_map(|t| TileId::new(t.z, t.x, t.y).ok())
            .collect();
        let total = ids.len() as u32;
        let steps = (0..snapshot.project.settings.step_count)
            .map(|step| {
                let prepared = snapshot.prepared(step);
                let ready = ids
                    .iter()
                    .filter(|tile| state.tiles.contains(&prepared.key(state, **tile)))
                    .count() as u32;
                StepReadiness { step, ready, total }
            })
            .collect();
        Ok(TimelineReadiness {
            revision: snapshot.revision,
            steps,
        })
    }
}

/// The order steps are rendered in (spec.md 9.5): the current step, then the
/// steps adjacent to it, then the rest of the range outward — one below, one
/// above, alternating, until both ends are reached.
pub fn priority_order(current: u32, last: u32) -> Vec<u32> {
    let mut order = Vec::with_capacity(last as usize + 1);
    order.push(current);
    let mut below = current;
    let mut above = current;
    loop {
        let mut pushed = false;
        if above < last {
            above += 1;
            order.push(above);
            pushed = true;
        }
        if below > 0 {
            below -= 1;
            order.push(below);
            pushed = true;
        }
        if !pushed {
            break;
        }
    }
    order
}

/// Queues the viewport's tiles for rendering ahead of the playhead.
#[tauri::command]
pub fn render_ahead(
    state: tauri::State<'_, AppState>,
    pool: tauri::State<'_, Arc<RenderPool>>,
    current: u32,
    tiles: Vec<TileAddress>,
) -> Result<()> {
    pool.request(&state, current, &tiles)
}

/// How ready every step is, for the viewport.
#[tauri::command]
pub fn frame_readiness(
    state: tauri::State<'_, AppState>,
    pool: tauri::State<'_, Arc<RenderPool>>,
    tiles: Vec<TileAddress>,
) -> Result<TimelineReadiness> {
    pool.readiness(&state, &tiles)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spec 9.5: current, adjacent, then outward. Written out for a small case
    /// so the order is legible, then checked for the property on every case.
    #[test]
    fn steps_are_rendered_outward_from_the_playhead() {
        assert_eq!(priority_order(3, 6), vec![3, 4, 2, 5, 1, 6, 0]);
        assert_eq!(priority_order(0, 3), vec![0, 1, 2, 3]);
        assert_eq!(priority_order(3, 3), vec![3, 2, 1, 0]);
        assert_eq!(priority_order(0, 0), vec![0]);

        for last in 0..12 {
            for current in 0..=last {
                let order = priority_order(current, last);
                let mut sorted = order.clone();
                sorted.sort_unstable();
                sorted.dedup();
                assert_eq!(sorted.len(), last as usize + 1, "every step once");
                assert_eq!(order[0], current, "the playhead first");
                // Distance from the playhead never decreases along the order.
                let distance = |step: u32| step.abs_diff(current);
                for pair in order.windows(2) {
                    assert!(distance(pair[1]) >= distance(pair[0]));
                }
            }
        }
    }
}

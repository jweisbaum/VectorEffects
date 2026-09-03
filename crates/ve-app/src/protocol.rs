//! The `ve-tile://` custom URI scheme.
//!
//! Tiles are served through a URI scheme rather than an IPC command so the
//! webview can fetch them like any other resource: the browser's own cache
//! applies, requests pipeline, and a quarter-megabyte of pixels never has to be
//! serialised through the IPC channel.
//!
//! The address is `<base>/<revision>/<step>/<z>/<x>/<y>`. Carrying the document
//! revision means an edit makes every previously fetched tile *unreachable*
//! rather than stale, so a cached tile can never show a field that no longer
//! exists. A request for a revision that is no longer current is refused rather
//! than answered with current data under a stale URL.

use std::sync::Mutex;

use tauri::Manager;
use tauri::http::{Request, Response};
use ve_render::cache::{TileKey, scene_hash};
use ve_render::cpu::CpuEvaluator;
use ve_render::evaluator::FieldEvaluator;
use ve_render::preview::{Quality, render_tile};
use ve_render::scene::{Scene, flatten};
use ve_render::tile;

use crate::commands::AppState;

/// The scheme name.
pub const SCHEME: &str = "ve-tile";

/// The most recently flattened scene, reused across the tiles of one frame.
///
/// A viewport is over a hundred tiles and flattening is per *frame*, not per
/// tile, so without this every tile would redo the same work.
#[derive(Debug, Default)]
pub struct SceneCache(Mutex<Option<(u64, u32, Scene)>>);

impl SceneCache {
    /// Returns the scene for `(revision, step)`, flattening it if needed.
    fn scene_for(&self, state: &AppState, revision: u64, step: u32) -> Option<Scene> {
        if let Ok(cached) = self.0.lock()
            && let Some((cached_revision, cached_step, scene)) = cached.as_ref()
            && *cached_revision == revision
            && *cached_step == step
        {
            return Some(scene.clone());
        }

        let session = state.session.lock().ok()?;
        let open = session.open.as_ref()?;
        if open.revision != revision {
            return None;
        }
        let scene = flatten(&open.project, step);
        drop(session);

        if let Ok(mut cached) = self.0.lock() {
            *cached = Some((revision, step, scene.clone()));
        }
        Some(scene)
    }
}

/// The URL prefix the frontend should build tile addresses from.
///
/// Tauri maps custom schemes differently per platform, so the frontend is told
/// the prefix rather than guessing it.
pub fn base_url() -> String {
    if cfg!(windows) {
        format!("http://{SCHEME}.localhost/")
    } else {
        format!("{SCHEME}://localhost/")
    }
}

/// Parsed tile request.
#[derive(Debug, PartialEq, Eq)]
struct TileRequest {
    revision: u64,
    step: u32,
    id: tile::TileId,
}

/// Extracts `<revision>/<step>/<z>/<x>/<y>` from a request path.
fn parse(path: &str) -> Option<TileRequest> {
    let mut parts = path.trim_start_matches('/').split('/');
    let revision: u64 = parts.next()?.parse().ok()?;
    let step: u32 = parts.next()?.parse().ok()?;
    let z: u32 = parts.next()?.parse().ok()?;
    let x: u32 = parts.next()?.parse().ok()?;
    // The last segment may carry an extension; ignore anything after a dot.
    let y: u32 = parts.next()?.split('.').next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(TileRequest {
        revision,
        step,
        id: tile::TileId::new(z, x, y).ok()?,
    })
}

fn respond(status: u16, body: Vec<u8>) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/octet-stream")
        .header("Access-Control-Allow-Origin", "*")
        // A tile URL names one revision of one step, so it never changes.
        .header("Cache-Control", "public, max-age=31536000, immutable")
        .body(body)
        .unwrap_or_default()
}

/// Handles a `ve-tile://` request.
pub fn handle(app: &tauri::AppHandle, request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    let path = request.uri().path().to_owned();

    let Some(parsed) = parse(&path) else {
        tracing::warn!(%path, "rejected malformed tile request");
        return respond(404, Vec::new());
    };

    let state = app.state::<AppState>();
    let Some(scene) = app
        .state::<SceneCache>()
        .scene_for(&state, parsed.revision, parsed.step)
    else {
        // The document moved on, or nothing is open. Refusing beats answering
        // with current data under a URL that names an older revision.
        tracing::debug!(
            revision = parsed.revision,
            "tile request for a stale revision"
        );
        return respond(409, Vec::new());
    };

    match serve(&state, &scene, parsed.id) {
        Ok(encoded) => respond(200, encoded),
        Err(err) => {
            tracing::error!(%err, "tile evaluation failed");
            respond(500, Vec::new())
        }
    }
}

/// How a scene will be evaluated: which backend, and how coarsely.
///
/// One decision, made here for the protocol, the render pool and the readiness
/// probe alike. The key a tile is cached under includes the quality, so a pool
/// that chose differently from the protocol would fill the cache with tiles the
/// map never asks for and report frames ready that are not.
pub fn plan_for<'a>(
    state: &'a AppState,
    scene: &Scene,
) -> (Option<&'a ve_render::gpu::GpuEvaluator>, Quality) {
    // The GPU when it can take the scene, the CPU otherwise. A clone stamp
    // needs recursion, which a compute shader cannot do, so the fallback is
    // used even on a machine with a perfectly good GPU.
    let gpu = match &state.evaluators.gpu {
        Some(gpu) if ve_render::gpu::supports(scene) => Some(gpu),
        _ => None,
    };
    // Coarse evaluation pays on the CPU and costs on the GPU. Measured on a
    // dense scene: the CPU goes from 45.8 to 23.3 ms a tile with a stride of
    // four, while the GPU goes from 3.9 to 5.0 — three dispatches instead of
    // one, when the evaluation itself is nearly free. So the fast backend
    // simply evaluates every pixel.
    let quality = if gpu.is_some() {
        Quality::Exact
    } else {
        Quality::Standard
    };
    (gpu, quality)
}

/// The cache key a tile of `scene` is served under.
pub fn key_for(state: &AppState, scene: &Scene, id: tile::TileId) -> TileKey {
    let (_, quality) = plan_for(state, scene);
    TileKey {
        scene: scene_hash(scene),
        tile: id,
        quality,
    }
}

/// A tile of `scene`, from the cache or freshly rendered into it.
///
/// **The one path a tile takes**, whether the map asked for it now or the
/// render pool is working ahead of the playhead: the key, the backend, the
/// quality and the encoding are decided once here, so a tile rendered ahead
/// is exactly the tile that will later be served.
pub fn serve(
    state: &AppState,
    scene: &Scene,
    id: tile::TileId,
) -> ve_render::error::Result<Vec<u8>> {
    let (gpu, quality) = plan_for(state, scene);
    let key = TileKey {
        scene: scene_hash(scene),
        tile: id,
        quality,
    };
    if let Some(cached) = state.tiles.get(&key) {
        return Ok(cached);
    }

    // A preview may evaluate coarsely and interpolate: the view is a proxy,
    // never a source (spec.md, invariant 3). Cells straddling an edge are still
    // evaluated exactly, so nothing is smeared.
    let evaluator: &dyn FieldEvaluator = match gpu {
        Some(gpu) => gpu,
        None => &CpuEvaluator,
    };
    let samples = render_tile(evaluator, scene, id, quality)?;
    let encoded = tile::encode(&samples);
    if let Err(err) = state.tiles.put(&key, &encoded) {
        // A cache write failure must not fail the request.
        tracing::warn!(%err, "could not cache tile");
    }
    Ok(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_formed_paths_parse() {
        let r = parse("/7/3/2/5/1").expect("should parse");
        assert_eq!(r.revision, 7);
        assert_eq!(r.step, 3);
        assert_eq!((r.id.z, r.id.x, r.id.y), (2, 5, 1));

        // A trailing extension is tolerated so the frontend may use one.
        let r = parse("/1/0/0/1/0.bin").expect("should parse");
        assert_eq!((r.id.z, r.id.x, r.id.y), (0, 1, 0));
    }

    #[test]
    fn malformed_paths_are_rejected() {
        for path in [
            "",
            "/",
            "/1/0/0/0",     // too few segments
            "/1/0/0/0/0/0", // too many
            "/x/0/0/0/0",   // unparseable revision
            "/1/x/0/0/0",   // unparseable step
            "/1/0/0/9/0",   // column past the end of level 0
            "/1/0/99/0/0",  // level past the pyramid
        ] {
            assert!(parse(path).is_none(), "{path:?} should not parse");
        }
    }

    /// The revision is what keeps a cached tile from outliving its document.
    #[test]
    fn the_revision_is_part_of_the_address() {
        let a = parse("/1/0/0/0/0").expect("parses");
        let b = parse("/2/0/0/0/0").expect("parses");
        assert_ne!(a, b, "different revisions must be different tiles");
    }

    #[test]
    fn the_base_url_ends_with_a_separator() {
        assert!(base_url().ends_with('/'));
        assert!(base_url().contains(SCHEME));
    }
}

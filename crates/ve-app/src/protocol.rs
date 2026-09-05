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
//!
//! Image layers (spec.md 4.9, M18) come through the same scheme, at
//! `<base>/image/<revision>/<layer>/<max edge>`, and for the same reasons: a
//! chart scan is megabytes of PNG, which has no business crossing the IPC
//! channel as JSON. The caller sends the largest texture its GPU will take, so
//! the downsampling happens once, here, rather than in the webview.

use std::sync::{Arc, Mutex};

use tauri::Manager;
use tauri::http::{Request, Response};
use ve_render::cache::{SceneHash, TileKey, scene_hash};
use ve_render::cpu::CpuEvaluator;
use ve_render::evaluator::FieldEvaluator;
use ve_render::preview::{Quality, render_tile};
use ve_render::scene::{Scene, flatten};
use ve_render::tile;

use crate::commands::AppState;

/// The scheme name.
pub const SCHEME: &str = "ve-tile";

/// One frame, flattened and hashed: what every tile of it is served from.
#[derive(Debug)]
pub struct Frame {
    /// The scene at this step.
    pub scene: Scene,
    /// Its content hash, computed once for the frame rather than per tile.
    pub hash: SceneHash,
}

/// The most recently flattened frame, reused across the tiles of it.
///
/// A viewport is over a hundred tiles and flattening and hashing are per
/// *frame*, not per tile, so without this every tile would redo the same work.
#[derive(Debug, Default)]
pub struct SceneCache(Mutex<Option<(u64, u32, Arc<Frame>)>>);

impl SceneCache {
    /// Returns the frame for `(revision, step)`, flattening it if needed.
    fn frame_for(&self, state: &AppState, revision: u64, step: u32) -> Option<Arc<Frame>> {
        if let Ok(cached) = self.0.lock()
            && let Some((cached_revision, cached_step, frame)) = cached.as_ref()
            && *cached_revision == revision
            && *cached_step == step
        {
            return Some(Arc::clone(frame));
        }

        let session = state.session.lock().ok()?;
        let open = session.open.as_ref()?;
        if open.revision != revision {
            return None;
        }
        let scene = flatten(&open.project, step);
        drop(session);

        let hash = scene_hash(&scene);
        let frame = Arc::new(Frame { scene, hash });
        if let Ok(mut cached) = self.0.lock() {
            *cached = Some((revision, step, Arc::clone(&frame)));
        }
        Some(frame)
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

/// What a request is for.
#[derive(Debug, PartialEq, Eq)]
enum Served {
    /// A field tile.
    Tile {
        revision: u64,
        step: u32,
        id: tile::TileId,
    },
    /// An image layer's picture (spec.md 4.9, M18).
    Image {
        revision: u64,
        layer: u64,
        max_edge: u32,
    },
}

/// Extracts `<revision>/<step>/<z>/<x>/<y>`, or an image address, from a path.
fn parse(path: &str) -> Option<Served> {
    let mut parts = path.trim_start_matches('/').split('/');
    let first = parts.next()?;
    if first == "image" {
        let revision: u64 = parts.next()?.parse().ok()?;
        let layer: u64 = parts.next()?.parse().ok()?;
        // The last segment may carry an extension; ignore anything after a dot.
        let max_edge: u32 = parts.next()?.split('.').next()?.parse().ok()?;
        if parts.next().is_some() {
            return None;
        }
        return Some(Served::Image {
            revision,
            layer,
            max_edge,
        });
    }
    let revision: u64 = first.parse().ok()?;
    let step: u32 = parts.next()?.parse().ok()?;
    let z: u32 = parts.next()?.parse().ok()?;
    let x: u32 = parts.next()?.parse().ok()?;
    let y: u32 = parts.next()?.split('.').next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(Served::Tile {
        revision,
        step,
        id: tile::TileId::new(z, x, y).ok()?,
    })
}

fn respond(status: u16, body: Vec<u8>) -> Response<Vec<u8>> {
    typed(status, "application/octet-stream", body)
}

fn typed(status: u16, content_type: &str, body: Vec<u8>) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header("Content-Type", content_type)
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

    let parsed = match parsed {
        Served::Image {
            revision,
            layer,
            max_edge,
        } => return serve_image(app, revision, layer, max_edge),
        Served::Tile { revision, step, id } => TileRequest { revision, step, id },
    };

    let state = app.state::<AppState>();
    let Some(frame) = app
        .state::<SceneCache>()
        .frame_for(&state, parsed.revision, parsed.step)
    else {
        // The document moved on, or nothing is open. Refusing beats answering
        // with current data under a URL that names an older revision.
        tracing::debug!(
            revision = parsed.revision,
            "tile request for a stale revision"
        );
        return respond(409, Vec::new());
    };

    let key = key_with(&state, &frame.scene, frame.hash, parsed.id);
    match serve_keyed(&state, &frame.scene, key) {
        Ok(encoded) => respond(200, encoded),
        Err(err) => {
            tracing::error!(%err, "tile evaluation failed");
            respond(500, Vec::new())
        }
    }
}

/// A tile request, once the image case has been split off.
#[derive(Debug, PartialEq, Eq)]
struct TileRequest {
    revision: u64,
    step: u32,
    id: tile::TileId,
}

/// Serves an image layer's picture, decoded and downsampled here.
///
/// The revision in the address is the document's, so importing or replacing an
/// image makes the old address unreachable rather than stale — the same rule
/// the tiles follow, and the reason both can be served `immutable`.
fn serve_image(
    app: &tauri::AppHandle,
    revision: u64,
    layer: u64,
    max_edge: u32,
) -> Response<Vec<u8>> {
    let state = app.state::<AppState>();
    let path = {
        let Ok(session) = state.session.lock() else {
            return respond(500, Vec::new());
        };
        let Some(open) = session.open.as_ref() else {
            return respond(409, Vec::new());
        };
        if open.revision != revision {
            tracing::debug!(revision, "image request for a stale revision");
            return respond(409, Vec::new());
        }
        match open
            .project
            .layer(ve_core::id::Id::from_raw(layer))
            .map(|l| l.source.path().map(std::path::Path::to_path_buf))
        {
            Some(Some(path)) => path,
            _ => return respond(404, Vec::new()),
        }
    };

    // Decoded outside the session lock: a hundred-megapixel TIFF takes long
    // enough that holding the document while it decodes would stall every
    // edit, and nothing about the file depends on the document.
    match crate::image::render(&path, max_edge) {
        Ok((png, width, height)) => {
            tracing::debug!(path = %path.display(), width, height, "served image layer");
            typed(200, "image/png", png)
        }
        Err(err) => {
            tracing::warn!(path = %path.display(), %err, "could not serve image layer");
            respond(404, Vec::new())
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

/// The cache key a tile of `scene` is served under, hashing the scene.
pub fn key_for(state: &AppState, scene: &Scene, id: tile::TileId) -> TileKey {
    key_with(state, scene, scene_hash(scene), id)
}

/// The cache key a tile of `scene` is served under, given the scene's hash.
///
/// Hashing is per frame and a frame is a hundred tiles, so a caller that
/// serves many tiles of one scene hashes once and comes through here.
pub fn key_with(state: &AppState, scene: &Scene, hash: SceneHash, id: tile::TileId) -> TileKey {
    let (_, quality) = plan_for(state, scene);
    TileKey {
        scene: hash,
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
    serve_keyed(state, scene, key_for(state, scene, id))
}

/// [`serve`], with the key already made — see [`key_with`].
///
/// Single-flight through the cache: the map's own request for a tile and the
/// pool rendering ahead on the same step meet here, and one of them waits for
/// the other rather than evaluating the tile a second time.
pub fn serve_keyed(
    state: &AppState,
    scene: &Scene,
    key: TileKey,
) -> ve_render::error::Result<Vec<u8>> {
    state.tiles.get_or_render(&key, || {
        // A preview may evaluate coarsely and interpolate: the view is a proxy,
        // never a source (spec.md, invariant 3). Cells straddling an edge are
        // still evaluated exactly, so nothing is smeared.
        let (gpu, _) = plan_for(state, scene);
        let evaluator: &dyn FieldEvaluator = match gpu {
            Some(gpu) => gpu,
            None => &CpuEvaluator,
        };
        let samples = render_tile(evaluator, scene, key.tile, key.quality)?;
        Ok(tile::encode(&samples))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tile address, unpacked.
    fn tile_of(path: &str) -> (u64, u32, tile::TileId) {
        match parse(path).expect("should parse") {
            Served::Tile { revision, step, id } => (revision, step, id),
            other => panic!("{path:?} parsed as {other:?}"),
        }
    }

    #[test]
    fn well_formed_paths_parse() {
        let (revision, step, id) = tile_of("/7/3/2/5/1");
        assert_eq!(revision, 7);
        assert_eq!(step, 3);
        assert_eq!((id.z, id.x, id.y), (2, 5, 1));

        // A trailing extension is tolerated so the frontend may use one.
        let (_, _, id) = tile_of("/1/0/0/1/0.bin");
        assert_eq!((id.z, id.x, id.y), (0, 1, 0));
    }

    /// An image address is the same scheme, told apart by its first segment
    /// (spec.md 4.9, M18). `image` is not a number, so it can never collide
    /// with a revision.
    #[test]
    fn an_image_address_parses_as_an_image() {
        assert_eq!(
            parse("/image/12/34/4096"),
            Some(Served::Image {
                revision: 12,
                layer: 34,
                max_edge: 4096,
            })
        );
        assert_eq!(
            parse("/image/1/2/2048.png"),
            Some(Served::Image {
                revision: 1,
                layer: 2,
                max_edge: 2048,
            })
        );
        for path in [
            "/image/1/2",       // too few segments
            "/image/1/2/3/4",   // too many
            "/image/x/2/4096",  // unparseable revision
            "/image/1/2/large", // unparseable size
        ] {
            assert!(parse(path).is_none(), "{path:?} should not parse");
        }
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
        // And the same for an image, which is cached by the browser for a year.
        assert_ne!(parse("/image/1/9/4096"), parse("/image/2/9/4096"));
    }

    #[test]
    fn the_base_url_ends_with_a_separator() {
        assert!(base_url().ends_with('/'));
        assert!(base_url().contains(SCHEME));
    }
}

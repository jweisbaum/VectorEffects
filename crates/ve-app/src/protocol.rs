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
use ve_render::cache::TileKey;
use ve_render::cpu::CpuEvaluator;
use ve_render::cull::{Digests, TileScene, digests_of, tile_scene};
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
    /// The content hash of each of its objects and rasters, computed once
    /// for the frame: what each tile's key is combined from (M31).
    pub digests: Digests,
}

impl Frame {
    /// A frame of `scene`, its objects digested.
    pub fn of(scene: Scene) -> Self {
        let digests = digests_of(&scene);
        Self { scene, digests }
    }

    /// The part of the frame a tile sees, and the key it is cached under.
    ///
    /// The key names the sub-scene — the objects whose reach touches the
    /// tile — so an edit re-keys the tiles the edited object reaches and no
    /// others (spec.md 7.10, M31). The backend and the quality are decided
    /// for the sub-scene too: a tile the eraser or a clone stamp never
    /// reaches stays on the GPU whatever the rest of the scene holds.
    pub fn tile(&self, state: &AppState, id: tile::TileId) -> (Scene, TileKey) {
        let TileScene { scene, hash } = tile_scene(&self.scene, &self.digests, id);
        let (_, quality) = plan_for(state, &scene);
        (
            scene,
            TileKey {
                scene: hash,
                tile: id,
                quality,
            },
        )
    }
}

/// The most recently flattened frame, reused across the tiles of it.
///
/// A viewport is over a hundred tiles and flattening and hashing are per
/// *frame*, not per tile, so without this every tile would redo the same work.
#[derive(Debug, Default)]
pub struct SceneCache(Mutex<Vec<CachedFrame>>);

/// One frame the cache holds, and what it was flattened for.
#[derive(Debug)]
struct CachedFrame {
    revision: u64,
    step: u32,
    scope: TileScope,
    frame: Arc<Frame>,
}

/// How many frames are held at once.
///
/// Three, because a clone in progress draws from three of them on every
/// frame: the whole stack, the stack without the layer being edited, and that
/// layer by itself (M40, M44, M45). Fewer slots would re-flatten the project
/// several times per redraw, at pointer rate.
const FRAMES_HELD: usize = 3;

impl SceneCache {
    /// Returns the frame for an address, flattening it if it is not held.
    ///
    /// `scope` says which layers it holds: all of them, all but one, or one
    /// alone (see [`TileScope`]).
    pub fn frame_for(
        &self,
        state: &AppState,
        revision: u64,
        step: u32,
        scope: TileScope,
    ) -> Option<Arc<Frame>> {
        let matches = |held: &CachedFrame| {
            held.revision == revision && held.step == step && held.scope == scope
        };
        if let Ok(mut cached) = self.0.lock()
            && let Some(at) = cached.iter().position(matches)
        {
            // Most recently used first, so the frame a redraw asks for twice
            // is the one an eviction never takes.
            let held = cached.remove(at);
            let frame = Arc::clone(&held.frame);
            cached.insert(0, held);
            return Some(frame);
        }

        let session = state.session.lock().ok()?;
        // The document at its revision — every layer of every kind, which is
        // what the map draws (M31) — or the macro preview at its own revision
        // (D71): a one-object project the session holds while a capture is
        // being looked at before it is kept.
        let flatten_at = |project: &ve_core::project::Project| match scope {
            TileScope::Whole => flatten(project, step),
            TileScope::Without(raw) => {
                ve_render::scene::flatten_without(project, step, crate::document::object_id(raw))
            }
            TileScope::Only(raw) => {
                ve_render::scene::flatten_only(project, step, crate::document::object_id(raw))
            }
        };
        let scene = match session.open.as_ref() {
            Some(open) if open.revision == revision => flatten_at(&open.project),
            _ => match session.preview.as_ref() {
                Some(preview) if preview.revision == revision => flatten_at(&preview.project),
                _ => return None,
            },
        };
        drop(session);

        let frame = Arc::new(Frame::of(scene));
        if let Ok(mut cached) = self.0.lock() {
            cached.retain(|held| !matches(held));
            cached.insert(
                0,
                CachedFrame {
                    revision,
                    step,
                    scope,
                    frame: Arc::clone(&frame),
                },
            );
            cached.truncate(FRAMES_HELD);
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

/// Which layers a tile holds (spec.md 6.2).
///
/// A live edit acts on one layer while a tile is the whole visible stack, so
/// the map asks for two more scenes than it draws: the stack without the
/// edited layer, which says where that layer is what the composite is showing
/// (M40, M44), and the layer by itself, which is what a clone reads its
/// source from (M45).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileScope {
    /// Every visible layer, which is what the map draws.
    Whole,
    /// Every visible layer but this one.
    Without(u64),
    /// This layer and no other.
    Only(u64),
}

impl TileScope {
    /// The scope a path segment names, over the layer that follows it.
    fn named(word: &str, layer: u64) -> Option<Self> {
        match word {
            "without" => Some(Self::Without(layer)),
            "only" => Some(Self::Only(layer)),
            _ => None,
        }
    }
}

/// What a request is for.
#[derive(Debug, PartialEq, Eq)]
enum Served {
    /// A field tile.
    Tile {
        revision: u64,
        step: u32,
        /// Which layers of the scene this tile holds.
        scope: TileScope,
        id: tile::TileId,
    },
    /// An image layer's picture (spec.md 4.9, M18). The first segment is the
    /// opening's image token, not the document's revision (M37).
    Image {
        token: u64,
        layer: u64,
        max_edge: u32,
    },
}

/// Extracts `<revision>/<step>/<z>/<x>/<y>`, or an image address, from a
/// path. One tile holds every kind of field (M31).
///
/// A tile of the scene without one layer is the same address behind
/// `without/<layer>/` (M40). It is a prefix rather than a query string
/// because the frontend's tile cache treats everything before the `z/x/y` as
/// one opaque frame token, and a prefix keeps that true.
fn parse(path: &str) -> Option<Served> {
    let mut parts = path.trim_start_matches('/').split('/');
    let mut first = parts.next()?;
    let mut scope = TileScope::Whole;
    if first == "without" || first == "only" {
        scope = TileScope::named(first, parts.next()?.parse().ok()?)?;
        first = parts.next()?;
    }
    if first == "image" {
        let token: u64 = parts.next()?.parse().ok()?;
        let layer: u64 = parts.next()?.parse().ok()?;
        // The last segment may carry an extension; ignore anything after a dot.
        let max_edge: u32 = parts.next()?.split('.').next()?.parse().ok()?;
        if parts.next().is_some() {
            return None;
        }
        return Some(Served::Image {
            token,
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
        scope,
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
            token,
            layer,
            max_edge,
        } => return serve_image(app, token, layer, max_edge),
        Served::Tile {
            revision,
            step,
            scope,
            id,
        } => TileRequest {
            revision,
            step,
            scope,
            id,
        },
    };

    let state = app.state::<AppState>();
    let Some(frame) =
        app.state::<SceneCache>()
            .frame_for(&state, parsed.revision, parsed.step, parsed.scope)
    else {
        // The document moved on, or nothing is open. Refusing beats answering
        // with current data under a URL that names an older revision.
        tracing::debug!(
            revision = parsed.revision,
            "tile request for a stale revision"
        );
        return respond(409, Vec::new());
    };

    let (scene, key) = frame.tile(&state, parsed.id);
    match serve_keyed(&state, &scene, key) {
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
    scope: TileScope,
    id: tile::TileId,
}

/// Serves an image layer's picture, decoded and downsampled here.
///
/// The token in the address is the *opening's*, not the document's revision
/// (M37): a picture depends on its file and on nothing an edit does, so
/// addressing it by the revision meant every edit re-addressed every image —
/// a fresh decode of a chart scan per pointer report while one was dragged,
/// with the picture absent in between. The opening's token still keeps two
/// projects, or one reopened after its file changed, from sharing an
/// address that is served `immutable`.
fn serve_image(app: &tauri::AppHandle, token: u64, layer: u64, max_edge: u32) -> Response<Vec<u8>> {
    let state = app.state::<AppState>();
    let path = {
        let Ok(session) = state.session.lock() else {
            return respond(500, Vec::new());
        };
        let Some(open) = session.open.as_ref() else {
            return respond(409, Vec::new());
        };
        if open.image_token != token {
            tracing::debug!(token, "image request from another opening");
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

/// A tile of `scene`, from the cache or freshly rendered into it.
///
/// **The one path a tile takes**, whether the map asked for it now or the
/// render pool is working ahead of the playhead: the key, the backend, the
/// quality and the encoding are decided once here, so a tile rendered ahead
/// is exactly the tile that will later be served. Digests the whole scene
/// for one tile; a caller with many tiles of one frame builds a [`Frame`]
/// and asks it.
pub fn serve(
    state: &AppState,
    scene: &Scene,
    id: tile::TileId,
) -> ve_render::error::Result<Vec<u8>> {
    let (scene, key) = Frame::of(scene.clone()).tile(state, id);
    serve_keyed(state, &scene, key)
}

/// The keys the viewport's tiles of a frame are cached under (M31).
///
/// The map keeps its textures by key rather than by address: after an edit
/// it asks for the new revision's keys and fetches only the tiles whose key
/// it does not already hold, so a stroke redraws the tiles it reaches and
/// leaves the rest on screen, undimmed. The revision is the document's or
/// the macro preview's, exactly as a tile address is; a revision that is
/// neither is refused, and the map asks again with the one it has by then.
#[tauri::command]
pub fn tile_keys(
    app: tauri::AppHandle,
    revision: u64,
    step: u32,
    tiles: Vec<crate::render_pool::TileAddress>,
    // `scope` is "whole", "without" or "only"; `layer` is what the last two
    // are about. Two plain arguments rather than one enum, since this is the
    // IPC boundary and the frontend builds them from a frame token.
    scope: Option<String>,
    layer: Option<u64>,
) -> crate::error::Result<Vec<String>> {
    let scope = match (scope.as_deref(), layer) {
        (None | Some("whole"), _) => TileScope::Whole,
        (Some(word), Some(raw)) => {
            TileScope::named(word, raw).ok_or_else(|| crate::error::AppError::BadOption {
                field: "scope",
                value: format!("{word} is not a way of scoping a frame"),
            })?
        }
        (Some(word), None) => {
            return Err(crate::error::AppError::BadOption {
                field: "layer",
                value: format!("{word} needs a layer to be about"),
            });
        }
    };
    let state = app.state::<AppState>();
    let frame = app
        .state::<SceneCache>()
        .frame_for(&state, revision, step, scope)
        .ok_or_else(|| {
            crate::error::AppError::Internal(format!("revision {revision} is not the one open"))
        })?;
    Ok(tiles
        .iter()
        .map(
            |address| match tile::TileId::new(address.z, address.x, address.y) {
                Ok(id) => frame.tile(&state, id).1.digest(),
                Err(_) => String::new(),
            },
        )
        .collect())
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
            Served::Tile {
                revision, step, id, ..
            } => (revision, step, id),
            other => panic!("{path:?} parsed as {other:?}"),
        }
    }

    /// Which layers an address asks for (M40, M45).
    fn scope_of(path: &str) -> TileScope {
        match parse(path).expect("should parse") {
            Served::Tile { scope, .. } => scope,
            other => panic!("{path:?} parsed as {other:?}"),
        }
    }

    /// A "beneath" tile is the same address behind `without/<layer>/`
    /// (M40), and it must not be confused with an ordinary one: the two
    /// carry different scenes under otherwise identical `z/x/y`.
    /// Three scenes share the tile grid, and they must not share addresses:
    /// the same `z/x/y` holds different fields in each.
    #[test]
    fn an_address_names_which_layers_it_holds() {
        assert_eq!(scope_of("/7/3/2/5/1"), TileScope::Whole);
        assert_eq!(scope_of("/without/42/7/3/2/5/1"), TileScope::Without(42));
        assert_eq!(scope_of("/only/42/7/3/2/5/1"), TileScope::Only(42));
        // And the rest of the address is the same one in every case.
        assert_eq!(tile_of("/without/42/7/3/2/5/1"), tile_of("/7/3/2/5/1"));
        assert_eq!(tile_of("/only/42/7/3/2/5/1"), tile_of("/7/3/2/5/1"));
    }

    /// The prefix is a real segment, not a revision that happens to read as
    /// a word: a layer id that failed to parse must refuse the address
    /// rather than fall through to the ordinary shape.
    #[test]
    fn a_scoped_address_without_a_layer_is_refused() {
        assert!(parse("/without/7/3/2/5/1").is_none(), "too few segments");
        assert!(
            parse("/without/x/7/3/2/5/1").is_none(),
            "layer is not a number"
        );
        assert!(parse("/only/x/7/3/2/5/1").is_none());
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
                token: 12,
                layer: 34,
                max_edge: 4096,
            })
        );
        assert_eq!(
            parse("/image/1/2/2048.png"),
            Some(Served::Image {
                token: 1,
                layer: 2,
                max_edge: 2048,
            })
        );
        for path in [
            "/image/1/2",       // too few segments
            "/image/1/2/3/4",   // too many
            "/image/x/2/4096",  // unparseable token
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
            "/1/0/0/0/0/0", // too many: the kind segment of M29 and M30
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

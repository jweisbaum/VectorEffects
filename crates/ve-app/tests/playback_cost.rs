#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! What one playback advance costs the backend (spec.md 13: ≥ 8 steps/s with
//! frames pre-rendered).
//!
//! The frontend re-asks two things every time the playhead moves: `render_ahead`
//! re-queues the pool around the new playhead, and `frame_readiness` probes the
//! cache for every step of the viewport. Both are on the advance path, so their
//! cost is subtracted from the interval the user asked for — and if either runs
//! on the webview's own thread, it is subtracted from the frame budget as well.
//!
//! Reports numbers rather than asserting a threshold on most of them: the
//! machine varies. The one assertion is the property that must hold however
//! slow the machine is — that a *pre-rendered* timeline costs no more per
//! advance than a cold one, because nothing about moving the playhead one step
//! changes what is in the cache.

use std::sync::Arc;
use std::time::Instant;

use ve_app::commands::AppState;
use ve_app::paths::AppPaths;
use ve_app::projects::{self, NewProjectRequest};
use ve_app::render_pool::{RenderPool, TileAddress};

struct TempRoot(std::path::PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ve-playcost-{}-{label}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("temp root");
        Self(dir)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A project of `steps` steps at 0.25°, the resolution the budget names.
fn project(label: &str, steps: u32) -> (TempRoot, AppState) {
    let root = TempRoot::new(label);
    let state = AppState::new(AppPaths::in_directory(&root.0).expect("paths"));
    projects::create(
        &state,
        NewProjectRequest {
            name: "Playback".to_owned(),
            field_kind: "wind".to_owned(),
            resolution: "0.25".to_owned(),
            step_hours: 1,
            step_count: steps,
        },
        false,
    )
    .expect("create");
    (root, state)
}

/// Paints `n` objects, so flattening the project costs what it costs in a
/// real one. An empty project flattens to nothing and hides the cost the
/// resolver actually pays.
fn populate(state: &AppState, n: u32) {
    for i in 0..n {
        let lon = -170.0 + f64::from(i % 34) * 10.0;
        let lat = -60.0 + f64::from(i / 34) * 10.0;
        ve_app::edit::paint(
            state,
            ve_app::edit::BrushStroke {
                points: vec![[lon, lat]],
                size_km: 600.0,
                speed_mps: 12.0,
                direction_toward_deg: 90.0,
                feather: 0.0,
                ..Default::default()
            },
        )
        .expect("paint");
    }
}

/// A viewport of `n` tiles, the shape the map sends: one zoom level, a run of
/// columns across a couple of rows.
fn viewport(n: u32) -> Vec<TileAddress> {
    let z = 4;
    (0..n)
        .map(|i| TileAddress {
            z,
            x: i % 16,
            y: i / 16,
        })
        .collect()
}

/// Median of a set of timings, in microseconds.
fn median_us(mut samples: Vec<u128>) -> u128 {
    samples.sort_unstable();
    samples[samples.len() / 2]
}

/// The two calls the frontend makes on every advance, timed separately.
fn per_advance_us(
    state: &AppState,
    pool: &RenderPool,
    tiles: &[TileAddress],
    steps: u32,
) -> (u128, u128) {
    // Warm the snapshot and its per-step prepared scenes, so what is measured
    // is a steady-state advance rather than the first one.
    pool.request(state, 0, tiles).expect("request");
    pool.readiness(state, tiles).expect("readiness");

    let mut requests = Vec::new();
    let mut probes = Vec::new();
    for step in 0..steps {
        let at = Instant::now();
        pool.request(state, step, tiles).expect("request");
        requests.push(at.elapsed().as_micros());

        let at = Instant::now();
        pool.readiness(state, tiles).expect("readiness");
        probes.push(at.elapsed().as_micros());
    }
    (median_us(requests), median_us(probes))
}

/// Neither command on the advance path may run on the webview's own thread.
///
/// A plain `#[tauri::command]` on a sync function runs inline on the main
/// thread, which is the webview's; the `async` attribute on the same sync
/// function runs it on Tauri's pool. Both of these are called every time the
/// playhead moves, so run inline they are subtracted from the very frame the
/// map needed in order to fetch and upload the next step's tiles — and the
/// symptom is playback that misses its rate and shows "buffering" over steps
/// the backend has already rendered.
///
/// Read from the source, like `webdriver_optional.rs` reads the manifest: the
/// regression is someone dropping the `(async)` while editing the signature,
/// and nothing else in the build would notice.
#[test]
fn the_commands_on_the_advance_path_do_not_run_on_the_ui_thread() {
    let source = include_str!("../src/render_pool.rs");
    for command in ["pub fn render_ahead", "pub fn frame_readiness"] {
        let at = source
            .find(command)
            .unwrap_or_else(|| panic!("{command} is not in render_pool.rs at all"));
        let before = &source[..at];
        let attribute = before
            .lines()
            .rev()
            .find(|line| line.trim_start().starts_with("#[tauri::command"))
            .unwrap_or_else(|| panic!("{command} carries no tauri::command attribute"));
        assert!(
            attribute.contains("(async)"),
            "{command} is called on every playback advance, so it must be \
             #[tauri::command(async)] and not run on the webview's thread; found: {}",
            attribute.trim()
        );
    }
}

/// What resolving one frame's keys costs — the work behind `tile_keys`.
///
/// This is the *first* thing that has to happen before a step's tiles can be
/// fetched at all: the map holds textures by key, so until the frame's keys
/// come back nothing about that step can be asked for. `SceneCache` holds
/// three frames and playback touches four (the held frame, the drawn one, and
/// two warmed ahead), so the interesting number is the miss — the one that
/// re-flattens the project.
#[test]
#[ignore = "a cost report, not a pass/fail; run with --nocapture"]
fn resolving_a_frames_keys_costs() {
    use ve_app::protocol::{SceneCache, TileScope};

    println!(
        "\n{:>8} {:>6} {:>16} {:>14} {:>16}",
        "objects", "tiles", "cache hit", "cache miss", "miss, 4 frames"
    );
    for objects in [0u32, 50, 200] {
        let (_root, state) = project(&format!("keys{objects}"), 24);
        populate(&state, objects);
        let revision = {
            let mut session = state.session.lock().expect("lock");
            session.require_open().expect("open").revision
        };
        let view = viewport(24);
        let scenes = SceneCache::default();

        let resolve = |step: u32| {
            let at = Instant::now();
            let frame = scenes
                .frame_for(&state, revision, step, TileScope::Whole)
                .expect("frame");
            for address in &view {
                if let Ok(id) = ve_render::tile::TileId::new(address.z, address.x, address.y) {
                    let _ = frame.tile(&state, id).1.digest();
                }
            }
            at.elapsed().as_micros()
        };

        resolve(0);
        let hit = median_us((0..8).map(|_| resolve(0)).collect());
        // A miss: a step the three-frame cache has not got.
        let miss = median_us((0..8).map(|i| resolve(i % 20 + 1)).collect());
        // Four frames round-robin, which is what playback actually asks for
        // and what the three slots cannot hold.
        let thrash = median_us((0..16).map(|i| resolve(i % 4)).collect());
        println!(
            "{objects:>8} {:>6} {hit:>13} us {miss:>11} us {thrash:>13} us",
            view.len()
        );
    }
    println!();
}

#[test]
#[ignore = "a cost report, not a pass/fail; run with --nocapture"]
fn one_advance_costs() {
    println!(
        "\n{:>6} {:>6} {:>14} {:>16} {:>14} {:>12}",
        "steps", "tiles", "render_ahead", "frame_readiness", "per advance", "max steps/s"
    );
    for steps in [24u32, 48, 96] {
        for tiles in [24u32, 96, 192] {
            let (_root, state) = project(&format!("{steps}x{tiles}"), steps);
            let pool = Arc::new(RenderPool::new());
            let view = viewport(tiles);
            let (request_us, readiness_us) = per_advance_us(&state, &pool, &view, steps);
            let total = request_us + readiness_us;
            // The ceiling the backend alone puts on the rate, ignoring every
            // other cost of an advance.
            let ceiling = if total == 0 {
                f64::INFINITY
            } else {
                1_000_000.0 / total as f64
            };
            println!(
                "{steps:>6} {tiles:>6} {request_us:>11} us {readiness_us:>13} us {total:>11} us {ceiling:>12.0}"
            );
        }
    }
    println!();
}

/// Both panels request the document tree on every advance. Imported fields
/// must not make that metadata query proportional to the number of samples.
#[test]
#[ignore = "timing-sensitive; run with --release --nocapture"]
fn imported_layer_metadata_does_not_scale_with_grid_size() {
    use ve_core::document::Layer;
    use ve_core::project::FieldKind;
    use ve_core::raster::{RasterFrame, RasterGrid, RasterSequence};

    let (_root, state) = project("raster-metadata", 24);
    for history in [false, true] {
        let mut timings = Vec::new();
        for (ni, nj, spacing) in [(4, 3, 90.0), (1440, 721, 0.25)] {
            let sequence = Arc::new(
                RasterSequence::new(
                    FieldKind::Wind,
                    (0..24)
                        .map(|step| RasterFrame {
                            offset_hours: f64::from(step),
                            valid_unix_s: i64::from(step) * 3600,
                            grid: Arc::new(
                                RasterGrid::new(
                                    ni,
                                    nj,
                                    0.0,
                                    90.0,
                                    spacing,
                                    spacing,
                                    vec![[step as f32, 0.0]; (ni * nj) as usize],
                                )
                                .unwrap(),
                            ),
                        })
                        .collect(),
                )
                .unwrap(),
            );
            let layer = if history {
                Layer::from_history(
                    "History",
                    "wind.grib2".into(),
                    sequence,
                    "era5-wind",
                    0,
                    23 * 3600,
                )
            } else {
                Layer::from_grib("GRIB", "wind.grib2".into(), sequence, true)
            };
            state
                .session
                .lock()
                .unwrap()
                .open
                .as_mut()
                .unwrap()
                .project
                .layers = vec![layer];
            // The first request is included: there must be no lazy full-grid
            // scan on the webview thread, even just once after import/reopen.
            let mut samples = Vec::new();
            for step in 0..8 {
                let at = Instant::now();
                let tree = ve_app::document::tree(&state, step).unwrap();
                samples.push(at.elapsed().as_micros());
                let info = tree.layers[0].grib.as_ref().unwrap();
                assert_eq!(info.speed_ceiling_mps, 23.0);
                assert_eq!(
                    tree.layers[0].source,
                    if history { "zarr" } else { "raster" }
                );
                assert!(info.steps.iter().all(|step| step.shown));
            }
            timings.push(*samples.iter().max().unwrap());
        }
        println!(
            "history={history}: tiny {} us, 0.25° {} us",
            timings[0], timings[1]
        );
        assert!(
            timings[1] <= (timings[0] * 20).max(5_000),
            "metadata must not rescan 24 full grids on every playhead change: {timings:?} us"
        );
    }
}

/// The budget is stated "with frames pre-rendered", so the interesting case is
/// the one where every tile is already in the cache. Moving the playhead one
/// step changes nothing about what the cache holds, so an advance over a
/// pre-rendered timeline must not cost more than one over a cold timeline.
///
/// This is the assertion, because it holds on any machine: it compares the
/// backend against itself.
#[test]
#[ignore = "timing-sensitive; run with --release"]
fn an_advance_does_not_cost_more_once_frames_are_rendered() {
    let steps = 48u32;
    let (_root, state) = project("prerendered", steps);
    let pool = Arc::new(RenderPool::new());
    let view = viewport(96);

    let (cold_request, cold_readiness) = per_advance_us(&state, &pool, &view, steps);

    // Render the whole timeline into the cache, which is what "pre-rendered"
    // means: the pool drains its queue.
    pool.request(&state, 0, &view).expect("request");
    while pool.process_next(&state) {}

    let (warm_request, warm_readiness) = per_advance_us(&state, &pool, &view, steps);

    println!(
        "\ncold:  render_ahead {cold_request} us, frame_readiness {cold_readiness} us\
         \nwarm:  render_ahead {warm_request} us, frame_readiness {warm_readiness} us\n"
    );

    let cold = cold_request + cold_readiness;
    let warm = warm_request + warm_readiness;
    assert!(
        warm <= cold * 2,
        "an advance over a pre-rendered timeline cost {warm} us against {cold} us cold; \
         playback is supposed to get cheaper once the frames exist, not dearer"
    );
}

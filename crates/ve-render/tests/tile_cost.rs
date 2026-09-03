//! How long a preview tile takes to produce.
//!
//! The ceiling is asserted in release builds only, against the CPU-fallback
//! budget in spec.md 13. A debug number is reported but not enforced: it
//! depends on the profile, and the profile is a moving target.
//!
//! This exists because the cost is easy to regress invisibly. It already
//! caught one: `[profile.dev.package."*"]` optimises dependencies but not the
//! workspace's own crates, so tile sampling ran 2.4x slower under `tauri dev`
//! than in release until `[profile.dev]` was added.

use ve_core::angle::Angle;
use ve_render::aeqd::Frame;
use ve_render::cpu::CpuEvaluator;
use ve_render::gpu::GpuEvaluator;
use ve_render::preview::{Quality, render_tile};
use ve_render::scene::{DirectionMode, EdgeMode, FlatObject, Scene, SpeedMode};
use ve_render::sdf::Shape;
use ve_render::tile;

/// A scene with a few overlapping strokes: representative of real editing, and
/// deliberately full of edges so the adaptive refinement is exercised rather
/// than bypassed.
fn scene_with(id: tile::TileId, count: u32, reach_m: f64) -> Scene {
    scene_of(id, count, reach_m, false)
}

/// The same scene with the square brush's stamp, whose distance costs more per
/// segment than the disc's. Measured rather than assumed: it shares the dense
/// tile's budget with everything else.
fn square_scene_with(id: tile::TileId, count: u32, reach_m: f64) -> Scene {
    scene_of(id, count, reach_m, true)
}

fn scene_of(id: tile::TileId, count: u32, reach_m: f64, square: bool) -> Scene {
    let objects = (0..count)
        .map(|i| {
            let anchor = id.pixel_position((i * 37) % 256, (i * 53) % 256);
            let chains = vec![vec![[0.0, 0.0], [reach_m * 0.6, reach_m * 0.25]]];
            FlatObject {
                frame: Frame::new(anchor, f64::from(i) * 20.0, 100.0),
                shape: if square {
                    Shape::SweptSquare {
                        chains,
                        half_size_m: reach_m * 0.4,
                    }
                } else {
                    Shape::Capsule {
                        chains,
                        radius_m: reach_m * 0.4,
                    }
                },
                cap_radius_m: reach_m,
                speed: SpeedMode::Constant(8.0 + f64::from(i) * 3.0),
                direction: DirectionMode::Constant(Angle::new(f64::from(i) * 47.0)),
                feather: 0.3,
                divergence: 0.0,
                curl: 0.0,
                edge_mode: EdgeMode::Blend,
                gradient_axis: Angle::new(90.0),
                gradient_extent: reach_m,
                path: Vec::new(),
                clone_source: None,
            }
        })
        .collect();
    Scene { objects }
}

fn time(label: &str, tiles: u32, mut render: impl FnMut()) -> f64 {
    render(); // warm up
    let started = std::time::Instant::now();
    for _ in 0..tiles {
        render();
    }
    let per_tile = started.elapsed().as_secs_f64() * 1000.0 / f64::from(tiles);
    println!(
        "  {label:<24} {per_tile:6.2} ms/tile   {:6.0} ms for {tiles} tiles",
        per_tile * f64::from(tiles)
    );
    per_tile
}

#[test]
fn report_tile_render_cost() {
    let id = tile::TileId::new(3, 4, 3).expect("valid tile");
    let tiles = 24;
    println!("tile cost (debug_assertions={}):", cfg!(debug_assertions));

    // Two scenes, because they stress different things. A sparse one is
    // dominated by the spherical-cap cull, which rejects most pixels after a
    // single distance check. A dense one is dominated by actual evaluation,
    // and is the case that is genuinely slow.
    let mut standard_dense: f64 = 0.0;
    for (label, count, reach, square) in [
        ("sparse, 6 objects", 6u32, 500_000.0, false),
        ("dense, 60 objects", 60, 2_500_000.0, false),
        ("dense, 60 squares", 60, 2_500_000.0, true),
    ] {
        let scene = if square {
            square_scene_with(id, count, reach)
        } else {
            scene_with(id, count, reach)
        };
        println!("  {label}:");
        let exact = time("    exact", tiles, || {
            let _ = render_tile(&CpuEvaluator, &scene, id, Quality::Exact);
        });
        let standard = time("    standard (stride 4)", tiles, || {
            let _ = render_tile(&CpuEvaluator, &scene, id, Quality::Standard);
        });
        let draft = time("    draft (stride 8)", tiles, || {
            let _ = render_tile(&CpuEvaluator, &scene, id, Quality::Draft);
        });
        println!(
            "      speedup: standard {:.1}x, draft {:.1}x",
            exact / standard,
            exact / draft
        );
        // The GPU, where there is one. It is the reason the 8 ms preview
        // target in spec.md 13 is reachable at all.
        if let Ok(gpu) = GpuEvaluator::new() {
            let gpu_exact = time("    gpu exact", tiles, || {
                let _ = render_tile(&gpu, &scene, id, Quality::Exact);
            });
            let gpu_standard = time("    gpu standard", tiles, || {
                let _ = render_tile(&gpu, &scene, id, Quality::Standard);
            });
            println!(
                "      gpu vs cpu: exact {:.1}x, standard {:.1}x",
                exact / gpu_exact,
                standard / gpu_standard
            );
        }

        if count == 60 {
            standard_dense = standard_dense.max(standard);
            // The coarse path must genuinely pay for itself. It did not until
            // the evaluator's chunk size was derived from the thread count:
            // a fixed 4,096 left the lattice passes on one or two threads while
            // exact evaluation used the whole pool.
            assert!(
                standard < exact * 0.8,
                "coarse preview ({standard:.2} ms) should clearly beat exact ({exact:.2} ms)"
            );
        }
    }

    #[cfg(not(debug_assertions))]
    assert!(
        standard_dense <= 40.0,
        "preview tile took {standard_dense:.2} ms; the CPU-fallback budget is 40 ms"
    );
    let _ = standard_dense;
}

/// How much of a tile actually needs exact evaluation.
///
/// The coarse path only pays if this is small. Reported rather than asserted:
/// it is a property of the scene, not of the code.
#[test]
fn report_refined_fraction() {
    let id = tile::TileId::new(3, 4, 3).expect("valid tile");
    println!("cells needing exact evaluation:");
    for (label, count, reach) in [
        ("empty", 0u32, 1.0),
        ("sparse, 6 objects", 6, 500_000.0),
        ("dense, 60 objects", 60, 2_500_000.0),
    ] {
        let scene = scene_with(id, count, reach);
        for quality in [Quality::Standard, Quality::Draft] {
            let fraction = ve_render::preview::refined_fraction(&CpuEvaluator, &scene, id, quality)
                .expect("measures");
            println!("  {label:<20} {quality:?}: {:.1}%", fraction * 100.0);
        }
    }
}

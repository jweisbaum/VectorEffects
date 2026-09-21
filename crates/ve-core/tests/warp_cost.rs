#![allow(
    clippy::expect_used,
    reason = "test setup; clippy's allow-in-tests does not reach helpers in tests/"
)]
//! What a control-point warp costs at its worst: 50 pairs, the ceiling
//! `MAX_CONTROL_POINTS` enforces, and the 64x64 mesh a render rebuild
//! evaluates.
//!
//! Ignored and release-only, like the other cost harnesses (`tile_cost.rs`,
//! `export_cost.rs`): the debug numbers depend on the profile, and the
//! profile is a moving target. Run with:
//!
//! ```text
//! cargo test -p ve-core --release --test warp_cost -- --ignored --nocapture
//! ```
//!
//! The plan estimated ~205k radial-basis evaluations per rebuild (50 pairs x
//! 4,225 mesh vertices for a 64-cell mesh); this prints the wall time so that
//! estimate is measured rather than assumed (spec's performance rules).

use std::time::Instant;

use ve_core::document::{ControlPoint, Placement};
use ve_core::warp::{MAX_CONTROL_POINTS, Warp};

fn base() -> Placement {
    Placement::spanning(-71.0, 42.0, -70.0, 41.0, 800, 600)
}

/// Exactly `MAX_CONTROL_POINTS` pairs, on a 10x5 lattice, bent by a target no
/// homography reaches — the same shape as the module's own
/// `every_pair_lands_exactly_however_many_there_are` worst case, sized to the
/// ceiling `Warp::fit` enforces.
fn worst_case_points() -> Vec<ControlPoint> {
    let mut points = Vec::new();
    for i in 0..10 {
        for j in 0..5 {
            let u = f64::from(i) * 80.0;
            let v = f64::from(j) * 120.0;
            let lon = -71.0 + u / 800.0 + 0.02 * (v / 100.0).sin();
            let lat = 42.0 - v / 600.0 + 0.02 * (u / 100.0).cos();
            points.push(ControlPoint { u, v, lon, lat });
        }
    }
    points
}

fn ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

#[test]
#[ignore = "cost harness"]
fn fit_and_mesh_cost_at_the_control_point_ceiling() {
    let points = worst_case_points();
    assert_eq!(points.len(), MAX_CONTROL_POINTS);

    let runs = 20;

    // Warm up, then time `fit` alone.
    let _ = Warp::fit(&points, 800, 600, base());
    let started = Instant::now();
    let mut warp = Warp::fit(&points, 800, 600, base());
    for _ in 1..runs {
        warp = Warp::fit(&points, 800, 600, base());
    }
    let fit_ms = ms(started) / f64::from(runs);

    // Warm up, then time `mesh` alone, at the size a render rebuild uses.
    let cells = 64;
    let _ = warp.mesh(800, 600, cells);
    let started = Instant::now();
    let mut mesh = warp.mesh(800, 600, cells);
    for _ in 1..runs {
        mesh = warp.mesh(800, 600, cells);
    }
    let mesh_ms = ms(started) / f64::from(runs);

    let vertices = (cells + 1) * (cells + 1);
    println!(
        "warp cost ({} pairs, debug_assertions={}):",
        points.len(),
        cfg!(debug_assertions)
    );
    println!("  fit:              {fit_ms:7.3} ms");
    println!(
        "  mesh {cells}x{cells} ({vertices} vertices): {mesh_ms:7.3} ms ({:.0} radial evals)",
        f64::from(vertices) * points.len() as f64
    );
    println!("  total per rebuild: {:7.3} ms", fit_ms + mesh_ms);

    assert_eq!(mesh.len() as u32, vertices);
}

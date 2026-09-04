#![allow(
    clippy::expect_used,
    reason = "test setup; clippy's allow-in-tests does not reach helpers in tests/"
)]
//! How long it takes to put a projected grid on the project's lattice.
//!
//! This runs once per `u`/`v` pair per forecast step, at import and again
//! whenever a project holding a GRIB layer is opened — so a ten-step regional
//! file pays it twenty times. It is the only part of the import that is
//! proportional to the *source* grid rather than to the window, because every
//! grid-resolved node has to be rotated onto east and north before it can be
//! blended.
//!
//! The ceiling is asserted in release builds only; a debug number is reported
//! and not enforced. It exists because the cost is easy to regress
//! invisibly — it already found one. Rebuilding a Lambert cone's two
//! logarithms and two powers at every node rather than once per grid cost
//! HRRR 44 ms against 26 ms; the rotated grids were unaffected, their
//! constants being the basis vectors themselves.
//!
//! ```text
//! cargo test -p ve-grib --release --test resample_cost -- --nocapture
//! ```

use std::time::Instant;

use ve_core::project::Resolution;
use ve_grib::decode::ProjectedGrid;
use ve_grib::projection::{Earth, Projection, RotatedPole};
use ve_grib::resample;

/// The HRRR CONUS domain: 1799 x 1059 at 3 km on a Lambert cone, and the
/// largest projected grid a user is likely to import.
fn hrrr() -> ProjectedGrid {
    let earth = Earth::sphere(6_371_229.0);
    let projection = Projection::LambertConformal {
        lov: 262.5,
        latin1: 38.5,
        latin2: 38.5,
        north: true,
    };
    let (x1, y1) = projection.forward(&earth, 21.138_123, 237.280_472);
    ProjectedGrid {
        nx: 1799,
        ny: 1059,
        x0: x1,
        y0: y1 + 1058.0 * 3000.0,
        dx: 3000.0,
        dy: 3000.0,
        scan: 0x40,
        earth,
        projection,
        grid_relative: true,
    }
}

/// HRDPS's continental domain: 2540 x 1290 rotated, 3.3 million nodes, and
/// the most expensive convergence of the four projections — a rotated grid's
/// is a vector construction, not an `atan2`.
fn hrdps() -> ProjectedGrid {
    ProjectedGrid {
        nx: 2540,
        ny: 1290,
        x0: 0.0,
        y0: 16.711_25,
        dx: 0.0225,
        dy: 0.0225,
        scan: 0x40,
        earth: Earth::sphere(6_371_229.0),
        projection: Projection::Rotated {
            pole: RotatedPole::from_south_pole(-36.088_52, 245.305_142),
            lon_ref: 345.190_026,
        },
        grid_relative: true,
    }
}

fn cost(name: &str, grid: &ProjectedGrid) -> f64 {
    let count = grid.point_count();
    let u = vec![7.0f32; count];
    let v = vec![-3.0f32; count];
    let target = Resolution::Deg01.target_grid();
    // Once to warm the allocator and the thread pool, then measured.
    let _ = resample::resample(grid, &u, &v, &target).expect("a raster");
    let started = Instant::now();
    let raster = resample::resample(grid, &u, &v, &target).expect("a raster");
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    println!(
        "{name}: {} x {} source nodes -> {} x {} window in {ms:.1} ms",
        grid.nx, grid.ny, raster.ni, raster.nj
    );
    ms
}

/// Half a second for the largest grid anyone imports, which is the budget an
/// unstructured resample already works to (spec §4.8).
#[test]
fn a_projected_grid_resamples_within_its_budget() {
    let hrrr = cost("HRRR CONUS, Lambert", &hrrr());
    let hrdps = cost("HRDPS continental, rotated", &hrdps());
    if cfg!(debug_assertions) {
        println!("(debug build: not asserted)");
        return;
    }
    assert!(hrrr < 500.0, "HRRR took {hrrr:.1} ms");
    assert!(hrdps < 500.0, "HRDPS took {hrdps:.1} ms");
}

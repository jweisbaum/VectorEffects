#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! The window a regional fetch keeps (spec.md 4.10, M102): which columns
//! and rows of the common 1440 x 721 grid, in GRIB order. Nothing here
//! reaches the network.

use ve_core::project::Resolution;
use ve_core::region::Region;
use ve_core::regrid::TargetGrid;
use ve_zarr::source::Window;
use ve_zarr::{Field, NI, NJ, POINTS_PER_STEP, Variable};

fn lattice(west: f64, east: f64, south: f64, north: f64, full: bool) -> TargetGrid {
    Region::snapped(west, east, south, north, full, Resolution::Deg025)
        .expect("a region")
        .lattice(Resolution::Deg025)
}

/// A field whose every node holds its own column, so a crop shows which
/// columns it took; `v` holds the row.
fn indexed() -> Field {
    let (ni, nj) = (NI as usize, NJ as usize);
    Field {
        variable: Variable::Wind10m,
        u: (0..nj).flat_map(|_| (0..ni).map(|i| i as f32)).collect(),
        v: (0..nj)
            .flat_map(|j| (0..ni).map(move |_| j as f32))
            .collect(),
    }
}

/// 160 E to 160 W: GRIB counts from 0 E, so the window starts at column
/// 640 (160 / 0.25) and runs 161 columns to 200 E, contiguous.
#[test]
fn a_window_across_the_antimeridian_wraps_in_grib_columns() {
    let w = Window::of(&lattice(160.0, -160.0, -10.0, 10.0, false));
    assert_eq!((w.i0, w.ni), (640, 161));
    assert_eq!(
        (w.j0, w.nj),
        (320, 81),
        "10 N is row 320, 20 degrees is 81 rows"
    );
    assert_eq!(w.len(), 161 * 81);
    let c = indexed().cropped(&w);
    assert_eq!(c.u.len(), w.len());
    let first_row: Vec<f32> = c.u[..161].to_vec();
    let expected: Vec<f32> = (640..=800).map(|i| i as f32).collect();
    assert_eq!(first_row, expected);
    assert_eq!(c.v[0], 320.0);
    assert_eq!(c.v[c.v.len() - 1], 400.0, "10 S is row 400");
}

/// 20 W to 20 E: the window starts at 340 E, column 1360, and wraps past
/// the last column back to 0.
#[test]
fn a_window_across_the_prime_meridian_wraps_past_1440() {
    let w = Window::of(&lattice(-20.0, 20.0, 40.0, 60.0, false));
    assert_eq!((w.i0, w.ni), (1360, 161));
    let c = indexed().cropped(&w);
    let row: Vec<usize> = c.u[..161].iter().map(|x| *x as usize).collect();
    let expected: Vec<usize> = (1360..1440).chain(0..=80).collect();
    assert_eq!(row, expected);
}

/// A polar cap goes all the way round: every column once, no repeat at the
/// seam, and it reaches the pole row.
#[test]
fn a_polar_cap_window_is_every_column() {
    let w = Window::of(&lattice(-180.0, 180.0, 70.0, 90.0, true));
    assert_eq!(w.ni, 1440);
    assert_eq!((w.j0, w.nj), (0, 81));
    let c = indexed().cropped(&w);
    let mut row: Vec<usize> = c.u[..1440].iter().map(|x| *x as usize).collect();
    row.sort_unstable();
    assert_eq!(row, (0..1440).collect::<Vec<_>>());
    // And the south pole.
    let w = Window::of(&lattice(-180.0, 180.0, -90.0, -60.0, true));
    assert_eq!((w.j0, w.nj, w.ni), (600, 121, 1440));
}

/// The whole earth is the global window, which is the grid as it is: a
/// global project's file must not move.
#[test]
fn the_global_lattice_is_the_global_window() {
    let w = Window::of(&Resolution::Deg025.target_grid());
    assert_eq!(w, Window::global());
    assert_eq!(w.len(), POINTS_PER_STEP);
    let f = indexed();
    let c = f.cropped(&w);
    assert_eq!(c.u, f.u);
    assert_eq!(c.v, f.v);
}

/// A scalar field has no `v`, and its crop has none either.
#[test]
fn a_scalar_crop_keeps_v_empty() {
    let w = Window::of(&lattice(0.0, 10.0, 0.0, 10.0, false));
    let f = Field {
        variable: Variable::SeaSurfaceTemperature,
        u: vec![1.0; POINTS_PER_STEP],
        v: Vec::new(),
    };
    let c = f.cropped(&w);
    assert_eq!(c.u.len(), w.len());
    assert!(c.v.is_empty());
}

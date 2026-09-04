//! Putting a projected grid onto the project's own lattice.
//!
//! A Lambert, stereographic, Mercator or rotated grid is a regular lattice,
//! but not on lat/lon: its rows are not parallels and its columns are not
//! meridians. Nothing downstream can consume that — the render cache, both
//! evaluation kernels and the exporter all speak
//! [`RasterGrid`](ve_core::raster::RasterGrid), which is a lat/lon lattice —
//! so a projected field is resampled onto the project's grid once, at import,
//! and is an ordinary raster from then on. That is the same bargain
//! `ve_core::regrid` strikes for ICON's icosahedral mesh, and for the same
//! reason: the alternative is teaching the WGSL kernel a projection.
//!
//! Two things make this more than a loop.
//!
//! **The result is a window, not the globe.** A 2.5 km regional model covers
//! a few percent of the earth, and a global lattice of it at 0.1° would be
//! 52 MB of mostly nothing. The window is the span of *the project's own*
//! nodes that the source covers, so its nodes are the project's nodes and
//! everything outside it is simply not covered — which is what
//! `RasterGrid::sample` already returns nothing for, and is exactly the
//! behaviour a regional lat/lon grid has had all along.
//!
//! **The components are rotated at the source, not at the target.** A
//! grid-resolved message's `u` and `v` lie along the grid's axes, and the
//! angle between those and east/north varies across the grid. Rotating each
//! source node before the blend is the only order that is right near a
//! stereographic grid's pole, where the convergence turns through 360° in a
//! few cells and an interpolated grid-relative vector means nothing at all.

use rayon::prelude::*;
use ve_core::raster::{MISSING, RasterGrid, is_missing};
use ve_core::regrid::TargetGrid;

use crate::decode::ProjectedGrid;
use crate::projection::{to_earth_relative, wrap180};

/// Tolerance, in source cells, for a target node that lands a hair past the
/// last row or column.
///
/// The same slack `ve_core::raster` allows for the same reason: index
/// arithmetic puts a point exactly on the last node a few ULPs beyond it, and
/// without this the outermost row of every regional grid would drop out.
const EDGE_SLACK: f64 = 1e-6;

/// One extra target node of margin around the covered span.
///
/// The span is measured at the source's boundary *nodes*, and a projected
/// boundary bows between them. A node of margin costs a row and swallows the
/// bow; anything it wrongly includes samples as missing anyway.
const MARGIN: i64 = 1;

/// The span of target-lattice nodes a source grid reaches.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Window {
    /// First column, as an offset into the target lattice. May be negative or
    /// past the end: the lattice is periodic in longitude.
    i0: i64,
    /// First row, always within the target lattice.
    j0: i64,
    ni: u32,
    nj: u32,
}

/// The extent of the earth a projected grid covers.
#[derive(Debug, Clone, Copy)]
struct Coverage {
    lat_min: f64,
    lat_max: f64,
    /// Longitudes as a continuous run, which may leave `[-180, 180)` when the
    /// grid straddles the antimeridian.
    lon_min: f64,
    lon_max: f64,
    /// Whether the grid wraps the earth: it encircles a pole, so no walk of
    /// its boundary finds a westmost or an eastmost point.
    all_longitudes: bool,
}

/// Walks a grid's boundary and works out how much of the earth it covers.
///
/// The walk is a closed loop — top row eastward, right column down, bottom
/// row back, left column up — because the longitudes are only meaningful
/// **unwrapped**, each one carried on from the last by the short way round.
/// A grid over the pole then shows itself twice over: its boundary turns
/// through a full circle of longitude, and the pole lands inside its own
/// plane rectangle. Either is enough; both are checked, because the first
/// fails on a grid whose corner sits exactly on the pole and the second on a
/// projection that sends the pole to infinity.
fn coverage(grid: &ProjectedGrid) -> Coverage {
    let (nx, ny) = (grid.nx, grid.ny);
    let mut boundary = Vec::with_capacity(2 * (nx as usize + ny as usize));
    for i in 0..nx {
        boundary.push((i, 0));
    }
    for j in 1..ny {
        boundary.push((nx - 1, j));
    }
    for i in (0..nx.saturating_sub(1)).rev() {
        boundary.push((i, ny - 1));
    }
    for j in (1..ny.saturating_sub(1)).rev() {
        boundary.push((0, j));
    }
    // Close the loop, so the longitudes turn through a whole circle rather
    // than a circle less one step.
    if let Some(&first) = boundary.first() {
        boundary.push(first);
    }

    let prepared = grid.prepared();
    let (mut lat_min, mut lat_max) = (90.0f64, -90.0f64);
    let (mut lon_min, mut lon_max) = (f64::INFINITY, f64::NEG_INFINITY);
    let mut running = f64::NAN;
    let mut winding = 0.0f64;
    for &(i, j) in &boundary {
        let (x, y) = grid.node(i, j);
        let (lat, lon) = prepared.inverse(x, y);
        if !lat.is_finite() || !lon.is_finite() {
            continue;
        }
        lat_min = lat_min.min(lat);
        lat_max = lat_max.max(lat);
        running = if running.is_nan() {
            lon
        } else {
            let step = wrap180(lon - running);
            winding += step;
            running + step
        };
        lon_min = lon_min.min(running);
        lon_max = lon_max.max(running);
    }

    // Where the geographic poles land in the plane, and whether the grid's
    // own rectangle holds them.
    let (x_min, x_max) = (grid.x0, grid.x0 + f64::from(nx - 1) * grid.dx);
    let (y_max, y_min) = (grid.y0, grid.y0 - f64::from(ny - 1) * grid.dy);
    let holds = |point: Option<(f64, f64)>| {
        point.is_some_and(|(x, y)| (x_min..=x_max).contains(&x) && (y_min..=y_max).contains(&y))
    };
    let north = holds(prepared.pole(true));
    let south = holds(prepared.pole(false));
    let encircles = winding.abs() > 180.0;

    Coverage {
        lat_min: if south || (encircles && !north) {
            -90.0
        } else {
            lat_min
        },
        lat_max: if north || (encircles && !south) {
            90.0
        } else {
            lat_max
        },
        lon_min,
        lon_max,
        all_longitudes: north || south || encircles,
    }
}

impl Coverage {
    /// The target nodes this covers, with a node of margin.
    fn window(&self, target: &TargetGrid) -> Option<Window> {
        if !self.lat_min.is_finite() || !self.lat_max.is_finite() || self.lat_min > self.lat_max {
            return None;
        }
        let row = |lat: f64| (target.lat0 - lat) / target.dlat;
        let j0 = (row(self.lat_max).floor() as i64 - MARGIN).clamp(0, i64::from(target.nj) - 1);
        let j1 = (row(self.lat_min).ceil() as i64 + MARGIN).clamp(0, i64::from(target.nj) - 1);

        let (i0, ni) = if self.all_longitudes
            || !self.lon_min.is_finite()
            || self.lon_max - self.lon_min >= 360.0 - target.dlon
        {
            (0, target.ni)
        } else {
            let column = |lon: f64| (lon - target.lon0) / target.dlon;
            let a = column(self.lon_min).floor() as i64 - MARGIN;
            let b = column(self.lon_max).ceil() as i64 + MARGIN;
            let span = (b - a + 1).max(1);
            if span >= i64::from(target.ni) {
                (0, target.ni)
            } else {
                (a, span as u32)
            }
        };
        Some(Window {
            i0,
            j0,
            ni,
            nj: (j1 - j0 + 1) as u32,
        })
    }
}

/// Reorders a projected grid's values into canonical order and, if the file
/// resolved them along the grid, rotates them onto east and north.
///
/// Canonical is row-major from the plane's top-left corner, which is what
/// [`ProjectedGrid::node`] indexes and what the sampler below walks.
fn canonical_earth_relative(grid: &ProjectedGrid, u: &[f32], v: &[f32]) -> Vec<[f32; 2]> {
    let (nx, ny) = (grid.nx as usize, grid.ny as usize);
    let scan = grid.scan();
    let prepared = grid.prepared();
    (0..ny)
        .into_par_iter()
        .flat_map(|j| {
            (0..nx).into_par_iter().map(move |i| {
                let at = scan.file_index(i, j, nx, ny);
                let (Some(&a), Some(&b)) = (u.get(at), v.get(at)) else {
                    return [MISSING, MISSING];
                };
                if is_missing(a) || is_missing(b) || !a.is_finite() || !b.is_finite() {
                    return [MISSING, MISSING];
                }
                if !grid.grid_relative {
                    return [a, b];
                }
                let (x, y) = grid.node(i as u32, j as u32);
                let angle = prepared.convergence(x, y);
                let (a, b) = to_earth_relative(a, b, angle);
                [a, b]
            })
        })
        .collect()
}

/// Bilinear sample of a canonical array at fractional node indices.
///
/// The rule is `RasterGrid::sample`'s, because the result of this *is* a
/// `RasterGrid` and a value should not change meaning at import: a missing
/// corner is left out of the blend and the remaining weights renormalised, so
/// a coastline in a current field does not bleed calm into the water beside
/// it, and nothing at all is nothing.
fn bilinear(uv: &[[f32; 2]], nx: u32, ny: u32, fi: f64, fj: f64) -> [f32; 2] {
    let (last_i, last_j) = (f64::from(nx - 1), f64::from(ny - 1));
    if !(-EDGE_SLACK..=last_i + EDGE_SLACK).contains(&fi)
        || !(-EDGE_SLACK..=last_j + EDGE_SLACK).contains(&fj)
    {
        return [MISSING, MISSING];
    }
    let fi = fi.clamp(0.0, last_i);
    let fj = fj.clamp(0.0, last_j);
    let i0 = (fi.floor() as u32).min(nx - 1);
    let j0 = (fj.floor() as u32).min(ny - 1);
    let i1 = (i0 + 1).min(nx - 1);
    let j1 = (j0 + 1).min(ny - 1);
    let tx = (fi - f64::from(i0)) as f32;
    let ty = (fj - f64::from(j0)) as f32;

    let at = |i: u32, j: u32| uv[j as usize * nx as usize + i as usize];
    let corners = [
        (at(i0, j0), (1.0 - tx) * (1.0 - ty)),
        (at(i1, j0), tx * (1.0 - ty)),
        (at(i0, j1), (1.0 - tx) * ty),
        (at(i1, j1), tx * ty),
    ];
    let (mut total, mut u, mut v) = (0.0f32, 0.0f32, 0.0f32);
    for (sample, weight) in corners {
        if !is_missing(sample[0]) && !is_missing(sample[1]) {
            total += weight;
            u += sample[0] * weight;
            v += sample[1] * weight;
        }
    }
    if total <= 1e-6 {
        [MISSING, MISSING]
    } else {
        [u / total, v / total]
    }
}

/// Resamples a projected grid onto the span of `target` it covers.
///
/// The value at a target node is a **point sample**, interpolated bilinearly
/// from the four source nodes around it, for the reason spec §4.8 gives for
/// the unstructured case: the evaluator computes each cell's vector *at* the
/// cell, so an imported field has to mean the same thing as a painted one at
/// the same resolution. Averaging would make it mean something else.
pub fn resample(
    grid: &ProjectedGrid,
    u: &[f32],
    v: &[f32],
    target: &TargetGrid,
) -> std::result::Result<RasterGrid, String> {
    let count = grid.point_count();
    if u.len() != count || v.len() != count {
        return Err(format!(
            "a {} x {} grid with {} and {} values",
            grid.nx,
            grid.ny,
            u.len(),
            v.len()
        ));
    }
    if target.is_empty() {
        return Err("a target grid with no nodes".to_owned());
    }
    let window = coverage(grid)
        .window(target)
        .ok_or_else(|| "the grid does not land on the earth".to_owned())?;

    let uv = canonical_earth_relative(grid, u, v);
    let uv = uv.as_slice();
    let lon0 = target.lon0 + window.i0 as f64 * target.dlon;
    let lat0 = target.lat0 - window.j0 as f64 * target.dlat;

    let prepared = grid.prepared();
    let samples: Vec<[f32; 2]> = (0..window.nj)
        .into_par_iter()
        .flat_map(|j| {
            let lat = lat0 - f64::from(j) * target.dlat;
            (0..window.ni).into_par_iter().map(move |i| {
                let lon = lon0 + f64::from(i) * target.dlon;
                let (fi, fj) = grid.locate_with(&prepared, lat, lon);
                bilinear(uv, grid.nx, grid.ny, fi, fj)
            })
        })
        .collect();

    RasterGrid::new(
        window.ni,
        window.nj,
        lon0,
        lat0,
        target.dlon,
        target.dlat,
        samples,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::{Earth, Projection, RotatedPole};

    fn target(step: f64) -> TargetGrid {
        TargetGrid {
            ni: (360.0 / step).round() as u32,
            nj: (180.0 / step).round() as u32 + 1,
            lon0: -180.0,
            lat0: 90.0,
            dlon: step,
            dlat: step,
        }
    }

    /// NCEP's grid 212: Lambert conformal, 185 x 129 at 40.635 km, tangent at
    /// 25° N, oriented on 95° W. The published corner is 12.190° N,
    /// 133.459° W.
    fn grid_212() -> ProjectedGrid {
        let earth = Earth::sphere(6_371_229.0);
        let projection = Projection::LambertConformal {
            lov: 265.0,
            latin1: 25.0,
            latin2: 25.0,
            north: true,
        };
        let (x1, y1) = projection.forward(&earth, 12.190, 226.541);
        ProjectedGrid {
            nx: 185,
            ny: 129,
            x0: x1,
            y0: y1 + 128.0 * 40_635.0,
            dx: 40_635.0,
            dy: 40_635.0,
            // 0x40: west to east, then south to north, so the file's first
            // row is the bottom one.
            scan: 0x40,
            earth,
            projection,
            grid_relative: true,
        }
    }

    /// A field whose value is its own position, so a resample can be checked
    /// against where it says it is rather than against another resample.
    fn positional(grid: &ProjectedGrid) -> (Vec<f32>, Vec<f32>) {
        let scan = grid.scan();
        let (nx, ny) = (grid.nx as usize, grid.ny as usize);
        let mut u = vec![0.0f32; nx * ny];
        let mut v = vec![0.0f32; nx * ny];
        for j in 0..ny {
            for i in 0..nx {
                let (lat, lon) = grid.position(i as u32, j as u32);
                let at = scan.file_index(i, j, nx, ny);
                // Eastward from the true position, northward from the true
                // latitude — a smooth field with no seam inside the grid.
                u[at] = wrap180(lon - 265.0) as f32;
                v[at] = lat as f32;
            }
        }
        (u, v)
    }

    /// A resampled node must carry the value the source field has *there*.
    #[test]
    fn a_resampled_node_holds_the_value_at_its_own_position() {
        let grid = grid_212();
        let (u, v) = positional(&grid);
        // The field is earth-resolved for this test: it is a position, not a
        // wind, and rotating it would be meaningless.
        let grid = ProjectedGrid {
            grid_relative: false,
            ..grid
        };
        let raster = resample(&grid, &u, &v, &target(0.5)).expect("a raster");
        let mut checked = 0;
        for lat in [20.0, 30.0, 40.0, 50.0] {
            for lon in [-120.0, -100.0, -80.0, -70.0] {
                let sample = raster.sample(lon, lat).expect("inside grid 212");
                assert!(
                    (f64::from(sample.u) - wrap180(lon - 265.0)).abs() < 0.05,
                    "at ({lon}, {lat}) the eastward sample was {}",
                    sample.u
                );
                assert!(
                    (f64::from(sample.v) - lat).abs() < 0.05,
                    "at ({lon}, {lat}) the northward sample was {}",
                    sample.v
                );
                checked += 1;
            }
        }
        assert_eq!(checked, 16);
    }

    /// Outside the source grid there is nothing — that is the whole of what
    /// "a regional model" means once it is on a global lattice.
    #[test]
    fn a_regional_grid_covers_its_region_and_no_more() {
        let grid = ProjectedGrid {
            grid_relative: false,
            ..grid_212()
        };
        let (u, v) = positional(&grid);
        let raster = resample(&grid, &u, &v, &target(0.5)).expect("a raster");
        for (lon, lat) in [(-100.0, 40.0), (-80.0, 30.0)] {
            assert!(raster.sample(lon, lat).is_some(), "({lon}, {lat}) is CONUS");
        }
        for (lon, lat) in [
            (0.0, 50.0),     // Europe
            (140.0, 35.0),   // Japan
            (-100.0, 80.0),  // far north of the grid
            (-100.0, -20.0), // the southern hemisphere
            (-170.0, 40.0),  // the Pacific, west of the grid
        ] {
            assert!(
                raster.sample(lon, lat).is_none(),
                "({lon}, {lat}) is outside grid 212 but sampled"
            );
        }
    }

    /// The window is a span of the project's own nodes, not a lattice of its
    /// own: an imported field has to line up with what the project exports.
    #[test]
    fn the_window_sits_on_the_projects_nodes() {
        let grid = grid_212();
        let (u, v) = positional(&grid);
        let target = target(0.25);
        let raster = resample(&grid, &u, &v, &target).expect("a raster");
        assert_eq!(raster.dlon, target.dlon);
        assert_eq!(raster.dlat, target.dlat);
        let column = (raster.lon0 - target.lon0) / target.dlon;
        let row = (target.lat0 - raster.lat0) / target.dlat;
        assert!(
            (column - column.round()).abs() < 1e-9,
            "lon0 is off-lattice"
        );
        assert!((row - row.round()).abs() < 1e-9, "lat0 is off-lattice");
        // A CONUS grid is a window, not the globe.
        assert!(raster.ni < target.ni / 2, "{} columns", raster.ni);
        assert!(!raster.wraps);
    }

    /// A grid over the pole has no westmost point, and a boundary walk cannot
    /// find one. It must come out spanning every longitude and reaching the
    /// pole itself.
    #[test]
    fn a_grid_over_the_pole_covers_every_longitude() {
        let earth = Earth::sphere(6_371_229.0);
        let projection = Projection::PolarStereographic {
            lov: 0.0,
            lad: 60.0,
            north: true,
        };
        // 2000 km square centred on the pole.
        let grid = ProjectedGrid {
            nx: 201,
            ny: 201,
            x0: -1_000_000.0,
            y0: 1_000_000.0,
            dx: 10_000.0,
            dy: 10_000.0,
            scan: 0x40,
            earth,
            projection,
            grid_relative: false,
        };
        let (u, v) = positional(&grid);
        let raster = resample(&grid, &u, &v, &target(0.5)).expect("a raster");
        assert!(raster.wraps, "a grid over the pole must wrap");
        assert_eq!(raster.ni, target(0.5).ni);
        assert!(
            (raster.lat0 - 90.0).abs() < 1e-9,
            "the window stops at {}",
            raster.lat0
        );
        // The pole itself, and a ring around it, are covered from every side.
        assert!(raster.sample(0.0, 90.0).is_some());
        for lon in [-179.0, -90.0, 0.0, 90.0, 179.0] {
            assert!(
                raster.sample(lon, 85.0).is_some(),
                "({lon}, 85) should be under the cap"
            );
            assert!(
                raster.sample(lon, 60.0).is_none(),
                "({lon}, 60) is outside a 2000 km cap"
            );
        }
    }

    /// A grid that straddles the antimeridian is one grid, not two.
    #[test]
    fn a_grid_across_the_antimeridian_stays_whole() {
        let earth = Earth::sphere(6_371_229.0);
        let projection = Projection::Mercator {
            lad: 20.0,
            lon_ref: 170.0,
        };
        let (_, y1) = projection.forward(&earth, 10.0, 170.0);
        let grid = ProjectedGrid {
            nx: 121,
            ny: 61,
            x0: 0.0,
            y0: y1 + 60.0 * 25_000.0,
            dx: 25_000.0,
            dy: 25_000.0,
            scan: 0x40,
            earth,
            projection,
            grid_relative: false,
        };
        let (u, v) = positional(&grid);
        let raster = resample(&grid, &u, &v, &target(0.5)).expect("a raster");
        assert!(!raster.wraps, "a 3000 km grid does not wrap the earth");
        for lon in [172.0, 178.0, -179.0, -175.0] {
            let sample = raster
                .sample(lon, 15.0)
                .unwrap_or_else(|| panic!("({lon}, 15) is inside the grid"));
            assert!(
                (f64::from(sample.u) - wrap180(lon - 265.0)).abs() < 0.05,
                "at ({lon}, 15) the eastward sample was {}",
                sample.u
            );
        }
        assert!(raster.sample(120.0, 15.0).is_none());
    }

    /// A wind blowing along the grid's own `+y` axis is a wind blowing toward
    /// grid north, and grid north is not true north. The rotation has to
    /// happen, and it has to happen per node.
    #[test]
    fn a_grid_resolved_wind_comes_out_pointing_true() {
        let grid = grid_212();
        let count = grid.point_count();
        // Ten metres a second straight up the grid, everywhere.
        let (u, v) = (vec![0.0f32; count], vec![10.0f32; count]);
        let raster = resample(&grid, &u, &v, &target(0.5)).expect("a raster");

        // On the orientation meridian the grid's north is true north.
        let on_lov = raster.sample(-95.0, 40.0).expect("on the meridian");
        assert!(on_lov.u.abs() < 0.02, "eastward came out {}", on_lov.u);
        assert!((on_lov.v - 10.0).abs() < 0.02);

        // East of it the grid leans clockwise, so the same wind has an
        // eastward part; west of it, a westward one. The angle is
        // `sin(25°)·Δλ` for a cone tangent at 25°.
        for (lon, sign) in [(-75.0, 1.0f64), (-115.0, -1.0f64)] {
            let sample = raster.sample(lon, 40.0).expect("inside grid 212");
            let expected = (25f64.to_radians().sin() * (lon + 95.0).to_radians()).sin() * 10.0;
            assert!(
                (f64::from(sample.u) - expected).abs() < 0.05,
                "at {lon} the eastward part was {} not {expected}",
                sample.u
            );
            assert!(
                sign * f64::from(sample.u) > 0.5,
                "at {lon} it leaned the wrong way"
            );
            // The wind is still 10 m/s: a rotation is not a scaling.
            assert!(
                (f64::from(sample.u).hypot(f64::from(sample.v)) - 10.0).abs() < 0.05,
                "at {lon} the speed became {}",
                f64::from(sample.u).hypot(f64::from(sample.v))
            );
        }
    }

    /// A missing source node is missing where it lands, and does not bleed
    /// into the nodes beside it any further than a bilinear blend reaches.
    #[test]
    fn a_hole_in_the_source_stays_a_hole() {
        let grid = ProjectedGrid {
            grid_relative: false,
            ..grid_212()
        };
        let count = grid.point_count();
        let mut u = vec![3.0f32; count];
        let v = vec![4.0f32; count];
        // Knock out a block of about 3° square in the middle of the grid.
        let scan = grid.scan();
        let (nx, ny) = (grid.nx as usize, grid.ny as usize);
        for j in 60..70 {
            for i in 90..100 {
                u[scan.file_index(i, j, nx, ny)] = MISSING;
            }
        }
        let raster = resample(&grid, &u, &v, &target(0.5)).expect("a raster");
        let (lat, lon) = grid.position(95, 65);
        assert!(
            raster.sample(lon, lat).is_none(),
            "the middle of the hole sampled"
        );
        // Well outside the hole the field is untouched.
        let (lat, lon) = grid.position(40, 65);
        let sample = raster.sample(lon, lat).expect("outside the hole");
        assert!((sample.u - 3.0).abs() < 1e-4 && (sample.v - 4.0).abs() < 1e-4);
    }

    /// Every scanning mode is the same grid, so every scanning mode has to
    /// resample to the same raster.
    #[test]
    fn the_scanning_mode_does_not_change_the_field() {
        let base = ProjectedGrid {
            grid_relative: false,
            ..grid_212()
        };
        let (u, v) = positional(&base);
        let reference = resample(&base, &u, &v, &target(0.5)).expect("a raster");
        for scan in [0x00u8, 0x40, 0x80, 0xc0, 0x20, 0x60, 0x50, 0x10] {
            let grid = ProjectedGrid { scan, ..base };
            let (u, v) = positional(&grid);
            let raster = resample(&grid, &u, &v, &target(0.5)).expect("a raster");
            assert_eq!(
                raster.hash, reference.hash,
                "scanning mode {scan:#04x} produced a different field"
            );
        }
    }

    /// A rotated grid resamples like any other, and its convergence is the
    /// one the rotation implies.
    #[test]
    fn a_rotated_grid_resamples_onto_the_lattice() {
        // HRDPS's rotation, and a small window of its domain.
        let pole = RotatedPole::from_south_pole(-36.088_52, 245.305_142);
        let projection = Projection::Rotated {
            pole,
            lon_ref: -14.81,
        };
        let earth = Earth::sphere(6_371_229.0);
        let grid = ProjectedGrid {
            nx: 200,
            ny: 150,
            x0: 0.0,
            y0: 10.0,
            dx: 0.1,
            dy: 0.1,
            scan: 0x00,
            earth,
            projection,
            grid_relative: false,
        };
        let (u, v) = positional(&grid);
        let raster = resample(&grid, &u, &v, &target(0.5)).expect("a raster");
        // Sample at the position of a node well inside the grid and check it
        // reads back its own coordinates.
        let (lat, lon) = grid.position(100, 75);
        let sample = raster.sample(lon, lat).expect("inside the rotated grid");
        assert!(
            (f64::from(sample.v) - lat).abs() < 0.05,
            "the northward sample was {} at latitude {lat}",
            sample.v
        );
        assert!(raster.sample(lon + 180.0, lat).is_none());
    }
}

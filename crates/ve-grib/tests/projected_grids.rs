#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! Projected grids, against the real section 3 bytes that carry them.
//!
//! The fixtures below are section 3 of a real message from each of the five
//! grid definitions the forecast centres put regional models on, copied
//! verbatim. They are a few dozen octets each, which is why they are here
//! rather than as files: the messages they came from are megabytes, and the
//! grid definition is all this needs.
//!
//! **The assertions are geography, not arithmetic.** A projection that has
//! its cone constant, its hemisphere or its orientation wrong still produces
//! plausible numbers; what it does not do is put Alaska over Alaska. Each
//! grid is therefore checked to contain places it must contain and to
//! exclude places it must not — statements about what these models are, which
//! nothing in the decoder had a hand in.
//!
//! There is a second, sharper check where the file allows one. Templates 3.10
//! and 3.32769 state their **last** grid point as well as their first, and
//! nothing in the projection consumes it: walking the grid to that corner and
//! landing on the stated position exercises the earth, the projection, the
//! increments and the scanning order at once.
//!
//! The sweep at the bottom does all of this over a whole directory of files
//! and is `#[ignore]`d, since it needs one:
//!
//! ```text
//! VE_TEST_GRID_DIR=~/grib2 cargo test -p ve-grib --release \
//!     --test projected_grids -- --ignored --nocapture
//! ```

use std::collections::BTreeMap;
use std::path::PathBuf;

use ve_core::project::FieldKind;
use ve_core::project::Resolution;
use ve_core::raster::RasterGrid;
use ve_grib::ReferenceTime;
use ve_grib::decode::{self, Grid, Header, Message, ProjectedGrid, Scan};
use ve_grib::import;
use ve_grib::projection::Projection;
use ve_grib::resample;

// --- Fixtures ---------------------------------------------------------------

/// Environment Canada's 2.5 km continental model: template 3.1, a rotated
/// lat/lon grid whose southern pole sits at 36.09 S, 245.31 E.
///
/// From `HRDPS_CONTINENTAL_2KM_TEST_firstmessage.grib2`, section 3 verbatim.
const HRDPS_2P5KM: &str = concat!(
    "0000005403000031ff380000000106ffffffffffffffffffffffffffffff",
    "000009ec0000050a00000000ffffffff80bb8cb214932e8a3800fefe5202",
    "85b6d8000057e4000057e4408226aac80e9f0f3600000000",
);

/// NOAA's air-quality model over Hawaii: template 3.10, Mercator, true at
/// 20 N, 2.5 km.
///
/// From `AQM_HI_TEST_firstmessage.grib2`, section 3 verbatim.
const AQM_HI: &str = concat!(
    "00000048030000011a210000000a06000000000000000000000000000000",
    "00000141000000e10113c5a80bd47cf83801312d0001604b800c494f3840",
    "00000000002625a0002625a0",
);

/// The same model over Alaska: template 3.20, polar stereographic about the
/// north pole, true at 60 N, oriented on 150 W, 5.953 km.
///
/// From `AQM_AK_TEST_firstmessage.grib2`, section 3 verbatim.
const AQM_AK: &str = concat!(
    "0000004103000006f6210000001406000000000000000000000000000000",
    "0000033900000229026a70500ad0630808039387000c845880005ad5e800",
    "5ad5e80040",
);

/// The same model over the lower 48: template 3.30, Lambert conformal on
/// NCEP's grid 148 — 442 x 265 at 12 km, secant at 33 N and 45 N.
///
/// From `AQM_CONUS_PM25_TEST_firstmessage.grib2`, section 3 verbatim.
const AQM_CONUS: &str = concat!(
    "0000005103000001c98a0000001e06000000000000000000000000000000",
    "000001ba00000109014cf6480e4486e00801f78a400fad0fc000b71b0000",
    "b71b00004001f78a4002aea5400000000000000000",
);

/// NCEP's local template 3.32769, the Arakawa non-E staggered rotated grid,
/// centred at 54 N 106 W.
///
/// From `RAP_WRFMSLF_TEST_firstmessage.grib2`, section 3 verbatim.
const RAP_ROTATED: &str = concat!(
    "000000500300000c20b20000800106000000000000000000000000000000",
    "000003b900000342000000000000000080a1998b0d2ae1ea380337f9800f",
    "23bb800742b8080742b8084002c6efe80159c791",
);

/// NCEP's arctic wave grid: template 3.20 over the north pole itself, and
/// the one grid in the corpus defined on WGS 84 rather than a sphere.
///
/// From `GDAS_WAVE_ARCTIC_TEST_firstmessage.grib2`, section 3 verbatim.
const GDAS_WAVE_ARCTIC: &str = concat!(
    "000000410300000f71440000001405000000000000000000000000000000",
    "000003ee000003ee021b64f012c684c000042c1d80000000000089544000",
    "8954400040",
);

/// The National Blend of Models over Alaska: the same stereographic domain
/// as `AQM_AK` at a different resolution, on a 6 371 200 m sphere, and with
/// scanning mode 0x50 — earth-resolved components and alternate rows
/// reversed.
///
/// From `NBM_AK_TEST_firstmessage.grib2`, section 3 verbatim.
const NBM_AK: &str = concat!(
    "000000410300001bcdc100000014010000613780ff000000ffff000000ff",
    "0000067100000451026a70500ad0630830039387000c845880002d6b3000",
    "2d6b300050",
);

/// The blend over Hawaii: a Mercator grid whose orientation field says 200
/// degrees and whose geometry says none.
///
/// From `NBM_HI_TEST_firstmessage.grib2`, section 3 verbatim.
const NBM_HI: &str = concat!(
    "000000480300000559a10000000a010000613780ff000000ffff000000ff",
    "000002710000023100dafc8c0b9fede40001312d000199dbd40c83bb7850",
    "0bebc200002625a0002625a0",
);

// --- Helpers ----------------------------------------------------------------

fn bytes(hex: &str) -> Vec<u8> {
    assert!(hex.len().is_multiple_of(2), "an octet is two hex digits");
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex"))
        .collect()
}

fn grid_of(hex: &str) -> ProjectedGrid {
    match decode::grid_of(&bytes(hex)).expect("a grid") {
        Grid::Projected(grid) => grid,
        other => panic!("expected a projected grid, got {other:?}"),
    }
}

/// A field whose value at a node is that node's own position, so a resample
/// can be checked against where it claims to be.
fn positional(grid: &ProjectedGrid) -> (Vec<f32>, Vec<f32>) {
    let scan = Scan(grid.scan);
    let (nx, ny) = (grid.nx as usize, grid.ny as usize);
    let mut u = vec![0.0f32; nx * ny];
    let mut v = vec![0.0f32; nx * ny];
    for j in 0..ny {
        for i in 0..nx {
            let (lat, lon) = grid.position(i as u32, j as u32);
            let at = scan.file_index(i, j, nx, ny);
            u[at] = lon as f32;
            v[at] = lat as f32;
        }
    }
    (u, v)
}

/// The grid resampled onto a project's lattice, with position for a field.
fn raster(grid: &ProjectedGrid, resolution: Resolution) -> RasterGrid {
    // Position is not a wind: it is already east and north, whatever the
    // file's components are resolved along.
    let grid = ProjectedGrid {
        grid_relative: false,
        ..*grid
    };
    let (u, v) = positional(&grid);
    resample::resample(&grid, &u, &v, &resolution.target_grid()).expect("a raster")
}

/// Asserts a grid covers the places it is for and none of the places it is
/// not, by resampling it and asking the raster.
#[track_caller]
fn covers(grid: &ProjectedGrid, inside: &[(&str, f64, f64)], outside: &[(&str, f64, f64)]) {
    let raster = raster(grid, Resolution::Deg025);
    for &(what, lat, lon) in inside {
        let sample = raster
            .sample(lon, lat)
            .unwrap_or_else(|| panic!("{what} ({lon}, {lat}) is not covered but should be"));
        // Every node carries its own position, so a covered node proves the
        // projection put it where the sample asked for it.
        assert!(
            (f64::from(sample.v) - lat).abs() < 0.3,
            "{what}: the node at ({lon}, {lat}) says it is at latitude {}",
            sample.v
        );
    }
    for &(what, lat, lon) in outside {
        assert!(
            raster.sample(lon, lat).is_none(),
            "{what} ({lon}, {lat}) is covered but should not be"
        );
    }
}

// --- The five templates ------------------------------------------------------

/// Template 3.1: the WMO rotated lat/lon grid.
#[test]
fn a_rotated_lat_lon_grid_covers_canada() {
    let grid = grid_of(HRDPS_2P5KM);
    assert_eq!((grid.nx, grid.ny), (2540, 1290));
    assert!((grid.dx - 0.0225).abs() < 1e-12 && (grid.dy - 0.0225).abs() < 1e-12);
    assert!(
        grid.grid_relative,
        "HRDPS resolves its winds along the grid"
    );
    assert!(matches!(grid.projection, Projection::Rotated { .. }));
    // The rotation's whole point: rotated (0, 0) is not (0, 0).
    let (lat, lon) = grid.projection.inverse(&grid.earth, -345.190026, 0.0);
    assert!(
        (lat - 53.911_48).abs() < 1e-4 && ((lon + 360.0) % 360.0 - 245.305_142).abs() < 1e-4,
        "the rotated origin came out at ({lon}, {lat})"
    );
    covers(
        &grid,
        &[
            ("Toronto", 43.65, -79.38),
            ("Vancouver", 49.28, -123.12),
            ("Iqaluit", 63.75, -68.52),
            ("Chicago", 41.88, -87.63),
        ],
        &[
            ("London", 51.51, -0.13),
            ("Tokyo", 35.68, 139.69),
            ("Sydney", -33.87, 151.21),
            ("Honolulu", 21.31, -157.86),
        ],
    );
}

/// Template 3.10: Mercator.
#[test]
fn a_mercator_grid_covers_hawaii() {
    let grid = grid_of(AQM_HI);
    assert_eq!((grid.nx, grid.ny), (321, 225));
    assert!((grid.dx - 2500.0).abs() < 1e-9 && (grid.dy - 2500.0).abs() < 1e-9);
    assert!(matches!(
        grid.projection,
        Projection::Mercator { lad, .. } if (lad - 20.0).abs() < 1e-9
    ));
    // Mercator's rows are parallels, so its corners share latitudes.
    let (north_west, north_east) = (grid.position(0, 0), grid.position(grid.nx - 1, 0));
    assert!((north_west.0 - north_east.0).abs() < 1e-9);
    covers(
        &grid,
        &[("Honolulu", 21.31, -157.86), ("Hilo", 19.71, -155.09)],
        &[
            ("Anchorage", 61.22, -149.90),
            ("Denver", 39.74, -104.99),
            ("Guam", 13.44, 144.79),
        ],
    );
}

/// Template 3.20: polar stereographic.
#[test]
fn a_polar_stereographic_grid_covers_alaska() {
    let grid = grid_of(AQM_AK);
    assert_eq!((grid.nx, grid.ny), (825, 553));
    assert!((grid.dx - 5953.0).abs() < 1e-9);
    assert!(matches!(
        grid.projection,
        Projection::PolarStereographic { lov, lad, north }
            if (lov - 210.0).abs() < 1e-9 && (lad - 60.0).abs() < 1e-9 && north
    ));
    covers(
        &grid,
        &[
            ("Anchorage", 61.22, -149.90),
            ("Fairbanks", 64.84, -147.72),
            ("Utqiagvik", 71.29, -156.79),
            ("Adak", 51.88, -176.66),
        ],
        &[
            ("Denver", 39.74, -104.99),
            ("Honolulu", 21.31, -157.86),
            ("Reykjavik", 64.15, -21.94),
            ("the south pole", -89.9, 0.0),
        ],
    );
}

/// Template 3.30: Lambert conformal conic, and the one whose cone constant
/// has two standard parallels to satisfy.
#[test]
fn a_lambert_grid_covers_the_lower_48() {
    let grid = grid_of(AQM_CONUS);
    assert_eq!((grid.nx, grid.ny), (442, 265));
    assert!((grid.dx - 12_000.0).abs() < 1e-9 && (grid.dy - 12_000.0).abs() < 1e-9);
    assert!(matches!(
        grid.projection,
        Projection::LambertConformal { latin1, latin2, north, .. }
            if (latin1 - 33.0).abs() < 1e-9 && (latin2 - 45.0).abs() < 1e-9 && north
    ));
    covers(
        &grid,
        &[
            ("Denver", 39.74, -104.99),
            ("Miami", 25.77, -80.19),
            ("Seattle", 47.61, -122.33),
            ("Boston", 42.36, -71.06),
            ("San Diego", 32.72, -117.16),
        ],
        &[
            ("Anchorage", 61.22, -149.90),
            ("Honolulu", 21.31, -157.86),
            ("Panama City", 8.98, -79.52),
            ("Reykjavik", 64.15, -21.94),
        ],
    );
}

/// NCEP's template 3.32769, whose increments are derived from its corners.
#[test]
fn the_ncep_rotated_grid_covers_north_america() {
    let grid = grid_of(RAP_ROTATED);
    assert_eq!((grid.nx, grid.ny), (953, 834));
    assert!(matches!(grid.projection, Projection::Rotated { .. }));
    // The centre of the grid is what the template states the rotation by, so
    // the middle node lands on it.
    let (lat, lon) = grid.position(grid.nx / 2, grid.ny / 2);
    assert!(
        (lat - 54.0).abs() < 0.1 && ((lon + 360.0) % 360.0 - 254.0).abs() < 0.1,
        "the middle of the grid is at ({lon}, {lat}), not the stated centre"
    );
    covers(
        &grid,
        &[
            ("Denver", 39.74, -104.99),
            ("Winnipeg", 49.90, -97.14),
            ("Mexico City", 19.43, -99.13),
            ("Miami", 25.77, -80.19),
        ],
        &[("Tokyo", 35.68, 139.69), ("Cape Town", -33.92, 18.42)],
    );
}

// --- The awkward ones --------------------------------------------------------

/// A grid over the pole has no eastmost point, and the values must reach the
/// pole from every side rather than stopping at a boundary walk's idea of
/// where the grid ends.
#[test]
fn a_grid_over_the_pole_wraps_the_earth() {
    let grid = grid_of(GDAS_WAVE_ARCTIC);
    assert_eq!((grid.nx, grid.ny), (1006, 1006));
    // Code table 3.2's shape 5: this is the one grid in the corpus on an
    // ellipsoid rather than a sphere.
    assert!(
        grid.earth.e2 > 0.006 && (grid.earth.a - 6_378_137.0).abs() < 1e-6,
        "the arctic wave grid is on WGS 84, got {:?}",
        grid.earth
    );
    let raster = raster(&grid, Resolution::Deg025);
    assert!(raster.wraps, "a grid over the pole must wrap");
    assert!((raster.lat0 - 90.0).abs() < 1e-9);
    assert!(raster.sample(0.0, 90.0).is_some(), "the pole itself");
    for lon in [-179.0, -90.0, -1.0, 0.0, 1.0, 90.0, 179.0] {
        assert!(
            raster.sample(lon, 85.0).is_some(),
            "({lon}, 85) is under an arctic cap"
        );
    }
    assert!(
        raster.sample(0.0, 0.0).is_none(),
        "the equator is not arctic"
    );
}

/// Scanning mode 0x50 reverses alternate rows. Read as if it did not, the
/// field comes out combed into stripes — and the grid is the same grid.
#[test]
fn an_alternating_scan_reads_the_same_domain() {
    let alaska = grid_of(NBM_AK);
    assert_eq!(alaska.scan, 0x50);
    assert!(Scan(alaska.scan).boustrophedon());
    assert!(
        !alaska.grid_relative,
        "the blend resolves its components on the earth"
    );
    // A different resolution and a different sphere from AQM_AK, and the same
    // stereographic domain: two independent files agreeing is worth more than
    // either agreeing with itself. They share their first grid point exactly,
    // and their far corners to a tenth of a degree — which is as close as
    // their own increments agree, 2976.56 m being not quite half 5953 m.
    let reference = grid_of(AQM_AK);
    let corners = |grid: &ProjectedGrid| {
        [
            grid.position(0, grid.ny - 1),
            grid.position(grid.nx - 1, 0),
            grid.position(grid.nx / 2, grid.ny / 2),
        ]
    };
    for (k, ((lat_a, lon_a), (lat_b, lon_b))) in corners(&alaska)
        .into_iter()
        .zip(corners(&reference))
        .enumerate()
    {
        let tolerance = if k == 0 { 1e-6 } else { 0.1 };
        assert!(
            (lat_a - lat_b).abs() < tolerance && (lon_a - lon_b).abs() < tolerance,
            "the two Alaska grids disagree at corner {k}:              ({lon_a}, {lat_a}) against ({lon_b}, {lat_b})"
        );
    }
    covers(
        &alaska,
        &[("Anchorage", 61.22, -149.90), ("Fairbanks", 64.84, -147.72)],
        &[("Denver", 39.74, -104.99)],
    );
}

/// A Mercator grid whose orientation field is not zero, and whose geometry
/// says it is: the two corners it states are where an unrotated grid puts
/// them, to a few parts per million of its own increment.
#[test]
fn a_mercator_orientation_that_contradicts_the_grid_is_ignored() {
    let grid = grid_of(NBM_HI);
    // The increments here are derived from the corners, because this file's
    // resolution flags say it did not give them. That they come back as the
    // 2500 m the file also states is the whole argument for reading the
    // orientation as zero.
    assert!(
        (grid.dx - 2500.0).abs() < 0.05 && (grid.dy - 2500.0).abs() < 0.05,
        "derived increments {} x {} against a stated 2500 m",
        grid.dx,
        grid.dy
    );
    covers(
        &grid,
        &[("Honolulu", 21.31, -157.86), ("Hilo", 19.71, -155.09)],
        &[("Anchorage", 61.22, -149.90), ("Guam", 13.44, 144.79)],
    );
}

/// Both templates that state a last grid point must land on it exactly.
#[test]
fn a_stated_last_point_is_where_the_grid_walks_to() {
    // AQM_HI's octets 52-59: 23.088 N, 206.131 E. RAP's octets 73-80:
    // 46.591976 N, 22.661009 E.
    for (name, hex, lat2, lon2, tolerance) in [
        ("AQM_HI", AQM_HI, 23.088, 206.131, 0.01),
        ("RAP", RAP_ROTATED, 46.591_976, 22.661_009, 1e-6),
    ] {
        let grid = grid_of(hex);
        // Scanning mode 0x40 on both: west to east, then south to north, so
        // the file's last point is the north-east corner.
        assert_eq!(grid.scan, 0x40);
        let (lat, lon) = grid.position(grid.nx - 1, 0);
        let lon = (lon + 360.0) % 360.0;
        assert!(
            (lat - lat2).abs() < tolerance && (lon - lon2).abs() < tolerance,
            "{name}: walked to ({lon}, {lat}), the file states ({lon2}, {lat2})"
        );
    }
}

/// A regional grid on a global lattice is a window of that lattice, not a
/// globe of mostly nothing: its nodes are the project's nodes, and it holds
/// only the rows and columns it reaches.
#[test]
fn a_regional_grid_becomes_a_window_of_the_projects_lattice() {
    let target = Resolution::Deg01.target_grid();
    let raster = raster(&grid_of(AQM_CONUS), Resolution::Deg01);
    assert_eq!(raster.dlon, target.dlon);
    assert_eq!(raster.dlat, target.dlat);
    let column = (raster.lon0 - target.lon0) / target.dlon;
    let row = (target.lat0 - raster.lat0) / target.dlat;
    assert!(
        (column - column.round()).abs() < 1e-9,
        "off-lattice longitude"
    );
    assert!((row - row.round()).abs() < 1e-9, "off-lattice latitude");
    assert!(
        (raster.len() as f64) < 0.05 * target.len() as f64,
        "a CONUS grid took {} of the globe's {} nodes",
        raster.len(),
        target.len()
    );
    assert!(!raster.wraps);
}

/// The spacing a projected file reports is what picks a new project's
/// resolution, so it has to be in the same units the resolutions are.
#[test]
fn a_projected_grid_reports_its_spacing_in_degrees() {
    for (name, hex, expected) in [
        ("HRDPS 2.5 km", HRDPS_2P5KM, 0.0225),
        ("AQM Hawaii 2.5 km", AQM_HI, 0.0225),
        ("AQM Alaska 5.953 km", AQM_AK, 0.0535),
        ("AQM CONUS 12 km", AQM_CONUS, 0.1079),
    ] {
        let spacing = grid_of(hex).nominal_spacing_deg();
        assert!(
            (spacing - expected).abs() < 0.001,
            "{name} reported {spacing}°"
        );
    }
}

/// The whole import path, not just the grid: a `u` and a `v` message on a
/// Lambert grid must come out as a wind layer on the project's lattice, with
/// its components rotated onto east and north.
#[test]
fn a_projected_u_and_v_pair_imports_as_a_wind_field() {
    let grid = grid_of(AQM_CONUS);
    assert!(grid.grid_relative);
    let count = grid.point_count();
    // Ten metres a second straight up the grid: due grid north everywhere,
    // and true north nowhere but the orientation meridian.
    let messages = vec![
        message(&grid, 2, vec![0.0; count]),
        message(&grid, 3, vec![10.0; count]),
    ];
    let mut cache = BTreeMap::new();
    let mut resampling = import::Resampling::new(Resolution::Deg025.target_grid(), &mut cache);
    let sequences = import::sequences(messages, Some(&mut resampling)).expect("a wind sequence");
    assert_eq!(sequences.len(), 1);
    let sequence = &sequences[0];
    assert_eq!(sequence.kind, FieldKind::Wind);
    assert_eq!(sequence.frames.len(), 1);
    let raster = &sequence.frames[0].grid;

    // The orientation meridian is 263 E: there and only there is grid north
    // true north.
    let on_lov = raster.sample(-97.0, 40.0).expect("Kansas is in CONUS");
    assert!(on_lov.u.abs() < 0.05, "eastward came out {}", on_lov.u);
    assert!((on_lov.v - 10.0).abs() < 0.05);
    // East of it the grid leans clockwise; west of it, the other way. The
    // angle is `n·Δλ` for the cone through 33 N and 45 N.
    let east = raster.sample(-72.0, 42.0).expect("New England is in CONUS");
    let west = raster.sample(-122.0, 45.0).expect("Oregon is in CONUS");
    assert!(east.u > 1.0, "east of the meridian the wind leans east");
    assert!(west.u < -1.0, "west of the meridian it leans west");
    for sample in [on_lov, east, west] {
        let speed = f64::from(sample.u).hypot(f64::from(sample.v));
        assert!(
            (speed - 10.0).abs() < 0.05,
            "the rotation changed the speed to {speed}"
        );
    }
    // And a wind that was never resolved along the grid is left alone.
    let earth_relative = ProjectedGrid {
        grid_relative: false,
        ..grid
    };
    let messages = vec![
        message(&earth_relative, 2, vec![0.0; count]),
        message(&earth_relative, 3, vec![10.0; count]),
    ];
    let mut cache = BTreeMap::new();
    let mut resampling = import::Resampling::new(Resolution::Deg025.target_grid(), &mut cache);
    let sequences = import::sequences(messages, Some(&mut resampling)).expect("a sequence");
    let sample = sequences[0].frames[0]
        .grid
        .sample(-72.0, 42.0)
        .expect("New England");
    assert!(sample.u.abs() < 0.05, "eastward came out {}", sample.u);
}

/// A wind message on a projected grid, with `number` picking the component.
fn message(grid: &ProjectedGrid, number: u8, values: Vec<f32>) -> Message {
    Message {
        header: Header {
            index: 0,
            discipline: 0,
            centre: 7,
            reference_time: ReferenceTime {
                year: 2026,
                month: 9,
                day: 4,
                hour: 0,
                minute: 0,
                second: 0,
            },
            product_template: 0,
            category: 2,
            number,
            forecast_hours: 0.0,
            surface_type: 103,
            surface_value: 10.0,
            grid: Grid::Projected(*grid),
            packing_template: 0,
        },
        values,
    }
}

// --- The whole directory -----------------------------------------------------

/// Every message in a directory of forecast files, decoded.
///
/// This is the check that the octet layouts are right for grids no fixture
/// covers, and that nothing in a real file's section 3 is refused for a
/// reason that is not about the grid. It reports what it found rather than
/// asserting a count, because which files are in the directory is the
/// caller's business — but a grid that fails to parse, or a stated corner the
/// walk misses by more than a cell, fails it.
#[test]
#[ignore = "needs a directory of forecast files; see the module docs"]
fn every_grid_in_a_directory_parses_and_lands_where_it_says() {
    let dir = match std::env::var_os("VE_TEST_GRID_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => panic!("set VE_TEST_GRID_DIR to a directory of GRIB files"),
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("the directory")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_file())
        .collect();
    files.sort();
    assert!(!files.is_empty(), "{} holds no files", dir.display());

    let mut kinds: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut refused: BTreeMap<String, usize> = BTreeMap::new();
    let mut worst = (0.0f64, String::new());
    for path in &files {
        let bytes = std::fs::read(path).expect("a readable file");
        let name = path.file_name().expect("a name").to_string_lossy();
        for (index, section) in sections_3(&bytes).into_iter().enumerate() {
            match decode::grid_of(&section) {
                Ok(grid) => {
                    *kinds.entry(kind_of(&grid)).or_default() += 1;
                    if let Grid::Projected(grid) = grid
                        && let Some(error) = corner_error_cells(&section, &grid)
                    {
                        {
                            assert!(
                                error < 1.0,
                                "{name} message {}: the walk to the stated last point \
                                 missed it by {error:.2} cells",
                                index + 1
                            );
                            if error > worst.0 {
                                worst = (error, format!("{name} message {}", index + 1));
                            }
                        }
                    }
                }
                Err(error) => {
                    *refused.entry(error.to_string()).or_default() += 1;
                }
            }
        }
    }
    println!("{} files", files.len());
    for (kind, count) in &kinds {
        println!("  {count:6}  {kind}");
    }
    for (why, count) in &refused {
        println!("  {count:6}  REFUSED: {why}");
    }
    println!(
        "  worst stated-corner error {:.3} cells ({})",
        worst.0, worst.1
    );
    assert!(refused.is_empty(), "some grids were refused");
}

fn kind_of(grid: &Grid) -> &'static str {
    match grid {
        Grid::LatLon(_) => "3.0 lat/lon",
        Grid::Unstructured(_) => "3.101 unstructured",
        Grid::Projected(grid) => match grid.projection {
            Projection::Mercator { .. } => "3.10 mercator",
            Projection::PolarStereographic { .. } => "3.20 polar stereographic",
            Projection::LambertConformal { .. } => "3.30 lambert conformal",
            Projection::Rotated { .. } => "3.1 / 3.32769 rotated",
        },
    }
}

/// How far the walk to a grid's last node lands from where the file says it
/// is, in cells — for the two templates that say.
fn corner_error_cells(section: &[u8], grid: &ProjectedGrid) -> Option<f64> {
    let signed = |at: usize| {
        let raw = u32::from_be_bytes([
            section[at],
            section[at + 1],
            section[at + 2],
            section[at + 3],
        ]);
        let magnitude = f64::from(raw & 0x7fff_ffff) * 1e-6;
        if raw & 0x8000_0000 != 0 {
            -magnitude
        } else {
            magnitude
        }
    };
    let (lat2, lon2) = match u16::from_be_bytes([section[12], section[13]]) {
        // Template 3.10, octets 52-59.
        10 => (signed(51), signed(55)),
        // Template 3.32769, octets 73-80.
        32769 => (signed(72), signed(76)),
        _ => return None,
    };
    // Which node is the file's last one? A corner, and which corner is what
    // the scanning mode says.
    let (nx, ny) = (grid.nx as usize, grid.ny as usize);
    let scan = Scan(grid.scan);
    let last = nx * ny - 1;
    let (i, j) = [(0, 0), (nx - 1, 0), (0, ny - 1), (nx - 1, ny - 1)]
        .into_iter()
        .find(|&(i, j)| scan.file_index(i, j, nx, ny) == last)?;
    let (lat, lon) = grid.position(i as u32, j as u32);
    let dlat = lat - lat2;
    let dlon = ((lon - lon2 + 180.0).rem_euclid(360.0) - 180.0) * lat.to_radians().cos();
    // The rotated templates measure their plane in degrees, the map
    // projections in metres.
    let cell_deg = match grid.projection {
        Projection::Rotated { .. } => grid.dx,
        _ => grid.dx / (std::f64::consts::PI * grid.earth.a / 180.0),
    };
    Some(dlat.hypot(dlon) / cell_deg)
}

/// Section 3 of every message in a file, without decoding anything else.
fn sections_3(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 16 <= bytes.len() {
        if &bytes[at..at + 4] != b"GRIB" || bytes[at + 7] != 2 {
            at += 1;
            continue;
        }
        let total =
            u64::from_be_bytes(bytes[at + 8..at + 16].try_into().expect("eight octets")) as usize;
        if total < 20 || at + total > bytes.len() {
            break;
        }
        let message = &bytes[at..at + total];
        let mut cursor = 16;
        while cursor + 5 <= message.len() - 4 {
            let length =
                u32::from_be_bytes(message[cursor..cursor + 4].try_into().expect("four octets"))
                    as usize;
            if length < 5 || cursor + length > message.len() {
                break;
            }
            if message[cursor + 4] == 3 {
                out.push(message[cursor..cursor + length].to_vec());
                break;
            }
            cursor += length;
        }
        at += total;
    }
    out
}

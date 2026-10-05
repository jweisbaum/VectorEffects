#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! Imports read only a regional project's region (spec.md 4.8, M101).
//!
//! Every kind of grid the decoder reads is checked: a lat/lon file is cropped
//! to the region plus a node, a projected one is resampled onto the region's
//! nodes and no others, and an ICON mesh builds its neighbour set for the
//! region's lattice. The values are positions, so a crop that keeps the
//! wrong columns shows up as a number rather than as a plausible field.

use std::collections::BTreeMap;
use std::sync::Arc;

use ve_core::project::{FieldKind, Resolution};
use ve_core::raster::{RasterGrid, is_missing};
use ve_core::region::Region;
use ve_core::regrid::{Neighbours, TargetGrid};
use ve_grib::GribError;
use ve_grib::ReferenceTime;
use ve_grib::decode::{self, Grid, Header, Message, ProjectedGrid, Scan, UnstructuredGrid};
use ve_grib::import::{self, Resampling};
use ve_grib::writer::{self, GridSpec, MessageSpec, Parameter};

const Q: Resolution = Resolution::Deg025;

fn lattice(west: f64, east: f64, south: f64, north: f64, full_circle: bool) -> TargetGrid {
    Region::snapped(west, east, south, north, full_circle, Q)
        .expect("a region")
        .lattice(Q)
}

// --- Lat/lon ------------------------------------------------------------------

/// A 0.25° global wind file whose `u` is the node's longitude in
/// `[-180, 180)` and whose `v` is its latitude.
fn global_file() -> Vec<u8> {
    let grid = GridSpec {
        ni: 1440,
        nj: 721,
        micro_degrees: 250_000,
    };
    let (mut u, mut v) = (Vec::new(), Vec::new());
    for (lon, lat) in grid.points() {
        u.push(lon as f32);
        v.push(lat as f32);
    }
    let mut out = Vec::new();
    for (parameter, values) in [(Parameter::WindU, &u), (Parameter::WindV, &v)] {
        let spec = MessageSpec {
            parameter,
            grid,
            reference_time: writer::ReferenceTime {
                year: 2026,
                month: 10,
                day: 5,
                hour: 0,
                minute: 0,
                second: 0,
            },
            forecast_hour: 0,
            centre: 255,
            bits: 24,
        };
        out.extend(writer::message(&spec, values).expect("a message"));
    }
    out
}

fn read(target: Option<TargetGrid>) -> ve_grib::Result<RasterGrid> {
    let messages = decode::read_all(&global_file()).expect("decodes").messages;
    let mut cache = BTreeMap::new();
    let sequences = match target {
        Some(target) => {
            let mut resampling = Resampling::regional(target, &mut cache);
            import::sequences(messages, Some(&mut resampling))?
        }
        None => import::sequences(messages, None)?,
    };
    Ok((*sequences[0].frames[0].grid).clone())
}

#[test]
fn a_global_grib_in_a_small_region_holds_the_region() {
    let target = lattice(10.0, 20.0, 40.0, 50.0, false);
    assert_eq!((target.ni, target.nj), (41, 41));
    let regional = read(Some(target)).expect("a regional grid");
    let global = read(None).expect("the global grid");

    assert!(
        regional.ni * regional.nj <= 43 * 43,
        "{} x {} nodes held for a 41 x 41 region",
        regional.ni,
        regional.nj
    );
    assert!(!regional.wraps);
    assert_eq!(regional.sample(15.0, 45.0), global.sample(15.0, 45.0));
    // The margin node on every side is real, so the region's own edge
    // blends four present corners.
    for (lon, lat) in [(10.0, 40.0), (20.0, 50.0), (9.75, 50.25), (20.25, 39.75)] {
        let s = regional.sample(lon, lat).expect("inside the margin");
        assert!((f64::from(s.u) - lon).abs() < 1e-3, "{lon}: {s:?}");
        assert!((f64::from(s.v) - lat).abs() < 1e-3, "{lat}: {s:?}");
    }
    assert!(regional.sample(25.0, 45.0).is_none(), "east of the margin");
    assert!(regional.sample(15.0, 30.0).is_none(), "south of the margin");
}

#[test]
fn a_crop_across_the_antimeridian_reads_both_sides() {
    let target = lattice(160.0, -160.0, 40.0, 50.0, false);
    assert_eq!((target.lon0, target.ni), (160.0, 161));
    let regional = read(Some(target)).expect("a regional grid");
    assert_eq!(regional.ni, 163, "the region and a node either side");
    for lon in [179.5, -179.5, 160.0, -160.0] {
        let s = regional.sample(lon, 45.0).expect("both sides of 180");
        assert!((f64::from(s.u) - lon).abs() < 1e-3, "{lon}: {s:?}");
    }
    assert!(regional.sample(0.0, 45.0).is_none());
}

/// A polar cap wraps like the global grid and keeps the pole row.
#[test]
fn a_polar_cap_reads_every_longitude_down_to_its_edge() {
    let target = lattice(-180.0, 180.0, 60.0, 90.0, true);
    let regional = read(Some(target)).expect("a regional grid");
    assert!(regional.wraps);
    assert_eq!(regional.ni, 1440);
    assert_eq!(regional.nj, 122, "90 to 59.75");
    for (lon, lat) in [(0.0, 90.0), (179.75, 75.0), (-179.75, 60.0)] {
        let s = regional.sample(lon, lat).expect("in the cap");
        assert!((f64::from(s.v) - lat).abs() < 1e-3, "{s:?}");
    }
    assert!(regional.sample(0.0, 50.0).is_none());
}

/// A global project is read exactly as it always was: the same lattice and
/// the same hash, so the render cache does not move.
#[test]
fn a_global_project_reads_the_file_untouched() {
    let global = read(None).expect("global");
    let messages = decode::read_all(&global_file()).expect("decodes").messages;
    let mut cache = BTreeMap::new();
    let mut resampling = Resampling::new(Q.target_grid(), &mut cache);
    let sequences = import::sequences(messages, Some(&mut resampling)).expect("reads");
    assert_eq!(sequences[0].frames[0].grid.hash, global.hash);
}

/// A lat/lon file that covers none of the region says so, rather than adding
/// an empty layer (decision R10).
#[test]
fn a_lat_lon_file_outside_the_region_is_refused() {
    // A 1° patch over the North Sea, read for a region over Japan.
    let messages = patch_messages();
    let mut cache = BTreeMap::new();
    let mut resampling = Resampling::regional(lattice(130.0, 145.0, 30.0, 45.0, false), &mut cache);
    let err = import::sequences(messages, Some(&mut resampling)).expect_err("refused");
    assert!(matches!(err, GribError::OutsideRegion), "{err:?}");

    // The same patch read for a region it half covers imports the overlap.
    let messages = patch_messages();
    let mut cache = BTreeMap::new();
    let mut resampling = Resampling::regional(lattice(5.0, 20.0, 50.0, 60.0, false), &mut cache);
    let sequences = import::sequences(messages, Some(&mut resampling)).expect("the overlap");
    let grid = &sequences[0].frames[0].grid;
    assert!(grid.sample(8.0, 55.0).is_some());
    assert!(grid.sample(15.0, 55.0).is_none(), "beyond the file");
}

/// The North Sea at 1°, 0..10 E, 50..60 N, as a regional GRIB from a model
/// would be.
fn patch_messages() -> Vec<Message> {
    let grid = decode::LatLonGrid {
        ni: 11,
        nj: 11,
        la1: 60.0,
        lo1: 0.0,
        la2: 50.0,
        lo2: 10.0,
        di: 1.0,
        dj: 1.0,
        scan: 0,
    };
    [2u8, 3]
        .into_iter()
        .map(|number| message(Grid::LatLon(grid), number, vec![1.0; 121]))
        .collect()
}

// --- Projected ----------------------------------------------------------------

/// NCEP's grid 148: Lambert conformal over the lower 48 (see
/// `projected_grids.rs`, whose fixture this is).
const AQM_CONUS: &str = concat!(
    "0000005103000001c98a0000001e06000000000000000000000000000000",
    "000001ba00000109014cf6480e4486e00801f78a400fad0fc000b71b0000",
    "b71b00004001f78a4002aea5400000000000000000",
);

/// Polar stereographic over Alaska, whose first point is at 181.4 E: the
/// grid straddles the antimeridian.
const AQM_AK: &str = concat!(
    "0000004103000006f6210000001406000000000000000000000000000000",
    "0000033900000229026a70500ad0630808039387000c845880005ad5e800",
    "5ad5e80040",
);

/// GDAS wave over the Arctic: a polar stereographic grid holding the pole.
const GDAS_WAVE_ARCTIC: &str = concat!(
    "000000410300000f71440000001405000000000000000000000000000000",
    "000003ee000003ee021b64f012c684c000042c1d80000000000089544000",
    "8954400040",
);

fn projected(hex: &str) -> ProjectedGrid {
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex"))
        .collect();
    match decode::grid_of(&bytes).expect("a grid") {
        Grid::Projected(grid) => ProjectedGrid {
            // Position is already east and north.
            grid_relative: false,
            ..grid
        },
        other => panic!("expected a projected grid, got {other:?}"),
    }
}

/// `u` and `v` messages whose values are each node's longitude and latitude.
fn positional(grid: &ProjectedGrid) -> Vec<Message> {
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
    vec![
        message(Grid::Projected(*grid), 2, u),
        message(Grid::Projected(*grid), 3, v),
    ]
}

fn message(grid: Grid, number: u8, values: Vec<f32>) -> Message {
    Message {
        header: Header {
            index: 0,
            discipline: 0,
            centre: 7,
            reference_time: ReferenceTime {
                year: 2026,
                month: 10,
                day: 5,
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
            grid,
            packing_template: 0,
        },
        values,
    }
}

/// Every node of `grid` is a node of `target`, at the same place.
#[track_caller]
fn inside(grid: &RasterGrid, target: &TargetGrid) {
    let column = ((grid.lon0 - target.lon0).rem_euclid(360.0) / target.dlon).round();
    let row = ((target.lat0 - grid.lat0) / target.dlat).round();
    assert!(
        (target.lon0 + column * target.dlon - grid.lon0).rem_euclid(360.0) < 1e-9
            || (target.lon0 + column * target.dlon - grid.lon0).rem_euclid(360.0) > 360.0 - 1e-9,
        "column 0 is not a target node"
    );
    assert!(row >= 0.0, "row 0 at {} is north of the target", grid.lat0);
    assert!(
        row as u32 + grid.nj <= target.nj,
        "{} rows from {row} past the target's {}",
        grid.nj,
        target.nj
    );
    if (f64::from(target.ni) * target.dlon - 360.0).abs() > 1e-9 {
        assert!(
            column as u32 + grid.ni <= target.ni,
            "{} columns from {column} past the target's {}",
            grid.ni,
            target.ni
        );
    }
}

fn resample(grid: &ProjectedGrid, target: TargetGrid) -> ve_grib::Result<RasterGrid> {
    let mut cache = BTreeMap::new();
    let mut resampling = Resampling::regional(target, &mut cache);
    let sequences = import::sequences(positional(grid), Some(&mut resampling))?;
    Ok((*sequences[0].frames[0].grid).clone())
}

#[test]
fn a_projected_grid_is_windowed_inside_the_region() {
    let grid = projected(AQM_CONUS);
    // The eastern half of the lower 48, and the Atlantic beyond it.
    let target = lattice(-100.0, -40.0, 20.0, 55.0, false);
    let raster = resample(&grid, target).expect("the overlap");
    inside(&raster, &target);
    let s = raster.sample(-80.0, 40.0).expect("Pennsylvania");
    // A projection's longitudes come out in whatever turn it works in.
    let lon = (f64::from(s.u) + 80.0).rem_euclid(360.0);
    assert!(lon.min(360.0 - lon) < 0.05, "{s:?}");
    assert!((f64::from(s.v) - 40.0).abs() < 0.05, "{s:?}");
    assert!(raster.sample(-110.0, 40.0).is_none(), "west of the region");
}

#[test]
fn a_projected_grid_across_the_antimeridian_is_windowed_on_both_sides() {
    let grid = projected(AQM_AK);
    let target = lattice(160.0, -140.0, 40.0, 70.0, false);
    let raster = resample(&grid, target).expect("the overlap");
    inside(&raster, &target);
    for (lon, lat) in [(-150.0, 61.0), (-170.0, 55.0), (179.0, 52.0)] {
        let s = raster.sample(lon, lat).expect("Alaska and the Aleutians");
        assert!(
            (f64::from(s.v) - lat).abs() < 0.05,
            "({lon}, {lat}) says latitude {}",
            s.v
        );
    }
}

#[test]
fn a_projected_grid_over_the_pole_fills_a_polar_cap() {
    let grid = projected(GDAS_WAVE_ARCTIC);
    let target = lattice(-180.0, 180.0, 70.0, 90.0, true);
    let raster = resample(&grid, target).expect("the cap");
    inside(&raster, &target);
    assert!(raster.wraps);
    for (lon, lat) in [(0.0, 89.75), (179.75, 80.0), (-90.0, 72.0)] {
        assert!(raster.sample(lon, lat).is_some(), "({lon}, {lat})");
    }
}

#[test]
fn a_projected_grid_outside_the_region_is_refused() {
    let grid = projected(AQM_CONUS);
    let err = resample(&grid, lattice(0.0, 20.0, 40.0, 60.0, false)).expect_err("refused");
    assert!(matches!(err, GribError::OutsideRegion), "{err:?}");
}

// --- Unstructured -------------------------------------------------------------

/// ICON-EPS global R02B06, from the bundled grid asset: the fixture every
/// test in `icon.rs` reads its centres from.
const R02B06: [u8; 16] = [
    0xae, 0x48, 0x7d, 0x14, 0xfe, 0x2e, 0x11, 0xe4, 0xaf, 0x85, 0xe5, 0x0a, 0x2a, 0x56, 0xa3, 0x60,
];

fn icon_messages() -> Vec<Message> {
    let (_, centres) = ve_grib::icon::bundled(&R02B06)
        .expect("the asset parses")
        .expect("R02B06 is bundled");
    let grid = Grid::Unstructured(UnstructuredGrid {
        count: centres.len() as u32,
        uuid: R02B06,
        number_used: 24,
        number_in_reference: 1,
    });
    vec![
        message(grid, 2, centres.lon.clone()),
        message(grid, 3, centres.lat.clone()),
    ]
}

#[test]
fn an_icon_mesh_onto_a_region_builds_a_regional_neighbour_set() {
    let target = lattice(10.0, 20.0, 40.0, 50.0, false);
    let mut cache = BTreeMap::new();
    let sequences = {
        let mut resampling = Resampling::regional(target, &mut cache);
        let out = import::sequences(icon_messages(), Some(&mut resampling)).expect("resamples");
        assert_eq!(resampling.produced.len(), 1);
        out
    };
    let set = cache.values().next().unwrap();
    assert_eq!(set.len(), target.len());
    assert!(
        set.len() * 500 < Q.target_grid().len(),
        "{} nodes against a global {}",
        set.len(),
        Q.target_grid().len()
    );
    let grid = &sequences[0].frames[0].grid;
    assert_eq!((grid.ni, grid.nj), (target.ni, target.nj));
    assert!(grid.uv.iter().all(|s| !is_missing(s[0])));
    let s = grid.sample(15.0, 45.0).expect("inside");
    assert!((f64::from(s.u) - 15.0).abs() < 0.2 && (f64::from(s.v) - 45.0).abs() < 0.2);

    // The kept set, round-tripped through its encoding against the region's
    // lattice as a reopened project reads it, is enough the second time.
    let key = cache.keys().next().unwrap().clone();
    let set = cache.get(&key).unwrap();
    let back = Neighbours::decode(&set.encode(), &target, set.cells()).expect("decodes");
    let mut reloaded = BTreeMap::new();
    reloaded.insert(key, Arc::new(back));
    let mut resampling = Resampling::regional(target, &mut reloaded);
    let again = import::sequences(icon_messages(), Some(&mut resampling)).expect("resamples");
    assert!(resampling.produced.is_empty(), "the kept set was enough");
    assert_eq!(again[0].frames[0].grid.hash, grid.hash);
    assert_eq!(again[0].kind, FieldKind::Wind);
}

// --- Sea-surface temperature ----------------------------------------------------

/// A display-only temperature layer is cropped by the same rule, and refused
/// by it too: it is held in memory like any other lattice.
#[test]
fn a_temperature_file_is_cropped_to_the_region() {
    let grid = GridSpec {
        ni: 360,
        nj: 181,
        micro_degrees: 1_000_000,
    };
    let values = vec![15.0 + 273.15; grid.point_count() as usize];
    let spec = MessageSpec {
        parameter: Parameter::WaterTemperature,
        grid,
        reference_time: writer::ReferenceTime {
            year: 2026,
            month: 10,
            day: 5,
            hour: 0,
            minute: 0,
            second: 0,
        },
        forecast_hour: 0,
        centre: 255,
        bits: 16,
    };
    let dir = std::env::temp_dir().join(format!("ve-regional-sst-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let path = dir.join("sst.grib2");
    std::fs::write(&path, writer::message(&spec, &values).expect("encode")).expect("write");

    let one = Resolution::Deg1;
    let target = Region::snapped(160.0, -160.0, -10.0, 10.0, false, one)
        .expect("a region")
        .lattice(one);
    let sequence = import::read_temperature_file_onto(&path, Some(&target)).expect("read");
    let grid = &sequence.frames[0].grid;
    assert_eq!((grid.ni, grid.nj), (43, 23));
    let at = grid.sample(-179.5, 0.0).expect("across the seam");
    assert!((at.u - 15.0).abs() < 0.01, "{at:?}");

    let whole = import::read_temperature_file(&path).expect("read");
    assert_eq!(
        (whole.frames[0].grid.ni, whole.frames[0].grid.nj),
        (360, 181)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

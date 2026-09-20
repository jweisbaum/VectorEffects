#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! The S-57 reader against a real chart set.
//!
//! `#[ignore]`d and pointed at a directory by `$VE_TEST_ENC`, because an
//! exchange set is tens of megabytes and belongs to whoever downloaded it:
//!
//! ```bash
//! VE_TEST_ENC=~/Downloads/ENC_ROOT cargo test -p ve-chart --release \
//!     --test enc -- --ignored --nocapture
//! ```
//!
//! What it asserts is *geography*, not that the parser agrees with itself: a
//! cell has to put its land where the catalogue says its land is, and the
//! classes have to be the ones `catalogue.rs` claims for those numbers — an
//! area class full of depth ranges is DEPARE or the table is wrong.

use std::path::PathBuf;

use ve_chart::paint::TileFrame;
use ve_chart::s57::{Library, Palette, draw_tile};
use ve_chart::{Bounds, Geometry, Value};

fn root() -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os("VE_TEST_ENC")?);
    assert!(path.is_dir(), "VE_TEST_ENC is not a directory: {path:?}");
    Some(path)
}

#[test]
#[ignore = "reads the chart set named by VE_TEST_ENC"]
fn every_cell_reads_and_lands_inside_the_box_the_catalogue_gives_it() {
    let Some(root) = root() else {
        return;
    };
    let library = Library::open(&root).expect("indexes");
    assert!(!library.entries().is_empty(), "no cells found");
    println!("{} cells indexed", library.entries().len());

    let mut read = 0;
    let mut features = 0;
    for entry in library.entries() {
        let cell = library.cell(entry).expect("every cell reads");
        read += 1;
        features += cell.features.len();
        // The catalogue's box and the cell's own geometry have to agree. A
        // little slack: a catalogue box is stated to six decimals and an
        // edge may run a fraction past its own corner.
        let slack = 0.02;
        assert!(
            cell.bounds.west >= entry.bounds.west - slack
                && cell.bounds.east <= entry.bounds.east + slack
                && cell.bounds.south >= entry.bounds.south - slack
                && cell.bounds.north <= entry.bounds.north + slack,
            "{}: geometry at {:?} is outside the catalogue's {:?}",
            entry.name,
            cell.bounds,
            entry.bounds,
        );
        assert!(cell.scale > 0, "{}: no compilation scale", entry.name);
    }
    println!("{read} cells read, {features} features");
    assert!(
        features > 1000,
        "a real chart set holds more than {features}"
    );
}

#[test]
#[ignore = "reads the chart set named by VE_TEST_ENC"]
fn the_classes_are_the_ones_the_catalogue_claims_for_those_numbers() {
    let Some(root) = root() else {
        return;
    };
    let library = Library::open(&root).expect("indexes");
    let mut depth_areas = 0;
    let mut soundings = 0;
    let mut coastlines = 0;
    for entry in library.entries().iter().take(60) {
        let Some(cell) = library.cell(entry) else {
            continue;
        };
        for feature in &cell.features {
            match feature.class.as_str() {
                "DEPARE" => {
                    depth_areas += 1;
                    assert!(
                        matches!(feature.geometry, Geometry::Areas(_)),
                        "a depth area is an area"
                    );
                    // The class is identified by carrying a depth range; if
                    // this stops holding, the code is not DEPARE's.
                    assert!(
                        feature.attribute("DRVAL1").is_some()
                            || feature.attribute("DRVAL2").is_some(),
                        "DEPARE with no depth range: the class table is wrong"
                    );
                }
                "SOUNDG" => {
                    soundings += 1;
                    let Geometry::Points(points) = &feature.geometry else {
                        panic!("a sounding is a point");
                    };
                    // Soundings are the one class whose geometry is 3-D.
                    assert!(
                        points.iter().any(|p| p[2].is_finite()),
                        "a sounding carries a depth"
                    );
                    for point in points {
                        assert!(
                            (-200.0..=15000.0).contains(&point[2]) || !point[2].is_finite(),
                            "a sounding of {} m is not a depth",
                            point[2]
                        );
                    }
                }
                "COALNE" => {
                    coastlines += 1;
                    assert!(
                        matches!(feature.geometry, Geometry::Lines(_)),
                        "a coastline is a line"
                    );
                }
                "DEPCNT" => {
                    assert!(matches!(feature.geometry, Geometry::Lines(_)));
                    if let Some(depth) = feature.attribute("VALDCO").and_then(Value::number) {
                        assert!(
                            (-100.0..=12000.0).contains(&depth),
                            "a contour at {depth} m is not a depth"
                        );
                    }
                }
                _ => {}
            }
        }
    }
    println!("{depth_areas} depth areas, {soundings} soundings, {coastlines} coastlines");
    assert!(depth_areas > 100 && soundings > 10 && coastlines > 100);
}

#[test]
#[ignore = "reads the chart set named by VE_TEST_ENC"]
fn a_tile_over_the_charts_is_painted_and_one_away_from_them_is_not() {
    let Some(root) = root() else {
        return;
    };
    let library = Library::open(&root).expect("indexes");
    let covered = library.bounds().expect("some coverage");
    println!("coverage {covered:?}");

    // A tile in the middle of the coverage, at a few different zooms.
    for degrees in [8.0, 2.0, 0.5, 0.1] {
        let middle = (
            (covered.west + covered.east) / 2.0,
            (covered.south + covered.north) / 2.0,
        );
        let frame = TileFrame {
            bounds: Bounds {
                west: middle.0 - degrees / 2.0,
                south: middle.1 - degrees / 2.0,
                east: middle.0 + degrees / 2.0,
                north: middle.1 + degrees / 2.0,
            },
            size: 256,
        };
        let started = std::time::Instant::now();
        let tile = draw_tile(&library, frame, &Palette::default(), 12);
        let painted = tile.as_ref().map_or(0, |rgba| {
            rgba.chunks_exact(4).filter(|px| px[3] > 0).count()
        });
        println!(
            "{degrees}° tile: {painted} of 65536 pixels painted in {} ms",
            started.elapsed().as_millis()
        );
        assert!(
            painted > 65536 / 20,
            "{degrees}°: a tile over the charts should be mostly chart"
        );
    }

    // The Pacific, which these charts do not cover.
    let away = TileFrame {
        bounds: Bounds {
            west: -150.0,
            south: 10.0,
            east: -149.0,
            north: 11.0,
        },
        size: 256,
    };
    assert!(
        draw_tile(&library, away, &Palette::default(), 12).is_none(),
        "nothing to draw is drawn as nothing, not as an empty tile"
    );
}

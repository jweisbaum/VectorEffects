#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! The one test in the repository that reaches the network on purpose.
//!
//! `#[ignore]`d, so it runs only when asked — CI and every ordinary
//! `cargo test` stay offline, which is invariant 5's promise:
//!
//! ```bash
//! VE_TEST_OSM=1 cargo test -p ve-osm --test live -- --ignored --nocapture
//! ```
//!
//! What it checks is the thing no offline test can: that the address this
//! crate builds is one the tile server answers, that what comes back decodes
//! as a square image, and that the second ask does not leave the disk.

use ve_osm::{ATTRIBUTION, DEFAULT_TILE_URL, Tiles, source_tiles, source_zoom_for, warp};

#[test]
#[ignore = "fetches from tile.openstreetmap.org; set VE_TEST_OSM=1"]
fn a_real_tile_fetches_once_decodes_and_lands_where_it_belongs() {
    if std::env::var_os("VE_TEST_OSM").is_none() {
        return;
    }
    let root = tempfile::tempdir().expect("temp");
    let tiles = Tiles::new(
        DEFAULT_TILE_URL,
        root.path(),
        "VectorEffects/0.1.9 (https://github.com/jweisbaum/VectorEffects) test",
    )
    .expect("a source");

    // The Chesapeake, at a zoom the servers certainly publish.
    let (west, south, east, north) = (-76.6, 38.8, -76.2, 39.2);
    let level = 6;
    let zoom = source_zoom_for(level);
    let wanted = source_tiles(west, south, east, north, zoom);
    assert!(!wanted.is_empty(), "the box asks for something");
    println!("{} tiles at zoom {zoom}: {wanted:?}", wanted.len());

    let started = std::time::Instant::now();
    let mut decoded = Vec::new();
    for tile in &wanted {
        let got = tiles.tile(*tile, true).expect("the server answers");
        assert_eq!(got.size, 256, "a tile is 256 square");
        assert_eq!(got.rgba.len(), 256 * 256 * 4);
        decoded.push(got);
    }
    println!("fetched in {} ms", started.elapsed().as_millis());

    // Every tile is on disk now, so a fresh source reads them without the
    // network — which is what the usage policy asks of a client.
    let offline = Tiles::new("https://tile.invalid/{z}/{x}/{y}.png", root.path(), "test")
        .expect("a second source");
    for tile in &wanted {
        assert!(
            offline.tile(*tile, false).is_some(),
            "{tile:?} should have been kept"
        );
    }

    let painted = warp(west, south, east, north, 256, &decoded).expect("a tile is drawn");
    let opaque = painted.chunks_exact(4).filter(|px| px[3] == 255).count();
    assert_eq!(opaque, 256 * 256, "every pixel of the map is covered");
    // A map is not one flat colour: land, sea and roads are different.
    let distinct: std::collections::HashSet<&[u8]> = painted.chunks_exact(4).collect();
    assert!(
        distinct.len() > 16,
        "only {} colours: not a map",
        distinct.len()
    );
    println!("{} distinct colours in the drawn tile", distinct.len());
    assert!(ATTRIBUTION.contains("OpenStreetMap"));
}

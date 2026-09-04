#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! ICON's icosahedral grid, resampled onto the project's own.
//!
//! ICON does not run on a lat/lon grid. Its messages are a bare run of
//! 2,949,120 values whose positions come from the bundled grid definition
//! (spec §4.8), and what the app consumes is the regular lattice they are
//! resampled onto. Nothing about that is checkable by inspection: a mesh read
//! in the wrong order, or placed against the wrong coordinates, still
//! produces a plausible-looking global field.
//!
//! So the expected values below were computed **independently**: a brute-force
//! search over all 2,949,120 cells for the three nearest to each point,
//! inverse-distance weighted, in Python against ecCodes' own decode of the
//! same files. Eight points, including both poles and both sides of the
//! antimeridian, which are where a grid search goes wrong first.
//!
//! The files are 12 MB and not committed, so this is `#[ignore]`d and reads
//! `$VE_TEST_GRIBS` (default `~/temp_test_gribs`):
//!
//! ```text
//! VE_TEST_GRIBS=~/temp_test_gribs cargo test -p ve-grib --release \
//!     --test icon -- --ignored --nocapture
//! ```

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use ve_core::project::Resolution;
use ve_grib::decode;
use ve_grib::import::{self, Resampling};

const U_FILE: &str = "icon_global_icosahedral_single-level_2026090400_000_U_10M.grib2";
const V_FILE: &str = "icon_global_icosahedral_single-level_2026090400_000_V_10M.grib2";

/// ICON global R03B07, as template 3.101 names it.
const R03B07: [u8; 16] = [
    0xa2, 0x7b, 0x8d, 0xe6, 0x18, 0xc4, 0x11, 0xe4, 0x82, 0x0a, 0xb5, 0xb0, 0x98, 0xc6, 0xa5, 0xc0,
];

/// Independently computed: longitude, latitude, `u`, `v`.
const EXPECTED: &[(f64, f64, f32, f32)] = &[
    (0.0, 0.0, 1.562469, 5.836835),
    (-30.0, 45.0, -10.666238, -8.526414),
    (120.0, -30.0, 4.535813, -0.387147),
    (179.75, 60.0, -7.142053, 0.627067),
    (-179.75, 60.0, -7.116496, 0.581083),
    (0.0, 89.75, 2.39178, -1.930643),
    (0.0, -89.75, -3.003942, -7.02168),
    (-70.0, -50.0, 5.501788, 2.436568),
];

fn directory() -> PathBuf {
    match std::env::var_os("VE_TEST_GRIBS") {
        Some(dir) => PathBuf::from(dir),
        None => {
            let home = std::env::var_os("HOME").expect("HOME is set");
            PathBuf::from(home).join("temp_test_gribs")
        }
    }
}

/// The two components concatenated, which is what a real import sees.
///
/// `tag` keeps each test to its own scratch file: they run in parallel, and
/// two of them writing one path leaves a reader with half a file.
fn wind_file(tag: &str) -> PathBuf {
    let dir = directory();
    let mut bytes = std::fs::read(dir.join(U_FILE)).expect("the U file");
    bytes.extend_from_slice(&std::fs::read(dir.join(V_FILE)).expect("the V file"));
    let path = std::env::temp_dir().join(format!("ve_icon_uv_{tag}.grib2"));
    std::fs::write(&path, &bytes).expect("a scratch file");
    path
}

#[test]
#[ignore = "needs the ICON sample files; see the module docs"]
fn an_icon_message_names_its_grid_and_nothing_else() {
    let bytes = std::fs::read(directory().join(U_FILE)).expect("the U file");
    let decoded = decode::read_all(&bytes).expect("it decodes");
    assert!(decoded.skipped.is_empty(), "{:?}", decoded.skipped);
    let header = &decoded.messages[0].header;

    let grid = header
        .grid
        .unstructured()
        .expect("ICON is an unstructured grid");
    assert_eq!(grid.count, 2_949_120);
    assert_eq!(grid.uuid, R03B07);
    assert_eq!(grid.uuid_hex(), "a27b8de618c411e4820ab5b098c6a5c0");
    // The mesh has no lat/lon geometry at all, which is the whole point.
    assert!(header.grid.lat_lon().is_none());
    // And the packing is CCSDS, so this exercises that decoder too.
    assert_eq!(header.packing_template, 42);
}

/// The spacing a new project's grid is chosen from, for a file that states
/// none. ICON global's 2,949,120 cells work out at 0.118°, which is the 13 km
/// DWD publishes, and picks the app's 0.1° grid.
#[test]
#[ignore = "needs the ICON sample files; see the module docs"]
fn the_mesh_implies_a_spacing_and_therefore_a_resolution() {
    let bytes = std::fs::read(directory().join(U_FILE)).expect("the U file");
    let (messages, _) = {
        let decoded = decode::read_all(&bytes).expect("it decodes");
        (decoded.messages, decoded.skipped)
    };
    let spacing = import::nominal_spacing(&messages).expect("a spacing");
    assert!(
        (spacing - 0.1183).abs() < 0.001,
        "ICON global is about 13 km, got {spacing}"
    );
}

/// The resample lands on the values an independent implementation computes.
#[test]
#[ignore = "needs the ICON sample files; see the module docs"]
fn the_resampled_field_matches_an_independent_interpolation() {
    let path = wind_file("independent");
    let started = Instant::now();
    let (messages, skipped) = import::read_messages(&path).expect("it decodes");
    assert!(skipped.is_empty(), "{skipped:?}");
    assert_eq!(messages.len(), 2, "u and v");

    let mut cache = BTreeMap::new();
    let sequences = {
        let mut resampling = Resampling::new(Resolution::Deg025.target_grid(), &mut cache);
        let out = import::sequences(messages, Some(&mut resampling)).expect("it resamples");
        assert_eq!(resampling.produced.len(), 1, "one mesh, one neighbour set");
        out
    };
    println!(
        "import took {:.0} ms",
        started.elapsed().as_secs_f64() * 1e3
    );

    assert_eq!(sequences.len(), 1, "one field kind");
    let grid = &sequences[0].frames[0].grid;
    assert_eq!((grid.ni, grid.nj), (1440, 721));
    assert!(grid.wraps, "a global grid wraps");
    assert!(
        grid.uv.iter().all(|s| !ve_core::raster::is_missing(s[0])),
        "a global mesh leaves no node uncovered"
    );

    for &(lon, lat, want_u, want_v) in EXPECTED {
        let got = grid.sample(lon, lat).expect("a sample");
        // These are exact nodes of the 0.25 degree grid, so the sampler's
        // bilinear blend has weight one on a single node: what is compared is
        // the resampled value itself, not an interpolation of it.
        assert!(
            (got.u - want_u).abs() < 1e-4 && (got.v - want_v).abs() < 1e-4,
            "({lon}, {lat}): got ({}, {}) against ({want_u}, {want_v})",
            got.u,
            got.v
        );
    }
}

/// A field that comes back the same whichever way round it was built.
///
/// The neighbour set is the expensive half and is kept, so the second import
/// takes a path the first did not: it reads a set rather than searching for
/// one. The two must agree exactly, or a reopened project would differ from
/// the import that made it.
#[test]
#[ignore = "needs the ICON sample files; see the module docs"]
fn a_kept_neighbour_set_gives_the_same_field_as_a_fresh_search() {
    let path = wind_file("kept");
    let target = Resolution::Deg1.target_grid();

    let (messages, _) = import::read_messages(&path).expect("it decodes");
    let mut cache = BTreeMap::new();
    let first = {
        let mut resampling = Resampling::new(target, &mut cache);
        import::sequences(messages, Some(&mut resampling)).expect("it resamples")
    };
    assert_eq!(cache.len(), 1);

    // Round-trip the set through its encoding, as saving and reopening does.
    let key = cache.keys().next().expect("a key").clone();
    let set = cache.get(&key).expect("the set");
    let bytes = set.encode();
    let back =
        ve_core::regrid::Neighbours::decode(&bytes, &target, set.cells()).expect("it decodes back");
    let mut reloaded = BTreeMap::new();
    reloaded.insert(key, std::sync::Arc::new(back));

    let (messages, _) = import::read_messages(&path).expect("it decodes");
    let started = Instant::now();
    let second = {
        let mut resampling = Resampling::new(target, &mut reloaded);
        let out = import::sequences(messages, Some(&mut resampling)).expect("it resamples");
        assert!(
            resampling.produced.is_empty(),
            "the kept set should have been enough"
        );
        out
    };
    println!(
        "second import took {:.0} ms",
        started.elapsed().as_secs_f64() * 1e3
    );

    let (a, b) = (&first[0].frames[0].grid, &second[0].frames[0].grid);
    assert_eq!(a.hash, b.hash, "the same field, so the same content hash");
}

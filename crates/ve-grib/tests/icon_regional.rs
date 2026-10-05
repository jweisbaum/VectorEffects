#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! ICON's regional meshes, resampled onto a global project grid.
//!
//! ICON-D2 (Germany) and ICON-EU-EPS (Europe) are bundled beside the global
//! meshes, and cover a few percent of the earth. Two things are checked
//! against real forecasts: values inside the mesh match an independent
//! interpolation, and nodes beyond the mesh are missing rather than filled
//! with the nearest edge cell's wind (`ve_core::regrid::REACH`).
//!
//! The expected values were computed **independently**, in Python: a
//! brute-force search for the three nearest cells against ecCodes' decode of
//! DWD's own `CLAT`/`CLON` files (not the bundled asset), weighted by inverse
//! chord, over ecCodes' decode of the forecast. For the ensemble, its first
//! member, which is the one an import keeps.
//!
//! The files are DWD open data, 2026-10-04 12 UTC, 10 m wind, and not
//! committed; this is `#[ignore]`d and reads `$VE_TEST_GRIBS/icon_regional`
//! (default `~/temp_test_gribs`):
//!
//! ```text
//! VE_TEST_GRIBS=~/temp_test_gribs cargo test -p ve-grib --release \
//!     --test icon_regional -- --ignored --nocapture
//! ```

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use ve_core::project::Resolution;
use ve_core::raster::is_missing;
use ve_grib::import::{self, Resampling};

fn directory() -> PathBuf {
    let base = match std::env::var_os("VE_TEST_GRIBS") {
        Some(dir) => PathBuf::from(dir),
        None => {
            let home = std::env::var_os("HOME").expect("HOME is set");
            PathBuf::from(home).join("temp_test_gribs")
        }
    };
    base.join("icon_regional")
}

/// Imports `u` and `v` together onto the 0.25° global grid and checks the
/// expected values and that `far` points are missing.
fn check(
    tag: &str,
    u_file: &str,
    v_file: &str,
    expected: &[(f64, f64, f32, f32)],
    far: &[(f64, f64)],
) {
    let dir = directory();
    let mut bytes = std::fs::read(dir.join(u_file)).expect("the U file");
    bytes.extend_from_slice(&std::fs::read(dir.join(v_file)).expect("the V file"));
    let path = std::env::temp_dir().join(format!("ve_icon_regional_{tag}.grib2"));
    std::fs::write(&path, &bytes).expect("a scratch file");

    let started = Instant::now();
    let (messages, skipped) = import::read_messages(&path).expect("it decodes");
    assert!(skipped.is_empty(), "{tag}: {skipped:?}");
    let mut cache = BTreeMap::new();
    let sequences = {
        let mut resampling = Resampling::new(Resolution::Deg025.target_grid(), &mut cache);
        import::sequences(messages, Some(&mut resampling)).expect("it resamples")
    };
    let set = cache.values().next().expect("one neighbour set");
    println!(
        "{tag}: import took {:.0} ms; {} of {} nodes beyond the mesh",
        started.elapsed().as_secs_f64() * 1e3,
        set.beyond(),
        set.len()
    );

    assert_eq!(sequences.len(), 1, "{tag}: one field kind");
    let grid = &sequences[0].frames[0].grid;
    for &(lon, lat, want_u, want_v) in expected {
        let got = grid.sample(lon, lat).expect("a sample");
        // Exact nodes of the 0.25° grid, so what is compared is the
        // resampled value, not the sampler's blend of it.
        assert!(
            (got.u - want_u).abs() < 1e-3 && (got.v - want_v).abs() < 1e-3,
            "{tag} ({lon}, {lat}): got ({}, {}) against ({want_u}, {want_v})",
            got.u,
            got.v
        );
    }
    for &(lon, lat) in far {
        let i = ((lon + 180.0) / 0.25).round() as usize;
        let j = ((90.0 - lat) / 0.25).round() as usize;
        let node = grid.uv[j * grid.ni as usize + i];
        assert!(
            is_missing(node[0]) && is_missing(node[1]),
            "{tag} ({lon}, {lat}) is far outside the mesh: {node:?}"
        );
    }
    // Most of the earth is outside either mesh.
    let present = grid.uv.iter().filter(|s| !is_missing(s[0])).count();
    assert!(
        present < grid.uv.len() / 10,
        "{tag}: {present} of {} nodes filled",
        grid.uv.len()
    );
}

/// Points well outside both meshes: the equator, mid-Atlantic, Siberia, the
/// southern ocean.
const FAR: &[(f64, f64)] = &[(0.0, 0.0), (-30.0, 45.0), (100.0, 60.0), (120.0, -50.0)];

#[test]
#[ignore = "needs the DWD sample files; see the module docs"]
fn icon_d2_imports_onto_a_global_grid() {
    check(
        "d2",
        "icon-d2_germany_icosahedral_single-level_2026100412_000_2d_u_10m.grib2",
        "icon-d2_germany_icosahedral_single-level_2026100412_000_2d_v_10m.grib2",
        &[
            (10.0, 51.0, -0.704028, 0.486297),
            (13.5, 52.5, 2.352824, -0.650416),
            (7.0, 50.0, -0.427759, -0.432984),
            (11.5, 48.0, -0.785854, -1.292286),
            (6.0, 54.0, 3.65842, 2.652007),
        ],
        &[FAR, &[(20.0, 40.0), (-5.0, 51.0)]].concat(),
    );
}

#[test]
#[ignore = "needs the DWD sample files; see the module docs"]
fn icon_eu_eps_imports_onto_a_global_grid() {
    check(
        "eu-eps",
        "icon-eu-eps_europe_icosahedral_single-level_2026100412_000_u_10m.grib2",
        "icon-eu-eps_europe_icosahedral_single-level_2026100412_000_v_10m.grib2",
        &[
            (-10.0, 45.0, -5.005168, -7.503316),
            (20.0, 40.0, 1.788948, 1.072351),
            (0.0, 60.0, 8.116567, 10.929632),
            (30.0, 55.0, 1.925275, 0.209842),
            (-20.0, 35.0, -5.15573, -6.157292),
        ],
        FAR,
    );
}

/// ICON-D2-EPS is on ICON-D2's own grid (the same UUID), so it needs no
/// mesh of its own; its values are the ensemble's and not compared here.
#[test]
#[ignore = "needs the DWD sample files; see the module docs"]
fn icon_d2_eps_shares_icon_d2s_mesh() {
    check(
        "d2-eps",
        "icon-d2-eps_germany_icosahedral_single-level_2026100412_000_2d_u_10m.grib2",
        "icon-d2-eps_germany_icosahedral_single-level_2026100412_000_2d_v_10m.grib2",
        &[],
        FAR,
    );
}

/// A new project made from a regional file gets a grid fine enough for it.
/// Spacing from the earth's area over the cell count assumed a global mesh,
/// and put ICON-D2's 2 km cells at 0.28°.
#[test]
#[ignore = "needs the DWD sample files; see the module docs"]
fn a_regional_mesh_implies_its_own_spacing() {
    for (file, low, high) in [
        (
            "icon-d2_germany_icosahedral_single-level_2026100412_000_2d_u_10m.grib2",
            0.015,
            0.025,
        ),
        (
            "icon-eu-eps_europe_icosahedral_single-level_2026100412_000_u_10m.grib2",
            0.10,
            0.14,
        ),
    ] {
        let bytes = std::fs::read(directory().join(file)).expect("the file");
        let messages = ve_grib::decode::read_all(&bytes)
            .expect("it decodes")
            .messages;
        let started = Instant::now();
        let spacing = import::nominal_spacing(&messages).expect("a spacing");
        println!(
            "{file}: {spacing:.4}° in {:.0} ms",
            started.elapsed().as_secs_f64() * 1e3
        );
        assert!((low..high).contains(&spacing), "{file}: {spacing}");
    }
}

/// Onto a regional project (spec.md 4.8, M101): ICON-D2 read for a region
/// over Germany lands the same values as the global read, on a neighbour set
/// the size of the region; read for a region over Japan it is refused.
#[test]
#[ignore = "needs the DWD sample files; see the module docs"]
fn icon_d2_onto_a_region() {
    use ve_core::region::Region;
    let dir = directory();
    let mut bytes = std::fs::read(
        dir.join("icon-d2_germany_icosahedral_single-level_2026100412_000_2d_u_10m.grib2"),
    )
    .expect("the U file");
    bytes.extend_from_slice(
        &std::fs::read(
            dir.join("icon-d2_germany_icosahedral_single-level_2026100412_000_2d_v_10m.grib2"),
        )
        .expect("the V file"),
    );
    let path = std::env::temp_dir().join("ve_icon_regional_d2_region.grib2");
    std::fs::write(&path, &bytes).expect("a scratch file");
    let q = Resolution::Deg025;

    let germany = Region::snapped(5.0, 16.0, 46.0, 56.0, false, q)
        .expect("a region")
        .lattice(q);
    let (messages, _) = import::read_messages(&path).expect("it decodes");
    let mut cache = BTreeMap::new();
    let started = Instant::now();
    let sequences = {
        let mut resampling = Resampling::regional(germany, &mut cache);
        import::sequences(messages, Some(&mut resampling)).expect("it resamples")
    };
    println!(
        "d2 onto Germany: {:.0} ms",
        started.elapsed().as_secs_f64() * 1e3
    );
    assert_eq!(cache.values().next().expect("a set").len(), germany.len());
    let grid = &sequences[0].frames[0].grid;
    let got = grid.sample(10.0, 51.0).expect("a sample");
    assert!(
        (got.u - -0.704028).abs() < 1e-3 && (got.v - 0.486297).abs() < 1e-3,
        "{got:?}"
    );

    let japan = Region::snapped(130.0, 145.0, 30.0, 45.0, false, q)
        .expect("a region")
        .lattice(q);
    let (messages, _) = import::read_messages(&path).expect("it decodes");
    let mut cache = BTreeMap::new();
    let mut resampling = Resampling::regional(japan, &mut cache);
    let err = import::sequences(messages, Some(&mut resampling)).expect_err("refused");
    assert!(matches!(err, ve_grib::GribError::OutsideRegion), "{err:?}");
}

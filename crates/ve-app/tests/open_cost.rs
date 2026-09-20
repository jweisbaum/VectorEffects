#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! What opening costs: a project shaped by a file, and the same project read
//! back from disk, which decodes that file again (invariants 1 and 2).
//!
//! Reports numbers and asserts none: the files are the user's own, far too
//! large to commit, and the machine varies. `#[ignore]`d, and each half runs
//! only when its variable names something to open:
//!
//! ```bash
//! VE_TEST_OPEN_GRIB=~/Documents/era5-wind-globcurrent.grib2 \
//! VE_TEST_OPEN_ZARR=~/SampleRoutingData/routing_test \
//!     cargo test -p ve-app --release --test open_cost -- --ignored --nocapture
//! ```
//!
//! `VE_TEST_OPEN_KEEP=<directory>` keeps the saved projects there.

use std::path::{Path, PathBuf};
use std::time::Instant;

use ve_app::commands::AppState;
use ve_app::paths::AppPaths;
use ve_app::{import, projects, zarr};

fn source(variable: &str) -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os(variable)?);
    assert!(
        path.exists(),
        "{variable} names {path:?}, which is not there"
    );
    Some(path)
}

fn timed<T>(label: &str, work: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let out = work();
    println!("{label:<34} {:>9.3} s", start.elapsed().as_secs_f64());
    out
}

/// Shapes a project from `source`, saves it, and opens the saved file.
fn measure(name: &str, source: &Path, shape: impl FnOnce(&AppState, String)) {
    let root = tempfile::tempdir().unwrap();
    let app = AppState::new(AppPaths::in_directory(root.path()).unwrap());
    // Kept, when asked, for opening by hand: a project that takes a while to
    // open is what the loading page is looked at with.
    let saved = std::env::var_os("VE_TEST_OPEN_KEEP")
        .map_or_else(|| root.path().to_path_buf(), PathBuf::from)
        .join(format!("{name}.veproj"));
    println!("--- {name}: {}", source.display());
    timed("project from the file", || {
        shape(&app, source.to_string_lossy().into_owned());
    });
    timed("save", || {
        projects::save_as(&app, saved.to_string_lossy().into_owned()).unwrap();
    });
    projects::close_open(&app, true).unwrap();
    timed("the document alone (io::load)", || {
        ve_core::io::load(&saved).unwrap();
    });
    let summary = timed("open the saved project", || {
        projects::open(&app, saved.to_string_lossy().into_owned(), true).unwrap()
    });
    println!("opened: {}", summary.name);
}

#[test]
#[ignore = "reads $VE_TEST_OPEN_GRIB, a file too large to commit"]
fn opening_a_grib_project() {
    let Some(path) = source("VE_TEST_OPEN_GRIB") else {
        return;
    };
    measure("grib", &path, |app, path| {
        import::grib_project(app, path, true).unwrap();
    });
}

#[test]
#[ignore = "reads $VE_TEST_OPEN_ZARR, a store too large to commit"]
fn opening_a_zarr_project() {
    let Some(path) = source("VE_TEST_OPEN_ZARR") else {
        return;
    };
    measure("zarr", &path, |app, path| {
        zarr::zarr_project(app, path, true).unwrap();
    });
}

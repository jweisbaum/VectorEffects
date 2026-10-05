#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test setup: a panic naming the failed step is the right outcome, and clippy's allow-expect-in-tests does not reach helpers in tests/"
)]
//! Real forecast models, imported through the application's own commands.
//!
//! Each file in `$VE_TEST_MODELS/<centre>` is a model's 10 m `u` and `v` at
//! three forecast hours, packaged into one GRIB2 file as the centre publishes
//! them. Every file is imported twice — as a new project
//! (`grib_project`, **New project from GRIB**) and into an existing global
//! 0.25° project (`grib_import`, **Import GRIB**) — and read back through
//! the readout's sampler (`sample_points_at_step`), the same flatten and
//! evaluation the tiles use.
//!
//! The reference is `<file>.expected.json`, written by
//! `tools/model-imports/expected.py` without the application's code: ecCodes'
//! value at a lat/lon node, or for an ICON mesh a brute-force interpolation
//! over DWD's own cell positions. Points well outside a regional model must
//! come back undefined.
//!
//! The files are tens to hundreds of megabytes and not committed:
//!
//! ```text
//! VE_TEST_MODELS=~/temp_test_gribs/models VE_FORCE_CPU=1 \
//!     cargo test -p ve-app --release --test model_imports -- --ignored --nocapture
//! ```

use std::path::{Path, PathBuf};
use std::time::Instant;

use serde_json::Value;
use ve_app::commands::{AppState, sample_points_at_step};
use ve_app::import;
use ve_app::paths::AppPaths;
use ve_app::projects::{self, NewProjectRequest};

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("ve-model-imports-{}-{label}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp root");
        Self(dir)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn models(centre: &str) -> Vec<PathBuf> {
    let base = match std::env::var_os("VE_TEST_MODELS") {
        Some(dir) => PathBuf::from(dir),
        None => {
            let home = std::env::var_os("HOME").expect("HOME is set");
            PathBuf::from(home).join("temp_test_gribs/models")
        }
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(base.join(centre))
        .expect("the models directory")
        .map(|e| e.expect("an entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "grib2"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no models for {centre}");
    files
}

struct Expected {
    points: Vec<(f64, f64)>,
    far: Vec<(f64, f64)>,
    /// Per forecast time: `u` and `v` at each point.
    times: Vec<(Vec<f64>, Vec<f64>)>,
}

fn expected(file: &Path) -> Expected {
    let path = PathBuf::from(format!("{}.expected.json", file.display()));
    let json: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("the expected values"))
            .expect("json");
    let pairs = |v: &Value| -> Vec<(f64, f64)> {
        v.as_array()
            .expect("a list")
            .iter()
            .map(|p| (p[0].as_f64().unwrap(), p[1].as_f64().unwrap()))
            .collect()
    };
    let numbers = |v: &Value| -> Vec<f64> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_f64().unwrap())
            .collect()
    };
    Expected {
        points: pairs(&json["points"]),
        far: pairs(&json["far"]),
        times: json["times"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| (numbers(&t["u"]), numbers(&t["v"])))
            .collect(),
    }
}

/// Reads every time back and compares it; returns the failures, named.
fn compare(state: &AppState, name: &str, how: &str, steps: &[u32], want: &Expected) -> Vec<String> {
    let mut failures = Vec::new();
    for (time, (&step, (u, v))) in steps.iter().zip(&want.times).enumerate() {
        let got = sample_points_at_step(state, &want.points, step, None).expect("samples");
        for (k, sample) in got.iter().enumerate() {
            let (lon, lat) = want.points[k];
            if !sample.defined {
                failures.push(format!("{name} {how} t{time} ({lon}, {lat}): undefined"));
                continue;
            }
            let az = sample.azimuth_toward_deg.to_radians();
            let (gu, gv) = (sample.speed_mps * az.sin(), sample.speed_mps * az.cos());
            if (gu - u[k]).abs() > 1e-3 || (gv - v[k]).abs() > 1e-3 {
                failures.push(format!(
                    "{name} {how} t{time} ({lon}, {lat}): got ({gu:.4}, {gv:.4}) against ({:.4}, {:.4})",
                    u[k], v[k]
                ));
            }
        }
        let far = sample_points_at_step(state, &want.far, step, None).expect("samples");
        for (k, sample) in far.iter().enumerate() {
            if sample.defined {
                let (lon, lat) = want.far[k];
                failures.push(format!(
                    "{name} {how} t{time} ({lon}, {lat}) is outside the model but defined: {} m/s",
                    sample.speed_mps
                ));
            }
        }
    }
    failures
}

fn check_centre(centre: &str) {
    let mut failures = Vec::new();
    for file in models(centre) {
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        let want = expected(&file);
        let path = file.display().to_string();

        // As a new project: the settings come from the file.
        let root = TempRoot::new(&format!("{name}-new"));
        let state = AppState::new(AppPaths::in_directory(&root.0).expect("paths"));
        let started = Instant::now();
        match import::grib_project(&state, path.clone(), true) {
            Ok(summary) => {
                println!(
                    "{name}: new project {} in {:.0} ms, {} steps of {} h",
                    summary.resolution_label,
                    started.elapsed().as_secs_f64() * 1e3,
                    summary.step_count,
                    summary.step_hours
                );
                let steps: Vec<u32> = (0..want.times.len() as u32)
                    .map(|t| t * 3 / summary.step_hours)
                    .collect();
                failures.extend(compare(&state, &name, "new project", &steps, &want));
            }
            Err(e) => failures.push(format!("{name}: new project refused: {e}")),
        }

        // Into an existing global 0.25° project of 3-hour steps.
        let root = TempRoot::new(&format!("{name}-into"));
        let state = AppState::new(AppPaths::in_directory(&root.0).expect("paths"));
        projects::create(
            &state,
            NewProjectRequest {
                name: "Into".to_owned(),
                field_kind: "wind".to_owned(),
                resolution: "0.25".to_owned(),
                step_hours: 3,
                step_count: 3,
            },
            false,
        )
        .expect("create");
        let started = Instant::now();
        match import::grib_import(&state, path) {
            Ok(_) => {
                println!(
                    "{name}: import into 0.25° in {:.0} ms",
                    started.elapsed().as_secs_f64() * 1e3
                );
                failures.extend(compare(&state, &name, "import", &[0, 1, 2], &want));
            }
            Err(e) => failures.push(format!("{name}: import refused: {e}")),
        }
    }
    for f in &failures {
        println!("FAIL {f}");
    }
    assert!(failures.is_empty(), "{} failures", failures.len());
}

#[test]
#[ignore = "needs the downloaded model files; see the module docs"]
fn dwd_icon_models_import() {
    check_centre("icon");
}

#[test]
#[ignore = "needs the downloaded model files; see the module docs"]
fn meteo_france_arome_and_arpege_models_import() {
    check_centre("meteofrance");
}

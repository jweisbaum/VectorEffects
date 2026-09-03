//! Exports a painted project, for checking against an external decoder.
//!
//!     cargo run -p ve-app --example export_sample -- out.grib2
//!
//! Runs the real pipeline — project, brush strokes, evaluator, writer — so what
//! lands on disk is what the application produces, not a synthetic stand-in.

use std::sync::atomic::AtomicBool;

use ve_app::commands::AppState;
use ve_app::edit::{self, BrushStroke};
use ve_app::export::{self, ExportRequest};
use ve_app::paths::AppPaths;
use ve_app::projects::{self, NewProjectRequest};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "sample.grib2".to_owned());

    let root = std::env::temp_dir().join(format!("ve-export-sample-{}", std::process::id()));
    std::fs::create_dir_all(&root)?;
    let state = AppState::new(AppPaths::in_directory(&root)?);

    projects::create(
        &state,
        NewProjectRequest {
            name: "Sample".to_owned(),
            field_kind: "wind".to_owned(),
            resolution: "1.0".to_owned(),
            step_hours: 6,
            step_count: 3,
        },
        false,
    )?;

    // Due east on the equator, due north in the mid-latitudes: two headings a
    // decoder can be checked against without ambiguity.
    edit::paint(
        &state,
        BrushStroke {
            points: vec![[-40.0, 0.0], [0.0, 0.0], [40.0, 0.0]],
            size_km: 1500.0,
            speed_mps: 25.0,
            direction_toward_deg: 90.0,
            feather: 0.0,
            layer: None,
            ..Default::default()
        },
    )?;
    edit::paint(
        &state,
        BrushStroke {
            points: vec![[-120.0, 40.0]],
            size_km: 2500.0,
            speed_mps: 18.0,
            direction_toward_deg: 0.0,
            feather: 0.0,
            layer: None,
            ..Default::default()
        },
    )?;

    let project = {
        let mut session = state
            .session
            .lock()
            .map_err(|_| "session lock was poisoned")?;
        session.require_open()?.project.clone()
    };

    let result = export::run(
        &project,
        &ExportRequest {
            path: path.clone(),
            year: 2026,
            month: 9,
            day: 2,
            hour: 0,
            centre: 255,
        },
        &AtomicBool::new(false),
        |progress| println!("  step {}/{}", progress.step, progress.total),
    )?;

    println!(
        "wrote {}: {} bytes, {} messages in {} ms",
        result.path, result.bytes, result.messages, result.elapsed_ms
    );
    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}

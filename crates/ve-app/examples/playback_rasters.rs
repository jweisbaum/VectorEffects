//! Offline GRIB and Zarr playback fixtures with 0.25° time-varying fields.
//!
//! cargo run -p ve-app --example playback_rasters -- /tmp/ve-raster-fixtures 24
//! node tools/webdriver/playback.mjs 60 24 8,12,24,30 --project=/tmp/ve-raster-fixtures/grib.veproj
//! Repeat with zarr.veproj. The Zarr fixture uses the local GRIB backing file
//! a history import saves, so playback testing never contacts an archive.

use std::io::Write;
use std::sync::Arc;

use ve_core::document::Layer;
use ve_core::project::{FieldKind, MAX_STEPS, Project, ProjectSettings, Resolution, StepHours};
use ve_grib::writer::{GridSpec, MessageSpec, Parameter, ReferenceTime, message};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let out = std::path::PathBuf::from(args.next().ok_or("expected an output directory")?);
    let steps: u32 = args.next().unwrap_or_else(|| "24".to_owned()).parse()?;
    if !(2..=MAX_STEPS).contains(&steps) {
        return Err("steps must be in 2..=240".into());
    }
    std::fs::create_dir_all(&out)?;
    let out = out.canonicalize()?;
    let path = out.join("wind.grib2");
    let grid = GridSpec {
        ni: 1440,
        nj: 721,
        micro_degrees: 250_000,
    };
    let mut file = std::io::BufWriter::new(std::fs::File::create(&path)?);
    for step in 0..steps {
        for parameter in [Parameter::WindU, Parameter::WindV] {
            let samples: Vec<f32> = (0..grid.point_count())
                .map(|at| {
                    let lon =
                        (at % u64::from(grid.ni)) as f32 * std::f32::consts::TAU / grid.ni as f32;
                    let lat = (at / u64::from(grid.ni)) as f32 * std::f32::consts::PI
                        / (grid.nj - 1) as f32;
                    let phase = step as f32 * std::f32::consts::TAU / steps as f32;
                    match parameter {
                        Parameter::WindU => 15.0 + 10.0 * (lon + phase).sin() * lat.sin(),
                        _ => 8.0 * (lon - phase).cos() * lat.sin(),
                    }
                })
                .collect();
            file.write_all(&message(
                &MessageSpec {
                    parameter,
                    grid,
                    reference_time: ReferenceTime {
                        year: 2026,
                        month: 9,
                        day: 1,
                        hour: 0,
                        minute: 0,
                        second: 0,
                    },
                    forecast_hour: step,
                    centre: 255,
                    bits: 16,
                },
                &samples,
            )?)?;
        }
    }
    file.flush()?;
    let sequence = Arc::new(ve_grib::import::read_file(&path, None)?.sequences.remove(0));
    for history in [false, true] {
        let name = if history { "zarr" } else { "grib" };
        let mut project = Project::new(
            format!("{name} playback"),
            ProjectSettings::new(FieldKind::Wind, Resolution::Deg025, StepHours::H1, steps),
        );
        project.layers.push(if history {
            Layer::from_history(
                "Synthetic history",
                path.clone(),
                Arc::clone(&sequence),
                "era5-wind",
                sequence.frames[0].valid_unix_s,
                sequence.frames[steps as usize - 1].valid_unix_s,
            )
        } else {
            Layer::from_grib("Synthetic GRIB", path.clone(), Arc::clone(&sequence), true)
        });
        let project_path = out.join(format!("{name}.veproj"));
        ve_core::io::save(&project, &project_path)?;
        println!("{}", project_path.display());
    }
    Ok(())
}

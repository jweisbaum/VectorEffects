//! Writes four regional GRIB2 files, for checking against an external decoder.
//!
//!     cargo run -p ve-grib --example emit_regional -- outdir
//!
//! `u` is each node's longitude in [-180, 180) and `v` its latitude, so a node
//! the decoder places differently from the writer shows as a wrong number.

use ve_core::regrid::TargetGrid;
use ve_grib::writer::{GridSpec, MessageSpec, Parameter, ReferenceTime, write_message};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".to_owned()));
    std::fs::create_dir_all(&dir)?;
    let cases = [
        ("plain", 10.0, 50.0, 41, 41),
        ("antimeridian", 160.0, 10.0, 161, 5),
        ("prime", -20.0, 10.0, 161, 5),
        ("arctic", -180.0, 90.0, 1440, 121),
    ];
    for (name, lon0, lat0, ni, nj) in cases {
        let target = TargetGrid {
            ni,
            nj,
            lon0,
            lat0,
            dlon: 0.25,
            dlat: 0.25,
        };
        let grid = GridSpec::of_lattice(&target, 250_000);
        let points: Vec<(f64, f64)> = grid.points().collect();
        let mut out = std::fs::File::create(dir.join(format!("{name}.grib2")))?;
        for (parameter, values) in [
            (
                Parameter::WindU,
                points.iter().map(|p| p.0 as f32).collect::<Vec<_>>(),
            ),
            (
                Parameter::WindV,
                points.iter().map(|p| p.1 as f32).collect::<Vec<_>>(),
            ),
        ] {
            let spec = MessageSpec {
                parameter,
                grid,
                reference_time: ReferenceTime {
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
            write_message(&mut out, &spec, &values)?;
        }
    }
    Ok(())
}

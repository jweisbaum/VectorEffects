//! Writes a sample GRIB2 file, for checking against an external decoder.
//!
//!     cargo run -p ve-grib --example emit -- out.grib2
//!
//! The field is analytic and deliberately asymmetric — `u` varies with
//! latitude, `v` with longitude — so a transposed axis, a swapped `u`/`v`, or a
//! grid that starts in the wrong place all show up in the decoded values rather
//! than having to be inferred from headers.

use std::io::BufWriter;

use ve_grib::packing;
use ve_grib::writer::{GridSpec, MessageSpec, Parameter, ReferenceTime, write_message};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "sample.grib2".to_owned());
    let grid = GridSpec {
        ni: 360,
        nj: 181,
        micro_degrees: 1_000_000,
    };

    let points: Vec<(f64, f64)> = grid.points().collect();
    let u: Vec<f32> = points
        .iter()
        .map(|(_, lat)| (lat / 90.0 * 20.0) as f32)
        .collect();
    let v: Vec<f32> = points
        .iter()
        .map(|(lon, _)| (lon / 180.0 * 10.0) as f32)
        .collect();

    let base = MessageSpec {
        parameter: Parameter::WindU,
        grid,
        reference_time: ReferenceTime {
            year: 2026,
            month: 9,
            day: 2,
            hour: 0,
            minute: 0,
            second: 0,
        },
        forecast_hour: 0,
        centre: 255,
    };

    let file = std::fs::File::create(&path)?;
    let mut out = BufWriter::new(file);
    let mut total = 0;

    for step in 0..3u32 {
        for (parameter, values) in [(Parameter::WindU, &u), (Parameter::WindV, &v)] {
            let spec = MessageSpec {
                parameter,
                forecast_hour: step * 3,
                ..base
            };
            total += write_message(&mut out, &spec, values)?;
        }
    }

    println!("wrote {path}: {total} bytes, 6 messages");
    println!(
        "u range {:.2}..{:.2}, v range {:.2}..{:.2}",
        u.iter().copied().fold(f32::INFINITY, f32::min),
        u.iter().copied().fold(f32::NEG_INFINITY, f32::max),
        v.iter().copied().fold(f32::INFINITY, f32::min),
        v.iter().copied().fold(f32::NEG_INFINITY, f32::max),
    );
    println!(
        "packing step {:?}",
        packing::pack(&u).map(|p| p.binary_scale)
    );
    Ok(())
}

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! Where an export's time goes: evaluation, packing, or the write itself.
//!
//! Ignored and release-only, like the other cost harnesses. Run with
//! `cargo test -p ve-app --release --test export_cost -- --ignored --nocapture`.

use std::io::{BufWriter, Write};
use std::time::Instant;

use ve_core::document::{Geometry, LocalPoint, Object};
use ve_core::project::{FieldKind, Project, ProjectSettings, Resolution, StepHours};
use ve_core::schema::{PropId, ToolKind};
use ve_core::{LonLat, PropValue};
use ve_grib::writer::{GridSpec, MessageSpec, Parameter, ReferenceTime};
use ve_render::cpu::CpuEvaluator;
use ve_render::evaluator::FieldEvaluator;
use ve_render::scene::flatten_kind;

const OBJECTS: usize = 60;
const STEPS: u32 = 8;

fn project(resolution: Resolution) -> Project {
    let settings = ProjectSettings::new(FieldKind::Wind, resolution, StepHours::H1, STEPS);
    let mut project = Project::new("Export cost", settings);
    let layer = &mut project.layers[0];
    for i in 0..OBJECTS {
        let lon = -180.0 + (i as f64 * 37.3) % 360.0;
        let lat = -70.0 + (i as f64 * 23.7) % 140.0;
        let mut object = Object::new(ToolKind::Brush, format!("Stroke {i}"), STEPS);
        object.geometry = Geometry::Stroke {
            chains: vec![vec![
                LocalPoint::new(0.0, 0.0),
                LocalPoint::new(400_000.0, 200_000.0),
                LocalPoint::new(900_000.0, 0.0),
            ]],
        };
        if let Some(position) = object.props.get_mut(PropId::Position) {
            position.set_base(PropValue::LonLat(LonLat::new(lon, lat).expect("position")));
        }
        layer.objects.push(object);
    }
    project
}

fn ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

#[test]
#[ignore = "cost harness"]
fn where_the_export_time_goes() {
    for resolution in [Resolution::Deg05, Resolution::Deg025] {
        let project = project(resolution);
        let settings = &project.settings;
        let grid = GridSpec {
            ni: settings.resolution.ni(),
            nj: settings.resolution.nj(),
            micro_degrees: settings.resolution.micro_degrees(),
        };
        let points: Vec<ve_core::LonLat> = grid
            .points()
            .map(|(lon, lat)| ve_core::LonLat { lon, lat })
            .collect();
        let reference_time = ReferenceTime {
            year: 2026,
            month: 9,
            day: 20,
            hour: 0,
            minute: 0,
            second: 0,
        };
        let dir = std::env::temp_dir().join(format!("ve-export-cost-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp");
        let path = dir.join("cost.grib2");

        let mut flatten_ms = 0.0;
        let mut evaluate_ms = 0.0;
        let mut split_ms = 0.0;
        let mut encode_ms = 0.0;
        let mut write_ms = 0.0;
        let mut bytes = 0u64;

        let whole = Instant::now();
        {
            let mut out = BufWriter::new(std::fs::File::create(&path).expect("create"));
            let mut u = vec![0f32; points.len()];
            let mut v = vec![0f32; points.len()];
            for step in 0..STEPS {
                let hour = settings.forecast_hour(step);
                let started = Instant::now();
                let scene = flatten_kind(&project, step, FieldKind::Wind);
                flatten_ms += ms(started);

                let started = Instant::now();
                let samples = CpuEvaluator
                    .evaluate_samples(&scene, &points)
                    .expect("eval");
                evaluate_ms += ms(started);

                let started = Instant::now();
                for (index, sample) in samples.iter().enumerate() {
                    u[index] = if sample.coverage > 0.0 {
                        sample.uv.u
                    } else {
                        f32::NAN
                    };
                    v[index] = if sample.coverage > 0.0 {
                        sample.uv.v
                    } else {
                        f32::NAN
                    };
                }
                split_ms += ms(started);

                for (parameter, values) in [(Parameter::WindU, &u), (Parameter::WindV, &v)] {
                    let spec = MessageSpec {
                        parameter,
                        grid,
                        reference_time,
                        forecast_hour: hour,
                        centre: u16::MAX,
                        bits: ve_grib::packing::BITS_PER_VALUE,
                    };
                    let started = Instant::now();
                    let message = ve_grib::writer::message_masked(&spec, values).expect("message");
                    encode_ms += ms(started);
                    let started = Instant::now();
                    out.write_all(&message).expect("write");
                    write_ms += ms(started);
                    bytes += message.len() as u64;
                }
            }
            let started = Instant::now();
            out.flush().expect("flush");
            write_ms += ms(started);
        }
        let total_ms = ms(whole);
        let _ = std::fs::remove_dir_all(&dir);

        println!(
            "\n{} — {} objects, {STEPS} steps, {} points/message, {:.1} MB",
            resolution.label(),
            OBJECTS,
            points.len(),
            bytes as f64 / (1024.0 * 1024.0),
        );
        for (label, value) in [
            ("flatten", flatten_ms),
            ("evaluate", evaluate_ms),
            ("split u/v", split_ms),
            ("encode", encode_ms),
            ("write", write_ms),
        ] {
            println!(
                "  {label:10} {value:9.1} ms   {:5.1}%",
                100.0 * value / total_ms
            );
        }
        println!("  {:10} {total_ms:9.1} ms", "total");
    }
}

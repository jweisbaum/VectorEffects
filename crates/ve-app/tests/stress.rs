#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! M10's large-project stress: 5,000 objects across 240 steps, against the
//! budgets in spec.md 13 that can be measured without a screen.
//!
//! The numbers are printed in every build and asserted only in release, as
//! `tile_cost` does: a debug figure depends on the profile, and the profile is
//! a moving target. Run with `--nocapture` to read them.

use std::time::Instant;

use ve_core::document::{Geometry, LocalPoint, Object};
use ve_core::project::{FieldKind, Project, ProjectSettings, Resolution, StepHours};
use ve_core::schema::{PropId, ToolKind};
use ve_core::{LonLat, PropValue};

const OBJECTS: usize = 5_000;
const STEPS: u32 = 240;

/// The document spec 13 names: five thousand painted strokes spread over the
/// globe, each a short chain, on a 240-step timeline.
fn large_project() -> Project {
    let settings = ProjectSettings::new(FieldKind::Wind, Resolution::Deg1, StepHours::H1, STEPS);
    let mut project = Project::new("Stress", settings);
    let layer = &mut project.layers[0];
    for i in 0..OBJECTS {
        let lon = -180.0 + (i as f64 * 7.3) % 360.0;
        let lat = -80.0 + (i as f64 * 3.7) % 160.0;
        let mut object = Object::new(ToolKind::Brush, format!("Stroke {i}"), STEPS);
        object.geometry = Geometry::Stroke {
            chains: vec![vec![
                LocalPoint::new(0.0, 0.0),
                LocalPoint::new(150_000.0, 40_000.0),
                LocalPoint::new(300_000.0, 0.0),
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
fn five_thousand_objects_over_240_steps() {
    let project = large_project();
    let dir = std::env::temp_dir().join(format!("ve-stress-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp");
    let path = dir.join("stress.veproj");
    println!("stress (debug_assertions={}):", cfg!(debug_assertions));

    let started = Instant::now();
    ve_core::io::save(&project, &path).expect("save");
    let save_ms = ms(started);
    let bytes = std::fs::metadata(&path).expect("metadata").len();
    println!("  save   {save_ms:8.1} ms   {} KB", bytes / 1024);

    // Spec 13: project open, 5,000 objects, ≤ 500 ms.
    let started = Instant::now();
    let loaded = ve_core::io::load(&path).expect("load");
    let open_ms = ms(started);
    println!("  open   {open_ms:8.1} ms   budget 500 ms");
    assert_eq!(loaded.object_count(), OBJECTS);

    // Flattening a step is what every tile and every export message begins
    // with; 240 of them is the whole timeline.
    let started = Instant::now();
    let mut objects = 0;
    for step in 0..STEPS {
        objects += ve_render::scene::flatten(&loaded, step).objects.len();
    }
    let flatten_ms = ms(started) / f64::from(STEPS);
    println!("  flatten {flatten_ms:7.2} ms/step   ({objects} flat objects over {STEPS} steps)");

    // One preview tile of the whole scene at the CPU fallback's quality. Spec
    // 13's 40 ms is stated for a scene of 200 objects; this one has 5,000, so
    // the number is reported against that budget for scale, not asserted.
    let scene = ve_render::scene::flatten(&loaded, 0);
    let id = ve_render::tile::TileId::new(3, 4, 3).expect("tile");
    let started = Instant::now();
    let _ = ve_render::preview::render_tile(
        &ve_render::cpu::CpuEvaluator,
        &scene,
        id,
        ve_render::preview::Quality::Standard,
    );
    let tile_ms = ms(started);
    println!("  tile    {tile_ms:7.1} ms   (spec 13's 40 ms is for 200 objects; this is 5,000)");

    let _ = std::fs::remove_dir_all(&dir);

    if !cfg!(debug_assertions) {
        assert!(
            open_ms <= 500.0,
            "opening 5,000 objects took {open_ms:.0} ms, budget 500"
        );
    }
}

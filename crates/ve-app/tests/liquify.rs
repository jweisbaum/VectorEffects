#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! M17's acceptance: a liquify stroke drags the field along the hand, and
//! nowhere else (spec.md 6.3).

use ve_app::commands::AppState;
use ve_app::create::{self, Gesture, NewObject, Tool, ToolOption};
use ve_app::document::{self, PropertyValue};
use ve_app::edit::{self, BrushStroke};
use ve_app::paths::AppPaths;
use ve_app::projects::{self, NewProjectRequest};
use ve_core::LonLat;

struct TempRoot(std::path::PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ve-liquify-{}-{label}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("temp root");
        Self(dir)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A project with a northward field whose speed steps up at the prime
/// meridian: 5 m/s to the west of it and 15 to the east, both hard-edged.
fn stepped(label: &str) -> (TempRoot, AppState) {
    let root = TempRoot::new(label);
    let state = AppState::new(AppPaths::in_directory(&root.0).expect("paths"));
    projects::create(
        &state,
        NewProjectRequest {
            name: "Liquify".to_owned(),
            field_kind: "wind".to_owned(),
            resolution: "1.0".to_owned(),
            step_hours: 3,
            step_count: 2,
        },
        false,
    )
    .expect("create");
    for (points, speed) in [
        ([[-20.0, 0.0], [-0.5, 0.0]], 5.0),
        ([[0.5, 0.0], [20.0, 0.0]], 15.0),
    ] {
        edit::paint(
            &state,
            BrushStroke {
                points: points.to_vec(),
                size_km: 1_400.0,
                speed_mps: speed,
                direction_toward_deg: 0.0,
                feather: 0.0,
                layer: None,
                ..Default::default()
            },
        )
        .expect("paint");
    }
    (root, state)
}

fn ll(lon: f64, lat: f64) -> LonLat {
    LonLat::new(lon, lat).expect("position")
}

fn speed(state: &AppState, at: LonLat) -> f64 {
    let project = {
        let mut session = state.session.lock().expect("lock");
        session.require_open().expect("open").project.clone()
    };
    let scene = ve_render::scene::flatten(&project, 0);
    let uv = ve_render::cpu::sample_scene(&scene, at);
    ve_core::vector::speed_azimuth_from_uv(uv).0
}

fn number(property: &str, value: f64) -> ToolOption {
    ToolOption {
        property: property.to_owned(),
        value: PropertyValue::Number { value },
    }
}

/// The acceptance case: a liquify stroke east over a northward field reads the
/// field from west of each cell, so the slow western field is dragged across
/// the step and shows up east of it.
#[test]
fn a_stroke_east_drags_the_western_field_over_the_step() {
    let (_root, state) = stepped("east");
    assert!(
        (speed(&state, ll(3.0, 0.0)) - 15.0).abs() < 0.5,
        "east of the step is fast"
    );

    // Ten degrees of stroke, eastward, at full strength and hard-edged, so a
    // cell three degrees east of the step reads from well west of it.
    create::create(
        &state,
        NewObject {
            tool: Tool::Liquify,
            gesture: Gesture::Stroke {
                points: vec![[-5.0, 0.0], [0.0, 0.0], [5.0, 0.0]],
            },
            options: vec![
                number("SizeKm", 1_200.0),
                number("Strength", 100.0),
                number("Feather", 0.0),
            ],
            layer: None,
        },
    )
    .expect("liquify");

    assert!(
        (speed(&state, ll(3.0, 0.0)) - 5.0).abs() < 0.5,
        "the slow field was not dragged east: {} m/s",
        speed(&state, ll(3.0, 0.0))
    );
    // Well outside the stroke nothing moved.
    assert!((speed(&state, ll(15.0, 0.0)) - 15.0).abs() < 0.5);
    assert!((speed(&state, ll(-15.0, 0.0)) - 5.0).abs() < 0.5);
}

/// Half strength drags half as far — which here is not far enough to cross the
/// step from three degrees out.
#[test]
fn strength_scales_the_drag() {
    let (_root, state) = stepped("strength");
    create::create(
        &state,
        NewObject {
            tool: Tool::Liquify,
            gesture: Gesture::Stroke {
                points: vec![[-5.0, 0.0], [0.0, 0.0], [5.0, 0.0]],
            },
            options: vec![
                number("SizeKm", 1_200.0),
                number("Strength", 20.0),
                number("Feather", 0.0),
            ],
            layer: None,
        },
    )
    .expect("liquify");
    assert!(
        (speed(&state, ll(4.5, 0.0)) - 15.0).abs() < 0.5,
        "a fifth of the drag should not reach four and a half degrees: {} m/s",
        speed(&state, ll(4.5, 0.0))
    );
}

/// Two liquify strokes stay two objects: a smear's deltas are its own.
#[test]
fn a_liquify_never_merges() {
    let (_root, state) = stepped("merge");
    for points in [
        vec![[-4.0, 0.0], [-1.0, 0.0]],
        vec![[-2.0, 0.0], [1.0, 0.0]],
    ] {
        create::create(
            &state,
            NewObject {
                tool: Tool::Liquify,
                gesture: Gesture::Stroke { points },
                options: vec![number("SizeKm", 1_200.0)],
                layer: None,
            },
        )
        .expect("liquify");
    }
    let objects = document::tree(&state, 0).expect("tree").layers[0]
        .objects
        .len();
    assert_eq!(
        objects, 4,
        "two brush strokes and two liquifies, none absorbed"
    );
}

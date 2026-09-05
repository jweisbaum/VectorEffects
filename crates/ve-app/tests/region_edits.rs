#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! The brush and the operators, applied to a selected region, make an object
//! from the region's boundary and act on nothing else (spec.md 8.2).

use ve_app::commands::AppState;
use ve_app::create::{self, Gesture, NewObject, Tool};
use ve_app::document;
use ve_app::edit::{self, BrushStroke};
use ve_app::paths::AppPaths;
use ve_app::projects::{self, NewProjectRequest};
use ve_core::LonLat;

struct TempRoot(std::path::PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ve-region-edit-{}-{label}-{}",
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

/// A project with one wide stroke along the equator: 15 m/s due east from
/// 20°W to 20°E, hard-edged so a sampled speed is a plain yes or no.
fn painted(label: &str) -> (TempRoot, AppState) {
    let root = TempRoot::new(label);
    let state = AppState::new(AppPaths::in_directory(&root.0).expect("paths"));
    projects::create(
        &state,
        NewProjectRequest {
            name: "Region".to_owned(),
            field_kind: "wind".to_owned(),
            resolution: "1.0".to_owned(),
            step_hours: 3,
            step_count: 2,
        },
        false,
    )
    .expect("create");
    edit::paint(
        &state,
        BrushStroke {
            points: vec![[-20.0, 0.0], [20.0, 0.0]],
            size_km: 1200.0,
            speed_mps: 15.0,
            direction_toward_deg: 90.0,
            feather: 0.0,
            layer: None,
            ..Default::default()
        },
    )
    .expect("paint");
    (root, state)
}

fn ll(lon: f64, lat: f64) -> LonLat {
    LonLat::new(lon, lat).expect("position")
}

/// Speed at a position, from the authoritative evaluator.
fn speed(state: &AppState, at: LonLat) -> f64 {
    let project = {
        let mut session = state.session.lock().expect("lock");
        session.require_open().expect("open").project.clone()
    };
    let scene = ve_render::scene::flatten(&project, 0);
    let uv = ve_render::cpu::sample_scene(&scene, at);
    ve_core::vector::speed_azimuth_from_uv(uv).0
}

fn objects(state: &AppState) -> usize {
    document::tree(state, 0).expect("tree").layers[0]
        .objects
        .len()
}

/// A brush made from a region paints the region and nothing else: the field
/// appears inside the ring and not outside it.
#[test]
fn a_brush_from_a_ring_paints_inside_the_ring_only() {
    let root = TempRoot::new("brush");
    let state = AppState::new(AppPaths::in_directory(&root.0).expect("paths"));
    projects::create(
        &state,
        NewProjectRequest {
            name: "Region".to_owned(),
            field_kind: "wind".to_owned(),
            resolution: "1.0".to_owned(),
            step_hours: 3,
            step_count: 2,
        },
        false,
    )
    .expect("create");
    assert!(
        speed(&state, ll(5.0, 0.0)) < 0.01,
        "an empty project is calm"
    );

    create::create(
        &state,
        NewObject {
            tool: Tool::Brush,
            gesture: Gesture::Ring {
                points: vec![[0.0, -5.0], [10.0, -5.0], [10.0, 5.0], [0.0, 5.0]],
            },
            options: vec![],
            layer: None,
        },
    )
    .expect("brush the region");

    assert!(
        speed(&state, ll(5.0, 0.0)) > 0.1,
        "inside the region is painted"
    );
    assert!(
        speed(&state, ll(-5.0, 0.0)) < 0.01,
        "outside it is still calm"
    );
    assert!(speed(&state, ll(15.0, 0.0)) < 0.01, "on both sides");
}

/// A mask made from a polygon region masks the polygon and nothing else.
#[test]
fn a_mask_from_a_ring_masks_inside_the_ring_only() {
    let (_root, state) = painted("ring");
    assert!(
        speed(&state, ll(10.0, 0.0)) > 14.0,
        "the stroke is there to mask"
    );

    // The eastern half of the stroke, as a rectangle's four corners — which is
    // how the frontend sends a selected rectangle.
    create::create(
        &state,
        NewObject {
            tool: Tool::Mask,
            gesture: Gesture::Ring {
                points: vec![[0.0, -8.0], [25.0, -8.0], [25.0, 8.0], [0.0, 8.0]],
            },
            options: vec![],
            layer: None,
        },
    )
    .expect("mask the region");

    assert!(
        speed(&state, ll(10.0, 0.0)) < 0.01,
        "inside the region is masked"
    );
    assert!(
        speed(&state, ll(-10.0, 0.0)) > 14.0,
        "outside it is untouched"
    );
}

/// And from a circle, sent as an extent, which for an operator is a disc.
#[test]
fn a_mask_from_an_extent_is_a_disc() {
    let (_root, state) = painted("disc");
    create::create(
        &state,
        NewObject {
            tool: Tool::Mask,
            gesture: Gesture::Extent {
                centre: [10.0, 0.0],
                rim: [10.0, 4.0],
            },
            options: vec![],
            layer: None,
        },
    )
    .expect("mask the disc");

    assert!(speed(&state, ll(10.0, 0.0)) < 0.01, "the centre is masked");
    assert!(
        speed(&state, ll(10.0, 3.0)) < 0.01,
        "and so is a point inside the radius"
    );
    // Four degrees north-east is outside a four-degree disc; a rectangle of the
    // same extents would have covered it.
    assert!(
        speed(&state, ll(13.5, 3.5)) > 14.0,
        "a corner of the bounding box is not"
    );
    assert!(
        speed(&state, ll(-10.0, 0.0)) > 14.0,
        "the rest of the stroke is untouched"
    );
}

/// A region-made operator stays its own object: it never merges into a
/// painted stroke of the same tool, because it is not a stroke.
#[test]
fn a_region_operator_does_not_merge_with_a_painted_one() {
    let (_root, state) = painted("merge");
    let before = objects(&state);

    create::create(
        &state,
        NewObject {
            tool: Tool::Mask,
            gesture: Gesture::Ring {
                points: vec![[0.0, -5.0], [10.0, -5.0], [10.0, 5.0], [0.0, 5.0]],
            },
            options: vec![],
            layer: None,
        },
    )
    .expect("region mask");
    // A painted mask straight over it, which two painted masks would merge.
    create::create(
        &state,
        NewObject {
            tool: Tool::Mask,
            gesture: Gesture::Stroke {
                points: vec![[2.0, 0.0], [8.0, 0.0]],
            },
            options: vec![],
            layer: None,
        },
    )
    .expect("painted mask");

    assert_eq!(
        objects(&state),
        before + 2,
        "two objects, not one merged one"
    );
}

/// The tools that are measured from their anchor to somewhere else do not take
/// a region: "the region" does not say where.
#[test]
fn the_anchored_tools_refuse_a_region() {
    let (_root, state) = painted("refuse");
    for tool in [Tool::CloneStamp, Tool::Warp] {
        let result = create::create(
            &state,
            NewObject {
                tool,
                gesture: Gesture::Ring {
                    points: vec![[0.0, 0.0], [5.0, 0.0], [5.0, 5.0]],
                },
                options: vec![],
                layer: None,
            },
        );
        assert!(result.is_err(), "{tool:?} must not take a ring");
    }
}

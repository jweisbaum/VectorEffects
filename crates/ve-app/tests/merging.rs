#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! Merging overlapping strokes that share every property.
//!
//! The point of the feature is that going over an area repeatedly leaves one
//! thing to select, move and animate. The point of these tests is that it never
//! does so by changing what the layer looks like: not by bridging the gap
//! between two strokes, and not by moving a stroke beneath something it was
//! painted on top of.

use ve_app::commands::AppState;
use ve_app::document;
use ve_app::edit::{self, BrushStroke};
use ve_app::paths::AppPaths;
use ve_app::projects::{self, NewProjectRequest};
use ve_core::document::Geometry;

struct TempRoot(std::path::PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ve-merging-{}-{label}-{}",
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

/// An empty project.
fn project(label: &str) -> (TempRoot, AppState) {
    let root = TempRoot::new(label);
    let state = AppState::new(AppPaths::in_directory(&root.0).expect("paths"));
    projects::create(
        &state,
        NewProjectRequest {
            name: "Merge".to_owned(),
            field_kind: "wind".to_owned(),
            resolution: "1.0".to_owned(),
            step_hours: 3,
            step_count: 12,
        },
        false,
    )
    .expect("create");
    (root, state)
}

/// A stroke with the defaults these tests vary from.
fn stroke(points: Vec<[f64; 2]>) -> BrushStroke {
    BrushStroke {
        points,
        size_km: 800.0,
        speed_mps: 15.0,
        direction_toward_deg: 90.0,
        feather: 0.3,
        layer: None,
        ..Default::default()
    }
}

fn paint(state: &AppState, stroke: BrushStroke) {
    edit::paint(state, stroke).expect("paint");
}

fn object_count(state: &AppState) -> usize {
    document::tree(state, 0).expect("tree").layers[0]
        .objects
        .len()
}

/// The layer's geometry, read back through a save so the assertions cover what
/// actually reaches disk.
fn geometries(root: &TempRoot, state: &AppState) -> Vec<Geometry> {
    let path = root.0.join("readback.veproj");
    projects::save_as(state, path.to_string_lossy().into_owned()).expect("save");
    let project = ve_core::io::load(&path).expect("load");
    project.layers[0]
        .objects
        .iter()
        .map(|o| o.geometry.clone())
        .collect()
}

fn chain_count(geometry: &Geometry) -> usize {
    geometry.stroke_chains().len()
}

#[test]
fn an_overlapping_identical_stroke_joins_the_one_beneath_it() {
    let (root, state) = project("overlap");
    paint(&state, stroke(vec![[0.0, 0.0], [4.0, 0.0]]));
    // 800 km diameter is 400 km of reach, so a stroke starting 2 degrees away
    // (about 222 km) overlaps.
    paint(&state, stroke(vec![[6.0, 0.0], [10.0, 0.0]]));

    assert_eq!(object_count(&state), 1, "the two strokes are one object");
    let geometries = geometries(&root, &state);
    assert_eq!(
        chain_count(&geometries[0]),
        2,
        "and hold their polylines separately, so the gap stays unpainted"
    );
}

#[test]
fn a_distant_stroke_stays_its_own_object() {
    let (_root, state) = project("distant");
    paint(&state, stroke(vec![[0.0, 0.0], [4.0, 0.0]]));
    paint(&state, stroke(vec![[80.0, 0.0], [84.0, 0.0]]));

    assert_eq!(object_count(&state), 2);
}

#[test]
fn a_stroke_with_different_properties_stays_its_own_object() {
    let (_root, state) = project("different");
    paint(&state, stroke(vec![[0.0, 0.0], [4.0, 0.0]]));

    let mut faster = stroke(vec![[2.0, 0.0], [6.0, 0.0]]);
    faster.speed_mps = 25.0;
    paint(&state, faster);

    assert_eq!(
        object_count(&state),
        2,
        "a different speed renders differently and cannot share an object"
    );
}

/// Merging pulls a stroke down to the target's place in the stack. If anything
/// overlapping sits in between, that would change what covers what.
#[test]
fn a_stroke_is_not_pulled_beneath_something_it_overlaps() {
    let (_root, state) = project("blocked");
    paint(&state, stroke(vec![[0.0, 0.0], [4.0, 0.0]]));

    let mut blocker = stroke(vec![[2.0, 0.0], [6.0, 0.0]]);
    blocker.direction_toward_deg = 270.0;
    paint(&state, blocker);

    // Overlaps the first stroke and would merge with it, but the blocker is
    // above it and in the way.
    paint(&state, stroke(vec![[3.0, 0.0], [7.0, 0.0]]));

    assert_eq!(object_count(&state), 3, "z-order comes before tidiness");
}

/// A blocker that is nowhere near the new stroke is no obstacle to merging.
#[test]
fn an_unrelated_object_above_does_not_block_a_merge() {
    let (root, state) = project("unrelated");
    paint(&state, stroke(vec![[0.0, 0.0], [4.0, 0.0]]));

    let mut elsewhere = stroke(vec![[100.0, 40.0], [104.0, 40.0]]);
    elsewhere.direction_toward_deg = 270.0;
    paint(&state, elsewhere);

    paint(&state, stroke(vec![[6.0, 0.0], [10.0, 0.0]]));

    assert_eq!(object_count(&state), 2);
    let geometries = geometries(&root, &state);
    assert_eq!(chain_count(&geometries[0]), 2, "the merge still happened");
}

#[test]
fn undoing_a_merge_restores_the_single_chain() {
    let (root, state) = project("undo");
    paint(&state, stroke(vec![[0.0, 0.0], [4.0, 0.0]]));
    paint(&state, stroke(vec![[6.0, 0.0], [10.0, 0.0]]));
    assert_eq!(chain_count(&geometries(&root, &state)[0]), 2);

    edit::undo_for_test(&state).expect("undo");

    assert_eq!(object_count(&state), 1);
    assert_eq!(
        chain_count(&geometries(&root, &state)[0]),
        1,
        "undo puts the first stroke back as it was"
    );
}

/// An object that only exists later still covers the stroke when it appears,
/// so it blocks a merge that would slide the stroke underneath it.
#[test]
fn an_object_active_only_later_still_blocks() {
    let (_root, state) = project("later");
    paint(&state, stroke(vec![[0.0, 0.0], [4.0, 0.0]]));

    let mut blocker = stroke(vec![[2.0, 0.0], [6.0, 0.0]]);
    blocker.direction_toward_deg = 270.0;
    paint(&state, blocker);
    let id = document::tree(&state, 0).expect("tree").layers[0].objects[1].id;
    document::object_range(&state, id, 6, 11).expect("range");

    paint(&state, stroke(vec![[3.0, 0.0], [7.0, 0.0]]));

    assert_eq!(object_count(&state), 3);
}

/// Two strokes aimed at the same point render identically apart from where
/// they sit, exactly as two constant strokes do, so they merge the same way.
#[test]
fn aimed_strokes_with_the_same_target_join_like_constant_ones() {
    let (_root, state) = project("aimed");
    let mut first = stroke(vec![[0.0, 0.0], [4.0, 0.0]]);
    first.direction_mode = ve_app::edit::BrushDirectionMode::TowardPoint;
    first.target = Some([20.0, 10.0]);
    let mut second = stroke(vec![[3.0, 0.0], [7.0, 0.0]]);
    second.direction_mode = ve_app::edit::BrushDirectionMode::TowardPoint;
    second.target = Some([20.0, 10.0]);
    paint(&state, first);
    paint(&state, second);
    assert_eq!(object_count(&state), 1, "aimed strokes merge");
}

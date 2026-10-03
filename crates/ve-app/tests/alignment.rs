#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! A layer's file against the timeline's valid times (spec.md 4.8, M91):
//! the timeline says where a file sits, and one click puts it right.

use std::sync::Arc;

use ve_app::commands::AppState;
use ve_app::document::{self, LayerAlignment};
use ve_app::edit;
use ve_app::paths::AppPaths;
use ve_app::projects::{self, NewProjectRequest};
use ve_core::document::Layer;
use ve_core::project::FieldKind;
use ve_core::raster::{RasterFrame, RasterGrid, RasterSequence};

const HOUR: i64 = 3600;
/// 2026-10-01T00Z, worked out in `tests/nrt.rs`.
const OCT_1: i64 = 20_727 * 86_400;

struct TempRoot(std::path::PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ve-align-{}-{label}-{}",
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

/// A three-hourly project of eight steps, starting on 1 October.
fn app(root: &TempRoot) -> AppState {
    let state = AppState::new(AppPaths::in_directory(&root.0).expect("paths"));
    projects::create(
        &state,
        NewProjectRequest {
            name: "Align".to_owned(),
            field_kind: "wind".to_owned(),
            resolution: "1.0".to_owned(),
            step_hours: 3,
            step_count: 8,
        },
        false,
    )
    .expect("create");
    ve_app::animation::start_at(&state, Some(OCT_1)).expect("dated");
    state
}

/// A file whose messages are valid at `times`, pushed straight into the
/// project the way an import would leave it: first message on step 0.
fn file_layer(state: &AppState, times: &[i64]) -> u64 {
    let frames = times
        .iter()
        .map(|&t| RasterFrame {
            offset_hours: (t - times[0]) as f64 / 3600.0,
            valid_unix_s: t,
            grid: Arc::new(RasterGrid::new(2, 2, 0.0, 1.0, 1.0, 1.0, vec![[2.0, 0.0]; 4]).unwrap()),
        })
        .collect();
    let sequence = Arc::new(RasterSequence::new(FieldKind::Wind, frames).unwrap());
    let layer = Layer::from_grib("file", "file.grib2".into(), sequence, true);
    let id = layer.id.raw();
    let mut session = state.session.lock().expect("lock");
    session
        .require_open()
        .expect("open")
        .project
        .layers
        .push(layer);
    id
}

fn alignment(state: &AppState, at: usize) -> Option<LayerAlignment> {
    let tree = document::tree(state, 0).expect("tree");
    tree.layers[at]
        .grib
        .as_ref()
        .expect("a file layer")
        .misaligned
        .clone()
}

fn covered(state: &AppState, at: usize) -> String {
    let tree = document::tree(state, 0).expect("tree");
    tree.layers[at]
        .grib
        .as_ref()
        .expect("a file layer")
        .covered_steps
        .iter()
        .map(|c| if *c { '#' } else { '.' })
        .collect()
}

/// A file three hours later than the timeline's start is marked as three
/// hours off, aligning it lands its first message on step 1, and one undo
/// puts it back.
#[test]
fn a_late_file_is_marked_aligned_in_one_step_and_undone_in_one() {
    let root = TempRoot::new("late");
    let state = app(&root);
    let layer = file_layer(
        &state,
        &[OCT_1 + 3 * HOUR, OCT_1 + 6 * HOUR, OCT_1 + 9 * HOUR],
    );
    assert_eq!(covered(&state, 1), "###.....");

    let off = alignment(&state, 1).expect("marked");
    assert_eq!(off.first_valid_unix_s, OCT_1 + 3 * HOUR);
    assert_eq!(off.step_unix_s, OCT_1);
    assert_eq!(off.offset_hours, 3.0);
    assert_eq!(off.aligned_lead, Some(1));

    document::layer_aligned(&state, layer).expect("align");
    assert_eq!(alignment(&state, 1), None, "aligned");
    assert_eq!(covered(&state, 1), ".###....");

    edit::undo_for_test(&state).expect("undo");
    assert_eq!(covered(&state, 1), "###.....");
    assert!(alignment(&state, 1).is_some());
    edit::redo_for_test(&state).expect("redo");
    assert_eq!(covered(&state, 1), ".###....");
}

/// A file that begins before the timeline aligns to a negative lead: its
/// first message is before step 0 and its second lands there.
#[test]
fn an_early_file_aligns_behind_the_timeline() {
    let root = TempRoot::new("early");
    let state = app(&root);
    let layer = file_layer(&state, &[OCT_1 - 3 * HOUR, OCT_1, OCT_1 + 3 * HOUR]);
    let off = alignment(&state, 1).expect("marked");
    assert_eq!(off.offset_hours, -3.0);
    assert_eq!(off.aligned_lead, Some(-1));
    document::layer_aligned(&state, layer).expect("align");
    assert_eq!(covered(&state, 1), "##......");
    assert_eq!(alignment(&state, 1), None);
}

/// An hour off on a three-hourly timeline has no step to land on: marked,
/// with no lead offered, and the one-click fix is refused with the offset.
#[test]
fn an_offset_that_is_not_whole_steps_is_marked_but_cannot_be_fixed() {
    let root = TempRoot::new("odd");
    let state = app(&root);
    let layer = file_layer(&state, &[OCT_1 + HOUR, OCT_1 + 4 * HOUR]);
    let off = alignment(&state, 1).expect("marked");
    assert_eq!(off.offset_hours, 1.0);
    assert_eq!(off.aligned_lead, None);
    let refused = document::layer_aligned(&state, layer)
        .expect_err("no step to land on")
        .to_string();
    assert!(refused.contains("1 h"), "{refused}");
}

/// An aligned file is not marked, and a timeline with no start has nothing
/// to be aligned with; a painted layer has no times at all.
#[test]
fn nothing_is_marked_without_a_disagreement() {
    let root = TempRoot::new("quiet");
    let state = app(&root);
    file_layer(&state, &[OCT_1, OCT_1 + 3 * HOUR]);
    assert_eq!(alignment(&state, 1), None);
    let tree = document::tree(&state, 0).expect("tree");
    assert!(tree.layers[0].grib.is_none(), "a painted layer has no file");

    let late = file_layer(&state, &[OCT_1 + 3 * HOUR]);
    assert!(alignment(&state, 2).is_some());
    ve_app::animation::start_at(&state, None).expect("undated");
    assert_eq!(
        alignment(&state, 2),
        None,
        "no start, nothing to align with"
    );
    assert!(document::layer_aligned(&state, late).is_err());
}

/// A lead made for a start the timeline no longer has is not left unseen:
/// it is marked, and the fix puts the first message back on step 0, which
/// is where an undated timeline puts every file.
#[test]
fn a_lead_left_over_from_a_start_is_marked_and_reset() {
    let root = TempRoot::new("orphan");
    let state = app(&root);
    let layer = file_layer(&state, &[OCT_1 + 3 * HOUR, OCT_1 + 6 * HOUR]);
    document::layer_aligned(&state, layer).expect("align");
    ve_app::animation::start_at(&state, None).expect("undated");
    let left = alignment(&state, 1).expect("marked");
    assert!(left.undated);
    assert_eq!((left.lead_steps, left.aligned_lead), (1, Some(0)));
    assert_eq!(covered(&state, 1), ".##.....");
    document::layer_aligned(&state, layer).expect("reset");
    assert_eq!(alignment(&state, 1), None);
    assert_eq!(covered(&state, 1), "##......");
}

/// A hidden message, a pasted run and a one-step erasure name a message,
/// so they move with it: the bad message hidden on step 0 is still hidden
/// on step 1 after the file moves there, and one undo puts all of it back.
#[test]
fn hidden_pasted_and_erased_steps_move_with_the_messages() {
    use ve_core::document::{FrameOverride, RasterErasure};
    let root = TempRoot::new("edits");
    let state = app(&root);
    let layer = file_layer(
        &state,
        &[OCT_1 + 3 * HOUR, OCT_1 + 6 * HOUR, OCT_1 + 9 * HOUR],
    );
    {
        let mut session = state.session.lock().expect("lock");
        let open = session.require_open().expect("open");
        let l = &mut open.project.layers[1];
        l.set_frame_overrides(vec![
            FrameOverride {
                step: 0,
                source: None,
            },
            FrameOverride {
                step: 5,
                source: Some(2),
            },
            FrameOverride {
                step: 7,
                source: Some(1),
            },
        ]);
        l.erased = vec![RasterErasure {
            chains: vec![],
            radius_m: 1000.0,
            square: false,
            projected: false,
            projection: 0,
            projection_origin: None,
            feather: 0.0,
            step: Some(2),
        }];
    }
    document::layer_aligned(&state, layer).expect("align");
    {
        let mut session = state.session.lock().expect("lock");
        let l = &session.require_open().expect("open").project.layers[1];
        assert_eq!(
            l.frame_overrides,
            vec![
                FrameOverride {
                    step: 1,
                    source: None
                },
                FrameOverride {
                    step: 6,
                    source: Some(3)
                },
            ],
            "the run pasted onto step 7 would land past the end, and goes"
        );
        assert_eq!(l.erased[0].step, Some(3));
        assert_eq!(l.lead_steps, 1);
    }
    edit::undo_for_test(&state).expect("undo");
    let mut session = state.session.lock().expect("lock");
    let l = &session.require_open().expect("open").project.layers[1];
    assert_eq!(
        (l.lead_steps, l.frame_overrides.len(), l.erased[0].step),
        (0, 3, Some(2))
    );
}

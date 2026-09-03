#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! Rendering ahead, readiness, and — above all — invalidation (spec.md 9.5).
//!
//! The plan names this milestone's risk exactly: the readiness indicator is
//! only trustworthy if invalidation is exact. Invalidation here is not a
//! mechanism but a consequence of content-addressed keys (spec.md 7.10), so
//! what these tests hold is the property itself — that an edit changes the
//! hash of precisely the steps it changes the look of — and then that the pool
//! and the readiness probe honour that property rather than tracking anything
//! of their own.

use ve_app::animation;
use ve_app::commands::AppState;
use ve_app::document::{self, PropertyValue};
use ve_app::edit;
use ve_app::paths::AppPaths;
use ve_app::projects::{self, NewProjectRequest};
use ve_app::render_pool::{RenderPool, TileAddress};
use ve_render::cache::{SceneHash, scene_hash};
use ve_render::scene::flatten;

struct TempRoot(std::path::PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ve-ready-{}-{label}-{}",
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

const STEPS: u32 = 12;

fn project(label: &str) -> (TempRoot, AppState) {
    let root = TempRoot::new(label);
    let state = AppState::new(AppPaths::in_directory(&root.0).expect("paths"));
    projects::create(
        &state,
        NewProjectRequest {
            name: "Ready".to_owned(),
            field_kind: "wind".to_owned(),
            resolution: "1.0".to_owned(),
            step_hours: 3,
            step_count: STEPS,
        },
        false,
    )
    .expect("create");
    (root, state)
}

fn document_of(state: &AppState) -> ve_core::project::Project {
    let mut session = state.session.lock().expect("lock");
    session.require_open().expect("open").project.clone()
}

/// The session's current document revision.
fn revision_of(state: &AppState) -> u64 {
    let mut session = state.session.lock().expect("lock");
    session.require_open().expect("open").revision
}

fn newest_object(state: &AppState) -> u64 {
    document_of(state).layers[0]
        .objects
        .last()
        .expect("an object")
        .id
        .raw()
}

fn stamp(state: &AppState, lon: f64, lat: f64) -> u64 {
    edit::paint(
        state,
        edit::BrushStroke {
            points: vec![[lon, lat]],
            size_km: 600.0,
            speed_mps: 12.0,
            direction_toward_deg: 90.0,
            feather: 0.0,
            ..Default::default()
        },
    )
    .expect("paint");
    newest_object(state)
}

/// Makes every step look different: a speed ramp keyed from the first step to
/// the last. Without this a still scene hashes the same at every step, and the
/// steps *share* their tiles — which is a property worth its own test, and a
/// trap for any test that counts entries per step.
fn animate(state: &AppState, object: u64) {
    animation::key_at(
        state,
        object,
        "Speed",
        0,
        Some(PropertyValue::Number { value: 5.0 }),
    )
    .expect("key");
    animation::key_at(
        state,
        object,
        "Speed",
        STEPS - 1,
        Some(PropertyValue::Number { value: 40.0 }),
    )
    .expect("key");
}

/// The content hash of every step.
fn hashes(state: &AppState) -> Vec<SceneHash> {
    let doc = document_of(state);
    (0..STEPS)
        .map(|step| scene_hash(&flatten(&doc, step)))
        .collect()
}

/// Which steps changed between two hash lists.
fn changed(before: &[SceneHash], after: &[SceneHash]) -> Vec<u32> {
    before
        .iter()
        .zip(after)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(step, _)| step as u32)
        .collect()
}

/// Level 0: the whole globe in two tiles. Enough to be a viewport.
fn viewport() -> Vec<TileAddress> {
    vec![
        TileAddress { z: 0, x: 0, y: 0 },
        TileAddress { z: 0, x: 1, y: 0 },
    ]
}

// --- Invalidation is exact ---------------------------------------------------

/// The acceptance criterion, verbatim: editing an object with a 3-step
/// `active_range` invalidates exactly 3 frames.
#[test]
fn editing_an_object_alive_for_three_steps_changes_exactly_three_hashes() {
    let (_root, state) = project("three");
    let id = stamp(&state, 0.0, 0.0);
    document::object_range(&state, id, 4, 6).expect("range");
    let before = hashes(&state);

    document::set_property(&state, id, "Speed", PropertyValue::Number { value: 25.0 })
        .expect("edit");

    assert_eq!(changed(&before, &hashes(&state)), vec![4, 5, 6]);
}

/// An edit that changes nothing a frame looks like changes no hash: renaming
/// is the case, and it is what lets the whole timeline stay ready through it.
#[test]
fn renaming_changes_no_hash() {
    let (_root, state) = project("rename");
    let id = stamp(&state, 0.0, 0.0);
    let before = hashes(&state);

    document::object_rename(&state, id, "Somewhere else".to_owned()).expect("rename");
    document::layer_rename(
        &state,
        document_of(&state).layers[0].id.raw(),
        "Wind".to_owned(),
    )
    .expect("rename layer");

    assert_eq!(changed(&before, &hashes(&state)), Vec::<u32>::new());
}

/// A keyframe edit invalidates the segment it changes and nothing else. With
/// linear keys at 3 and 9, moving the value at 6 changes steps 4..8 — the two
/// segments through it — and leaves 3 and 9 themselves alone.
#[test]
fn a_keyframe_edit_changes_only_the_segments_through_it() {
    let (_root, state) = project("segment");
    let id = stamp(&state, 0.0, 0.0);
    animation::key_at(
        &state,
        id,
        "Speed",
        3,
        Some(PropertyValue::Number { value: 10.0 }),
    )
    .expect("key");
    animation::key_at(
        &state,
        id,
        "Speed",
        9,
        Some(PropertyValue::Number { value: 10.0 }),
    )
    .expect("key");
    let before = hashes(&state);

    animation::key_at(
        &state,
        id,
        "Speed",
        6,
        Some(PropertyValue::Number { value: 30.0 }),
    )
    .expect("key");

    assert_eq!(changed(&before, &hashes(&state)), vec![4, 5, 6, 7, 8]);
}

/// Switching an object off at a step invalidates that step and every one after
/// until it comes back — and, per spec 4.5, every one before its first key.
#[test]
fn disabling_invalidates_the_steps_it_darkens() {
    let (_root, state) = project("disable");
    let id = stamp(&state, 0.0, 0.0);
    animation::key_at(
        &state,
        id,
        "Enabled",
        0,
        Some(PropertyValue::Bool { value: true }),
    )
    .expect("key");
    let before = hashes(&state);

    animation::key_at(
        &state,
        id,
        "Enabled",
        5,
        Some(PropertyValue::Bool { value: false }),
    )
    .expect("key");

    assert_eq!(
        changed(&before, &hashes(&state)),
        (5..STEPS).collect::<Vec<_>>()
    );
}

// --- The pool and the probe honour it ------------------------------------------

/// Rendering ahead makes every step solid, and the probe says so from the cache
/// alone. The two share one notion of a tile's key, so a tile the pool made is
/// a tile the probe sees.
#[test]
fn rendering_ahead_makes_every_step_ready() {
    let (_root, state) = project("ahead");
    stamp(&state, 0.0, 0.0);
    let pool = RenderPool::new();

    let before = pool.readiness(&state, &viewport()).expect("readiness");
    assert!(before.steps.iter().all(|s| s.ready == 0 && s.total == 2));

    pool.request(&state, 0, &viewport()).expect("request");
    pool.drain(&state);

    let after = pool.readiness(&state, &viewport()).expect("readiness");
    assert_eq!(after.steps.len() as u32, STEPS);
    assert!(
        after.steps.iter().all(|s| s.ready == s.total),
        "not every step is solid: {:?}",
        after.steps
    );
}

/// The acceptance property, on the pool: after an edit to an object alive for
/// three steps, exactly those three steps stop being ready and the rest stay —
/// nothing was re-rendered to keep them, because nothing about them changed.
#[test]
fn an_edit_unreadies_exactly_the_steps_it_changed() {
    let (_root, state) = project("unready");
    let id = stamp(&state, 0.0, 0.0);
    document::object_range(&state, id, 4, 6).expect("range");
    let pool = RenderPool::new();
    pool.request(&state, 0, &viewport()).expect("request");
    pool.drain(&state);

    document::set_property(&state, id, "Speed", PropertyValue::Number { value: 25.0 })
        .expect("edit");

    let readiness = pool.readiness(&state, &viewport()).expect("readiness");
    let stale: Vec<u32> = readiness
        .steps
        .iter()
        .filter(|s| s.ready < s.total)
        .map(|s| s.step)
        .collect();
    assert_eq!(stale, vec![4, 5, 6]);
    // The answer names the revision it describes, which is the edit's.
    assert_eq!(readiness.revision, revision_of(&state));
}

/// Steps that look the same share their tiles. Twelve steps of a still scene
/// are one frame rendered once, and an object alive for three of them adds one
/// more — four entries for a viewport of two tiles, not twenty-four. This is
/// the content-addressed cache paying for itself (spec.md 7.10), and the reason
/// the tests below animate their scenes before counting anything per step.
#[test]
fn steps_that_look_the_same_share_their_tiles() {
    let (_root, state) = project("share");
    let id = stamp(&state, 0.0, 0.0);
    document::object_range(&state, id, 4, 6).expect("range");
    let pool = RenderPool::new();
    pool.request(&state, 0, &viewport()).expect("request");
    pool.drain(&state);

    assert_eq!(state.tiles.stats().entries, 4, "two frames, two tiles each");
    let readiness = pool.readiness(&state, &viewport()).expect("readiness");
    assert!(
        readiness.steps.iter().all(|s| s.ready == s.total),
        "and every step is ready"
    );
}

/// After the edit, re-requesting renders only what is missing: the units for
/// the untouched steps are found in the cache and skipped.
#[test]
fn re_requesting_after_an_edit_renders_only_the_changed_steps() {
    let (_root, state) = project("requeue");
    let id = stamp(&state, 0.0, 0.0);
    animate(&state, id);
    let pool = RenderPool::new();
    pool.request(&state, 0, &viewport()).expect("request");
    pool.drain(&state);
    let entries_before = state.tiles.stats().entries;
    assert_eq!(
        entries_before,
        (STEPS * 2) as usize,
        "animated: a frame per step"
    );

    // A key in the middle changes the two segments through it: steps 1..11
    // except the endpoints, so ten steps.
    animation::key_at(
        &state,
        id,
        "Speed",
        6,
        Some(PropertyValue::Number { value: 0.0 }),
    )
    .expect("edit");
    let before = pool.readiness(&state, &viewport()).expect("readiness");
    let unready = before.steps.iter().filter(|s| s.ready < s.total).count();
    assert_eq!(unready, 10);

    pool.request(&state, 0, &viewport()).expect("request");
    pool.drain(&state);

    // Only the unready steps were rendered: ten steps, two tiles each.
    assert_eq!(state.tiles.stats().entries, entries_before + 20);
    let readiness = pool.readiness(&state, &viewport()).expect("readiness");
    assert!(readiness.steps.iter().all(|s| s.ready == s.total));
}

/// Spec 9.5: the current step is rendered first, then its neighbours. Checked
/// by rendering one unit at a time and reading which step became ready.
#[test]
fn the_playhead_is_rendered_first_and_its_neighbours_next() {
    let (_root, state) = project("priority");
    let id = stamp(&state, 0.0, 0.0);
    animate(&state, id);
    let pool = RenderPool::new();
    pool.request(&state, 5, &viewport()).expect("request");

    let mut order = Vec::new();
    while pool.process_next(&state) {
        let readiness = pool.readiness(&state, &viewport()).expect("readiness");
        for s in readiness.steps {
            if s.ready == s.total && !order.contains(&s.step) {
                order.push(s.step);
            }
        }
    }
    assert_eq!(order[0], 5, "the playhead first");
    assert!(
        order[1..3].contains(&4) && order[1..3].contains(&6),
        "then its neighbours: {order:?}"
    );
    assert_eq!(order.len() as u32, STEPS);
}

/// A new request replaces the queue. What was queued for the old playhead is
/// dropped, not appended behind the new work.
#[test]
fn a_new_request_replaces_the_queue() {
    let (_root, state) = project("replace");
    let id = stamp(&state, 0.0, 0.0);
    animate(&state, id);
    let pool = RenderPool::new();
    pool.request(&state, 0, &viewport()).expect("first");
    pool.request(&state, 11, &viewport()).expect("second");

    assert!(pool.process_next(&state));
    let readiness = pool.readiness(&state, &viewport()).expect("readiness");
    let first_ready = readiness.steps.iter().find(|s| s.ready > 0).map(|s| s.step);
    assert_eq!(
        first_ready,
        Some(11),
        "the second request's playhead goes first"
    );
}

/// Acceptance: a time-step switch with a warm cache is fast. The backend's
/// share is a cache read, and it has to be far inside the 50 ms budget to
/// leave room for the fetch, the decode and the upload on the other side.
#[test]
fn a_warm_tile_is_served_in_well_under_the_budget() {
    let (_root, state) = project("warm");
    stamp(&state, 0.0, 0.0);
    let pool = RenderPool::new();
    pool.request(&state, 0, &viewport()).expect("request");
    pool.drain(&state);

    let doc = document_of(&state);
    let scene = flatten(&doc, 3);
    let id = ve_render::tile::TileId::new(0, 0, 0).expect("tile");
    // Once to make sure it is warm, then timed.
    ve_app::protocol::serve(&state, &scene, id).expect("serve");
    let started = std::time::Instant::now();
    for _ in 0..24 {
        ve_app::protocol::serve(&state, &scene, id).expect("serve");
    }
    let per_tile = started.elapsed() / 24;
    println!("warm tile served in {per_tile:?}");
    assert!(
        per_tile < std::time::Duration::from_millis(5),
        "a warm tile took {per_tile:?}; the budget for the whole switch is 50 ms"
    );
}

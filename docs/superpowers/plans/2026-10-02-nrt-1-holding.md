# Near-real-time data, milestone 1: holding — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A fetched layer can say how long each of its times stands for, and a step inside that period shows the field — on the map and in the export — where today it shows nothing.

**Architecture:** The period is provenance on `LayerSource::Zarr` (hours, zero meaning "none"). One new lookup on `RasterSequence` answers "the frame whose period contains this hour", and `Layer::file_frame` chooses between it and today's exact match. Everything downstream — the scene, the tile cache, the export, the timeline's frame row — already asks the layer, so nothing else changes behaviour.

**Tech Stack:** Rust (`ve-core`, `ve-render`, `ve-app`), serde, the existing test suites.

**Spec:** `docs/superpowers/specs/2026-10-02-near-real-time-data-design.md`, §6.2. This is the first of nine milestones (§8); each later one gets its own plan when it is reached, because its code depends on what the ones before it establish.

## Global Constraints

- `spec.md` D48 stays true for every layer without a period: an imported GRIB, a local Zarr and a history layer show a message only on its own step.
- **Past the last frame's period there is nothing.** No hold runs to the end of a timeline.
- Determinism: no `HashMap` iteration in any evaluation or export path.
- No new `f64` reaches the project file (the period is `u32` hours), so no `canonical::*_field` helper is needed.
- A project file written before this change must open unchanged, and a layer with no period must serialise byte for byte as it does today (`skip_serializing_if`).
- No interface text is added: nothing in this milestone is reachable from the interface until milestone 2 fetches a product. Nine-language text is therefore not owed here.
- One commit for the milestone, on a branch, fast-forwarded to `main` (the project's milestone workflow) — not one per task.
- Checks before the commit: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `VE_FORCE_CPU=1 cargo test --workspace`, `npm run ui:typecheck`, `npm run ui:test`, `npm run check:offline`. The disk is nearly full: run cargo with `CARGO_INCREMENTAL=0`, and if a link fails with "No space left on device", `cargo clean -p ve-app` and add `--config 'profile.dev.package.ve-app.debug=false'`.

## Review Focus

1. **A missing day inside a run.** Frames at day 0 and day 2: day 1 must show nothing, not day 0 held on. (Task 1.)
2. **A pasted or hidden frame inside a period** (D59). An override naming a source step serves what the *file* shows at that step, held or not; an override naming nothing hides the step while its neighbours hold. (Task 2.)
3. **A period no longer than the timeline's step.** An hourly product on a three-hourly timeline must behave exactly as the exact match does. (Task 1.)
4. **An older project, and a history layer.** No `period_hours` in the file means no hold; the layer's JSON is unchanged. (Task 2.)
5. **The export.** A held field is written at every step of its period and at none after it. (Task 4.)

---

### Task 1: The lookup — `RasterSequence::frame_within`

**Files:**
- Modify: `crates/ve-core/src/raster.rs` (beside `frame_at`, about line 507; tests in the module's `tests`, beside `a_frame_is_shown_only_at_the_hour_it_is_valid_for`)

**Interfaces:**
- Consumes: `RasterSequence::frames`, `MATCH_TOLERANCE_HOURS`, the test helper `sequence(offsets: &[f64]) -> RasterSequence`.
- Produces: `pub fn frame_within(&self, hour: f64, period_hours: f64) -> Option<&RasterFrame>`.

- [ ] **Step 1: Write the failing tests**

```rust
    /// Three daily fields on an hourly timeline: each stands for its own day,
    /// and the last stands for no longer than that (spec.md 4.10).
    #[test]
    fn a_frame_holds_for_its_own_period_and_no_longer() {
        let s = sequence(&[0.0, 24.0, 48.0]);
        let at = |hour: f64| s.frame_within(hour, 24.0).map(|frame| frame.offset_hours);
        assert_eq!(at(0.0), Some(0.0));
        assert_eq!(at(1.0), Some(0.0));
        assert_eq!(at(23.0), Some(0.0));
        assert_eq!(at(24.0), Some(24.0));
        assert_eq!(at(47.0), Some(24.0));
        assert_eq!(at(71.0), Some(48.0));
        // Past the last day there is nothing, which is the failure D48 was
        // written against and holding must not bring back.
        assert_eq!(at(72.0), None);
        assert_eq!(at(240.0), None);
    }

    /// A day the product did not publish is a day with nothing to show: the
    /// day before does not run on into it.
    #[test]
    fn a_missing_day_is_not_filled_by_the_day_before() {
        let s = sequence(&[0.0, 48.0]);
        let at = |hour: f64| s.frame_within(hour, 24.0).map(|frame| frame.offset_hours);
        assert_eq!(at(23.0), Some(0.0));
        assert_eq!(at(24.0), None);
        assert_eq!(at(47.0), None);
        assert_eq!(at(48.0), Some(48.0));
    }

    /// An hourly product on a three-hourly timeline: a period no longer than
    /// the spacing is the exact match, on every step.
    #[test]
    fn a_period_no_longer_than_the_spacing_is_the_exact_match() {
        let s = sequence(&[0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        for hour in [0.0, 3.0, 6.0, 9.0] {
            assert_eq!(
                s.frame_within(hour, 1.0).map(|frame| frame.offset_hours),
                s.frame_at(hour).map(|frame| frame.offset_hours),
                "hour {hour}"
            );
        }
        // And no period at all is the exact match by definition.
        let sparse = sequence(&[0.0, 3.0]);
        assert_eq!(sparse.frame_within(1.0, 0.0).map(|f| f.offset_hours), None);
        assert_eq!(sparse.frame_within(3.0, 0.0).map(|f| f.offset_hours), Some(3.0));
    }

    /// The boundaries survive the division that makes hours of seconds, and
    /// nothing wider: a second before a boundary is still the earlier field.
    #[test]
    fn a_period_s_edges_survive_the_rounding_of_their_own_arithmetic() {
        let s = sequence(&[0.0, 6.0]);
        let at = |hour: f64| s.frame_within(hour, 6.0).map(|frame| frame.offset_hours);
        assert_eq!(at(6.0 - 1e-9), Some(6.0));
        assert_eq!(at(6.0 + 1e-9), Some(6.0));
        assert_eq!(at(6.0 - 1.0 / 3600.0), Some(0.0));
        assert_eq!(at(12.0 - 1e-9), None, "the end of a period is not in it");
        assert_eq!(at(12.0 - 1.0 / 3600.0), Some(6.0));
    }
```

- [ ] **Step 2: Run them and see them fail**

Run: `CARGO_INCREMENTAL=0 cargo test -p ve-core --lib raster`
Expected: a compile error, `no method named frame_within`.

- [ ] **Step 3: Implement**

Directly after `frame_at`:

```rust
    /// The frame whose own period contains a forecast hour, or `None`.
    ///
    /// For a layer fetched from a product that says how long each of its
    /// times stands for (spec.md 4.10): a daily analysis is the field for its
    /// day, so every step of that day shows it. The newest frame at or before
    /// the hour answers, and only while the hour is inside that frame's own
    /// period — so a day the product did not publish shows nothing, and so
    /// does everything past the last frame's period. That last part is what
    /// keeps [`Self::frame_at`]'s reasoning true here: nothing is held to the
    /// end of a timeline.
    ///
    /// A period of zero or less is no period, and is the exact match.
    pub fn frame_within(&self, hour: f64, period_hours: f64) -> Option<&RasterFrame> {
        if period_hours <= 0.0 {
            return self.frame_at(hour);
        }
        let after = self
            .frames
            .partition_point(|frame| frame.offset_hours <= hour + MATCH_TOLERANCE_HOURS);
        let frame = self.frames.get(after.checked_sub(1)?)?;
        (hour < frame.offset_hours + period_hours - MATCH_TOLERANCE_HOURS).then_some(frame)
    }
```

- [ ] **Step 4: Run them and see them pass**

Run: `CARGO_INCREMENTAL=0 cargo test -p ve-core --lib raster`
Expected: all pass, the four new ones included.

---

### Task 2: The period on a layer, and the layer's own answer

**Files:**
- Modify: `crates/ve-core/src/document.rs` — `LayerSource::Zarr` (about line 675), `impl LayerSource` (about line 688), `Layer::from_history` (about line 946), `Layer::imported_frame` (about line 997), the module's tests
- Modify: `crates/ve-core/src/io.rs` — the test `a_history_layer_keeps_its_archive_and_its_hours` (about line 1095) and a new one beside it

**Interfaces:**
- Consumes: `RasterSequence::frame_within` (Task 1), `RasterSequence::frame_at`, `ProjectSettings::forecast_hour(step) -> u32`, `FrameOverride { step, source }`.
- Produces:
  - `LayerSource::Zarr { …, period_hours: u32 }` — serde default 0, not written when 0.
  - `LayerSource::period_hours(&self) -> Option<u32>` — `Some` only when above zero.
  - `Layer::holding(self, hours: u32) -> Self` — sets the period on a fetched layer; a no-op on any other source.
  - `Layer::file_frame(&self, settings: &ProjectSettings, step: u32) -> Option<&RasterFrame>` — what the file shows at a step, before the user's overrides.
  - `Layer::imported_frame` unchanged in signature, now built on `file_frame`.

- [ ] **Step 1: Write the failing tests**

In `document.rs`'s tests:

```rust
    fn fetched(offsets: &[f64], period_hours: u32) -> Layer {
        use crate::raster::{RasterFrame, RasterGrid, RasterSequence};
        let frames = offsets
            .iter()
            .map(|&h| RasterFrame {
                offset_hours: h,
                valid_unix_s: (h * 3600.0) as i64,
                grid: std::sync::Arc::new(
                    RasterGrid::new(2, 2, 0.0, 1.0, 1.0, 1.0, vec![[h as f32, 0.0]; 4]).unwrap(),
                ),
            })
            .collect();
        let sequence = RasterSequence::new(crate::project::FieldKind::Wind, frames).unwrap();
        Layer::from_history(
            "fetched",
            "fetched.grib2".into(),
            std::sync::Arc::new(sequence),
            "test",
            0,
            0,
        )
        .holding(period_hours)
    }

    fn hourly(steps: u32) -> crate::project::ProjectSettings {
        crate::project::ProjectSettings::new(
            crate::project::FieldKind::Wind,
            crate::project::Resolution::Deg1,
            crate::project::StepHours::H1,
            steps,
        )
    }

    /// A six-hourly product on an hourly timeline holds for its six hours; the
    /// same file with no period is the forecast it always was (D48).
    #[test]
    fn a_fetched_layer_holds_and_a_history_layer_does_not() {
        let settings = hourly(24);
        let shown = |layer: &Layer| -> Vec<Option<f64>> {
            (0..14)
                .map(|s| layer.imported_frame(&settings, s).map(|f| f.offset_hours))
                .collect()
        };
        let held = fetched(&[0.0, 6.0], 6);
        assert_eq!(held.source.period_hours(), Some(6));
        let mut expected = vec![Some(0.0); 6];
        expected.extend(vec![Some(6.0); 6]);
        expected.extend(vec![None; 2]);
        assert_eq!(shown(&held), expected);

        let exact = fetched(&[0.0, 6.0], 0);
        assert_eq!(exact.source.period_hours(), None);
        let mut expected = vec![None; 14];
        expected[0] = Some(0.0);
        expected[6] = Some(6.0);
        assert_eq!(shown(&exact), expected);
    }

    /// An override is resolved against the file (D59), and the file now holds:
    /// a step pasted from inside a period shows that period's field, and a
    /// hidden step is hidden while its neighbours go on holding.
    #[test]
    fn overrides_inside_a_period_read_the_held_file() {
        let settings = hourly(24);
        let mut layer = fetched(&[0.0, 6.0], 6);
        layer.set_frame_overrides(vec![
            FrameOverride { step: 2, source: Some(9) },
            FrameOverride { step: 3, source: None },
            FrameOverride { step: 4, source: Some(20) },
        ]);
        let at = |s: u32| layer.imported_frame(&settings, s).map(|f| f.offset_hours);
        assert_eq!(at(1), Some(0.0));
        assert_eq!(at(2), Some(6.0), "hour 9 is inside the second period");
        assert_eq!(at(3), None, "hidden");
        assert_eq!(at(4), None, "hour 20 is past the last period");
        assert_eq!(at(5), Some(0.0));
        // And what the file itself says is unchanged by any of them.
        assert_eq!(layer.file_frame(&settings, 3).map(|f| f.offset_hours), Some(0.0));
    }

    /// Only a fetched layer has a period to set.
    #[test]
    fn holding_is_a_fetched_layer_s_and_nothing_else_s() {
        let painted = Layer::new("paint").holding(24);
        assert_eq!(painted.source.period_hours(), None);
        assert!(painted.source.is_painted());
    }
```

In `io.rs`, add `period_hours: 0,` to the `LayerSource::Zarr { … }` expected value in `a_history_layer_keeps_its_archive_and_its_hours`, add this assertion to that test after `assert!(json.contains("era5-wind"), …)`:

```rust
        assert!(
            !json.contains("period_hours"),
            "a layer with no period is written exactly as it was before there were periods"
        );
```

and add beside it:

```rust
    /// A fetched layer's period is provenance, so it is in the file; a file
    /// from before there were periods has none and opens as no hold.
    #[test]
    fn a_fetched_layer_keeps_its_period_and_an_older_file_has_none() {
        use crate::document::LayerSource;
        use crate::raster::{RasterFrame, RasterGrid, RasterSequence};
        use std::sync::Arc;

        let grid = RasterGrid::new(2, 2, 0.0, 1.0, 1.0, 1.0, vec![[3.0, 4.0]; 4]).unwrap();
        let sequence = RasterSequence::new(
            FieldKind::Current,
            vec![RasterFrame {
                offset_hours: 0.0,
                valid_unix_s: 1_700_000_000,
                grid: Arc::new(grid),
            }],
        )
        .unwrap();
        let mut project = sample();
        project.layers.push(
            Layer::from_history(
                "DUACS",
                PathBuf::from("/nrt/duacs.grib2"),
                Arc::new(sequence),
                "duacs",
                1_700_000_000,
                1_700_086_400,
            )
            .holding(24),
        );

        let dir = TempDir::new();
        let path = dir.path("held.veproj");
        save(&project, &path).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.layers[1].source.period_hours(), Some(24));

        // The same layer as a version without periods wrote it.
        let older = to_canonical_json(&project)
            .unwrap()
            .replace(",\"period_hours\":24", "")
            .replace("\"period_hours\":24,", "");
        assert!(!older.contains("period_hours"));
        let reopened: Project = serde_json::from_str(&older).unwrap();
        assert_eq!(reopened.layers[1].source.period_hours(), None);
        assert!(matches!(reopened.layers[1].source, LayerSource::Zarr { .. }));
    }
```

- [ ] **Step 2: Run them and see them fail**

Run: `CARGO_INCREMENTAL=0 cargo test -p ve-core --lib -- document:: io::`
Expected: compile errors — `holding`, `period_hours`, `file_frame` do not exist.

- [ ] **Step 3: Implement**

In `LayerSource::Zarr`, after `end_unix_s`:

```rust
        /// How long each fetched time stands for, in hours: 24 for a daily
        /// product, 6 for a six-hourly one (spec.md 4.10).
        ///
        /// Zero is no period, which is what a history layer has and what a
        /// file from before there were periods opens as: the layer then shows
        /// a message only on its own step, as a forecast does (D48). Not
        /// written when zero, so those layers serialise as they always did.
        #[serde(default, skip_serializing_if = "is_zero")]
        period_hours: u32,
```

Above `pub enum LayerSource`:

```rust
/// Whether a count is zero, for fields left out of the file when it is.
fn is_zero(value: &u32) -> bool {
    *value == 0
}
```

In `impl LayerSource`:

```rust
    /// How long each of a fetched layer's times stands for, in hours, if the
    /// layer holds at all (spec.md 4.10).
    pub fn period_hours(&self) -> Option<u32> {
        match self {
            Self::Zarr { period_hours, .. } if *period_hours > 0 => Some(*period_hours),
            _ => None,
        }
    }
```

In `Layer::from_history`, add `period_hours: 0,` to the `LayerSource::Zarr { … }` it builds. After `from_history`:

```rust
    /// The same layer, holding each of its times for `hours` (spec.md 4.10).
    ///
    /// For a layer fetched from a product that says what span each of its
    /// fields is valid for. Only a fetched layer has a period: on any other
    /// source this changes nothing, because a forecast's message is for its
    /// own hour and no other (D48).
    pub fn holding(mut self, hours: u32) -> Self {
        if let LayerSource::Zarr { period_hours, .. } = &mut self.source {
            *period_hours = hours;
        }
        self
    }
```

Replace the body of `imported_frame` and add `file_frame` above it:

```rust
    /// What the file itself shows at a step, before the user's overrides.
    ///
    /// The message valid at the step's own hour (spec.md 4.8, D48) — or, for
    /// a fetched layer with a period, the field whose period contains that
    /// hour (spec.md 4.10). This is the one place that choice is made:
    /// everything that asks what a step shows asks here or asks
    /// [`Self::imported_frame`], which is built on it.
    pub fn file_frame(
        &self,
        settings: &crate::project::ProjectSettings,
        step: u32,
    ) -> Option<&crate::raster::RasterFrame> {
        let sequence = self.raster.as_deref()?;
        let hour = f64::from(settings.forecast_hour(step));
        match self.source.period_hours() {
            Some(period) => sequence.frame_within(hour, f64::from(period)),
            None => sequence.frame_at(hour),
        }
    }
```

and in `imported_frame`:

```rust
        match self.frame_override(step) {
            Some(FrameOverride {
                source: Some(s), ..
            }) => self.file_frame(settings, s),
            Some(FrameOverride { source: None, .. }) => None,
            None => self.file_frame(settings, step),
        }
```

Then fix every other place that names all of `LayerSource::Zarr`'s fields. Find them with `grep -rn "LayerSource::Zarr {" crates`; a pattern ending in `..` needs nothing.

- [ ] **Step 4: Run them and see them pass**

Run: `CARGO_INCREMENTAL=0 cargo test -p ve-core`
Expected: all pass. `hostile_floats_survive_a_round_trip` and the golden-file tests are unchanged, since a layer with no period writes no new key.

---

### Task 3: The scene and the timeline's frame row follow

**Files:**
- Modify: `crates/ve-render/tests/evaluation.rs` — a new test after `a_step_shows_only_the_message_valid_at_its_own_hour` (about line 990)
- Modify: `crates/ve-app/src/document.rs:513-530` — `covered_steps` and `GribStepView::in_file`
- Modify: `crates/ve-app/tests/grib_import.rs` — a new test

**Interfaces:**
- Consumes: `Layer::holding`, `Layer::file_frame` (Task 2); `ve_render::scene::flatten`, `ve_render::cpu::sample_scene`, `ve_render::cache::scene_hash`; `ve_app::document::tree`.
- Produces: nothing new. `scene.rs` already asks `Layer::imported_frame`, so the scene needs a test and no change; the two view fields move from `RasterSequence::frame_at` to `Layer::file_frame`.

- [ ] **Step 1: Write the failing tests**

In `evaluation.rs`:

```rust
/// A daily product on an hourly timeline (spec.md 4.10): every step of the
/// day shows the day's field, the steps of one day flatten to the *same*
/// scene — so they share tiles — and past the last day there is nothing.
#[test]
fn a_held_field_is_on_every_step_of_its_period() {
    let frames = [0.0_f64, 24.0]
        .iter()
        .map(|&h| RasterFrame {
            offset_hours: h,
            valid_unix_s: (h * 3600.0) as i64,
            grid: atlantic(5.0 + h as f32, 0.0),
        })
        .collect();
    let sequence = Arc::new(RasterSequence::new(FieldKind::Wind, frames).unwrap());
    let mut project = Project::new(
        "daily on hourly",
        ProjectSettings::new(FieldKind::Wind, Resolution::Deg1, StepHours::H1, 60),
    );
    project.layers[0] = ve_core::document::Layer::from_history(
        "daily",
        "daily.grib2".into(),
        sequence,
        "test",
        0,
        0,
    )
    .holding(24);

    let at = ll(-30.0, 45.0);
    let u_at = |step: u32| sample_scene(&flatten(&project, step), at).u;
    assert_eq!(u_at(0), 5.0);
    assert_eq!(u_at(13), 5.0);
    assert_eq!(u_at(23), 5.0);
    assert_eq!(u_at(24), 29.0);
    assert_eq!(u_at(47), 29.0);
    assert_eq!(u_at(48), 0.0, "past the last day there is no field");

    let hash = |step: u32| ve_render::cache::scene_hash(&flatten(&project, step));
    assert_eq!(hash(0), hash(23), "one day is one scene, and one set of tiles");
    assert_ne!(hash(23), hash(24));
}
```

In `grib_import.rs`, using that file's own `TempRoot` and `app` helpers and its project-creation pattern (3-hourly steps):

```rust
/// The timeline's frame row says what the map shows: a held field covers
/// every step of its period (spec.md 4.10).
#[test]
fn a_held_layer_covers_every_step_of_its_period() {
    use std::sync::Arc;
    use ve_core::document::Layer;
    use ve_core::project::FieldKind;
    use ve_core::raster::{RasterFrame, RasterGrid, RasterSequence};

    let root = TempRoot::new("held-row");
    let state = app(&root);
    projects::create(
        &state,
        NewProjectRequest {
            name: "Held".to_owned(),
            field_kind: "wind".to_owned(),
            resolution: "1.0".to_owned(),
            step_hours: 3,
            step_count: 6,
        },
        true,
    )
    .expect("create");

    let frame = |h: f64| RasterFrame {
        offset_hours: h,
        valid_unix_s: (h * 3600.0) as i64,
        grid: Arc::new(RasterGrid::new(2, 2, 0.0, 1.0, 1.0, 1.0, vec![[4.0, 0.0]; 4]).unwrap()),
    };
    let sequence =
        Arc::new(RasterSequence::new(FieldKind::Wind, vec![frame(0.0), frame(6.0)]).unwrap());
    let layer = |period: u32| {
        Layer::from_history("six-hourly", "x.grib2".into(), Arc::clone(&sequence), "test", 0, 0)
            .holding(period)
    };
    {
        let mut session = state.session.lock().expect("lock");
        let open = session.require_open().expect("open");
        open.project.layers.push(layer(6));
        open.project.layers.push(layer(0));
    }

    let tree = document::tree(&state, 0).expect("tree");
    let covered = |at: usize| tree.layers[at].grib.as_ref().expect("a field layer").covered_steps.clone();
    // Steps at 0, 3, 6, 9, 12, 15 h. Held: 0-5 h and 6-11 h. Not held: 0 and 6.
    assert_eq!(covered(1), [true, true, true, true, false, false]);
    assert_eq!(covered(2), [true, false, true, false, false, false]);
    let held = tree.layers[1].grib.as_ref().expect("a field layer");
    assert!(held.steps[1].in_file && held.steps[1].shown);
    assert!(!held.steps[4].in_file && !held.steps[4].shown);
}
```

- [ ] **Step 2: Run them**

Run: `CARGO_INCREMENTAL=0 cargo test -p ve-render --test evaluation a_held_field` — expected **PASS** already: the scene asks `imported_frame`, which Task 2 changed. This test is the proof that it does, and that the hash follows.

Run: `CARGO_INCREMENTAL=0 VE_FORCE_CPU=1 cargo test -p ve-app --test grib_import a_held_layer` — expected **FAIL**: `covered(1)` is `[true, false, true, false, false, false]`, because the row still asks the sequence for an exact match.

- [ ] **Step 3: Implement**

In `crates/ve-app/src/document.rs`, both closures ask the layer:

```rust
                        covered_steps: (0..project.settings.step_count)
                            .map(|s| layer.file_frame(&project.settings, s).is_some())
                            .collect(),
                        steps: (0..project.settings.step_count)
                            .map(|s| {
                                let over = layer.frame_override(s);
                                GribStepView {
                                    in_file: layer.file_frame(&project.settings, s).is_some(),
                                    source: over.and_then(|o| o.source),
                                    hidden: over.is_some_and(|o| o.source.is_none()),
                                    shown: layer.imported_frame(&project.settings, s).is_some(),
                                }
                            })
                            .collect(),
```

- [ ] **Step 4: Run them and see them pass**

Run: `CARGO_INCREMENTAL=0 VE_FORCE_CPU=1 cargo test -p ve-app --test grib_import --test zarr_import`
Expected: all pass, the existing `covered_steps` assertions unchanged.

---

### Task 4: The export carries the held field, and the documents say so

**Files:**
- Modify: `crates/ve-app/tests/export.rs` — a new test
- Modify: `spec.md` §4.8 (after the paragraph on a step the file has no message for) and §4.10
- Modify: `plan.md` — a milestone entry above `### M87`, and a row in the decisions log after D72
- Modify: `CLAUDE.md` — the paragraph "A step the file has no message for has no raster" in *Touching the raster sampler*

**Interfaces:**
- Consumes: `Layer::holding`; that file's `TempRoot`, `app`, `new_project` (3-hourly, 2 steps), `request`, `messages`; `export::run`.
- Produces: nothing new.

- [ ] **Step 1: Write the test**

```rust
/// A held field is exported on every step of its period (spec.md 4.10): a
/// six-hourly wind on this three-hourly project is in both steps' messages,
/// and the same layer with no period is in the first step's alone.
#[test]
fn a_held_field_is_exported_on_every_step_of_its_period() {
    use std::sync::Arc;
    use ve_core::document::Layer;
    use ve_core::project::FieldKind;
    use ve_core::raster::{RasterFrame, RasterGrid, RasterSequence};

    let root = TempRoot::new("held-export");
    let state = app(&root);
    projects::create(&state, new_project("wind"), false).expect("create");
    let base = {
        let mut session = state.session.lock().expect("lock");
        session.require_open().expect("open").project.clone()
    };

    // 7 m/s eastward over a patch of the Atlantic, valid at hour 0.
    let grid = RasterGrid::new(41, 31, -60.0, 60.0, 1.0, 1.0, vec![[7.0, 0.0]; 41 * 31]).unwrap();
    let sequence = Arc::new(
        RasterSequence::new(
            FieldKind::Wind,
            vec![RasterFrame {
                offset_hours: 0.0,
                valid_unix_s: 0,
                grid: Arc::new(grid),
            }],
        )
        .unwrap(),
    );
    // The strongest eastward wind in each step's u message.
    let strongest_u_by_hour = |period: u32, name: &str| -> Vec<(u32, f32)> {
        let mut project = base.clone();
        project.layers.push(
            Layer::from_history("fetched", "x.grib2".into(), Arc::clone(&sequence), "test", 0, 0)
                .holding(period),
        );
        let path = root.0.join(name);
        export::run(&project, &request(&path), &AtomicBool::new(false), |_| {}).expect("export");
        messages(&std::fs::read(&path).expect("read"))
            .iter()
            .filter(|m| m.discipline == 0 && m.category == 2 && m.number == 2)
            .map(|m| {
                let strongest = m
                    .values
                    .iter()
                    .copied()
                    .filter(|v| v.is_finite() && v.abs() < 1.0e6)
                    .fold(0.0_f32, f32::max);
                (m.forecast_hour, strongest)
            })
            .collect()
    };

    let held = strongest_u_by_hour(6, "held.grib2");
    assert_eq!(held.len(), 2, "{held:?}");
    for (hour, strongest) in &held {
        assert!((strongest - 7.0).abs() < 0.01, "hour {hour}: {strongest}");
    }

    let exact = strongest_u_by_hour(0, "exact.grib2");
    let at = |hour: u32| exact.iter().find(|(h, _)| *h == hour).map_or(0.0, |(_, s)| *s);
    assert!((at(0) - 7.0).abs() < 0.01, "{exact:?}");
    assert_eq!(at(3), 0.0, "with no period the field is on its own step only: {exact:?}");
}
```

(How this file's other tests tell a missing value from a real one is in its own `strongest` closure, about line 229; if a missing value is not what the filter above assumes, use that closure's rule.)

- [ ] **Step 2: Run it**

Run: `CARGO_INCREMENTAL=0 VE_FORCE_CPU=1 cargo test -p ve-app --test export a_held_field`
Expected: **PASS** — the export flattens each step through the same `imported_frame`. If it fails, the export has a path to a layer's field that does not go through the layer, and that path is the bug to fix here.

- [ ] **Step 3: Write the documents**

`spec.md` §4.8, after the paragraph that states a step the file has no message for shows no imported field:

> **The one exception is a layer that says how long each of its times stands
> for** (§4.10, D73). A layer fetched from a daily or six-hourly product
> carries that period, and a step inside a field's period shows the field.
> It is the product's own statement of validity, not a hold: a day the
> product did not publish shows nothing, and so does every step past the
> last field's period. An imported file and a history layer have no period
> and keep the rule above exactly.

`spec.md` §4.10, a new paragraph before "Times are UTC and land on the hour":

> **A fetched layer may hold each time for its product's period.** The
> layer's provenance carries the period in hours — zero, for the two
> archives here, which are hourly — and the timeline's frame row, the map
> and the export all show a field on every step of its period. Steps that
> show the same field flatten to the same scene and share its tiles. The
> exported file therefore carries a daily field once per step of its day:
> that is what the period means, and a reader of the GRIB sees the same
> thing the map showed.

`plan.md`, decisions log, after D72:

> | D73 | A fetched layer holds each of its times for the product's own period — an exception to D48, not a reversal | D48 refused to hold a forecast message forward because a message is a measurement for one hour, and holding it drew data for times it was never made for, worst past a short file's end. A daily analysis is different in kind: the product itself says it is the field for that day. So the period is provenance on the fetched layer, the lookup answers only inside a field's own period, a missing day stays empty, and nothing runs past the last period — every failure D48 named stays impossible. Imported files and history layers have no period and are untouched. The export carries the held field on each step, since it must agree with the map (invariant 3). Settled with the user 2026-10-02 (M88) |

`plan.md`, a milestone entry above `### M87`:

> ### M88 — A fetched layer holds for its product's period
>
> The first milestone of the near-real-time import
> (`docs/superpowers/specs/2026-10-02-near-real-time-data-design.md`), and
> the only one with nothing to fetch: six of the ten products are daily and
> two are six-hourly, and on an hourly timeline D48 would show each on one
> step in twenty-four. `LayerSource::Zarr` gains `period_hours`,
> `RasterSequence::frame_within` answers inside a field's own period, and
> `Layer::file_frame` is the one place that chooses between it and the exact
> match — the scene, the cache, the export and the timeline's frame row
> already ask the layer. Nothing is reachable from the interface yet; the
> next milestone adds the button. D73.

`CLAUDE.md`, replace the paragraph beginning "**A step the file has no message for has no raster**" with:

> **A step the file has no message for has no raster**, not the previous one:
> `Layer::file_frame` returns an `Option` and `flatten` pushes nothing
> when it is `None` (spec §4.8, D48). A message is a measurement and does not
> hold the way a keyframe does. **The one exception is a fetched layer with a
> period** (spec §4.10, D73): `LayerSource::Zarr::period_hours` says how long
> each time stands for, and `RasterSequence::frame_within` answers inside
> that period and nowhere past it. Ask the layer (`file_frame`,
> `imported_frame`), never the sequence: a caller that reaches for
> `frame_at` itself shows a held layer as empty.

- [ ] **Step 4: Run every check**

```bash
cargo fmt --all --check
CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets -- -D warnings
CARGO_INCREMENTAL=0 VE_FORCE_CPU=1 cargo test --workspace
npm run ui:typecheck && npm run ui:test && npm run check:offline
```

Expected: all clean. No binding changes: `LayerSource` does not cross IPC, so `npm run bindings` should produce no diff — run it and confirm.

- [ ] **Step 5: Commit, and fast-forward `main`**

```bash
git switch -c m88-holding
git add -A
git commit -m "A fetched layer holds each time for its product's period (M88, D73)"
git switch main && git merge --ff-only m88-holding && git branch -d m88-holding
```

# Regional Projects Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A project can be Global or Regional. A regional project has a lat/lon rectangle that may cross the antimeridian or reach a pole. The map is bounded by it, and every import, fetch and export is clipped to it in what is held, written and exported.

**Architecture:** `ve-core` gains a `Region` on `ProjectSettings`. `ProjectSettings::lattice()` replaces every `Resolution::target_grid()` caller, so one regional `TargetGrid` flows to the decoders (which crop), the ICON regrid (which builds smaller neighbour sets), the fetch writer (which writes a regional GRIB) and the exporters. The frontend reads the region from `ProjectSummary`, carries it on the `Camera` as it carries the projection, and `clampCamera` enforces it.

**Tech Stack:** Rust (ve-core, ve-grib, ve-zarr, ve-app), ts-rs bindings, React + TypeScript, vitest.

**Spec:** `docs/superpowers/specs/2026-10-05-regional-projects-design.md` (decisions R1–R10). Read it first, together with `CLAUDE.md` and `spec.md` §4.1, §4.2, §4.8, §4.10, §5.1, §12.

## Global Constraints

- Longitude is `[-180, 180)` everywhere. Only `ve-grib` converts to `[0, 360)` (CLAUDE.md Conventions).
- A global project's export is byte-identical to today. `the_encoding_is_identical_on_every_platform` (FNV `0x769b_cfb2_fb07_77c3`, 130,530 bytes) must not change.
- `ProjectSettings.region` is `#[serde(default, skip_serializing_if = "Option::is_none")]`. `None` is global. Old projects open unchanged, so there is **no** `SCHEMA_VERSION` bump.
- The region is immutable after creation (R1). No command may change it.
- No `HashMap` iteration in evaluation or export paths.
- Geodesy and lattice changes need **antimeridian and polar** test cases, both (CLAUDE.md Testing rules).
- Every new string goes through `t()`, in all nine languages. Every new control gets `data-feature` and a Help search entry (CLAUDE.md *Adding interface text*).
- Any IPC-facing type is declared in `ve-app` with `ts-rs` (ve-core has none). Run `npm run bindings` after changing one.
- No new network host. ERDDAP and ARCO subsetting go to the hosts already named in `tools/check-offline.sh`.
- `plan.md` gets a milestone entry and `spec.md` is updated in the **same commit** as each behaviour change (milestone workflow). Number them M100 onwards.
- Before declaring each task done: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `VE_FORCE_CPU=1 cargo test --workspace`, `npm run ui:typecheck`, `npm run ui:test`, `npm run check:offline`.

## Review Focus

1. **A region crossing 180° that is opened, panned and exported.** Expect the map to pan smoothly across the seam within the region, and the exported GRIB to show the field on both sides in wgrib2 (Lo1 > 180 in GRIB terms). The tests that pin it are in Tasks 1, 2, 4 and 8.
2. **An Arctic cap (span 360°, 60°N to 90°N) under equirectangular, the globe and polar stereographic.** Expect free longitude wrap, no zooming out past 60°N, and the pole row exported once with `Ni = 360/d`. Pinned in Tasks 1, 4 and 8.
3. **A 0.1° global GRIB imported into a small region.** Expect it to finish quickly and hold about region-sized memory, not 6.5 M nodes. Pinned in Task 3 (node-count assertion).
4. **A file that does not touch the region.** Expect a refusal that names the file, not a silent empty layer. Pinned in Task 3.
5. **Reopening a regional project.** `attach_rasters` must crop identically on every open (same hash), and stored ICON neighbour sets must be reused rather than rebuilt. Pinned in Task 3 (open twice, same `RasterSequence::hash`, `produced` empty the second time).

---

## File map

| File | Responsibility |
|---|---|
| `crates/ve-core/src/region.rs` (new) | `Region`, snapping, validation, `lattice()`, `contains()` |
| `crates/ve-core/src/project.rs` | `ProjectSettings.region`, `ProjectSettings::lattice()` |
| `crates/ve-core/src/raster.rs` | `RasterGrid::cropped_to(&TargetGrid, margin)` |
| `crates/ve-core/src/io.rs` | `read_regrid` decodes against `settings.lattice()` |
| `crates/ve-grib/src/import.rs` | lat/lon path crops; projected window clipped to a regional target |
| `crates/ve-grib/src/resample.rs` | `Coverage::window` clamps `i0` for a non-wrapping target |
| `crates/ve-grib/src/writer.rs`, `reader.rs` | `GridSpec` first point; section 3 |
| `crates/ve-zarr/src/source.rs` | `Window` (index ranges on the 1440×721 grid), `Field::cropped` |
| `crates/ve-zarr/src/arco.rs`, `erddap.rs`, `regrid.rs` | server-side windows |
| `crates/ve-app/src/history.rs`, `nrt.rs` | regional `GridSpec`; file name carries region key |
| `crates/ve-app/src/import.rs`, `zarr.rs` | lattice from settings; refusal; seeding from a file |
| `crates/ve-app/src/export.rs` | regional GRIB grid; Zarr evaluates only region nodes |
| `crates/ve-app/src/projects.rs` | `RegionRequest` in `NewProjectRequest`; `ProjectRegion` in `ProjectSummary` |
| `crates/ve-app/src/mcp/tools/project.rs` | `project_new` region param; status reports it |
| `ui/src/map/extent.ts` (new) | region bounds maths for the camera and overlay |
| `ui/src/map/camera.ts` | `Camera.region`; `minPxPerDeg` and `clampCamera` honour it |
| `ui/src/map/MapView.tsx` | region onto camera; dim exterior; refuse outside gestures; tile cull |
| `ui/src/project/RegionPicker.tsx` (new) | drag/numeric/cap picker |
| `ui/src/project/NewProjectForm.tsx`, `format.ts` | Extent choice; regional estimate |
| `ui/src/i18n/locales/*/project.ts`, `ui/src/help/*` | strings, help page, features |

---

### Task 1: `Region` in the document (M100, part 1)

**Files:**
- Create: `crates/ve-core/src/region.rs`
- Modify: `crates/ve-core/src/lib.rs` (add `pub mod region;`), `crates/ve-core/src/project.rs` (`ProjectSettings`), `crates/ve-core/src/error.rs` (new variant), `crates/ve-core/src/io.rs` (round-trip test)
- Test: in-module tests in `region.rs` and `io.rs`

**Interfaces:**
- Produces:
  - `pub struct Region { pub west_udeg: i32, pub span_udeg: i32, pub north_udeg: i32, pub south_udeg: i32 }` (serde, `Copy`, `Eq`)
  - `Region::snapped(west: f64, east: f64, south: f64, north: f64, full_circle: bool, res: Resolution) -> Result<Region, CoreError>`. `east` is read as the eastward arc from `west`. Snaps **outward** to the lattice.
  - `Region::validate(&self, res: Resolution) -> Result<(), CoreError>`
  - `Region::is_full_circle(&self) -> bool`
  - `Region::lattice(&self, res: Resolution) -> TargetGrid`
  - `Region::contains(&self, lon: f64, lat: f64) -> bool`
  - `Region::bounds_deg(&self) -> (f64, f64, f64, f64)` → `(west, east_unwrapped, south, north)`
  - `Region::key(&self) -> String` (e.g. `"r160000000_40000000_70000000_50000000"`), for file names
  - `ProjectSettings.region: Option<Region>`
  - `ProjectSettings::lattice(&self) -> TargetGrid` (global ⇒ `resolution.target_grid()`)
  - `CoreError::InvalidRegion { reason: String }`

- [ ] **Step 1: Write the failing tests** in `region.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Resolution;

    const Q: Resolution = Resolution::Deg025;

    #[test]
    fn a_box_snaps_outward_to_the_lattice() {
        let r = Region::snapped(10.1, 19.9, 40.1, 49.9, false, Q).unwrap();
        assert_eq!(r.west_udeg, 10_000_000);
        assert_eq!(r.span_udeg, 10_000_000);
        assert_eq!((r.south_udeg, r.north_udeg), (40_000_000, 50_000_000));
        let g = r.lattice(Q);
        assert_eq!((g.ni, g.nj), (41, 41));
        assert_eq!((g.lon0, g.lat0), (10.0, 50.0));
    }

    #[test]
    fn a_box_across_the_antimeridian_runs_east_past_180() {
        // 160°E to 160°W: east is smaller than west, and the arc is 40°.
        let r = Region::snapped(160.0, -160.0, -10.0, 10.0, false, Q).unwrap();
        assert_eq!(r.span_udeg, 40_000_000);
        let g = r.lattice(Q);
        assert_eq!(g.ni, 161);
        assert_eq!(g.lon0, 160.0);
        assert!(r.contains(179.9, 0.0));
        assert!(r.contains(-179.9, 0.0));
        assert!(r.contains(-160.0, 0.0));
        assert!(!r.contains(-159.0, 0.0));
        assert!(!r.contains(0.0, 0.0));
    }

    #[test]
    fn an_arctic_cap_wraps_and_holds_the_pole_row_once() {
        let r = Region::snapped(0.0, 0.0, 60.0, 90.0, true, Q).unwrap();
        assert!(r.is_full_circle());
        assert_eq!(r.west_udeg, -180_000_000);
        let g = r.lattice(Q);
        assert_eq!(g.ni, 1440); // no duplicated column, as the global grid
        assert_eq!(g.nj, 121);
        assert_eq!(g.lat0, 90.0);
        assert!(r.contains(123.0, 89.99));
        assert!(r.contains(-180.0, 60.0));
        assert!(!r.contains(0.0, 59.0));
    }

    #[test]
    fn an_antarctic_cap_reaches_the_south_pole() {
        let r = Region::snapped(0.0, 0.0, -90.0, -60.0, true, Q).unwrap();
        let g = r.lattice(Q);
        assert_eq!(g.nj, 121);
        assert_eq!(g.lat0 - f64::from(g.nj - 1) * g.dlat, -90.0);
    }

    #[test]
    fn a_wedge_that_touches_the_pole_is_an_ordinary_rectangle() {
        let r = Region::snapped(-30.0, 30.0, 70.0, 90.0, false, Q).unwrap();
        assert!(!r.is_full_circle());
        assert_eq!(r.lattice(Q).ni, 241);
        assert!(r.contains(0.0, 90.0));
    }

    #[test]
    fn the_whole_earth_is_global_not_a_region() {
        assert!(Region::snapped(0.0, 0.0, -90.0, 90.0, true, Q).is_err());
    }

    #[test]
    fn degenerate_and_inverted_boxes_are_refused() {
        assert!(Region::snapped(10.0, 10.0, 40.0, 50.0, false, Q).is_err()); // zero span
        assert!(Region::snapped(10.0, 20.0, 50.0, 40.0, false, Q).is_err()); // south > north
        assert!(Region::snapped(10.0, 20.0, 40.0, 95.0, false, Q).is_err()); // past the pole
    }

    #[test]
    fn a_region_off_the_lattice_does_not_validate() {
        let r = Region { west_udeg: 10_100_000, span_udeg: 10_000_000, north_udeg: 50_000_000, south_udeg: 40_000_000 };
        assert!(r.validate(Q).is_err());
        assert!(r.validate(Resolution::Deg01).is_ok());
    }
}
```

- [ ] **Step 2: Run them and check they fail**

Run: `cargo test -p ve-core region::`
Expected: FAIL to compile, `Region` not found.

- [ ] **Step 3: Implement `region.rs`.** Rules:
  - `snapped`:
    1. Normalise `west` with `geo::normalize_lon`.
    2. If not `full_circle`, set `span = (east - west).rem_euclid(360)`. A zero span is refused.
    3. Floor `west` to a multiple of `res.micro_degrees()`, and ceil `west + span` likewise.
    4. Floor `south` and ceil `north`, then clamp both to ±90e6.
    5. If `full_circle`, set `west = -180e6` and `span = 360e6`.
    6. Refuse `south >= north`, and refuse full circle from −90 to 90 with the reason `"the whole earth is a global project"`.
  - `lattice`:
    - `ni = span/d` when full circle, else `span/d + 1`.
    - `nj = (north-south)/d + 1`.
    - `lon0 = west`, `lat0 = north`, `dlon = dlat = res.degrees()`.
  - `contains`:
    - `(lon - west).rem_euclid(360) <= span + 1e-9`, or always true when full circle.
    - and `south - 1e-9 <= lat <= north + 1e-9`.
  - Doc comments say *why* integers (R2) and why span, not east.

- [ ] **Step 4: Add the setting.** In `ProjectSettings`:

```rust
    /// The part of the earth the project covers; `None` is the whole of it
    /// (spec.md 4.2). Immutable after creation, like the resolution: every
    /// import is cropped to it and every export written on it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<crate::region::Region>,
```

  Set `region: None` in `ProjectSettings::new`. Add:

```rust
    /// The lattice this project exports on and crops every import to.
    pub fn lattice(&self) -> crate::regrid::TargetGrid {
        match self.region {
            Some(region) => region.lattice(self.resolution),
            None => self.resolution.target_grid(),
        }
    }
```

  In `Project::validate`, call `region.validate(self.settings.resolution)` when it is set.

- [ ] **Step 5: Add serde tests in `io.rs`:**
  - `a_regional_project_round_trips`: save and load a project whose region crosses 180°, then assert the settings are equal.
  - `a_project_without_a_region_is_global`: hand-built JSON without the key → `region == None` → `lattice() == resolution.target_grid()`.
  - Extend `the_archive_holds_no_unexpected_entries` to use a regional project. There must be no new entries.

- [ ] **Step 6: Run** `cargo test -p ve-core` and expect PASS. Then run the full check list from Global Constraints.

- [ ] **Step 7: Commit** with `spec.md` §4.1 (add the `region` row: *Immutable: Yes*) and §4.2 (*Regional projects*: the lattice is the region's nodes, inclusive; a full-circle region wraps like the global grid). Add a `plan.md` entry, *M100 — A project may cover a region*, marked part 1.

```bash
git add crates/ve-core spec.md plan.md
git commit -m "ve-core: a project may cover a region (M100)"
```

---

### Task 2: Cropping a lattice (M100, part 2)

**Files:**
- Modify: `crates/ve-core/src/raster.rs`
- Test: in-module tests

**Interfaces:**
- Consumes: `TargetGrid` (from Task 1's `lattice()`).
- Produces: `RasterGrid::cropped_to(&self, target: &TargetGrid, margin: u32) -> Option<RasterGrid>`.
  - Returns `None` when no node of `self` lies within the target, grown by `margin` nodes.
  - The result is a new `RasterGrid::new(...)`, so it gets its own hash and `wraps`.
  - When `self` wraps and the window crosses `self`'s seam, the columns are copied modulo `self.ni`, so the crop is contiguous.

The margin is **1** at every call site. The sampler blends four corners, and without the extra node the outermost region row would read missing.

- [ ] **Step 1: Write the failing tests.** Use the existing `grid(ni, nj, lon0, lat0, step, f)` helper. Give it values that encode their position, e.g. `|lon, lat| [lon as f32, lat as f32]`, so every assertion checks the right node rather than a count.

```rust
    #[test]
    fn a_crop_keeps_the_region_and_one_node_round_it() {
        let world = grid(360, 181, -180.0, 90.0, 1.0, |lon, lat| [lon as f32, lat as f32]);
        let target = TargetGrid { ni: 11, nj: 11, lon0: 10.0, lat0: 50.0, dlon: 1.0, dlat: 1.0 };
        let c = world.cropped_to(&target, 1).unwrap();
        assert_eq!((c.ni, c.nj), (13, 13));
        assert_eq!((c.lon0, c.lat0), (9.0, 51.0));
        assert!(!c.wraps);
        assert_eq!(c.sample(15.0, 45.0), world.sample(15.0, 45.0));
    }

    #[test]
    fn a_crop_across_the_antimeridian_is_contiguous() {
        let world = grid(360, 181, -180.0, 90.0, 1.0, |lon, lat| [lon as f32, lat as f32]);
        let target = TargetGrid { ni: 41, nj: 21, lon0: 160.0, lat0: 10.0, dlon: 1.0, dlat: 1.0 };
        let c = world.cropped_to(&target, 1).unwrap();
        assert_eq!(c.ni, 43);
        assert_eq!(c.lon0, 159.0);
        for lon in [170.0, 179.5, -179.5, -170.0] {
            assert_eq!(c.sample(lon, 0.0), world.sample(lon, 0.0), "at {lon}");
        }
        assert!(c.sample(0.0, 0.0).is_none());
    }

    #[test]
    fn a_crop_to_a_polar_cap_keeps_the_wrap() {
        let world = grid(360, 181, -180.0, 90.0, 1.0, |lon, lat| [lon as f32, lat as f32]);
        let target = TargetGrid { ni: 360, nj: 31, lon0: -180.0, lat0: 90.0, dlon: 1.0, dlat: 1.0 };
        let c = world.cropped_to(&target, 1).unwrap();
        assert!(c.wraps);
        assert_eq!((c.ni, c.nj), (360, 32)); // margin only southward: the pole is the top
        assert_eq!(c.sample(-179.5, 75.0), world.sample(-179.5, 75.0));
    }

    #[test]
    fn a_crop_that_misses_is_none() {
        let patch = grid(20, 10, -10.0, 60.0, 1.0, |_, _| [1.0, 1.0]);
        let target = TargetGrid { ni: 11, nj: 11, lon0: 100.0, lat0: 0.0, dlon: 1.0, dlat: 1.0 };
        assert!(patch.cropped_to(&target, 1).is_none());
    }

    #[test]
    fn a_crop_of_a_regional_grid_takes_the_overlap() {
        let patch = grid(21, 11, 0.0, 50.0, 1.0, |lon, lat| [lon as f32, lat as f32]);
        let target = TargetGrid { ni: 41, nj: 41, lon0: 10.0, lat0: 60.0, dlon: 1.0, dlat: 1.0 };
        let c = patch.cropped_to(&target, 1).unwrap();
        assert_eq!((c.lon0, c.ni), (9.0, 12));
        assert_eq!(c.lat0, 50.0);
    }
```

- [ ] **Step 2: Run** `cargo test -p ve-core raster::` and expect a compile failure.
- [ ] **Step 3: Implement.**
  1. Compute the target's west and east edges, unwrapped, grown by `margin * target.dlon`, plus its north and south edges likewise.
  2. Map them to `self`'s column indices with `((lon - self.lon0).rem_euclid(360) / self.dlon)`. Floor the start and ceil the end.
  3. If `self.wraps`, take `count = min(end - start + 1, self.ni)` and read column `(start + k) % ni`.
  4. If it does not wrap, intersect with `0..self.ni` and return `None` when the result is empty.
  5. Rows: intersect `[(self.lat0 - north)/dlat, (self.lat0 - south)/dlat]` with `0..nj`.
  6. Build with `RasterGrid::new`.

  `wraps` is derived again by `new`, which is why a full-circle crop keeps it.
- [ ] **Step 4: Run** `cargo test -p ve-core` and expect PASS. The sampler is unchanged, so the fidelity suite does not need a new case. Still run `cargo test -p ve-render --test fidelity` to confirm a cropped grid samples identically on the GPU (add one cropped-across-180 raster scene to its generator).
- [ ] **Step 5: Commit** `ve-core: crop a lattice to a region (M100)`.

---

### Task 3: Imports read the region (M101)

**Files:**
- Modify:
  - `crates/ve-grib/src/import.rs` (`raster_of` lat/lon path, and `Resampling` gains `crop: bool`)
  - `crates/ve-grib/src/resample.rs` (`Coverage::window` clamps `i0` and `ni` for a non-wrapping target)
  - `crates/ve-app/src/import.rs` (every `resolution.target_grid()` → `settings.lattice()`; refusal)
  - `crates/ve-app/src/zarr.rs` (`read_store` crops)
  - `crates/ve-core/src/io.rs` (`read_regrid` uses `settings.lattice()`)
- Test: `crates/ve-grib/tests/` (new `regional_import.rs`), `crates/ve-app/tests/import.rs` (or the existing import test file)

**Interfaces:**
- Consumes: `ProjectSettings::lattice()`, `RasterGrid::cropped_to`.
- Produces:
  - `Resampling { target, neighbours, produced, crop: bool }`. When `crop` is set, the lat/lon path calls `cropped_to(&target, 1)` and the projected path clips its window to the target. `Resampling::new` keeps `crop: false`; `Resampling::regional(target, neighbours)` sets it.
  - `AppError` text: `"{file} covers none of this project's region ({west}°…{east}°, {south}°…{north}°)"`.

Global projects pass `crop: false`, so their lattices are untouched and their hashes, and so the render cache, do not move.

- [ ] **Step 1: Failing tests, `ve-grib/tests/regional_import.rs`:**
  - `a_global_grib_in_a_small_region_holds_the_region`: write a 0.25° global field with `ve_grib::writer` (u = lon, v = lat). Read it with `Resampling::regional` onto the 10°×10° target from Task 1 at 10°E 40–50°N. Assert `grid.ni * grid.nj <= 43 * 43`, and assert `sample(15, 45)` equals the uncropped read's.
  - `a_crop_across_the_antimeridian_reads_both_sides`: the same file, target 160°E…160°W. Sample at ±179.5 and assert both are present.
  - `an_icon_mesh_onto_a_region_builds_a_regional_neighbour_set`: use the existing ICON fixture path from `icon.rs` tests. Read onto a 10°×10° region and assert `neighbours.values().next().unwrap().len() == target.len()`, which is far below the global count.
  - `a_projected_grid_is_windowed_inside_the_region`: use a Lambert section-3 fixture from `projected_grids.rs`, with a region overlapping half of it. Assert every node is inside the target's index range.
- [ ] **Step 2: Failing tests in ve-app:**
  - `importing_a_file_outside_the_region_is_refused`: the error names the file.
  - `reopening_a_regional_project_crops_identically`: save, open twice, and compare `RasterSequence::hash` across the opens. For an ICON layer, assert `Resampling.produced` is empty on the second open.
- [ ] **Step 3: Run** `cargo test -p ve-grib --test regional_import` and `cargo test -p ve-app` and expect FAIL.
- [ ] **Step 4: Implement.**
  - Replace every `resolution.target_grid()` the surveys found: `ve-app/src/import.rs` `resample_into`, `grib_project`, `attach_rasters`; `io.rs` `read_regrid`. Where the project is regional, use `settings.lattice()` and `Resampling::regional`.
  - In `zarr.rs` `read_store`, crop each frame's grid with `cropped_to(&lattice, 1)`. That leaves `RoutingStore::read_block` reading all longitudes, so the in-memory size shrinks while the read cost stays the same.
  - Optional: if `open_cost` shows that read time matters, restrict `read_block`'s rows to the region's band. Lat bands are cheap to skip; longitudes are not, because chunks span them.
  - Refuse when every frame's crop is `None`.
- [ ] **Step 5: Run** the tests and expect PASS. Then run the reference set, since this touches the decoder path: `VE_TEST_GRIBS=~/temp_test_gribs cargo test -p ve-grib --release --test reference_set -- --ignored --nocapture`. Global behaviour is unchanged, so it must pass unchanged.
- [ ] **Step 6: Measure.** Run `cargo test -p ve-app --release --test open_cost -- --ignored --nocapture` with `VE_TEST_OPEN_GRIB` for a global and a regional project. Record both numbers in the plan.md entry.
- [ ] **Step 7: Commit** with `spec.md` §4.8 (*a regional project crops every import to its region plus one node, on every read; a file that misses it is refused*) and plan.md *M101 — Imports read only the region*.

---

### Task 4: A GRIB grid with a first point (M102, part 1)

**Files:**
- Modify: `crates/ve-grib/src/writer.rs` (`GridSpec`, `points`, `section3`), `crates/ve-grib/src/reader.rs` (expose La1/Lo1/La2/Lo2 if not already), `crates/ve-grib/tests/roundtrip.rs`
- Every `GridSpec { ni, nj, micro_degrees }` literal in the workspace becomes `GridSpec::global(ni, nj, micro_degrees)`. Find them with `grep -rn "GridSpec {" crates`.

**Interfaces:**
- Produces:
  - `GridSpec { ni, nj, micro_degrees, la1_udeg: i32, lo1_udeg: u32 }`. `lo1_udeg` is in `[0, 360e6)`.
  - `GridSpec::global(ni, nj, micro_degrees) -> GridSpec` sets `la1 = 90e6` and `lo1 = 0`.
  - `GridSpec::of_lattice(target: &TargetGrid, micro_degrees: u32) -> GridSpec`.
    - `lo1 = (lon0·1e6).round().rem_euclid(360e6)`.
    - For a global target (`lon0 = -180`, `ni·d = 360`), it returns `global(...)`. The global export keeps its prime-meridian start, so the byte-identical constraint holds.
  - `points()` yields `lat = la1 - j·d` and `lon = lo1 + i·d`, normalised to `[-180, 180)`.
  - `section3`:
    - La1 = `la1_udeg`, Lo1 = `lo1_udeg`.
    - La2 = `la1 - (nj-1)·d`.
    - Lo2 = `(lo1 + (ni-1)·d) mod 360e6`.

- [ ] **Step 1: Failing tests in `roundtrip.rs`:**
  - `a_regional_grid_header_names_its_corners`: region 10°E–20°E, 40–50°N at 0.25°. Assert La1 = 50e6, Lo1 = 10e6, La2 = 40e6, Lo2 = 20e6, Ni = 41, Nj = 41.
  - `a_grid_across_the_antimeridian_is_contiguous_in_grib_longitude`: 160°E…160°W. Assert Lo1 = 160e6 and Lo2 = 200e6. Decode with `ve_grib::decode` and assert a value written at 179.75 reads back at −180.25 ≡ 179.75.
  - `a_grid_across_the_prime_meridian_has_lo2_below_lo1`: 20°W…20°E. Assert Lo1 = 340e6 and Lo2 = 20e6, and that our decoder reads it.
  - `a_polar_cap_header`: span 360°, 60–90°N. Assert Ni = 1440, La1 = 90e6, La2 = 60e6, Lo1 = 180e6 (the cap's −180 in GRIB terms), and Lo2 = 179.75e6.
  - `the_encoding_is_identical_on_every_platform` is unchanged and must still pass.
- [ ] **Step 2: Run** `cargo test -p ve-grib --test roundtrip` and expect FAIL.
- [ ] **Step 3: Implement.** Section 3 comments cite the WMO octets (template 3.0, octets 47–50 La1, 51–54 Lo1, 56–59 La2, 60–63 Lo2), as the file already does.
- [ ] **Step 4: External decode (CLAUDE.md *Changing GRIB output*).**
  1. Write the four regional files with a small example.
  2. Run `wgrib2 -grid -V file` and `grib_dump -O file` on each. There must be no warnings.
  3. Open the antimeridian and prime-meridian files in an independent viewer (Panoply or XyGrib) and confirm the field sits where it was painted and the arrows point as the app shows them.
  4. Record the commands and results in the plan.md entry.
- [ ] **Step 5: Commit** `ve-grib: a lat/lon grid with its own first point (M102)`.

---

### Task 5: Fetches write the region (M102, part 2)

**Files:**
- Modify:
  - `crates/ve-zarr/src/source.rs`
  - `crates/ve-zarr/src/arco.rs` (`slice()` takes a `Window`)
  - `crates/ve-zarr/src/erddap.rs` (`subscript` takes index bounds; two requests across a `LonPM180` seam)
  - `crates/ve-zarr/src/regrid.rs` (`to_era5_grid` then crop)
  - `crates/ve-app/src/history.rs` (`GRID` becomes a function of the project; file name; `encode_hour`)
  - `crates/ve-app/src/nrt.rs`
- Test: `crates/ve-zarr/tests/` (window maths, offline), `crates/ve-app/tests/history.rs` (with the existing fake sources)

**Interfaces:**
- Consumes: `ProjectSettings::lattice()`, `GridSpec::of_lattice`.
- Produces:
  - `ve_zarr::source::Window { i0: u32, ni: u32, j0: u32, nj: u32 }` on the common 1440×721 grid. Columns are counted from 0°E, GRIB order, and may wrap past 1440. `Window::global()`, `Window::of(&TargetGrid)`.
  - `Field::cropped(&self, w: &Window) -> Field`, plus `Window::len()`.
  - `FieldSource::read_step` is unchanged. Sources that can subset gain `fn set_window(&mut self, w: Window)` with a default no-op on the trait, so ERA5, GlobCurrent and the whole-file products keep fetching globally and are cropped after.
  - History file name: `{origin.id}-{start}-{end}.grib2` when global, `{origin.id}-{start}-{end}-{region.key()}.grib2` when regional.

A regional project fetches at the source's 0.25° and is cropped on that grid. When the project's resolution is coarser than 0.25°, the fetch still writes 0.25° as now: the history layer is read like any GRIB and sampled. Only the extent changes.

- [ ] **Step 1: Failing tests:**
  - `a_window_across_the_antimeridian_wraps_in_grib_columns`: `Window::of` a 160°E…160°W lattice gives `i0 = 640`, `ni = 161`, and `cropped` takes columns 640..800 in order.
  - `a_window_across_the_prime_meridian_wraps_past_1440`: for 20°W…20°E, `i0 = 1360`, `ni = 161`, and columns 1360..1439 are followed by 0..80.
  - `a_polar_cap_window_is_every_column`.
  - In `history.rs` tests, with the existing fake `FieldSource`: `a_regional_history_import_writes_a_regional_file`. The written file's section 3 has the region's Ni and Nj, its size is under 2 % of the global run's, and its name carries the key.
  - `arco_reads_only_the_window`: assert the `ArraySubset` the ARCO path builds, without the network, as a pure function `subset_for(window, axes)`.
  - `erddap_subscript_splits_at_its_seam`: for a `LonPM180` dataset and a window across 180, assert two subscripts, `[i0:stride:last]` and `[0:stride:i1]`.
- [ ] **Step 2: Run** and expect FAIL.
- [ ] **Step 3: Implement.** In `fetch_source_to_file`, build the regional `GridSpec` from `settings.lattice()` at 0.25°, call `source.set_window(window)`, and crop each `Field` before `encode_hour`. The empty-field padding (`empty_field`) becomes `window.len()` NaNs.
- [ ] **Step 4: Live check (fetches on purpose; say so first).** Run one ERA5 hour and one Copernicus product in a regional project with the app open, and compare the `history/` file sizes against a global import of the same hours. Record both sizes.
- [ ] **Step 5: Commit** with `spec.md` §4.10 (*a regional project's fetch writes its region; sources that can subset at the server do; the rest download the globe and crop before writing*) and plan.md *M102 — Fetches keep only the region*.

---

### Task 6: Exports on the region (M103)

**Files:**
- Modify: `crates/ve-app/src/export.rs` (`run`, `run_zarr`, `estimate`)
- Test: `crates/ve-app/tests/export.rs`

**Interfaces:**
- Consumes: `GridSpec::of_lattice`, `ProjectSettings::lattice()`, `Region::contains`.
- Produces: `estimate()` counts the region's points. For Zarr, `ExportZarrResult.chunks` reports the chunks actually written.

- [ ] **Step 1: Failing tests.** Paint one brush stroke in each case, then read the result back with the crate's own reader:
  - `a_regional_grib_export_is_on_the_region`: the header matches the Task 4 numbers, and the value at a painted node equals `CpuEvaluator` at that lon/lat.
  - `a_regional_export_across_the_antimeridian`: stroke at 179°E–179°W, with values present on both sides.
  - `an_arctic_export_holds_the_pole_once`: Ni = 1440, and row 0 is 90°N.
  - `a_regional_zarr_export_writes_only_the_regions_chunks`: the metadata text still equals the `routing_layout.rs` fixture's, and the shard files on disk are only those intersecting the region.
  - `a_global_export_is_unchanged`: the existing tests stay as they are and pass.
- [ ] **Step 2: Run** `cargo test -p ve-app --test export` and expect FAIL.
- [ ] **Step 3: Implement.**
  - `run`: build the grid with `GridSpec::of_lattice(&settings.lattice(), µdeg)`. Everything after `grid.points()` is already shape-agnostic.
  - `run_zarr`: keep the global `Layout`. Evaluate only the points for which `region.contains(lon, lat)`, and write NaN for the rest. The existing all-NaN-chunk skip does the shrinking.
- [ ] **Step 4:** Run the export tests on all three platforms through CI. The determinism test must still give the old digest.
- [ ] **Step 5: Commit** with `spec.md` §12.1/§12.2 (regional La1/Lo1 table), §12.3 (*a regional project writes the global-shaped store with only its region's chunks; see R7*) and plan.md *M103 — Exports on the region*.

---

### Task 7: Creating a regional project, over IPC and MCP (M104, part 1)

**Files:**
- Modify:
  - `crates/ve-app/src/projects.rs` (`NewProjectRequest`, `into_settings`, `ProjectSummary`)
  - `crates/ve-app/src/import.rs` (`settings_for` seeds a region from a non-global file)
  - `crates/ve-app/src/zarr.rs` (same)
  - `crates/ve-app/src/mcp/tools/project.rs` (`ProjectNewParams`, `project_status` output)
  - `crates/ve-app/src/mcp/tools/guide.rs` (one line in `GUIDE`)
  - `ui/src/generated/*` (via `npm run bindings`)
- Test: in-module tests in `projects.rs`; `crates/ve-app/tests/mcp.rs`

**Interfaces:**
- Produces (all `ts-rs` exported):

```rust
/// A region as the interface sends it: edges in degrees, `east` read as the
/// arc east of `west` (spec 4.2). Snapped outward to the lattice by
/// `ve_core::region::Region::snapped`.
#[derive(Debug, Clone, Deserialize, Serialize, TS, JsonSchema)]
#[ts(export, export_to = "RegionRequest.ts")]
pub struct RegionRequest { pub west: f64, pub east: f64, pub south: f64, pub north: f64, #[serde(default)] pub full_circle: bool }

/// The region a project covers, for the map and the inspector.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "ProjectRegion.ts")]
pub struct ProjectRegion { pub west: f64, pub east: f64, pub south: f64, pub north: f64, pub full_circle: bool }
```

  - `NewProjectRequest.region: Option<RegionRequest>` (`#[serde(default)]`).
  - `ProjectSummary.region: Option<ProjectRegion>`. `east` here is unwrapped (`west + span`), so the frontend never has to guess the wrap.
  - `ProjectSummary.grid_ni` and `grid_nj` are now the region's.
  - `ProjectNewParams.region: Option<RegionRequest>`, with a doc comment per field.

- [ ] **Step 1: Failing tests:**
  - `a_request_with_a_region_snaps_it`
  - `a_request_across_the_antimeridian_keeps_its_arc`
  - `a_file_that_is_not_global_seeds_its_region`. A regional GRIB fixture (`ve-grib/tests/fixtures`) gives a project whose region contains the file's extent, snapped outward.
  - `a_global_file_makes_a_global_project`.
  - In `tests/mcp.rs`, `project_new_with_a_region`. Drive it over HTTP, then read `project_status` and check the region.
- [ ] **Step 2: Run** and expect FAIL. **Step 3: Implement**, then run `npm run bindings`. **Step 4: Run** `cargo test -p ve-app` and `npm run ui:typecheck` and expect PASS.
- [ ] **Step 5: MCP scenario.** Run `tools/mcp-scenarios/run.sh` with "make me a project covering the Bering Sea" and check that the agent passes a region crossing 180°. Run it with `VE_SCENARIO_TOOLS=all` too.
- [ ] **Step 6: Commit** `ve-app: create a regional project (M104)`.

---

### Task 8: The camera is held to the region (M105, part 1)

**Files:**
- Create: `ui/src/map/extent.ts`, `ui/src/map/extent.test.ts`
- Modify: `ui/src/map/camera.ts` (`Camera.region`, `minPxPerDeg`, `clampCamera`), `ui/src/map/camera.test.ts`

**Interfaces:**
- Consumes: `ProjectRegion` from `ui/src/generated/ProjectRegion.ts`.
- Produces:
  - `Camera.region?: ProjectRegion`. It rides on the camera like `projection`, so a spread keeps it.
  - `extent.ts`:
    - `regionOutline(region, stepDeg = 2): {lon, lat}[]`: the subdivided ring. A full-circle cap is one parallel, closed.
    - `regionContains(region, lon, lat): boolean`, which uses `containsLon` from `marquee.ts`.
    - `regionCentre(region): {lon, lat}`. A cap's centre is its pole.
    - `regionFitPxPerDeg(region, view, projection, centre): number`. It projects the outline (plus interior samples for a cap) about `centre` and returns the zoom at which the projected box fits the window.
    - `clampToRegion(camera, view): Camera`.

  `clampCamera` calls `clampToRegion` last, when `camera.region` is set. `minPxPerDeg` takes an optional region and returns `max(world, regionFit)`.

- [ ] **Step 1: Failing tests in `extent.test.ts`.** Build the view with the helpers already in `camera.test.ts`.

```ts
const pacific = { west: 160, east: 200, south: -10, north: 10, full_circle: false };
const arctic = { west: -180, east: 180, south: 60, north: 90, full_circle: true };
const view = { width: 800, height: 600 };

it("cannot zoom out past a region", () => {
  const cam = clampCamera({ centerLon: 180, centerLat: 0, pxPerDeg: 0.1, region: pacific }, view);
  expect(cam.pxPerDeg).toBeCloseTo(Math.min(800 / 40, 600 / 20), 3);
});

it("pans across the antimeridian inside a region and stops at its edge", () => {
  let cam = clampCamera({ centerLon: 175, centerLat: 0, pxPerDeg: 100, region: pacific }, view);
  cam = panBy(cam, view, 2000, 0); // 20° east: past 180
  expect(normalizeLon(cam.centerLon)).toBeCloseTo(-165, 6);
  cam = panBy(cam, view, 5000, 0); // far past the east edge
  expect(normalizeLon(cam.centerLon + 400 / cam.pxPerDeg)).toBeCloseTo(-160, 6);
});

it("a region narrower than the window is centred", () => {
  const thin = { west: 10, east: 12, south: 0, north: 40, full_circle: false };
  const cam = clampCamera({ centerLon: 50, centerLat: 20, pxPerDeg: 15, region: thin }, view);
  expect(cam.centerLon).toBeCloseTo(11, 6);
});

it("an arctic cap wraps in longitude and stops at 60N", () => {
  const cam = clampCamera({ centerLon: 170, centerLat: 0, pxPerDeg: 10, region: arctic }, view);
  expect(cam.centerLon).toBeCloseTo(170, 6); // free wrap
  expect(projectionFor(cam).latOf(projectionFor(cam).yOf(cam.centerLat) - 300 / cam.pxPerDeg)).toBeGreaterThanOrEqual(60 - 1e-6);
});

it("under the globe the centre stays in the region", () => {
  const cam = clampCamera({ centerLon: 0, centerLat: 0, pxPerDeg: 50, region: pacific, projection: "globe" }, view);
  expect(regionContains(pacific, cam.centerLon, cam.centerLat)).toBe(true);
});

it("an arctic cap under polar stereographic fits around the pole", () => {
  const cam = clampCamera({ centerLon: 0, centerLat: 0, pxPerDeg: 0.01, region: arctic, projection: "stereographic" }, view);
  expect(cam.centerLat).toBeCloseTo(90, 6);
  expect(cam.pxPerDeg).toBeCloseTo(regionFitPxPerDeg(arctic, view, projectionFor(cam), { lon: 0, lat: 90 }), 3);
});

it("a global camera is untouched", () => {
  const cam = { centerLon: 10, centerLat: 20, pxPerDeg: 3 };
  expect(clampCamera(cam, view)).toEqual(clampCamera({ ...cam }, view));
});
```

  Add a test per fixed general map family as well: Robinson, and one EPSG regional preset. Each asserts that the projected region's box covers the window or is centred.

- [ ] **Step 2: Run** `npm run ui:test -- extent camera` and expect FAIL.
- [ ] **Step 3: Implement** per spec R8, in this order:
  1. Cylindrical: an unwrapped-arc clamp on `centerLon`, skipped for a full circle; the existing y-clamp, run against the region's north and south instead of the world's.
  2. Movable: clamp the centre into the region (nearest point on the rectangle in lat/lon), then raise `pxPerDeg` to `regionFitPxPerDeg` at that centre.
  3. Fixed general: project the outline once per (projection, region) and memoise it by that key. **Never per camera** (CLAUDE.md: nothing on a movable projection may be built per camera). Then clamp the plane centre and zoom against that box.
- [ ] **Step 4: Run** `npm run ui:test` and expect PASS. Run `node tools/webdriver/projections.mjs` and confirm the turn timing has not regressed.
- [ ] **Step 5: Commit** `map: the camera is held to the project's region (M105)`.

---

### Task 9: The map shows the region (M105, part 2)

**Files:**
- Modify: `ui/src/map/MapView.tsx` (camera gets `summary.region`; overlay; gesture refusal; tile cull), `ui/src/map/camera.ts` (`visibleTiles` skips tiles outside the region), `ui/src/map/allowed.ts` (a refusal reason)
- Test: `ui/src/map/tileRange.test.ts` or `camera.test.ts`, `ui/src/map/allowed.test.ts`

**Interfaces:**
- Consumes: `Camera.region`, `regionOutline`, `regionContains`.
- Produces:
  - An `allowed.ts` reason `"outside-region"`, whose hint is `t("Outside this project's region")`.
  - `visibleTiles(camera, view, budget)` drops tiles whose `tileBounds` miss the region.

- [ ] **Step 1: Failing tests:**
  - `visibleTiles` for the Pacific region at mid zoom returns no tile entirely west of 150°E.
  - `allowed` refuses a press at (0°, 0°) in the Pacific region with `outside-region`, and allows one at 179°E.
- [ ] **Step 2: Implement.**
  1. Where the summary is applied to the camera (where `cameraForProjection` runs on a projection change, and on open), spread `region: summary.region ?? undefined` onto `cameraRef.current` and clamp it. The opening camera is `regionCentre` at the fit zoom.
  2. In `drawOverlay`, before the edge bands: when `camera.region` is set, fill the window rectangle plus `projectedRing(regionOutline(region))` with `evenodd`, in the theme's dim colour at the same alpha the start screen uses for modal scrims. Add a 1 px edge in the graticule colour. Both are drawn inside `draw()`'s frame (CLAUDE.md: the overlay is drawn inside `draw()`).
  3. Refuse a gesture that **starts** outside the region through `allowed.ts`, as M68 does for a hidden layer.
- [ ] **Step 3: Run** `npm run ui:test` and expect PASS.
- [ ] **Step 4: In the app**, with the driver and isolated storage (`VE_AUTOMATION_ROOT`, a separate `VE_DEV_PORT`; never kill by pattern):
  1. Create the Pacific region, Arctic and Antarctic projects.
  2. Take spaced captures at min zoom in equirectangular, Mercator, the globe and polar stereographic.
  3. Pan into each edge.
  4. Note what each capture shows in plan.md.
- [ ] **Step 5: Commit** `map: the region is drawn, culled and enforced (M105)`.

---

### Task 10: Choosing the region (M104, part 2)

**Files:**
- Create: `ui/src/project/RegionPicker.tsx`, `ui/src/project/RegionPicker.test.tsx` (`// @vitest-environment happy-dom`), `ui/src/project/regionPick.ts` + `.test.ts` (pure maths)
- Modify:
  - `ui/src/project/NewProjectForm.tsx` (Extent: Global | Regional; passes `region`)
  - `ui/src/project/NewProjectDialog.tsx` (*Use current view*, from `MapHandle.bounds()`)
  - `ui/src/project/format.ts` (`estimatedGribBytes(resolution, steps, region?)`)
  - `ui/src/i18n/locales/<9 langs>/project.ts`
  - `ui/src/help/topics.ts` + `ui/src/help/locales/<9 langs>.ts` (a *Regional projects* section)
  - `ui/src/help/features/project.ts` (`project:extent`, `project:region-picker`, `project:region-arctic`, `project:region-antarctic`)

**Interfaces:**
- Consumes: `RegionRequest`, `api.basemap()` + `parseBasemap` (coarsest LOD's `lineVertices`/`lineIndices` for the coast), `marqueeBounds`, `NumberField`.
- Produces:
  - `regionPick.ts`:
    - `snapEdges(req: RegionRequest, resolutionDeg: number): RegionRequest`, which mirrors `Region::snapped` for display only. Rust is the authority.
    - `pickerToLonLat(x, y, centreLon, width, height)`.
    - `regionSpan(req): number`.
  - `<RegionPicker value={RegionRequest | null} resolution={string} onChange={(r) => void} />`.

- [ ] **Step 1: Failing tests (`regionPick.test.ts`):**
  - A drag from 170°E rightwards to 170°W gives `{west: 170, east: -170}` with span 20.
  - The same drag leftwards gives a span of 340.
  - Snapping at 0.25° moves 10.1 to 10.0 and 19.9 to 20.0.
  - *Arctic* gives `{west: -180, east: 180, south: 60, north: 90, full_circle: true}`.
  - The picker's centre longitude follows the drag, so a box can be drawn across 180° without leaving the canvas.

  In `RegionPicker.test.tsx`:
  - Typing N/S/W/E in the `NumberField`s calls `onChange` with the snapped region.
  - Checking *Full circle* sets the span to 360.
  - A full circle from −90 to 90 shows *Choose Global for the whole earth* and disables Create.
- [ ] **Step 2: Implement.**
  - The picker is a 2D canvas, 2:1, drawing the coast segments in equirectangular about `centreLon`. Dragging on empty space draws a box; dragging inside the box moves it; Shift-drag pans the centre longitude.
  - The four numeric fields are the source of truth.
  - The estimate under the form uses the region's node count.
  - Every string goes through `t()`. Add the keys in all nine catalogues, in the glossary's terms. Add "region" and "extent" to `GLOSSARY.md` and every `GLOSSARY.<lang>.md` if they are not already there.
- [ ] **Step 3: Run** `npm run ui:test` (coverage, topics and features tests included) and `npm run ui:typecheck`, and expect PASS.
- [ ] **Step 4: Reachability (milestone workflow).**
  - Grep `ui/src/ipc.ts` for any `api.*` method with no caller.
  - In the app, create a regional project from the start screen and from the open-project dialog (including *Use current view*), and use *Open from GRIB* on a regional file.
  - Confirm the title bar or inspector says the project is regional. Add a line to the project summary panel: *Region: 160°E – 160°W, 10°S – 10°N*, formatted at the view boundary.
- [ ] **Step 5: Commit** with plan.md *M104 — Choosing a region* and the help page.

---

### Task 11: Close-out (M106)

- [ ] **Step 1:** Run a whole-feature pass in the app on one regional project crossing 180°:
  1. Import a global 0.25° GRIB.
  2. Fetch a day of ERA5 and of one NRT product.
  3. Paint across the seam.
  4. Save and close, then reopen.
  5. Export GRIB and Zarr.

  Record the numbers in plan.md: the `history/` file sizes against a global run of the same hours, the `.veproj` size with an ICON layer at 0.1° (global versus regional), and open time from `open_cost`.
- [ ] **Step 2:** Run wgrib2 on the exported GRIB and check the arrows in an independent viewer (CLAUDE.md *Changing GRIB output* step 3).
- [ ] **Step 3:** Add the region to `spec.md` §14 as future work: *a project's region changed after creation, by duplicating*.
- [ ] **Step 4:** Run the full Definition of Done list, then commit `Regional projects: close-out (M106)`.

---

## Decisions to confirm before Task 1

These are recommendations, made in the spec, that change what gets built:

1. **R1: the region cannot be changed after creation.** The alternative, allowing growth, means re-fetching every history layer.
2. **R4: a user's own GRIB or Zarr file is cropped in memory and not copied.** A clipped copy would make the project folder *larger*, because the original stays.
3. **R7: the Zarr export keeps the global-shaped routing store**, with only the region's chunks written, so `chunk_loader` keeps reading it.
4. **R5: ERA5 and GlobCurrent still download whole-globe chunks** and crop before writing. Their stores give no smaller unit.

# Near-real-time data, milestone 2: the button, the dialog and three Copernicus products — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A button beside the calendar button fetches the last N days of MULTIOBS surface current, DUACS geostrophic current and the Copernicus L4 hourly wind, sets or moves the timeline's start, and adds one held layer per product.

**Architecture:** The history import's pipeline is reused whole: a `FieldSource` is walked, its fields are packed into a GRIB2 file in the data directory, and the layer reads that file. What is new is a generic Copernicus ARCO source found through the public STAC catalogue, a small product catalogue, an import command that works from "now" and a number of days, and a dialog.

**Tech Stack:** Rust (`ve-zarr`, `ve-app`), `zarrs`, `reqwest` blocking; React + TypeScript; the nine-language catalogues.

**Spec:** `docs/superpowers/specs/2026-10-02-near-real-time-data-design.md`, §3, §4, §5.1, §6.1, §7, and §8 milestone 2.

## Global Constraints

- Invariant 5: nothing fetches except when the button is pressed. `check-offline.sh` names every new host. No C library, no `-sys` crate.
- The fetched fields land on the history import's 0.25° grid (`ve_zarr::source::NI`/`NJ`), through `regrid::to_era5_grid`.
- The period starts at 00:00 UTC, N days before today, and runs to the current hour; at most 240 steps.
- Every layer made here carries its product's period (`Layer::holding`): 24 h for DUACS, 1 h for the two hourly products.
- The history import's behaviour does not change: its tests pass untouched.
- Every interface string in all nine languages; every new control in the Help search; a help page in every language.
- A command that takes longer than a frame is `#[tauri::command(async)]`, has a `LONG_RUNNING` label, and runs its network work on a plain thread (the blocking client will not run on the async runtime).
- Every new command is in `mcp/invoke.rs`'s table or in `EXCLUDED` with a reason.
- One commit for the milestone, on a branch, fast-forwarded to `main`.
- Disk: `CARGO_INCREMENTAL=0`; `--config 'profile.dev.package.ve-app.debug=false'` for `ve-app` test builds.

## Rulings taken while planning

- **The Copernicus reader lives in `ve-zarr`, not a new `ve-nrt` crate.** It is a Zarr reader over the host `ve-zarr` already names; the offline check gains one path on that host. `ve-nrt` arrives with the ERDDAP client (milestone 4), which is not Zarr. Cost if wrong: one module moves later.
- **No schema bump** for `period_hours` (M88's review): the recipe bumps only when an existing field changes meaning. Cost if wrong: an older build shows a held layer on one step in 24.
- **A product whose first time is missing gets an empty first message.** The importer rebases a file onto its own first message (§4.8), so a file that began a day late would land a day early. An all-missing message at the period's start keeps every layer on the same origin with no schema change. Cost if wrong: the frame row marks that first period as present though it is empty.
- **A product that cannot be fetched does not stop the others.** The import adds what arrived and reports the rest by name. Nothing arriving at all is an error.
- **Only the three products that work are shown.** The other seven arrive with their milestones rather than as dead checkboxes.
- **This plan gives interfaces and test cases, not full bodies**: the same session writes and executes it. Cost if wrong: a later executor transcribes less.

## Review Focus

1. **The clock.** "Now" one second before midnight UTC and one second after: the period's start moves a day, the number of steps does not exceed 240, and no step lands past the current hour.
2. **A daily product on a timeline finer and coarser than a day**, and an hourly product on a 3-, 6- and 24-hourly one: only times that land on a step are fetched, and each is fetched once.
3. **A product with nothing in the period** (its newest data is older than the period's start): named in the outcome, the others still import.
4. **A STAC catalogue that has moved a dataset** to a new version suffix or bucket: the newer one is found; a catalogue that cannot be reached falls back to the last known address.
5. **Undo.** One undo removes the layers, the timeline's new start and its new length together.

---

### Task 1: Time axes in seconds and days

**Files:** Modify `crates/ve-zarr/src/store.rs` (`parse_epoch`, `read_time_axis`, tests).

**Interfaces — produces:**
- `pub fn parse_time_units(units: &str) -> Result<(Utc, f64)>` — the epoch and the hours one unit is worth: `"hours since …"` → 1, `"seconds since …"` → 1/3600, `"days since …"` → 24. Anything else is refused by name.
- `read_time_axis` uses it; an entry that is not a whole hour after conversion is still refused.
- `parse_epoch` stays, built on it, refusing non-hour units as today (ERA5 and the existing tests rely on it).

**Tests (in `store.rs`):**
- `"seconds since 1990-01-01"` → epoch 1990-01-01T00, factor 1/3600; `"days since 1950-01-01"` → factor 24; `"hours since 1950-01-01 00:00:00"` → factor 1; `"minutes since …"` refused.
- A pure helper `hours_from_axis(base_hours, factor, raw: &[f64])` converts `[1_139_616_000 s]` since 1990 to the hour of 2026-02-11T00 (hand-computed: 13 190 days × 24), `[27_668.0 days]` since 1950 to the hour of 2025-10-02T00, and refuses `1800 s` (half an hour).

Steps: write tests → `CARGO_INCREMENTAL=0 cargo test -p ve-zarr --lib store` fails to compile → implement → passes; the existing `units_other_than_hours_are_refused` still passes.

### Task 2: Finding a dataset's store in the STAC catalogue

**Files:** Create `crates/ve-zarr/src/stac.rs`; fixtures `crates/ve-zarr/tests/fixtures/stac/{wind-product,duacs-product,multiobs-product,wind-dataset,duacs-dataset}.json` (the documents as served on 2026-10-02, unedited); modify `crates/ve-zarr/src/lib.rs`, `crates/ve-zarr/src/http.rs` (a `get_text(url) -> Result<String>`).

**Interfaces — produces:**
- `pub const CATALOGUE: &str` — the metadata root on the Copernicus object store.
- `pub fn dataset_href(product_json: &str, dataset_id: &str) -> Result<String>` — the `item` link whose href is `<dataset_id>_<digits>/dataset.stac.json`; the highest suffix when there are several; an exact id match only (`…0.25deg_P1D` must not match `…0.25deg_P1D-m`).
- `pub fn time_chunked_url(dataset_json: &str) -> Result<String>` — `assets.timeChunked.href`.
- `pub fn discover(product_id: &str, dataset_id: &str) -> Result<String>` — the two fetches.

**Tests (fixtures, no network):** the wind product yields `cmems_obs-wind_glo_phy_nrt_l4_0.125deg_PT1H_202207/dataset.stac.json`; DUACS 0.125° is found and the 0.25° and demo datasets are not confused with it; MULTIOBS `…nrt_0.25deg_PT1H-i` is found among six; an unknown dataset is refused naming the product; two versions of one dataset pick the later (a fixture edited in the test, in memory); `time_chunked_url` of the wind dataset is the `mdl-arco-time-050` address in the fixture.

### Task 3: The generic ARCO source

**Files:** Create `crates/ve-zarr/src/arco.rs`; modify `crates/ve-zarr/src/lib.rs`; an ignored live test in `crates/ve-zarr/tests/arco_live.rs`.

**Interfaces — produces:**
- `pub struct ArcoSpec { pub name: &'static str, pub variable: Variable, pub u_path: &'static str, pub v_path: &'static str, pub level: Option<(&'static str, f32)> }` — `level` names a depth axis and the value wanted on it (MULTIOBS: `("/elevation", 0.0)`).
- `pub struct ArcoStore` with `pub fn open(url: &str, spec: ArcoSpec) -> Result<Self>` and `impl FieldSource`.
- `pub fn cell_grid_of(lat: &[f32], lon: &[f32]) -> Result<CellGrid>` — the grid the axes describe; refuses an irregular axis and one that does not span the circle.
- `pub fn unpack(raw: i64, fill: i64, scale: f32, offset: f32) -> f32`.

`open` reads the two arrays (int16 or int32, 3-D or 4-D with the level), their scale, offset and fill, the axes and the time axis (Task 1). `read_step` retrieves one time slice of `u` and `v` together (`try_join`), unpacks, regrids with `to_era5_grid`, and refuses a slice with no value in it, as `GlobCurrentStore` does.

**Tests:** `cell_grid_of` on a 0.125° axis (−89.9375…, −179.9375…) gives `CellGrid { lat0: -89.9375, dlat: 0.125, nlat: 1440, lon0: -179.9375, dlon: 0.125, nlon: 2880 }`, and on GlobCurrent's axis gives `CellGrid::GLOBCURRENT`; an axis with a doubled step in the middle is refused; `unpack(-32767, -32767, 0.01, 0.0)` is NaN and `unpack(1234, -32767, 0.01, 0.0)` is 12.34. A regrid check by a defining property: a field that is `lon` in degrees everywhere on the 0.125° grid regrids to `lon` at each node away from the seam, and a constant field regrids to the constant.

**Live test (`#[ignore]`, `VE_TEST_NRT=1`):** each of the three opens through discovery, reports coverage ending within three days of now, and reads its newest step: speeds finite somewhere, none over 80 m/s for wind or 5 m/s for current.

### Task 4: The products

**Files:** Create `crates/ve-zarr/src/product.rs`; modify `crates/ve-zarr/src/lib.rs`.

**Interfaces — produces:**
- `pub enum Product { Multiobs, Duacs, WindL4 }` with `ALL`, `id()` (`"multiobs"`, `"duacs"`, `"wind-l4"`), `label()`, `variable()`, `period_hours()` (1, 24, 1), `credit()`, `parse(id)`, and `open() -> Result<Box<dyn FieldSource>>` — discovery, falling back to `fallback_url()` (the address observed on 2026-10-02) when the catalogue cannot be read.

**Tests:** identifiers round-trip; every product has a period that is 1, 6 or 24; DUACS is a current held for 24 h.

### Task 5: The import

**Files:** Create `crates/ve-app/src/nrt.rs`; modify `crates/ve-app/src/history.rs` (make `fetch_to_file`, `history_layer` and their helpers usable for a product: an `Origin { id, label }`, the source passed in, `pad_first: bool`, `period_hours: u32`), `crates/ve-app/src/lib.rs`, `crates/ve-app/src/mcp/invoke.rs`, `crates/ve-app/examples/export_bindings.rs`, `crates/ve-app/src/document.rs` (a held layer's `span_hours` includes its last period); test `crates/ve-app/tests/nrt.rs`.

**Interfaces — produces:**
- `pub struct NrtRequest { pub products: Vec<String>, pub days: u32, pub set_start_time: bool, pub extend_timeline: bool }` (TS).
- `pub struct NrtOutcome { pub project: ProjectSummary, pub skipped: Vec<NrtSkipped> }`, `pub struct NrtSkipped { pub product: String, pub why: String }` (TS).
- `pub fn period(now_unix_s: i64, days: u32, step_hours: u32) -> Result<Period>` with `Period { start_unix_s, end_unix_s, steps: u32 }` — start is 00:00 UTC `days` before the day of `now`; end is the hour of `now`; `steps` is `(end − start) / step + 1`, refused above 240 and for `days == 0`.
- `pub fn wanted_times(period: &Period, step_hours: u32, step_count: u32, product_period_hours: u32) -> Vec<i64>` — every `max(step, product period)` from the start, on the timeline and not past the end.
- `pub fn nrt_import(state, request, now_unix_s, open: impl Fn(Product) -> ve_zarr::Result<Box<dyn FieldSource>>, on_progress) -> Result<NrtOutcome>` — the source is injected so a test supplies one.
- `#[tauri::command(async)] pub fn import_nrt<R: Runtime>(app, request: NrtRequest) -> Result<NrtOutcome>`, emitting `nrt://progress` with `HistoryProgress`.

**Tests (`tests/nrt.rs`, with an in-memory `FieldSource`):**
- `period`: now = 2026-10-02T15:40Z, 3 days, hourly → start 2026-09-29T00Z, end 2026-10-02T15Z, 88 steps; at 23:59:59Z and 00:00:01Z the start differs by a day; 11 days hourly is refused, naming 240; 30 days three-hourly is 240 or fewer.
- `wanted_times`: a daily product on an hourly timeline of 88 steps gives four times (the four midnights); on a six-hourly timeline the same four; an hourly product on a three-hourly timeline gives every third hour; a timeline shorter than the period cuts the list.
- The import end to end with the fake source: two products → two layers on top, each `Zarr` with its product id and period, the daily one covering 24 consecutive hourly steps per field; the timeline's start is the period's start when asked and untouched when not; the timeline is lengthened when asked and the fetch stops at its end when not; one undo takes all of it back, start and length included.
- A source whose first time is missing: the layer's first frame is empty and its second frame lands a day in, not on step 0.
- A source that fails to open: named in `skipped`, the other imported; both failing is an error.
- A source holding nothing in the period: skipped, with the reason.
- History import tests untouched and green.

### Task 6: The dialog

**Files:** Create `ui/src/panels/NrtImportDialog.tsx`, `ui/src/panels/nrtRange.ts`, `ui/src/panels/nrtRange.test.ts`, `ui/src/panels/NrtImportDialog.test.tsx`; modify `ui/src/panels/LayerPanel.tsx`, `ui/src/ipc.ts`, the progress listener that handles `history://progress`, `ui/src/i18n/locales/*/panels.ts`, `ui/src/help/topics.ts`, `ui/src/help/locales/*.ts`, `ui/src/help/features/panels.ts`.

**Interfaces — consumes:** `NrtRequest`, `NrtOutcome` (generated). **Produces:** `api.importNrt(request)`.

- `nrtRange.ts`: `NRT_PRODUCTS` (id, group, label, detail, periodHours, megabytesPerTime), `maxDays(stepHours)`, `periodOf(now, days, stepHours)`, `downloads(products, period, stepHours, stepCount, extend)`, `megabytes(...)`. The arithmetic mirrors `nrt::period` and `nrt::wanted_times` and is tested against the same hand-worked cases.
- The dialog: Days (`NumberField`, 1…`maxDays`), the start-time checkbox (worded "Set" or "Move", with the current start beside it), the lengthen checkbox (shown only when the period needs more steps than the project has), the products grouped as Currents and Wind with their credits, the cost line, Cancel and Import. Rendered through a portal, as the history dialog is.
- The button: `data-feature="layers:import-nrt"`, beside `layers:import-history`.
- After the import: `onProjectChanged(outcome.project)`; skipped products reported through `reportError`.

**Tests:** the range arithmetic; the dialog renders three products, disables Import with none ticked, sends the request it shows, and shows the lengthen box only when needed. `coverage.test.ts`, `features.test.ts` and `topics.test.ts` pass.

### Task 7: The documents, the offline check, and a live run

**Files:** `tools/check-offline.sh`, `CLAUDE.md` (invariant 5's wording, the layout line for `ve-zarr`), `spec.md` (§4.10: a near-real-time subsection), `plan.md` (M89).

- The offline check allows `s3.waw3-1.cloudferro.com/mdl-metadata` in `ve-zarr` beside `mdl-arco-time`.
- Live: `VE_TEST_NRT=1 cargo test -p ve-zarr --release --test arco_live -- --ignored --nocapture`, then the application driven through the dialog for two days of all three products on a new hourly project (`VE_AUTOMATION_ROOT` set), a capture taken, and the layers' frame rows read back.
- Every check; one commit; fast-forward `main`.

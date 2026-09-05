# CLAUDE.md

Guidance for Claude Code working in this repository.

**Read `spec.md` before changing behaviour. Read `plan.md` before starting new
work.** This file is the operational layer: invariants, layout, commands,
conventions, and recipes.

---

## What this is

A Tauri desktop app for painting global wind and ocean-current fields and
exporting them as GRIB2. Rust owns the entire domain — document model, field
evaluation, caching, GRIB encoding. React/TypeScript is a view layer only.

---

## Hard invariants

Violating any of these is a design regression. If a task seems to require it,
stop and raise it rather than working around it.

1. **No *rendered* raster is ever persisted as project data.** A render is a
   cache product, reproducible from the objects. Rendered rasters exist only in
   memory, in the evictable content-hashed render cache, or inside an exported
   `.grib2`. Deleting the cache directory must always be lossless.
   **A *captured* raster is allowed** (spec §8.5, D52): it is user content that
   stops being reproducible once its sources change, so it travels with the
   project as its own `captures/<hash>.vecap` archive entry. Rendered stays
   forbidden; captured is written only by the region capture and the macro
   library.
2. **The project file's JSON stores geometry and parameters, never pixels.** A
   captured field is a hash in the JSON and an archive entry beside it.
3. **The view is a proxy, never a source.** The preview renders from the objects
   at whatever resolution is fast; the GRIB is baked from the same objects at
   full grid resolution. **No preview pixel ever reaches an export.** The two
   need to agree perceptually, not numerically — speed within max(0.25 m/s, 2%),
   direction within 2° — so the preview may approximate and evaluate coarsely.
   A change to one backend still needs the matching change to the other in the
   same commit; only the tolerance is loose.
4. **Export is deterministic and byte-reproducible** across machines. Export
   uses `CpuEvaluator`, never the GPU, unless the user opts into `fast_export`.
5. **Zero runtime network access.** No CDN fonts, no map tiles, no telemetry, no
   remote schema fetches. `npm run check:offline` enforces this statically in CI
   (source URLs, remote references in the built bundle, and CSP strength). A
   socket-level test over the packaged app arrives with M10.
6. **Interaction stays fast; export may be slow.** Never trade frame rate for
   export throughput.

---

## Repository layout

```
crates/
  ve-core/     Document model, properties, keyframes/interpolation,
               undo/redo, IDs, geodesy, serde, .veproj I/O, migrations.
               `capture` holds the `.vecap` container a region capture
               travels in; `follow` resolves objects that follow objects;
               `annotation` holds the measurements of spec 10, which are
               document state that reaches no scene and no export
  ve-render/   Scene flattening, SDF rasterisation, compositing,
               FieldEvaluator trait, CpuEvaluator, GpuEvaluator (+ WGSL),
               tile pyramid, content-hashed render cache
  ve-grib/     GRIB2 writer: sections, templates, simple packing. And the
               import decoder (`decode`, `import`): every grid definition the
               centres ship — 3.0, 3.1, 3.10, 3.20, 3.30, 3.101 and NCEP's
               3.32769 — with `projection` holding the map maths and
               `resample` putting a projected grid on the project's lattice.
               5.0/5.2/5.3 packing hand-written, 5.40 and 5.42 through
               pure-Rust codec crates, bitmaps. `icon` holds the bundled
               unstructured-grid definitions. `reader` is the writer's
               test-only verifier.
  ve-app/      Tauri app: IPC commands, app state, background workers,
               custom URI scheme, autosave
ui/            React + TypeScript + Vite frontend (npm workspace member)
                 project/  start screen, native dialogs, display formatting
                 map/      camera, projections, tiles, WebGL renderer, shaders
assets/        Natural Earth source data, GRIB templates
tools/         Asset converters and the offline invariant check
package.json   Root npm workspace: owns the Tauri CLI and every script
```

**Dependency direction:** `ve-app` → everything; `ve-render` → `ve-core`;
`ve-grib` → `ve-core`. Never the reverse, and `ve-core` depends on none of
them.

---

## Commands

Everything runs from the **repository root**. The Tauri CLI only resolves the
app crate from here — run it from `ui/` and it silently fails to find the Rust
side.

```bash
npm install                 # root npm workspace; installs ui/ too

# Development
npm run dev                 # tauri dev: builds the app and opens the window
npm run ui:dev              # frontend only, no Rust backend
VE_FORCE_CPU=1 npm run dev  # force the CPU evaluator (also how CI runs)

# Checks — all of these before declaring work done
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm run ui:typecheck
npm run ui:test
npm run check:offline       # invariant 5

# Focused
cargo test -p ve-render parity   # GPU/CPU parity suite
cargo test -p ve-grib            # round-trip; external decoders run in CI
cargo bench -p ve-render         # perf budgets from spec.md 13
cargo test -p ve-grib --release --test resample_cost -- --nocapture
                                 # cost of putting a projected grid on the
                                 # project's lattice

# Regenerate TS bindings after changing any IPC-facing Rust type
npm run bindings            # cargo run -p ve-app --bin export-bindings

# Release bundle
npm run build               # tauri build

# Regenerate application icons (rarely needed)
python3 tools/make_icons.py
```

## Conventions

Getting these wrong produces silently incorrect output rather than a crash.
They are specified in full in `spec.md` §3.

| Concern | Rule |
|---|---|
| Earth radius | `EARTH_RADIUS_M = 6_371_229.0`, one constant, matches the GRIB shape-of-earth code we emit |
| Longitude | `[-180, 180)` everywhere; only `ve-grib` converts to `[0, 360)` |
| Direction storage | **Azimuth-toward**, degrees clockwise from north, `[0, 360)`. Always. |
| Direction display | Converted at the IPC boundary only. No domain code below IPC sees a "from" bearing. |
| Components | `u = speed·sin(az)` eastward, `v = speed·cos(az)` northward |
| Speed | m/s in storage and export, **always knots in the UI**. Not configurable. Convert only at the IPC boundary, via `ve_core::units`. |
| Sizes | Stored in **km**, measured north-south. Pixel inputs resolve to km at object creation, never later. |
| px sizes | A size in px also selects `stamp_space: projected` — a shape on the map, not on the ground (spec §3.5). km selects `geodesic`. The unit is not just a conversion. |
| Time | UTC only |
| Angle interpolation | Shortest arc, always |
| Geometry frame | Object-local AEQD, metres. Never lat/lon degree space — **except** an object with `stamp_space: projected`, which is *defined* on the map and whose frame is therefore map space, in north-equivalent metres (spec §7.2). Anything else in degree space is a bug. |
| Directions | Always true azimuths from geographic bearings, **never** from a local frame angle. An AEQD frame preserves bearings only from its centre; local grid north drifts by tens of degrees a few thousand km out. See `aeqd.rs`. |
| Z-order | Layer order, then object order within layer. Index 0 = bottom. |
| Determinism | No `HashMap` iteration in any evaluation or export path. Use `Vec` or `IndexMap`. |
| Project settings | The map takes its timeline length, direction convention, colour scale and glyph styles from `ProjectSummary`, never from constants. Nothing renders without an open project. |
| Grid resolution | Governs the export only. It must never reach the preview path — a 0.1° project pans and zooms exactly as fast as a 1° one. **Exception**: an import that is not on a lat/lon grid — unstructured or projected — is resampled onto it, since there is no other lattice to put the field on (spec §4.8). |
| Document `f64` | Every `f64` that reaches a project file needs a `canonical::*_field` serde helper. `serde_json`'s parser is one ULP off on ~10% of values, so a raw `f64` does not round-trip. `f32` is unaffected. See `canonical.rs`. |

---

## Recipes

### Adding a vector-creation tool option

Every one of these steps is required. Skipping the kernel pair breaks
invariant 3; skipping the migration breaks old projects.

1. Add the property to the tool's `PropertyMap` in `ve-core`, with its type,
   default, and allowed interpolations.
2. Add it to the `FlatObject` buffer layout in `ve-render` — **and to the WGSL
   struct, with matching alignment and padding.**
3. Implement it in `CpuEvaluator`.
4. Implement it in the WGSL kernel.
5. Extend the parity test's scene generator to cover its range.
6. Add serde round-trip coverage.
7. Bump `schema_version` and add a migration if the default is not backward
   compatible.
8. The inspector picks it up automatically from the property system — if it
   does not, fix the property system rather than hand-writing UI.

### Adding a whole tool

Everything above, plus: a `ToolKind` variant, a `Geometry` variant and its SDF,
the map interaction handler, a hover indicator (or an explicit decision that the
tool has none — see spec §6.2), a palette entry with a shortcut, an icon in
`ToolIcon.tsx` (the `Record` makes this a compile error, not a blank button),
an entry in `tool_catalogue.rs`'s `catalogue()` — which is what puts the new
tool into the seven shared-rule tests — and copy/paste plus keyframe coverage
in tests.

**Renaming a tool** is a migration, not a table edit: `ToolKind` is stored on
every object it drew, so an old file fails to deserialise rather than loading as
something else. Bump `SCHEMA_VERSION`, add the step to `io::MIGRATIONS`, and
leave object *names* alone — those were typed by whoever drew them.

**An outline is a band, never a stroke.** A footprint is a union of stamps and
`Path2D` has no union: stroking one traces every stamp's own circle. Grow it by
half the band, knock out a copy shrunk by half (`drawEdgeBand`, and `insetPx` in
`buildFootprintPath`), and draw it before anything else on the frame —
`destination-out` erases what is already there. `ObjectOutline::radius_km` is a
**radius**: convert it with `footprintOfOutline` and never halve it again.

**An operator** — a mask or a modifier — is invisible on the map, so it needs
an edge: `transform::operator_outlines` lists them, and the frontend both
outlines a selected one and highlights the one under the pointer. Anything with
`PreviewKind` other than `Field` is expected to be in that list.

**A modifier** (spec §6.3) instead of a creation tool: `ToolKind::is_modifier`,
a `Modifier` variant in `ve-render`, the branch in `sample_upto` *and* in
`evaluate.wgsl`'s composite loop, the packing in `gpu.rs`, the content hash in
`cache.rs`, and a case in `fidelity.rs`'s object generator. It carries no
`edge_mode` (`common_specs`), no speed and no direction. If anything it does is
measured from the object's *anchor*, exclude it from merging in
`create::merge_into` — a merge re-expresses the chain under the target's anchor
and would move what the field is measured from. If it reads the field
anywhere but the cell it is writing, it needs the clone stamp's recursion and
must be declined in `gpu::supports` — one line, and a test that says so.

**Then work spec §6.1's "what a tool inherits" checklist.** Every entry in it
came from a bug in the brush, and the brush is only the tool that exists first.
The ones that are actual code in a new tool, rather than free:

- **A size in px must select `stamp_space: projected`**, km `geodesic`. Free in
  the schema — the property is shared — but the tool's option bar has to send it
  and its preview has to draw it, or px paints an ellipse again.
- **The gesture preview draws the field**: the ramp colour for the speed and
  glyphs for the direction, on the globe-anchored lattice, held after release
  until the tiles land. `drawStrokePreview` and `previewHasLanded` in `MapView`
  do this generically; a new tool supplies its footprint, not its own preview.
- **Every `LonLat` option is placeable by pointing.** The inspector's picker is
  automatic (it keys off the value's kind); the *tool's* own picker is not.
- **Directions are shown in the project's convention.** Mark a flow direction
  `Unit::Direction` in the schema and a geometric bearing `Unit::Degrees` —
  getting this wrong shows the reciprocal of what the user set.
- **Declare the dependencies and the frozen options** (`schema::dependencies`,
  `frozen(...)`), or the inspector offers options that do nothing and options
  that must not change.
- **Transforms are free** — they act on the frame, not the shape. The drag
  preview needs the new geometry too, but `BaselineOutline::of` matches `Shape`
  exhaustively on purpose, so the compiler asks for it rather than the object
  silently vanishing mid-drag. Keep it that way: no wildcard arm.

### Adding a measurement

A measurement (spec §10) is *not* an object: it makes no field, reaches no
exported file, and has no properties for a schema to describe. It is document
state all the same, so it is saved and undone like everything else.

1. Add the variant to `ve_core::annotation::Measurement`, with its `handles`,
   its `move_handle` arm and its `measure()` arm. The geodesy goes in
   `ve_core::geo` beside the paths already there, never in the frontend.
2. Add the kind to `ve_app::measure`'s **mirror** of `MeasurementKind`
   (`ve-core` has no `ts-rs`, so a type crossing IPC is declared app-side and
   converted — the `StampSpace` precedent), and to `MEASURE_LABELS` and
   `pointsNeeded` in `ui/src/map/measure.ts`.
3. **Do not add a command.** Every edit is `Command::SetAnnotations`, replacing
   the whole list, which is what makes the inverse right by construction and a
   drag one history entry.
4. **Do not call `touch()`.** The tile revision addresses rendered tiles, and a
   measurement changes no pixel of the field; bumping it discards the cache for
   a mark on the chart.
5. Numbers leave `ve-core` in metres and degrees and are formatted in
   `ve_app::measure`, at the IPC boundary, like every other unit. **A bearing
   in a measurement is a course, not a wind**, so it is never converted to the
   project's direction convention.

### Adding a numeric input

Use `NumberField` (`ui/src/NumberField.tsx`). A plain controlled
`<input type="number">` with `Number(raw) || fallback` **cannot be cleared** —
the empty string parses as `NaN`, the fallback commits, and the box refills
under the cursor (D51). `commitWhileTyping={false}` for a field whose commit is
a document write or gated behind a confirmation.

### Adding a field to the document

1. Add it with `#[serde(default)]` if older projects will lack it, so they still
   open. A new *property* needs no migration at all: `PropertyMap::backfill`
   inserts the schema default on load.
2. **If it is an `f64`, give it a `canonical::*_field` serde helper.** Without
   one it will not survive a save and load, and the failure is silent -- the
   value shifts in its last bit and only the round-trip test notices.
3. Extend `io::tests::hostile_floats_survive_a_round_trip` with the new field.
4. Bump `SCHEMA_VERSION` and add a migration only if an existing field changes
   meaning or shape. Add the step to `io::MIGRATIONS`, keyed by the version it
   migrates *from*, and prove it with a test that hand-builds a document at the
   old version -- `a_version_1_stroke_opens_as_one_chain` is the pattern.

### Changing a shape's geometry

The CPU (`sdf.rs`) and the GPU (`gpu.rs` plus `evaluate.wgsl`) hold the same
shape twice, in different layouts. Change one and the other silently disagrees,
which the fidelity suite catches only if the suite generates the new case.

1. Change `Shape` and `sdf.rs`, which is the authority.
2. Change the packing in `gpu.rs` and the matching struct and switch arm in
   `evaluate.wgsl`. `OBJECT_WORDS` must equal the WGSL struct's word count, and
   the struct must stay a multiple of 16 bytes.
3. **Add the case to `tests/fidelity.rs`'s shape generator**, or the two
   backends are never compared on it.
4. If the shape holds points, check `cache.rs` hashes the new structure -- two
   different shapes that hash alike would share a render cache entry.

### Adding a multi-object edit

Anything that writes several objects in one user action goes through
`Command::Batch`, so one undo returns all of them. Batches merge element-wise,
which is what collapses a drag into a single history entry — but only when
successive batches have the same length and address the same objects and
properties in the same order. Build the batch deterministically or coalescing
silently stops working.

### Touching the raster sampler

An imported GRIB layer is a lattice sampled by both kernels (spec §4.8).
`RasterGrid::sample` in `ve-core/src/raster.rs` is the authority and
`sample_raster` in `evaluate.wgsl` is its port, decision for decision:
extent check, wrap at the seam, missing corners left out of the blend.
Change one and change the other in the same commit; the fidelity suite
generates raster scenes and is what catches drift. The lattice's content hash
(`RasterGrid::hash`) and its `z` are what the render cache keys on, so a
change to what a grid *means* without a change to its hash is a stale-tile
bug. The project file holds the GRIB's path and never its samples
(invariants 1 and 2): `Layer::raster` is `#[serde(skip)]` and
`import::attach_rasters` reads the file back on open.

**A step the file has no message for has no raster**, not the previous one:
`RasterSequence::frame_at` returns an `Option` and `flatten` pushes nothing
when it is `None` (spec §4.8, D48). A message is a measurement and does not
hold the way a keyframe does.

### Changing GRIB output

1. Change `ve-grib` and update the golden fixtures deliberately, never by
   blindly re-recording.
2. Confirm `wgrib2` and `grib_dump` still parse cleanly with no warnings.
3. Open the output in an independent viewer and confirm the arrows point the way
   the app displayed them. This is the check that catches a u/v swap or a
   from/toward inversion — the two most likely silent bugs in the codebase.
4. Confirm byte-identical output across all three CI platforms.

### Changing the GRIB decoder

The decoder's reference is `crates/ve-grib/tests/fixtures`: one analytic field
in seven packings, six of them made by repacking the **writer's own** output
with ecCodes, so the values are known without trusting any decoder.
`fixtures/README.md` has the recipe. Re-record them only when the field itself
changes — a fixture regenerated from our own output asserts nothing.

`reference_set.rs` is the other half and the one that finds real bugs: ten
forecast files from nine centres, checked against 200 values ecCodes decoded.
The files are 17 MB and not committed, so it is `#[ignore]`d and reads
`$VE_TEST_GRIBS`. **Run it after any decoder change**, in release:

```bash
VE_TEST_GRIBS=~/temp_test_gribs cargo test -p ve-grib --release \
    --test reference_set -- --ignored --nocapture
```

**A group length is read for every group, the last one included** — and only
then replaced by section 5's true length (template 5.2/5.3). Skipping the read
leaves the bit reader short, and the octet alignment that follows lands one
octet early whenever those bits cross a boundary. Nothing downstream notices,
because the lengths still sum to the value count; the data is simply read from
the wrong place, and only for the group counts where the two alignments happen
to differ. That was live in the shipped decoder and made most real GFS and
GEFS messages decode as garbage while others were perfect.

Both compressed packings are decoded by crates, not by us: `hayro-jpeg2000`
(5.40) and `rust-aec` (5.42). Neither may gain a `-sys` dependency — invariant
5 and the three-platform build both forbid a C library — so check `cargo tree`
after any version bump.

### Adding a grid definition template

A grid is a regular lattice in *some* plane, plus a map from that plane to the
earth. `decode::ProjectedGrid` is that shape and `projection::Projection` is
that map, so a new template is usually a parsing arm and nothing else:

1. Add the arm to `grid_of`'s match and to `projected_of`, reading the octets
   from the WMO table. **The octet numbers in the comments are the 1-based
   ones; `byte(s, n - 1)` reads octet `n`.** Templates 3.1 and 3.32769 carry a
   basic angle and its subdivisions and put everything eight octets later than
   3.10/3.20/3.30 do.
2. If it needs a projection nothing has yet, add the variant to `Projection`
   and implement `forward`, `inverse` and **`convergence`** — and add it to
   `Prepared`, or the constants are rebuilt at every one of a few million
   nodes.
3. Add it to `projected_grids.rs` with **section 3 of a real file**, copied
   verbatim as hex, and assert *geography*: the places that model covers and
   the places it does not. A projection with its cone constant, its hemisphere
   or its orientation wrong still produces plausible numbers; what it does not
   do is put Alaska over Alaska.

**`convergence` is not optional.** Flag table 3.5 bit 5 lets a message resolve
`u` and `v` along the grid's axes, and most regional models do. Reading those
as eastward and northward is a 30° error across a Lambert CONUS grid that
looks entirely plausible on screen. The rotation happens at the **source**
nodes, before the blend — near a stereographic grid's pole the convergence
turns through a full circle in a few cells.

The sweep over a directory of real files is what says the octets are right for
grids no fixture covers:

```bash
VE_TEST_GRID_DIR=~/grib2 cargo test -p ve-grib --release \
    --test projected_grids -- --ignored --nocapture
```

It refuses to pass if any grid fails to parse, or if a walk to a stated last
point misses it by more than a cell. Templates 3.10 and 3.32769 state that
last point and nothing in the projection consumes it, which makes it the one
free end-to-end check in the file.

### Adding an unstructured grid

A GRIB message on template 3.101 names its grid by a UUID and says nothing
about where its cells are. The positions are bundled, so a new mesh means a
new asset entry, not new code:

```bash
cargo run -p icon-grid-builder --release -- --out assets/icon_grids.bin \
    --grid "ICON global R03B07"     clat.grib2     clon.grib2 \
    --grid "ICON global EPS R02B06" eps_clat.grib2 eps_clon.grib2
```

**Every grid has to be passed in one run**: the tool writes the whole asset,
so leaving one out drops it. `ve_grib::icon::EMBEDDED` is what gets shipped,
and `icon.rs`'s tests assert the R03B07 mesh is in there and reaches both
poles, so a truncated asset fails the suite rather than the user's import.

The coordinates are stored on the lattice the source messages were packed on
— detected, then checked against every value — which is what keeps the asset
to 2.4 MB against 11.8 MB of raw f32. A proper LZMA encoder would reach
0.4 MB, but `lzma-rs` is the only pure-Rust one and its match finder is weak
enough to *lose* to deflate; nothing else is available without a C library.

**Resampling is point sampling, not averaging** (spec §4.8): the evaluator
computes each cell's vector at the cell, so an imported field has to mean the
same thing as a painted one at the same resolution. `u` and `v` interpolate as
components. The neighbour search is kept in the project because it costs half
a second at 0.1° and depends only on the mesh and the grid — it is derived
state, and `io::read_regrid` drops a set that no longer matches rather than
trusting it.

### Touching the render cache

Cache keys are BLAKE3 over the canonicalised flat scene. If an edit changes what
a frame looks like, it must change the hash; if it does not (renaming a layer,
moving the camera), it must not. Adding a field to `FlatScene` without adding it
to the hash input is a correctness bug that shows up as stale frames.

---

## Environment gotchas

- The Tauri CLI resolves the app crate by walking up from the cwd. It finds
  `crates/ve-app` from the repo root and **not** from `ui/`.
- `beforeDevCommand` / `beforeBuildCommand` in `tauri.conf.json` run from the
  repo root, not from the config's own directory.
- `ve-app` ships two binaries, so `default-run = "ve-app"` is required or
  `tauri dev`'s bare `cargo run` cannot choose between them.
- Frontend tests default to the `node` environment. Component tests needing a
  DOM opt in per file with `// @vitest-environment jsdom`.
- This machine has Homebrew Rust rather than rustup, so `rustup target add`
  is unavailable. macOS universal builds (M10) will need rustup installed.
- **`[profile.dev.package.ve-app] opt-level = 0` is load-bearing, not laziness.**
  At the `opt-level = 1` that `[profile.dev]` sets, an incremental rebuild of
  `ve-app` produces an rlib whose codegen units reference each other's
  internalised `.llvm.*` symbols, and the link fails with "Undefined symbols for
  architecture x86_64" — every binary in the package, `tauri dev` included, on
  every edit. If you see that error, check whether someone raised the app
  crate's dev optimisation; `CARGO_INCREMENTAL=0` recovers a build in the
  meantime. The reasoning is in `Cargo.toml`.
- **The 2D overlay is drawn inside `draw()`, not beside it.** Handles, brush
  footprints and drag outlines are positioned by the camera, so they have to be
  redrawn in the same frame as the GL map or the two canvases drift apart until
  something else schedules an overlay redraw. Every camera change must go
  through `requestDraw`; nothing should call `drawOverlay` after moving the
  camera. The bug this prevents: the wheel handler asked for an overlay redraw
  and the pan and resize handlers did not, so a selected object's handles sat
  still while the map moved under them. **Overlay-only changes go through
  `requestOverlay`**, which waits for the next frame and stands down when a GL
  frame is already pending (that frame draws the overlay). Pointer reports
  arrive faster than frames are shown; a synchronous `drawOverlay` per report
  drew the overlay twice per frame on a fast drag.
- **Nothing that changes per pointer move may be `MapView` state.** The cursor
  readout was, and every pointer report re-rendered two thousand lines of hooks
  and the whole toolbar to refresh four spans. It is an external store now
  (`createReadoutStore`), and only `MapReadout` subscribes. Its IPC sample is
  one-in-flight, latest-wins — the same pattern as the drag preview.
- **A tile rendered ahead is the tile that will be served.** `protocol::serve`
  is the one path a tile takes — the key, the backend, the quality and the
  encoding are decided there — and the render pool calls it. A pool that chose
  any of those differently would fill the cache with tiles the map never asks
  for and report frames ready that are not. Readiness (`frame_readiness`) is a
  probe of the cache index by content hash, via `RenderCache::contains`, which
  must not touch the disk or the LRU clock.
- **Steps that look the same share their tiles.** A still scene hashes alike at
  every step, so rendering one tile readies every step at once and the cache
  holds one frame, not `step_count`. Any test that counts entries or observes
  render order per step has to animate the scene first (`readiness.rs`).
- **Rendered is not shown.** The backend's readiness says a tile is in the
  cache; only the map knows whether the webview has fetched it onto the GPU.
  Playback asks the map (`MapHandle.warm`) and advances only when every tile
  of the next step is resident — advancing on the backend's word alone drew a
  blank map for a frame at every step, which looked like flicker. A frame that
  is not yet on screen draws its missing tiles from the last frame that was,
  dimmed (`heldFrame` in `RenderState`); never leave them blank.
- **A tile is rendered once.** `RenderCache::get_or_render` is single-flight:
  the map's request for a tile and the pool's unit for the same tile meet
  there and one waits for the other. Every path that renders a tile goes
  through `protocol::serve_keyed`; a new one that calls `render_tile` itself
  will evaluate tiles twice again.
- **Stale is frontend memory.** The backend reports what the cache holds now;
  "solid at an earlier revision, not at this one" is the timeline remembering
  (`playback.ts`, `classify`). Playback advances only into a *solid* step and
  otherwise holds with a buffering flag — never into stale.
- **A stroke preview is extended, never rebuilt.** `extendStrokePath` and
  `extendLatticeUnderStroke` resume from a carried `progress`; the from-scratch
  `buildStrokePath` / `latticeUnderStroke` are those same walks started at
  zero, and `incremental.test.ts` holds them equal op for op. Rebuilding on
  every pointer report was quadratic in the stroke: a thousand points was a
  million stamps over the drag. `previewCost.test.ts` reports the numbers.
- **WKWebView suspends `requestAnimationFrame`** whenever the window is not
  being composited — occluded, minimised, or on another space. The redraw
  scheduler in `MapView` pairs every frame request with a timer fallback; without
  it the map silently never draws and nothing reports why.
- **Parallel chunk sizes must derive from the thread count**, never a constant.
  A fixed size starves small batches: `chunk_size` in `cpu.rs`.
- **Pointer strokes must read `getCoalescedEvents()`.** A `pointermove` fires
  about once per frame; a fast drag covers a lot of ground between frames, and
  the single delivered position cuts corners off fast curves.
- **`Path2D.ellipse()` continues the current subpath.** Without a `moveTo`
  first, consecutive footprints are joined by a straight line and a brush stroke
  fills as one enormous polygon. `ui/src/map/footprint.ts` exists so this is
  covered by a test rather than by noticing it on screen.
- **`from` is a reserved word in WGSL.** So is `in`. Name shader parameters
  `origin`, `start`, and so on.
- **Preview quality depends on the backend**: coarse on the CPU, exact on the
  GPU, because dispatch overhead outweighs the saving when evaluation is cheap.
- **Tile textures must use `NEAREST`.** Speed and direction are 16-bit values
  split across byte pairs, so hardware filtering blends the high and low bytes
  independently and produces nonsense. Smoothing is done in the shader by
  decoding four texels and interpolating the decoded values.
- **Glyph sizes are CSS pixels scaled by `devicePixelRatio`.** Specified in
  device pixels they come out half-size on a retina display.
- **The glyph lattice is anchored to the globe, not to a tile or the viewport.**
  Per-tile anchoring leaves a gap of `tile width mod spacing` at every boundary,
  which looks like glyphs clustered into blocks. See `glyphLattice`.
- GLSL lives in template literals, so **a backtick in a shader comment
  terminates the string**. Write "modulo", not the operator in backticks.
- **An integer uniform in a shared prelude must state its precision.** An `int`
  defaults to `highp` in a vertex shader and `mediump` in a fragment one, so a
  bare `uniform int` in a block both stages include links with "Precisions of
  uniform 'x' differ between VERTEX and FRAGMENT shaders" and the map never
  mounts. Floats escape it only because every source opens with
  `precision highp float`. `shaders.test.ts` now checks this; it did not, and
  `uProjection` shipped bare.
- **The map projection lives twice**: `latToY`/`yToLat` in `shaders.ts` and
  `Projection.yOf`/`latOf` in `projection.ts`, because the map is drawn by the
  GPU and the pointer, the overlay and the tile cull are computed by the CPU.
  `shaders.test.ts` lifts the two functions out of the shipped GLSL and
  evaluates them against the TypeScript, so drift fails the suite rather than
  showing up as glyphs sitting slightly off the colour they describe. The mode
  is `Projection.mode`, which is what the shader branches on; equirectangular
  must stay 0, since that is the GLSL default branch.
- **The projection rides on the `Camera`**, not beside it. Everything that
  needs it — `project`, `unproject`, `panBy`, `visibleBounds`, the footprint
  radii, the renderer's uniforms — reads it off the camera it was already
  handed, and a camera without one is equirectangular. A new camera built by
  spreading an old one keeps it; one built from three literal fields silently
  flattens the map.
- **A screen distance is not a span of latitude.** Anything that divides pixels
  by `pxPerDeg` and treats the answer as degrees of latitude is correct only
  under equirectangular. Go through `panBy` or `visibleBounds`; that mistake
  has already been made in the pan, the tile cull, the glyph cull and
  `regionOfView`.
- **`[profile.dev.package."*"]` covers dependencies, not the workspace's own
  crates.** Both entries are needed; without `[profile.dev]`, tile sampling ran
  2.4x slower under `tauri dev` than in release. `cargo test -p ve-render --test
  tile_cost -- --nocapture` reports the current cost.
- `clippy.toml`'s `allow-expect-in-tests` covers `#[test]` bodies and
  `cfg(test)` modules, but **not helper functions in `tests/` integration
  files**. Those need an explicit crate-level allow with a reason.
- **A project's tile revision must be unique per opening, not per edit.** Tile
  URLs are `<revision>/<step>/...` and are served `immutable`; two projects that
  both start at revision 1 share addresses, and the webview serves the old
  project's pixels for the new project. `session::fresh_revision` seeds from the
  clock so this survives a restart too.
- **A transform drag is computed from a baseline in `Session::transform`**, not
  from the current document. The frontend sends absolute pointer positions and
  the same position twice must be a no-op. `document::finish_gesture` clears the
  baseline as well as breaking coalescing — a stale one makes the next drag
  compute from where the last one started.
- `visibleTiles` drops a zoom level rather than truncating its list when the
  ideal level exceeds the budget: a retina viewport at mid zoom wants ~286
  tiles, and an unpainted corner is a far worse artefact than soft pixels.
- `VE_CAPTURE=1 npm run dev` runs the development capture suite: it renders a
  few deliberate scenarios, writes each to the log directory, and logs the field
  values sampled at the same points. That makes "do the glyphs point the right
  way" checkable against numbers without screen-capture permissions.

---

## Testing rules

- Geodesy changes need antimeridian **and** polar test cases. Both. A grid is
  not exempt: a polar stereographic grid that contains the pole covers every
  longitude, and a Mercator one can straddle the antimeridian.
- A path builder is checked against the property that *defines* the path, not
  against a second copy of its formula: a great-circle vertex by the distances
  to the two ends summing to the whole, a rhumb-line vertex by walking the
  constant bearing in twenty thousand steps and arriving in the same place
  (`geo.rs`). Both are in the suite; extend them rather than restating a
  formula.
- Evaluation changes need the parity suite to pass — it is not optional and not
  slow enough to skip.
- GRIB changes need round-trip plus external decode.
- Document model changes need a save/load round-trip and an undo/redo inverse
  test.
- Performance-sensitive changes need a benchmark number, before and after.

Do not add a test that only asserts the code does what it currently does. Assert
against an independent reference: a known distance, a hand-computed vector, a
decoder's output.

---

## Performance rules

- Never evaluate the field on the UI thread.
- Never hold a full 0.1° global field in memory during export — stream one
  message at a time.
- Cull with the spherical cap before any SDF work.
- Preview evaluates the visible viewport at screen resolution, never the whole
  globe.
- Budgets live in `spec.md` §13. If a change misses one, say so explicitly
  rather than letting it slide.

---

## Style

- `thiserror` in library crates, `anyhow` at the app boundary, a serialisable
  `AppError` crossing IPC.
- No `unwrap()` or `expect()` in library crates outside tests. In `ve-app`,
  `expect()` with a message is acceptable only for genuine startup invariants.
- Public items in `ve-core`, `ve-render`, and `ve-grib` get doc comments; the
  non-obvious ones get a "why," not a restatement of the signature.
- Comment density should match the surrounding file. The geodesy and GRIB
  encoding code warrants heavy commenting — magic octet offsets need a citation
  to the spec section. Most other code does not.
- Frontend types are generated from Rust, never hand-written.

---

## Definition of done

- `cargo test --workspace` passes.
- `cargo clippy --workspace --all-targets -- -D warnings` is clean.
- `cargo fmt --all --check` is clean.
- The relevant recipe above was followed in full.
- Touched perf budgets were measured, not assumed.
- If behaviour changed, `spec.md` was updated in the same commit.

---

## Ask, don't guess

Raise these rather than picking a default:

- Anything that would violate an invariant above.
- Changes to the direction convention, units, or coordinate ranges.
- Changes to the `.veproj` schema that cannot be migrated forward.
- Making a project setting mutable that `spec.md` §4.1 marks immutable.
- Adding any runtime dependency that reaches the network.
- Reopening anything in `spec.md` §15 or the decisions log in `plan.md` §5.
  Those are settled, with recorded reasoning; if implementation reveals one was
  wrong, say so explicitly rather than quietly diverging.

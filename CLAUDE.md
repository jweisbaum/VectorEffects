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

1. **No raster is ever persisted as project data.** Rasters exist only in memory,
   in the evictable content-hashed render cache, or inside an exported `.grib2`.
   Deleting the cache directory must always be lossless.
2. **The project file stores geometry and parameters, never pixels.**
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
               undo/redo, IDs, geodesy, serde, .veproj I/O, migrations
  ve-render/   Scene flattening, SDF rasterisation, compositing,
               FieldEvaluator trait, CpuEvaluator, GpuEvaluator (+ WGSL),
               tile pyramid, content-hashed render cache
  ve-grib/     GRIB2 writer. Sections, templates, simple packing.
               (The reader lives in this crate's tests only.)
  ve-polar/    Boat polars, polar inversion, route tree, route solving
  ve-app/      Tauri app: IPC commands, app state, background workers,
               custom URI scheme, autosave
ui/            React + TypeScript + Vite frontend (npm workspace member)
                 project/  start screen, native dialogs, display formatting
                 map/      camera, tiles, WebGL renderer, shaders
assets/        Natural Earth source data, sample polars, GRIB templates
tools/         Asset converters and the offline invariant check
package.json   Root npm workspace: owns the Tauri CLI and every script
```

**Dependency direction:** `ve-app` → everything; `ve-render` → `ve-core`;
`ve-polar` → `ve-core`; `ve-grib` → `ve-core`. Never the reverse, and
`ve-core` depends on none of them.

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
| Grid resolution | Governs the export only. It must never reach the preview path — a 0.1° project pans and zooms exactly as fast as a 1° one. |
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
tool has none — see spec §6.2), a palette entry with a shortcut, and
copy/paste plus keyframe coverage in tests.

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

### Changing GRIB output

1. Change `ve-grib` and update the golden fixtures deliberately, never by
   blindly re-recording.
2. Confirm `wgrib2` and `grib_dump` still parse cleanly with no warnings.
3. Open the output in an independent viewer and confirm the arrows point the way
   the app displayed them. This is the check that catches a u/v swap or a
   from/toward inversion — the two most likely silent bugs in the codebase.
4. Confirm byte-identical output across all three CI platforms.

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
  still while the map moved under them.
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

- Geodesy changes need antimeridian **and** polar test cases. Both.
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

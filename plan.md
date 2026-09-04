# VectorEffects — Implementation Plan

**Companion to** `spec.md`. Section references below point into it.

**Status:** M0 through M5 complete and verified; M6 and M7 complete and
verified by test (2026-09-03). **The walking skeleton is closed**: a new project, a painted
stroke, and a GRIB2 file that ecCodes parses and whose values decode to exactly
what was painted. **The tool catalogue is complete**: all six tools of spec §6.2
draw, evaluate, save and export.

**M8 is next.**

**M7 complete.** Keyframe editing end to end, a render pool that works ahead
of the playhead, a readiness strip with the distinct stale state, and the
timeline dock: ruler, transport, playback that holds rather than stutters,
range bars, key diamonds with drag, box-select and per-segment easing, the
step-count shrink behind its quantified confirmation, and a start time for the
ruler's UTC labels.

**The design that made it small.** The render pool calls the same function the
map's own tile requests go through, so a tile rendered ahead is byte for byte
the tile that will later be served and "ready" means "will be a cache hit".
Invalidation was never implemented: keys are content hashes, so an edit changes
exactly the keys of the steps it changes the look of, and the pool only has to
look again. `readiness.rs` holds the property rather than a mechanism — the
acceptance criterion verbatim (an object alive for three steps invalidates
exactly three frames), plus renames changing nothing, a keyframe edit changing
only the segments through it, and a re-request rendering only what moved.

**Two things the tests taught.** Outside the keyed range the *nearest key*
holds, not the base (spec 4.5) — a lone "off" key at step 3 switches an object
off from step 0 — so the timeline marks where a value is held rather than
keyed. And steps that look the same share their tiles: a still scene is one
frame, rendered once, solid everywhere at once; three tests had counted entries
per step and were wrong for that reason.

**Measured.** A warm tile is served in 63 µs on the backend against a 50 ms
budget for the whole step switch. 583 Rust tests and 274 frontend tests pass.

**Not verified by hand.** The dock, like the tools before it, has been driven
only by tests: the pure rules (`playback.ts`) and the backend commands are
covered, the app builds and boots, and nobody has scrubbed the ruler, dragged
a key or watched playback buffer. That gap is now two milestones deep.

**M6 complete.** All six tools of the catalogue draw, evaluate, save and export.
Most of the *evaluation* already existed — every shape, speed mode and direction
mode landed in M3 — so the milestone was mostly the model's account of what each
tool offers and the interaction that drives it.

**The checklist became code.** Spec §6.1's "what a tool inherits" was written
after a series of bugs in the brush, and a checklist worked through by hand is a
checklist that gets missed. It is now structural: one creation command for the
whole catalogue, five gestures rather than one per tool, one schema-driven
option bar, and one footprint type that the gesture preview, the hover indicator
and the held preview all draw. A tool supplies its gesture and its footprint and
inherits the rest, so the eraser and the clone stamp are brush-like *because*
all three send the same gesture rather than because three code paths were
written to resemble each other.

The cross-tool rules are tested per tool, parameterised over the catalogue: px
is round on the map and km round on the ground at 0°, 45° and 70°; `toward` and
`away` are exact reciprocals; the inspector lists exactly what is live and
editable; every frozen option is refused after the fact; every tool round-trips
through save and load; every geometry moves when its frame does. A new tool
fails these without anyone remembering to write a test for it.

**Five bugs the milestone found, four of them pre-existing.**

- **`path_bearing` on the GPU passed `segment_distance` its arguments
  reversed**, so a curve's flow followed the tangent of whichever part of the
  path happened to win. Never caught because the parity generator produced every
  direction mode except `AlongPath` — the one mode with a shader loop of its own.
- **`to_global` in the shader implemented only the geodesic branch** while
  `to_local` handled both. Only `path_bearing` calls it, so a map-space curve
  went through the ground formula: 410 of 6400 samples outside tolerance in
  projected space, 14 after the fix.
- **`clone_source` was never part of the scene hash.** Two clone stamps
  differing only in where they read from produced the same cache key, so moving
  a source served the previous source's pixels.
- **A computed position did not survive a save.** Serde quantised on the way out
  but nothing quantised on the way in, so a centroid — or a dragged anchor —
  made the document stop equalling itself. `PropValue::canonical` now applies in
  `Animatable`'s writers: one place, off the evaluator's per-sample path, where
  `LonLat::new` would have been on it.
- **Switching a size between km and px converted in the wrong space.** A pixel
  number is always a projected measure, and converting the km side in its own
  space scales by `cos(lat)` — the brush changed size by a fifth at 40° on the
  way through.

**One design decision worth recording.** A polyline's tangent is genuinely
discontinuous on the bisector at a corner, so the two backends can land on
opposite sides of a tie and disagree by the angle of the corner. No tolerance is
meaningful across a discontinuity, so the fidelity suite steps around it — but
narrowly: only objects that actually cover the sample count, the exempted
samples are counted and reported, and the test fails if the exemption grows past
a twentieth of the suite. Measured, every disagreement sat inside 0.02 of the
corridor half-width of a tie; the guard is at 0.05, and 73 of 30,720 samples are
exempted.

**Not verified by hand.** The app builds, starts and loads the frontend with no
errors, and the tools are covered end to end by the Rust integration tests and
the frontend unit tests — but nobody has clicked through the six tools in the
running app. That is the gap M7 should close first.

**M3 progress.** The evaluation engine's core is built and tested: rhumb-line
geodesy, object-local AEQD frames, signed-distance shapes, scene flattening, and
the CPU evaluator with feather, divergence, curl, every direction mode, and
overwrite compositing. The tile pipeline now serves the real field, the cursor
readout samples the document, and the brush paints end to end through the undo
stack. **M3 complete.** The wgpu backend and its fidelity suite, adaptive coarse-lattice
preview evaluation, the content-hashed render cache, and clone-stamp nested
evaluation all landed. Measured on a dense 60-object tile: the GPU renders in
3.9 ms against the CPU's 45.8 — 11.8x, and inside the 8 ms preview target. The
fidelity suite compares 30,720 samples across 120 generated scenes and finds a
worst speed error of 0.031 m/s against a 0.25 tolerance and 0.016 degrees
against 2.

**M4 complete.** Simple packing, message assembly for both field kinds, a
streaming writer, the export dialog with a size estimate and cancellation, a
minimal reader behind a `testing` feature, and an ecCodes CI job. Verified
locally against ecCodes as well as in CI: an eastward stroke decodes as
u=+25/v=0, a northward stamp as u=0/v=+18, and a third-party decoder computes
the meteorological direction as 270 degrees for a flow toward the east — the
check that catches a transposed `u`/`v` or an inverted convention, both of which
produce a file that parses perfectly and is entirely wrong. 173 Rust tests
and 35 frontend tests pass, clippy clean at `-D warnings`. The map renders the
bundled basemap, the tile pipeline, both glyph styles, and the graticule;
glyph directions were checked against sampled field values numerically, the
dateline is seamless, and the draw path costs 0.12 ms/frame at 2880x1684
against a 16.7 ms budget. M3 (the evaluation engine) is next.

**Design correction found during M3.** The circle tool's rotation was specified
as sugar over `curl`, but curl *adds* to an object's base direction — and the
circle has no direction property, so that base defaulted to north and a
"rotating" circle came out drifting northward. Rotation is now its own direction
mode, with divergence and curl remaining available on top to make a circle
spiral. The circle's default curl changed from 1.0 to 0.0 to match.

**Performance, closed.** A dense tile now costs 3.9 ms on the GPU and 23.3 ms on
the CPU, both inside their budgets in spec.md 13. Three separate fixes got
there: `[profile.dev]` was missing so dev builds ran 2.4x slower than release;
the evaluator's chunk size was a constant that starved small batches of threads
(D20); and the coarse preview refines on non-linearity rather than variation.
All three are guarded by `tile_cost`.

**The brush's full option set landed** (spec.md 6.2), after M5: brush shape
(circle or square), a size enterable in px or km, and a direction that is
either a fixed bearing or aimed at a point. Only the fixed bearing and a round
stamp existed before; the other three properties were in the schema table and
read by nothing. The square stamp is a new `Shape`, so it went through the
whole "changing a shape's geometry" recipe — `sdf.rs`, the WGSL kernel, the
fidelity suite's generator, the cache hash. It is swept in the Chebyshev
metric, which is what makes its coverage exact on both backends rather than a
polygon approximation each has to reproduce (spec.md 7.3). Measured on the
dense tile: 22.7 ms/tile standard on the CPU and 4.9 on the GPU, against 25.0
and 4.8 for the round brush — no change worth the name, and both well inside
the budgets. `brush_shape` also moved off `fill_mode`, which the circle stamp
uses for something else entirely, hence schema version 3 and a migration.

**M5 complete.** The layer panel, the object list, the schema-driven inspector,
click-to-select with hit testing, on-map transform handles, copy/cut/paste with
both timing modes, and the history panel all landed, and every stated acceptance
criterion is met — including that a 90° rotation at 60°N turns the object about a
true bearing rather than shearing lat/lon space. Multi-selection, the marquee
with its cross-layer modifier, and drag-to-reorder in the layer panel closed
afterwards; the collective-centroid transform is a rigid sphere rotation, so a
group keeps its shape.

**A transform drag previews rather than writes** (D29). Applying it on every
pointer report bumped the revision, the revision addresses every tile, and each
report therefore asked for a full viewport re-render measured at 109–2000 ms per
24 tiles. The renders never completed and the object appeared to move only on
release. The pointer now follows a read-only preview costing 0.036 ms per report,
and the document is written once, when the pointer comes up. The preview and the
write share one `placement_of`, so the outline cannot land anywhere but where the
release puts it.

**The brush's option set is closed** (spec §6.2). Added after M5: `stamp_space`
(D28) — a size in px now paints a stamp that is a circle on the *map* at any
latitude, since a ground disc is an ellipse there and no px number describes it;
`away_from_point`, the reciprocal of `toward_point` at every cell; and the aim
point is now placeable by pointing at it, on the tool and on an existing object.
Removed: `divergence` and `curl`, which are defined against an anchor the brush
does not meaningfully have (schema version 4 migrates them off existing
strokes). `brush_shape` and `stamp_space` are **creation-only**: they are the
stamp's geometry, and changing one afterwards re-makes the object into something
nobody drew.

**Two bugs worth remembering.** The inspector showed the *stored* azimuth where
every other direction input shows the project's convention, so a stroke painted
at 270 read back as 90; the schema now marks which angles are flow directions,
because converting all of them would flip every object's rotation instead. And
`[profile.dev] opt-level = 1` made incremental rebuilds of `ve-app` produce an
rlib whose codegen units reference each other's internalised `.llvm.*` symbols,
failing every link — including `tauri dev`, on every edit. The app crate now
builds at `opt-level = 0`, which costs nothing: its hot loops are all in
`ve-render` and `ve-core`.

**Second correction to M1.** Its deliverables list "clipboard model with
keyframe remapping (spec.md 8.5)", and that was not built either — only the
command layer. Closed during M5: `ve_core::clipboard` now handles fresh
identities, relative and absolute timing, keyframe shifting with clamping, and
the nudge that stops a copy hiding underneath its original. This is the second
M1 line item that turned out to be model-only; the first was the project UI.

**Correction to M1.** Its deliverables list "New / Open / Save / Save As /
Recent Projects", and that was originally delivered as the persistence layer
only -- no commands, no UI, with `ve_core::io` never called from the app. The
app launched straight into a map with no document behind it, contradicting
spec.md 2. Closed afterwards, before M3: project commands over the existing
`ve-core::io`, a start screen with creation and recent files, and the map now
takes its timeline, direction convention, colour scale and glyph styles from
`ProjectSettings` instead of constants.

Two findings worth carrying forward:
- `serde_json`'s float parser is one ULP off on ~10% of `f64` values, which
  broke the byte-identical round trip until `ve-core::canonical` quantised the
  affected fields. See decision D17.
- WKWebView suspends `requestAnimationFrame` when the window is not being
  composited, so any redraw scheduler needs a timer fallback. This will matter
  again for M7's background frame rendering.

---

## 1. Sequencing strategy

The build is organised around getting a **walking skeleton** working end to end
as early as possible: create a project → paint one brush stroke → see it on the
map → export a valid GRIB2 that a third-party tool can read. That path touches
the document model, the evaluation engine, the tile pipeline, and the GRIB
writer — every architecturally risky piece. Everything after M4 is breadth on a
proven spine.

The two riskiest components are front-loaded deliberately:

- **The evaluation engine (M3)** carries the GPU/CPU parity requirement, the
  AEQD rasterisation model, and the clone-stamp nested evaluation. If any of
  those is wrong, it is wrong for every tool.
- **The GRIB writer (M4)** is the actual product. A file that a routing engine
  refuses to open makes everything else worthless, so it gets validated against
  external decoders before any breadth work begins.

Tool breadth (M6), timeline (M7), and route solving (M9) are comparatively
low-risk once the spine holds.

```
M0 ─ M1 ─ M2 ─ M3 ─ M4 ══ walking skeleton complete
                    │
                    ├─ M5 ─ M6 ── tools & editing
                    ├─ M7 ─────── timeline & animation
                    ├─ M8 ─────── measurement
                    └─ M9 ─────── sailboat routes   (needs M6 curve tool)
                                        │
                                       M10 ── hardening & release
```

M5–M8 are largely parallelisable once M4 lands. M9 depends on the curve tool
from M6. M11 (projections) is independent of all of it and deliberately last:
it touches only the view, and is best attempted once the editing surface has
stopped changing shape.

---

## 2. Milestones

Each milestone lists its deliverables and the acceptance criteria that decide
whether it is done. "Done" always includes: tests pass, `clippy -D warnings`
clean, and the perf budgets touched by that milestone (§13 of the spec) are
measured, not assumed.

---

### M0 — Foundations

**Goal:** an empty but correctly-shaped repository that builds and ships on all
three platforms.

**Deliverables**

- Cargo workspace with the crate layout from `CLAUDE.md`.
- Tauri 2 shell; React + TypeScript + Vite frontend, fully offline-bundled (no
  CDN references anywhere).
- CI: fmt, clippy (`-D warnings`), test, and build on macOS (arm64 + x86_64),
  Windows, Linux.
- `tracing`-based logging to file and console; log location surfaced in the UI.
- Error taxonomy: `thiserror` in library crates, `anyhow` at the app boundary,
  a serialisable `AppError` for IPC.
- Application directory layout: config, cache, autosave, logs.
- IPC scaffolding with a typed command registry and generated TypeScript types
  (`ts-rs` or `specta`) so the frontend never hand-writes Rust-shaped types.

**Acceptance**

- `cargo test && cargo clippy -- -D warnings` clean.
- A signed-nothing dev build launches to an empty window on all three platforms.
- Adding a Rust type and regenerating TS bindings is a documented one-liner.

**Risks:** Tauri 2 + Vite offline bundling has sharp edges around asset
protocols; validate early that no request escapes to the network (verified by a
CI test that fails on any outbound socket attempt).

---

### M1 — Document model, persistence, history

**Goal:** the full data model exists, round-trips to disk, and is editable with
undo/redo — before any of it is visible.

**Deliverables**

- All types from spec §4: `Project`, `Layer`, `Object`, `Geometry`,
  `PropertyMap`, `Animatable<T>`, `Keyframe<T>`.
- Property system: typed property ids per tool, value enum, per-type
  interpolation (spec §4.5) including shortest-arc angles and great-circle
  `LonLat` slerp.
- `.veproj` ZIP container read/write; canonical JSON with stable key ordering.
- Migration framework keyed on `schema_version`, with a no-op v1 migration and a
  test proving the harness runs.
- New / Open / Save / Save As / Recent Projects.
- Autosave and crash recovery.
- Undo/redo engine: command trait with inverses, coalescing for drags and
  scrubs, history panel data model.
- Clipboard model with keyframe remapping (spec §8.5).

**Acceptance**

- Property-based test: any generated `Project` survives save → load → save with
  byte-identical JSON.
- Property-based test: any sequence of N commands followed by N undos restores
  the exact starting document.
- Interpolating an angle from 350° to 10° yields 0°, not 180°.
- A 5,000-object project opens in ≤ 500 ms.

**Risks:** the property system's ergonomics determine how painful every later
tool is. Prototype the "add a property" path (see `CLAUDE.md` recipe) before
committing to the design.

---

### M2 — Map view

**Goal:** a pannable, zoomable global map with a basemap — no field data yet.

**Deliverables**

- Build-time converter from Natural Earth shapefiles/GeoJSON to a compact
  binary of tessellated land triangles + coastline strips, at 110m and 50m.
- WebGL2 renderer: equirectangular projection, basemap with zoom-driven LOD,
  procedural graticule, longitude wrap.
- Camera: continuous zoom, longitude-wrapping pan, latitude clamp, inertia.
- Tile pyramid addressing and the `ve-tile://` custom URI scheme, served with a
  stub evaluator returning a synthetic test field.
- Speed colour ramp + legend; instanced glyphs sampling the tile texture
  in-shader; adaptive glyph spacing.
- **Both glyph styles** (spec §5.3): arrows with optional speed-scaled length,
  and wind barbs with variable per-instance flag geometry (half-barb / full
  barb / pennant), switchable from display settings. Barbs are hidden for
  current projects. The variable-geometry instancing for barbs is the larger of
  the two and should be scheduled as its own task, not folded into "glyphs."
- Cursor readout: lon/lat, grid cell index, speed, direction in the project's
  display convention.
- Stale-tile dimming and a rendering activity indicator.

**Acceptance**

- 60 fps sustained pan and zoom with the synthetic field.
- Dragging west across the antimeridian is seamless — no seam, no jump.
- Glyph direction visually matches the readout at ten spot-checked locations,
  including near both poles.
- Barb flag counts match the conventional 5/10/50 kt encoding across the full
  speed range, verified against a reference chart; barbs are absent in current
  projects.

**Risks:** the shader-sampled glyph approach is the main unknown; fall back to
CPU-generated glyph instances if in-shader sampling proves awkward, at some
frame-rate cost.

---

### M3 — Evaluation engine

**Goal:** the field is real. One tool (brush) paints, both backends agree, tiles
come from the cache.

**Deliverables**

- Scene flattening (spec §7.1) producing a `FlatScene` buffer whose layout is
  shared verbatim between Rust and WGSL.
- AEQD frame construction and geodesic primitives: distance, initial bearing,
  destination point, rhumb distance/bearing.
- SDF implementations for capsule chain, disc, annulus, polygon, corridor.
- Feather, divergence, curl, direction modes (`Constant`, `TowardPoint`).
- Compositing with overwrite semantics; clone-stamp nested sub-scene evaluation
  with depth cap (scaffolded here, wired to the tool in M6).
- Spherical-cap culling with correct pole and antimeridian row-range clamping.
- **Preview decoupling** (decisions D18, D19): tiles evaluated on a coarse
  lattice and interpolated, dropping coarser still while the camera moves, with
  the sample density independent of the project's grid resolution.
- `FieldEvaluator` trait; `CpuEvaluator` (rayon) and `GpuEvaluator` (wgpu
  compute, WGSL); adapter selection, capability check, and fallback path.
- GPU/CPU parity test harness (spec §7.9).
- Content-hashed render cache: BLAKE3 keys, on-disk store, LRU eviction, cap.
- Brush tool wired end to end: paint on the map, see the field update.

**Acceptance**

- Fidelity: across 1,000 randomised scenes the two backends agree to within
  max(0.25 m/s, 2%) of speed and 2° of direction, and agree on structure —
  rotation sense, band latitudes, footprint extent (spec §7.9). Deliberately
  perceptual, not numerical: see D18.
- A 0.1° project renders the viewport in the same time as a 1° one, measured,
  not assumed (D19).
- `cargo test -p ve-render --test tile_cost` stays inside the 40 ms CPU-fallback
  budget in release.
- `VE_FORCE_CPU=1` produces a working app; the About box reports the active
  backend and any fallback reason.
- A brush stroke crossing the antimeridian renders continuously.
- A brush stroke centred on the north pole renders without artefacts.
- An object at 100% scale with a 500 km radius measures 500 km at the equator
  and at 70°N (verified with the geodesic primitives).
- Benchmarks recorded for: 0.1° full-grid CPU evaluation, GPU tile p50/p95, and
  clone-stamp nested evaluation at depth 4.

**Risks:** highest-risk milestone. Nested clone evaluation could blow the tile
budget; if the depth-4 benchmark misses, reduce the cap and document it. Keeping
the WGSL and Rust kernels in step is an ongoing tax — the parity test is the
control, and it must run on every PR.

---

### M4 — GRIB2 writer and export · **walking skeleton complete**

**Goal:** produce a file that real tools open correctly.

**Deliverables**

- `ve-grib`: section builders 0–8, templates 3.0 / 4.0 / 5.0, simple packing.
- Message assembly for both field kinds and their level encodings (spec §12.1).
- Streaming writer — one message at a time, constant memory.
- Minimal GRIB2 reader in the test suite for round-trip verification.
- Export UI: forecast start time, name, location, centre code, size and duration
  estimate, progress, cancel, atomic temp-file rename.
- Golden fixtures at 1° committed for byte-level determinism tests.
- CI job decoding output with `wgrib2` and ecCodes and diffing inventories.

**Acceptance**

- `wgrib2 -V` and `grib_dump` both parse every generated message without
  warnings.
- Round-trip: decoded values match source within packing tolerance for all four
  resolutions.
- A wind GRIB opened in an independent viewer shows arrows in the direction the
  app displayed them — this specifically catches a `u`/`v` swap or a
  from/toward inversion, the two most likely silent bugs.
- Byte-identical output across macOS, Windows, and Linux CI runners.
- Cancelling mid-export leaves no `.grib2` file behind.
- **End to end:** new project → one brush stroke → export → open in an external
  tool and see the painted feature.

**Risks:** GRIB2 has many ways to be subtly non-compliant that still parse. The
external-decoder CI job is the mitigation and should land with the first
message, not at the end of the milestone.

---

### M5 — Object editing UX

**Goal:** objects become manipulable first-class citizens.

**Deliverables**

- Layer panel: create, delete, rename, reorder by drag, visibility, lock.
- Object list per layer with rename, reorder, enable/disable, delete.
- Hand tool; selection and multi-select; marquee; cross-layer modifier.
- Transform handles: move, rotate about the anchor, scale, draggable anchor,
  multi-selection about the collective centroid.
- Property inspector, driven by the property system so it is generated per
  tool rather than hand-written per tool.
- Copy / cut / paste / duplicate UI with both paste-timing modes.
- History panel with jump-to-entry.
- Hover indicators for tools that specify them.
- Overlapping strokes with identical properties merge into one object
  (spec §6.1), including the schema migration that gives a stroke several
  chains.
- New Project and Open without leaving the app, guarded by a Save / Don't save /
  Cancel prompt (spec §4.7). No Close button: the window's own close is the way
  out, and the command it used remains behind New and Open.
- An active layer: new objects join it, and a plain marquee is scoped to it
  (spec §6.1, §8.2).

**Acceptance**

- Rotating an object 90° at 60°N rotates it about a true bearing, not in lat/lon
  space (verified against a reference computation).
- Pan and zoom leave every object visually pinned to the earth.
- Undo restores transforms, reorders, and visibility exactly.
- Paste into a different layer and a different time step behaves per spec §8.5
  in both timing modes.
- Painting the same stroke twice over an area leaves one object, and the gap
  between two merged strokes stays unpainted on both backends.
- Creating or opening a project over unsaved changes is refused by the backend,
  not merely discouraged by the dialog, and "don't save" followed by cancelling
  the file dialog leaves the project open and unsaved.
- A group dragged across the globe keeps the distances between its members, a
  group rotation lands them where a rigid turn would, and one undo returns all
  of them.
- Repeating a drag update with an unchanged pointer position changes nothing.
- Moving an object's anchor leaves its strokes covering exactly the ground they
  covered before.
- A new project never shows the previous project's field.

---

### M6 — Remaining tools · **complete**

**Goal:** the full tool catalogue from spec §6.2.

**What it turned out to be.** Most of the *evaluation* already existed: every
shape, speed mode and direction mode landed in M3, so the circle stamp's
gradient, the shape fill's ramp and the curve's along-path flow needed no new
kernel work. What was missing was the model's account of what each tool
*offers* — the eraser and the clone stamp had no `brush_shape` or `stamp_space`,
the shape fill no `shape_source`, the curve no `curve_kind`, and only the brush
had any dependency rules — and the interaction that drives it, which existed
only for the brush.

The milestone's real content is therefore D32: the shared behaviour became
shared code rather than a checklist. See the status notes at the top of this
file for the five bugs that surfaced along the way, four of them pre-existing.

**Measured on completion.** 542 Rust tests and 256 frontend tests pass, clippy
clean at `-D warnings`. The fidelity suite compares 30,647 samples across 120
generated scenes — worst speed error 0.108 m/s against a 0.25 tolerance, worst
direction 0.186° against 2° — with 73 samples (0.24%) exempted at a path tangent
tie. Tile cost is unchanged: 3.87 ms/tile exact on the CPU and 4.20 on the GPU.

**A divergence still open.** Spec 8.1 describes a *left-hand vertical* palette;
the implementation has it horizontal, in the toolbar across the top of the map,
which is also where 5.5 puts the option bar. The two need reconciling — either
the spec follows the build or the palette moves — and the icons make the
question live, since a vertical strip of marks is what 8.1 was describing. Not
touched here: it is a layout change, and nobody asked for one.

**A handle bug found by hand, after the fact.** The drag preview's handles
were built by one function and the selection's by another, and the preview
scaled a reach that already carried the object's scale. At 100% the two agreed
by coincidence; on any object that had ever been scaled, the dashed circle and
both knobs jumped the instant it was grabbed and snapped back on release. Two
frontend faults compounded it: the handles were drawn from the live preview but
hit-tested from the document, so after a release you grabbed where the knob
*had* been; and the held preview retired when the tiles landed, independently
of the refetched handles, so with a warm cache they flashed back to the old
position and forward again. All three are closed — one source for "the handles
as shown", used for drawing and grabbing alike; the held preview waits for the
handles to catch up; and `selection.rs` now pins the preview to the committed
handles at 40%, 100% and 250%. The write and the gesture's end are sequenced
too, rather than raced.

**A performance pass over the UI, with no behaviour change.** Three structural
costs, each paid on every pointer move whatever the tool: an IPC field sample
whose result was `MapView` state, so every report re-rendered the whole
component to refresh the readout; an overlay drawn synchronously per report,
twice per frame on a fast drag; and a stroke preview rebuilt from its first
point on every report, quadratic in the stroke. The readout is an external
store now and only its own component renders; overlay redraws wait for the
frame and stand down behind a pending GL frame; and the preview's path and
lattice are extended by the new segment — the same walk resumed, held equal to
a rebuild by 36 op-for-op tests. Measured on the pure geometry: a 1000-point
drag fell from 580 ms (1,001,000 stamps) to 1.0 ms (2,000). The canvas fill,
which cannot be timed here, scales the same way. Also: the renderer computes
each camera's visible tiles once per frame rather than once per pass, and the
option bar is memoised.

**Left open.** Nobody has clicked through the six tools in the running app; the
app builds, starts and loads the frontend cleanly, and the tools are covered end
to end by tests, but the interaction itself is unexercised by hand. That is the
first thing M7 should do. The gesture state machine — what a press, a move and a
release do — is now a pure module with its own tests rather than something only
a pointer could reach, which narrows the gap considerably but does not close it.

**Deliverables**

- Circle stamp, including `ScreenCircular` vs `GeodesicCircular` and the
  gradient fill.
- Shape fill: freehand polygon plus square / rectangle / circle presets;
  constant and gradient vector modes.
- Eraser.
- Clone stamp, including source-point picking and `Aligned` / `Fixed` offset
  modes, wired to the M3 nested evaluation.
- Curve tool: polyline and cubic Bézier, with `Absolute` and `RelativeToPath`
  direction modes.
- Each tool implemented in **both** kernels, with parity tests extended.
- **Every tool inherits spec §6.1's shared behaviour**, which is the checklist
  for calling one done. Not one of these is the brush's own; each was written
  after a bug in the brush, and the point of writing it down is that the next
  tool starts where the brush finished:
  - Its option bar spans the map and wraps (§5.5).
  - A size in px selects `stamp_space: projected`, km selects `geodesic` — one
    property, one vocabulary, every sized tool (§3.5).
  - Its gesture previews the *field*, in the speed colour with direction
    glyphs, and the preview is held until the new tiles are drawn (§6.1).
  - Every `LonLat` option is placeable by pointing, on the tool and on the
    object (§6.1).
  - The shared direction modes mean the same thing everywhere; a tool may add
    its own — the curve does — but may not redefine a shared one (§6.2).
  - Flow directions display in the project's convention wherever they appear;
    geometric bearings do not (§3.3).
  - Options the object's mode never reads are not shown; options that define
    what the object *is* are creation-only and refused at the write path (§6.1).
  - Move, rotate, scale and re-anchor work identically, because they act on the
    frame rather than the shape — including the drag preview (§8.2).
- **The eraser and the clone stamp are brush-like by construction**, not by
  resemblance: they share `Geometry::Stroke`, the swept-stamp SDFs and the
  preview path, so they inherit `brush_shape` and `stamp_space` and any
  difference between them and the brush is a bug in one of them.

**Acceptance**

- Every tool round-trips through save/load with all options intact.
- Parity tests cover every tool and every option range.
- A `GeodesicCircular` circle at 80°N renders as a true constant-radius cap.
- `RelativeToPath` at 0° produces flow along the curve; at 90°, across it.
- `edge_mode` implemented in both kernels, defaulting to `Blend` (spec §7.4). A
  test asserts `Blend` and `Replace` produce identical output over a calm
  background and divergent output over a non-calm one — this is the property the
  default rests on.
- **The cross-tool rules are tested per tool, not assumed.** For each tool:
  - A shape sized in px is the same number of pixels across *and* tall at 0°,
    45° and 70°; the same size in km is a constant ground distance at all three.
    This is one assertion parameterised over the catalogue, not one per tool.
  - `TowardPoint` and `AwayFromPoint` are exact reciprocals at every cell, for
    every tool that has a `target`.
  - Moving, rotating and scaling an object leaves its footprint where the
    handles said it would, for every geometry kind — the drag preview and the
    write are built from one placement, so a new geometry that previews wrongly
    fails this without a bespoke test.
  - The inspector lists exactly the options that are live and editable: nothing
    the mode makes inert, nothing creation-only.
- A tool whose option bar does not fit the map at 1280 px wide, wrapped, is not
  done.

---

### M7 — Timeline and animation · **complete**

**Goal:** everything animates; playback is smooth; readiness is visible.

**Against the acceptance list.** Position, scale, rotation, an enum and a
boolean each have a test asserting spec 4.5's rule for that kind, read back
through the evaluator. An object alive for three steps invalidates exactly
three frames, by hash and by the pool's readiness alike. Playback holds and
reports buffering rather than advancing into anything but a solid frame
(`playback.test.ts`). A warm tile is served in 63 µs. The step-count shrink
reports an exact count and names the objects, deletes exactly that, clamps
lifetimes, and undoes to an identical document. Three decisions recorded as
D39–D42.

**Deliverables**

- Timeline dock: ruler with forecast-hour and UTC labels, scrub, step selection.
- Collapsible layer → object → property track tree.
- Object `active_range` bars with draggable ends.
- Keyframe editing: add, move, delete, box-select, per-segment interpolation.
- Auto-key mode and explicit per-property key buttons.
- One write rule for the inspector and the map's transform drags: an animated
  property keys the current step, an unkeyed one edits its base unless auto-key
  is on (spec §9.3, D42). Found by hand: the first version of the drags
  rebuilt the property as a constant and erased every key.
- Playback: play, pause, stop, loop, configurable rate, buffering hold.
- Background render worker pool with the priority policy from spec §9.5.
- Per-step readiness indicators including the distinct **stale** state.
- Cancellation and requeue on edits, driven by content-hash invalidation.

**Acceptance**

- Animating position, scale, rotation, and at least one enum and one boolean
  property each behaves per spec §4.5.
- Editing an object with a 3-step `active_range` invalidates exactly 3 frames,
  demonstrating hash-based invalidation works.
- Playback holds rather than stutters when frames are not ready.
- Time-step switch with a warm cache ≤ 50 ms.
- Reducing `step_count` prompts with an accurate keyframe count and object list,
  deletes on confirm, clamps affected `active_range`s, and is undoable in-session
  (spec §4.1).

**Risks:** the readiness indicator is only trustworthy if invalidation is exact.
This milestone is where the content-hash cache design gets proven; budget time
for a dedicated invalidation test suite.

---

### M8 — Measurement tools

**Goal:** spec §10, complete.

**Deliverables**

- Dividers with draggable multi-point chains, per-segment and total distance,
  bearings, km and nm.
- Great circle + rhumb line pair between two points, both drawn and labelled.
- Range rings with editable centre, interval, and count.
- Per-tool clear plus global clear-all; persistence in `Project.annotations`.

**Acceptance**

- Distances match an independent reference implementation within 0.1% for a set
  of known pairs, including antimeridian-crossing and near-polar cases.
- The great circle and rhumb line visibly diverge on a long high-latitude pair,
  and the rhumb line renders as a straight line in equirectangular projection.
- Annotations survive save/load and never appear in exported GRIB output.

---

### M9 — Sailboat route definition

**Goal:** spec §11, complete. Depends on M6's curve tool.

**Deliverables**

- Polar parsing (CSV / `.pol`), bilinear interpolation, embedding in the project.
- Two or three bundled sample polars.
- Route mode state machine and its map interaction layer.
- Reachability disc computation and click rejection outside it.
- Route tree editing: add, move, delete nodes and branches; post-solve editing.
  Node `Id`s must be stable across every edit short of deletion — leg identity
  depends on it (spec §11.6).
- Polar inversion with the documented selection strategy (spec §11.4), behind a
  named-enum strategy interface.
- Generation of tagged curve objects into an auto-created `Routes` layer.
- **Merge-on-re-solve machinery** (spec §11.6): per-property user-modified
  flags, the solver-owned / user-owned split, leg identity keyed on
  `(parent_node_id, child_node_id)`, and "detach from route."
- `max_tws` defaulting to the polar's own maximum TWS, displayed in the route
  panel and user-adjustable.
- Conflict detection and a route report, including solver-owned overwrites and
  legs solved above 35 kt.
- Infeasible-leg flagging.

**Acceptance**

- Round-trip test: define a 3-branch tree, solve, export the GRIB, then run a
  simple built-in forward simulation using the same polar and confirm every
  defined route is achievable within the safety margin. **This is the feature's
  real acceptance test** — everything else is UI around it.
- Clicking outside the reachability disc is rejected with an explanatory
  message.
- Re-solving is idempotent. Specifically: edit `width_km` on a leg's object,
  move that leg's child node, re-solve — the width survives and the wind is
  recomputed. Then delete a different node and confirm only its own legs'
  objects are removed.
- The feature is entirely absent in current projects.

**Risks:** the polar inversion is under-determined and the chosen selection rule
is a judgement call. The forward-simulation acceptance test is what keeps it
honest. The merge behaviour is the second risk — it is safe only as long as node
`Id` stability holds, so that is worth an explicit test rather than an
assumption.

---

### M10 — Hardening and release

**Goal:** shippable.

**Deliverables**

- Perf pass against every budget in spec §13; profile and fix the misses.
- Memory pass: peak RSS, cache eviction under sustained editing, leak checks.
- Crash recovery verified by killing the process mid-edit and mid-export.
- Large-project stress: 5,000 objects × 240 steps.
- Packaging: macOS universal `.dmg` (signed, notarised), Windows MSI, Linux
  AppImage/deb.
- Offline verification test: run the packaged app with the network disabled and
  confirm every feature works.
- User documentation and 3–4 sample projects, including one demonstrating the
  route feature.

**Acceptance**

- All spec §13 budgets met or consciously renegotiated with a recorded reason.
- Packaged apps launch and pass a smoke suite on clean machines.
- Zero network traffic observed during a full feature exercise.

---

### M11 — Alternative map projections

**Goal:** the map can be drawn in projections other than equirectangular,
including a globe.

Sequenced after the editing surface has stopped changing shape. Nothing below
the view is coupled to the projection — objects are stored in geodesic AEQD
frames, the evaluator works in lat/lon and true bearings, and the GRIB writer
has its own grid — so this cannot affect a stored project or an exported file
(invariant 3). There is no migration and no fidelity question; the cost is
entirely in the frontend.

**Four coupling points**, all in `ui/src/map`:

| | assumes today |
|---|---|
| `geoToScreen` in the shaders | one linear transform, shared by four programs |
| `project` / `unproject` in `camera.ts` | a closed-form inverse pair |
| tile quads in the renderer | a lat/lon rectangle is a screen rectangle |
| `worldOffsets` | the world repeats horizontally |

**Deliverables, in increasing cost**

- **Cylindrical (Mercator, Miller).** A lat/lon rectangle is still an
  axis-aligned screen rectangle, so tiles stay two triangles, wrapping is
  unchanged and the inverse is closed-form. Roughly a day. Mercator cannot
  reach the poles, so latitude clamps near 85° and the top and bottom grid rows
  sit off-screen — worth stating in the UI rather than leaving a user to notice.
- **Pseudo-cylindrical (Mollweide, Robinson, Winkel Tripel).** Lat/lon
  rectangles become curved quads, so tile quads and basemap triangles both need
  subdivision. Robinson and Winkel have no closed-form inverse, and `unproject`
  runs on every pointer move *and* on every painted point, so the Newton
  iteration has to be fast enough for painting rather than just for a readout.
- **Globe (orthographic).** Everything above, plus back-face culling, tile
  selection over a spherical cap instead of a lat/lon box, an interaction model
  that rotates rather than pans, and a glyph lattice whose on-screen spacing
  varies across the disc — which makes the current "pick a step near 40 px"
  rule meaningless as written.
- **Basemap edge subdivision**, in the builder. Long edges must become arcs.
  The concrete case is Antarctica: its ring runs to 180°, drops to 90°S and
  crosses the pole edge to −180°. Under equirectangular that edge is a straight
  line along the bottom and renders correctly; under any curved projection it is
  a 360° span that renders as a wedge unless subdivided.

**Acceptance**

- Switching projection never alters a project or an export: the same document
  exports byte-identical GRIB2 in every projection.
- Painting is accurate in each: a stroke drawn at 60°N lands where the cursor
  was, verified by sampling the field at the click positions.
- Antarctica, the dateline and both poles render without artefacts in every
  projection — the three cases that have already caught bugs twice.
- On the globe, the far hemisphere is not painted through.

**Risks:** `unproject` is on the painting path, not just the readout, so an
iterative inverse must be fast and must converge everywhere — including at the
poles and at the projection's own edge cases, where these formulations tend to
be least well behaved.


---

## 3. Testing strategy

| Layer | Approach |
|---|---|
| Document model | Property-based (`proptest`): save/load round-trip, undo/redo inverse, interpolation edge cases. |
| Geodesy | Comparison against reference values, with deliberate antimeridian and polar cases. |
| Evaluation | GPU/CPU parity on randomised scenes; golden raster snapshots at 1° for regression. |
| GRIB | Self-round-trip via the in-test reader; external decode by `wgrib2`/ecCodes in CI; committed byte-level golden files. |
| Routes | Forward simulation against the same polar. |
| Frontend | Component tests for the timeline and layer panel; Playwright smoke over the packaged app. |
| Performance | Criterion benchmarks in CI with regression thresholds on the spec §13 budgets. |

**A test that must exist from M0:** an assertion that the app makes no outbound
network connections. Invariant 5 is easy to violate accidentally by pulling in a
font or a map style from a CDN.

---

## 4. Risk register

| Risk | Impact | Mitigation |
|---|---|---|
| GPU/CPU kernels drift as tools are added | Preview stops matching export; user distrust | Parity test on every PR; the "add a tool" checklist in `CLAUDE.md` requires both kernels in one commit |
| GRIB output subtly non-compliant | The product doesn't work | External decoder validation in CI from the first message; independent-viewer visual check |
| Clone stamp nested evaluation too slow | Missed tile budget | Benchmarked in M3 before the tool is built in M6; depth cap is the release valve |
| 0.1° export memory or duration blowout | Unusable at the highest resolution | Streaming writer, one message at a time; size estimate shown before the user commits |
| Cache invalidation wrong | Readiness indicator lies; stale frames exported | Content-hash keys make invalidation structural rather than manual; dedicated invalidation suite in M7 |
| Direction convention inverted somewhere | Silently wrong GRIBs | Conversion at exactly one boundary (spec §3.3); the M4 independent-viewer check targets this specifically |
| Property system too rigid for later tools | Painful retrofit across all tools | Prototype the "add a property" path in M1 before the design is locked |
| wgpu unavailable on a user's machine | App unusable | CPU fallback exists from M3, is exercised in CI, and is user-forceable |
| Natural Earth asset size bloats the bundle | Slow install | Build-time conversion to a compact binary; measure the delta in M2 |

---

## 5. Decisions log

Architectural choices already made, with their reasons, so they are not
relitigated by accident.

| # | Decision | Rationale |
|---|---|---|
| D1 | Rust owns the entire domain; React/TS is a view layer only | Keeps evaluation, model, and encoding testable without a browser; the webview holds no truth |
| D2 | wgpu compute for preview, CPU rayon for export | GPU floats aren't bit-identical across vendors; export bytes are the product and export may be slow (spec §7.8) |
| D3 | Equirectangular projection | The grid is global lat/lon; Mercator cannot show the poles that a global GRIB requires |
| D4 | Bundled Natural Earth basemap | Invariant 5 — no tile server, no network |
| D5 | Object-local AEQD frames + SDF rasterisation | Makes the antimeridian and poles ordinary cases instead of special cases (spec §7.2) |
| D6 | Content-hashed render cache | Invalidation becomes structural, not hand-maintained; enables trustworthy readiness indicators |
| D7 | `.veproj` = ZIP + canonical JSON | Diffable, recoverable, portable; no rasters (invariant 1) |
| D8 | Grid resolution, field kind, step size immutable after creation | Changing them would invalidate every object's grid relationship |
| D9 | px sizes resolve to km at creation | Objects must stay pinned to the earth under zoom (spec §3.5) |
| D10 | One gesture = one object; options freeze at creation | Directly from the requirement that changing a tool option produces a new object |
| D11 | GRIB reader lives only in tests | Invariant 5 — no runtime dependency on ecCodes or any external decoder |
| D12 | `edge_mode` defaults to `Blend` | Identical to `Replace` over calm areas, so the default only governs soft edges over existing data — where fading to calm is never wanted (spec §7.4) |
| D13 | Reducing `step_count` deletes keyframes, behind a quantified confirmation | Keeps the document free of invisible state; the confirmation carries the cost (spec §4.1) |
| D14 | `max_tws` defaults to the polar's own maximum TWS | Uses all real data, never extrapolates past the table; the min-TWS solver means a high cap doesn't generally strengthen fields (spec §11.3) |
| D15 | Route objects are editable, with merge on re-solve | Leg identity is the node-`Id` pair, so topology changes never misattribute edits — this is what makes merge safe rather than fragile (spec §11.6) |
| D16 | Both arrow and barb glyphs ship in v1 | Barbs are the sailing audience's native idiom; the colour ramp still carries unquantised speed, so barbs never become the only reference (spec §5.3) |
| D18 | The view is a proxy: preview fidelity is perceptual, not numerical | No preview pixel can reach an export, so requiring 1e-3 m/s agreement between backends bought nothing and would have forbidden fast approximate trigonometry and coarse-lattice preview evaluation. Tolerance is now set where a difference would be seen: max(0.25 m/s, 2%) and 2° (spec §7.9) |
| D22 | The GPU declines clone-stamp scenes rather than approximating them | A clone stamp reads the composite beneath itself, which needs recursion; a compute shader has none. Declining routes the scene to the CPU, which is the same fallback that covers a machine with no GPU |
| D21 | Preview quality is chosen by backend: coarse on CPU, exact on GPU | Measured on a dense tile: coarse evaluation takes the CPU from 45.8 to 23.3 ms, and the GPU from 3.9 to 5.0 — three dispatches instead of one, when evaluation is nearly free |
| D20 | Evaluator chunk size derives from the thread count, not a constant | A fixed 4,096 left the preview's 4,225-node pass on two threads and its 4,096 cell centres on **one**, while a full tile used sixteen. That alone made the coarse preview slower than evaluating every pixel, for reasons unrelated to how much work it saved |
| D19 | Grid resolution governs the export only, never the preview | Preview tiles are sized by zoom. A 0.1° project must pan and zoom exactly as fast as a 1° one; the finest grid should cost export time and file size, not interactivity (spec §4.2) |
| D23 | A merged stroke holds several chains, and the GPU uploads capsules as segment pairs | Appending one stroke's points to another's would sweep the brush across the gap between them — a line nobody drew. Chains keep them separate; pairs let the shader walk them with no boundary markers, so both backends agree by construction (spec §7.3) |
| D24 | Merging is refused whenever it could change the composite | It needs identical properties *and* overlap *and* nothing overlapping in between in z-order, because absorbing a stroke moves it down the stack. Tidiness is worth nothing if the layer stops looking like what was painted (spec §6.1) |
| D25 | A drag's baseline lives in the session, and the frontend sends absolute pointer positions | The pointer reports the same place many times a second. Computing from a baseline makes every update idempotent, keeps all the geodesy in Rust, and makes the whole drag one `Command::Batch` — so one undo returns a six-object transform (spec §8.2) |
| D26 | The rubber band is `Shift`-drag, not plain drag | Spec §8.1 requires a plain drag on empty map to pan and §8.2 requires a marquee on empty map. A modifier is the only way to have both, and `Shift` is the conventional "extend the selection" key |
| D27 | A project's tile revision is seeded from the clock, not from 1 | Tiles are served `immutable` for a year and addressed by revision. Two documents both starting at 1 share addresses, so a new project was served the previous one's tiles and showed its strokes. A counter would restart with the process; the webview's cache would not (spec §4.7) |
| D31 | One property for "which space is this shape defined in", shared by every sized tool | The circle stamp's `circle_space` (`screen_circular`/`geodesic_circular`) and the brush's `stamp_space` (`projected`/`geodesic`) were the same question with two names and two vocabularies — and opposite variant orders, so the index could not even be carried across. Unified onto `stamp_space` *before* the circle tool ships, because "px" meaning one thing in one tool and another elsewhere is the exact inconsistency the shared-behaviour rules exist to prevent (schema version 5, spec §3.5, §6.1) |
| D30 | ~~The brush has no `divergence` or `curl`~~ — superseded by D38, which removes them from every tool. Its stamp geometry is creation-only | Both components are measured from the object's anchor, and a stroke's anchor is wherever the gesture began — not a centre its flow turns about; the tools that place one keep them. `brush_shape` and `stamp_space` are the stamp's geometry, so changing either re-rasterises the stroke into one nobody drew: they are set at creation and refused afterwards, at the write path and not only in the panel. Removing a property from a tool needs a migration, not just a table edit — `value_at` returns whatever the map holds and only falls back to the schema when the key is absent, so a leftover entry goes on bending a flow nothing shows (schema version 4, spec §6.1, §6.2, §7.5) |
| D29 | A transform drag previews rather than writes, and commits once on release | A write per pointer report bumps the revision, and the revision addresses every tile, so each report invalidated the visible field and asked for a re-render measured at 109–2000 ms per 24-tile viewport. The renders never completed, so the object moved only when the drag ended. A read-only preview costs 0.036 ms per report and re-renders nothing. The preview and the write are built from the same `placement_of`, so the outline cannot land anywhere but where the release puts it (spec §8.2) |
| D28 | A size in px selects a projected stamp, not just a conversion factor | A ground disc is an ellipse on the map, twice as wide as tall at 60°, so no px number describes it — measured against either axis the footprint visibly deforms. Asking in pixels is asking for a shape on the map, so px paints one and km keeps painting a ground shape. The object records which (`stamp_space`), because the two paint different fields and export different GRIBs. This replaces §3.5's horizontal-scale rule, whose reasoning was sound only while the ellipse was accepted as the answer (spec §3.5, §7.2) |
| D32 | One creation command, five gestures, one option bar and one footprint type for the whole catalogue | Spec §6.1's "what a tool inherits" was a checklist, and a checklist worked through by hand is one that gets missed — the brush's bug list is the evidence. The shared behaviour is now shared code: a tool sends a gesture and a set of property values and inherits the aim-mode check, the anchor, the frame, the layer choice, the merge test, the option bar and all three previews. The eraser and the clone stamp are brush-like *because* all three send the same gesture, which is a stronger guarantee than three code paths written to resemble each other (spec §6.1) |
| D33 | The shape fill's presets are dragged out from their centre, and their size is geometry rather than a property | Centre-out gives all three presets the same anchor as the shape they produce, which is what divergence and curl are measured from and what the handles turn about. The size is part of what the user drew, like a polygon's vertices, so it lives in the geometry and is resized by the same scale handle as everything else — where the circle *stamp*'s diameter is typed and therefore an animatable property. `Geometry::Disc` carries an optional radius to say which of the two a disc is (schema version 6, spec §6.2) |
| D34 | A clone stamp never merges | Merging re-expresses the absorbed stroke in the target's frame, and a clone stamp samples at a displacement off its own anchor — a different anchor is a different patch of the field. It is the one exception to D24's rule, and it is stated at the merge test rather than buried in it (spec §6.1, §6.2) |
| D35 | The size's unit is the only control for `stamp_space`, and there is one per tool | The unit already asks the space's question — px is a shape on the map, km one on the ground — so a bar offering both would have two controls for one property, and the space would be the one that did nothing, since the unit is what the gesture freezes. `stamp_space` is therefore not among a tool's options at all; the unit stands in its place and inherits its dependencies, so the shape fill's freehand polygon, which has no size, is offered no unit either. One unit per tool rather than per field for the same reason: a circle with a diameter in px and a ring width in km would be asking for a shape that is on the map in one measurement and on the ground in the other. A tool whose sizes are dragged rather than typed still has the unit, with no number beside it — a drag is a measurement too (spec §3.5) |
| D41 | Nothing in the render pool is cancelled; the queue is replaced | A unit in flight when an edit lands finishes into a content-hashed key that is merely unreachable — one tile of wasted work and no bookkeeping that could be wrong. Cancellation would need the pool to know which units an edit affected, which is exactly the tracking content addressing exists to avoid (spec §9.5) |
| D42 | A change to an animated property keys the current step, auto-key or not | Once a property has a key its base is unreachable (nearest key holds, spec §4.5), so a base write is an invisible edit and an undo entry that does nothing. Keying the current step is the only write that shows on screen. Auto-key therefore only decides what happens to a property that has no keys yet. The rule lives in one function (`animation::written`) so the inspector and the map's drags cannot diverge again |
| D40 | "Stale" is the frontend's memory | The backend reports what the cache holds now; "was solid at an earlier revision, is not at this one" needs the earlier revision, which only the timeline saw. Playback treats stale as not ready: the tiles on screen for it belong to a revision that no longer exists (spec §9.4, §9.5) |
| D39 | The render pool renders through `protocol::serve`, the function that answers the map's own tile requests | The key, the backend, the quality and the encoding are decided once, so a tile rendered ahead is byte for byte the tile the map will fetch and "ready" means "will be a cache hit". A pool that chose any of those differently would fill the cache with tiles nobody asks for and report frames ready that are not (spec §9.5) |
| D38 | No tool has `divergence` or `curl`; an object's direction mode is the whole of its direction | Reverses the half of D30 that kept them on the tools with a centre. What it costs is the spiral: a radial component is what makes a low converge rather than merely turn, so a cyclone is now built from more than one object — a circle for the rotation, a larger one aimed at its centre for the inflow. What it buys is that a direction is decided in one place, and that nothing in the evaluator depends on an object's *anchor* rather than its geometry any more. Measured: GPU–CPU agreement tightened from 0.108 m/s to 0.0009 and from 0.186° to 0.004°, the radial terms having been the main source of `f32` disagreement between the kernels. Removing a property from a tool is a migration, never only a table edit (schema version 7, spec §7.5) |
| D37 | The eraser and the clone stamp are previewed by operating on the map, not by drawing over it | Both are defined against what is already beneath them, so a coloured wash on the overlay would show something neither tool does — and an overlay cannot show a removal at all, being a canvas above the field that can add pixels and never take them away. A gesture with either one becomes a screen-space mask and the map is drawn through it: the eraser's region loses its field, the clone's loses it and gains the source's, through a camera shifted so the source lands under the brush. The basemap is never masked, since something has to be left to see. A tool declares *how* it previews (`PreviewKind`) and supplies a footprint; nothing else about it is per tool (spec §6.1) |
| D36 | Values are quantised where they enter the document, not where they leave it | D17 quantises at the serialisation boundary, which covers a value the user typed and misses one the application computed — a polygon's centroid, a dragged anchor. `PropValue::canonical` now applies in `Animatable`'s writers, which is one place and off the evaluator's per-sample path. Doing it in `LonLat::new` instead would have put a rounding on the clone stamp's inner loop (`ve-core::canonical`) |
| D17 | Document `f64` values are quantised at the serialisation boundary | `serde_json``s parser is one ULP off on ~10% of `f64` values, so raw floats do not round-trip and a project would not equal itself across save/load. Chosen precisions are far finer than anything observable; `f32` is unaffected (`ve-core::canonical`) |

---

## 6. What to settle before coding starts

The specification questions are all resolved (spec §15, decisions D12–D17
above). Two logistical items remain:

- Confirm the Tauri 2 + React + Vite offline bundling story on all three
  platforms in the first days of M0, since it gates everything.
- Decide whether M5–M8 run in parallel (needs more than one contributor) or
  strictly in sequence; the plan is written so either works.

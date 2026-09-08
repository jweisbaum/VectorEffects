# VectorEffects — Implementation Plan

**Companion to** `spec.md`. Section references below point into it.

**Status:** M0 through M7 complete and verified by test (2026-09-03). **The
walking skeleton is closed**: a new project, a painted stroke, and a GRIB2 file
that ecCodes parses and whose values decode to exactly what was painted. **The
tool catalogue is complete**: all six tools of spec §6.2 draw, evaluate, save
and export.

**The plan after M7 was replaced on 2026-09-04.** M7 is closed as delivered,
and the hand-verification pass it left open is no longer a gate on anything.
The measurement tools (M8) and hardening (M10) stand but move
behind a new run of milestones, **M12–M19**, written from a feature request of
that date: a decoder for the forecast files the import still refuses (M12);
copying a GRIB layer's frames between steps (M20, added 2026-09-04 and
sequenced with it); motion vectors from an object's own movement, and one object following another
(M13); a region-selection tool with fill and copy/paste (M14); application
settings with rebindable shortcuts and colour scales (M15); macros — captured
fields kept across projects and re-inserted (M16); the warp and liquify split
(M17); image layers (M18); and an export precision option (M19). Map
projections (M11) are pulled forward into the same run. The seven questions
the re-plan raised — one of them touching invariants 1 and 2 — were put to the
user the same day and are settled: D52–D58 in §5, and §6 records them.

**M12 complete (2026-09-04): every packing the forecast centres ship.** All ten
files in the reference set now import, and every value matches ecCodes. CCSDS
(5.42) and JPEG 2000 (5.40) are decoded by `rust-aec` and `hayro-jpeg2000`,
both pure Rust: no `-sys` crate, no C library, nothing to install on any of the
three platforms. Both were verified bit-exact on the real files before being
chosen, which is what the spike was for — 100% of 1,038,240 values on each
ECMWF-family file and of 2,882,400 on GEM's 0.15° grid.

**What the reference set actually found was a bug in the packing that already
"worked".** Complex packing with spatial differencing (5.3) read the group
lengths for every group *but the last*, whose length section 5 gives
separately. The skipped read left the bit reader seven bits short, and the
octet alignment that follows then started the data one octet early — but only
when those bits crossed a boundary, which is a property of the group count. So
half the messages in the set decoded perfectly and the other half came out as
smooth garbage drifting to ±10⁹, with nothing complaining, because the group
lengths still summed to the value count. Five of the eight messages in the four
GFS-family files were wrong. It had been in the shipped decoder since import
landed and no test could see it: the import suite checks the decoder against
the *writer's* output, and the writer only ever emits simple packing.

That is the whole argument for the fixture set. There are seven packings of one
analytic field in `crates/ve-grib/tests/fixtures` now, six of them the writer's
own output repacked by ecCodes, and the complex one fails against the old code.
The ten real files stay out of the repository and run as an `#[ignore]`d test
keyed on `$VE_TEST_GRIBS`, against 200 spot values ecCodes decoded.

**Measured.** GEM's 0.15° message, the densest in the set at 2,882,400 values,
decodes in 283 ms — inside the one-second budget the milestone set, and the
number that had to be checked before a pure-Rust JPEG 2000 decoder could be
trusted on a file with one message per component per step. The CCSDS files take
about 60 ms per file and the complex-packed ones about 30. 694 Rust tests and
312 frontend tests pass.

**One acceptance criterion was written wrong and is corrected rather than
quietly met**: "no `cc` in `cargo tree`". `blake3` has carried a `cc` build
dependency for its SIMD assembly since M3, by way of `ve-core`. Neither new
codec adds one, which is what the criterion was for.

**M20 complete (2026-09-04): a GRIB layer's frames copy between steps.** A
step of an imported layer can be told which of the file's messages to show —
click a mark, `Shift`-click a run, `Cmd`-`C`, move the playhead, `Cmd`-`V` —
which is the choice §4.8's hold rule took away for good reason and this gives
back as an *instruction* rather than a measurement. `Layer::frame_overrides`
is a sorted list of step numbers and **never a sample**, so the frame served
is one the file already holds and the render cache, readiness and both kernels
were correct without a line changed; two steps showing one message hash alike
and the cache holds one frame for the pair. `Delete` restores a pasted step or
hides a file's own message. Eight acceptance tests in
`ve-app/tests/grib_frames.rs`, nine frontend ones over the mark rules.

**M22 complete (2026-09-04): every grid definition the centres ship.** Asked
for after M21, against a directory of 1,735 sample forecast files. Almost every
regional model runs on a projection rather than on lat/lon, and the decoder
refused all of them by name; five grid definitions were added — rotated
lat/lon (3.1), Mercator (3.10), polar stereographic (3.20), Lambert conformal
(3.30) and NCEP's rotated Arakawa non-E staggered grid (3.32769) — and **all
1,653 GRIB2 messages in the set now parse.** Each is resampled onto the span
of the project's own lattice it covers, and components the file resolved along
the grid's axes are rotated onto east and north at the source, which is the
part that is silently wrong rather than loudly wrong when it is wrong at all.
GRIB edition 1 (82 files), oversized JPEG 2000 codestreams (55) and the
probability product templates (70) are named in M22 as deliberately not done.

**M21 complete (2026-09-04): ICON's icosahedral grid imports.** Asked for
after M12. ICON does not run on a lat/lon grid at all: a message
(template 3.101) is a bare run of 2,949,120 values, a cell count and a UUID
naming the mesh, and says nothing whatever about where any cell is. The
positions are a separate pair of files DWD publishes, and they are now
**bundled** — `tools/icon-grid-builder` converts them into
`assets/icon_grids.bin` at build time, so an ICON import needs no files but
the forecast and touches no network. The asset carries ICON global R03B07 and
the ICON-EPS global R02B06 mesh in 3.5 MB.

An unstructured field is **resampled onto the project's own grid at import**
and is an ordinary raster after that, so the render cache, both kernels and
the exporter never learn that such files exist. That was the design's whole
economy: the alternative, teaching the WGSL kernel to search a point cloud,
would have touched everything.

**The measurement that set the design.** The neighbour search — the three
nearest mesh cells to every target node — costs 494 ms for a global 0.1° grid
and 7 ms for 1°, against the 155 MB it would take to store the weights. So
nothing is stored but the three cell *indices*, delta-encoded, which the
project keeps: 1.9 MB at 0.1° and 0.4 MB at 0.25°. Reopening then costs a
decode and an interpolation rather than the search as well, measured at 48 ms
against 477. The weights themselves are recomputed from the coordinates, which
is cheaper than storing them.

**Checked against an independent implementation**, because nothing here is
checkable by eye: a mesh read in the wrong order still produces a plausible
global field. Eight points — both poles, both sides of the antimeridian —
were interpolated by brute force over all 2,949,120 cells in Python against
ecCodes' own decode, and the Rust agrees to six decimals. The `#[ignore]`d
suite that holds those numbers also proves a reopened neighbour set gives a
field with the same content hash as a fresh search.

The bug the unit tests caught along the way was in the bucket search: near a
pole a latitude band holds fewer longitude buckets than the search ring is
wide, so the ring wrapped onto the same bucket repeatedly and filled two of
the three neighbour slots with one cell. Found by a brute-force comparison at
89.5°N, which is exactly why that test aims there.

**M13, first half, complete (2026-09-04): motion in the field.** An object
that moves can put its own movement into the wind — one switch per track, so a
system that spins and travels can contribute the spin alone (D57). The
velocity is one angular velocity per object, because a position segment is a
great-circle slerp and a turn about the anchor is a rotation about it, and both
are exact at the poles and across the seam where "the anchor's speed and
bearing applied uniformly" is not. Five hand-computed acceptance tests, and the
fidelity suite generates motion on a third of its objects: the real GPU agrees
with the CPU to 0.005 m/s and 0.018° over 30,657 samples.

**M13, second half, complete (2026-09-04): objects that follow objects.** A
follower keeps its offset **in the primary's frame**, so a turning primary
carries it round rather than sliding it sideways; its own keys are kept and go
dormant beneath the link, and unlinking wakes them while holding the value
where it stands (D42). Links resolve once per step for the whole project, in
dependency order, so the field, the map's outlines and hit testing agree about
where a follower is. A cycle is refused when it is made; a link to an object
that is gone is inert; deleting a primary frees its followers in the same
history entry. Six model tests and four end-to-end ones.

**M14, the region, complete (2026-09-04).** The select tool draws a region of
*ground* — rectangle, centre-out circle or lasso — in map space, as session
state rather than document or history, with `Cmd`-`A`, `Cmd`-`Shift`-`A` and
`Cmd`-`D` bound for the first time. Neither it nor the fill tool is in the
backend palette: that palette describes vector-creation tools, and one of these
makes no object while the other makes a shape fill and borrows its bar. Fifteen
tests over the region geometry, which is where the seam and the poles are.

The **fill tool** followed in the same shape: a region already *is* one of the
three things the shape fill draws, so a rectangle region is its rectangle
preset, a circle its circle and a lasso its freehand polygon. No new gesture,
no new shape, no backend change at all — which is what keeps the object a shape
fill and the seven shared-rule tests covering it unchanged.

**M15 complete (2026-09-04): settings and shortcuts.** One bindings table,
read by the palette's tooltips, the timeline's keys and the map's handlers
alike — each of them used to wire its own, which is why a key could be bound
twice with nothing to say so. A collision or a reserved key is refused on
entry, naming what already holds it. The colour scale turns out to be a
*project* setting rather than an application one: two people opening one file
should see the same map, so the dialog edits the open project's in place,
undoably, and holds separately the default a new project of each kind gets.
A scale change costs no tile, which the acceptance test asserts by hashing the
flattened scene before and after.

**M16 complete (2026-09-04): macros.** A capture over a run of frames, kept in
a library of `.vemacro` files and inserted into any project. The lockout turned
out to want to be **one flag on the history** rather than a set of disabled
controls: every write path in the application already goes through `push`, so
one `lock()` covers the ones nobody remembered — which is precisely the failure
the plan warned about. Static versus record-movement is the acceptance that
matters, and a recorded macro moves the *whole object*, anchor and footprint
together: shifting only the lattice left the field trying to draw outside the
shape that admits it, which is how the first attempt failed. Six end-to-end
tests and four hand-computed resample ones.

**Every milestone is delivered; what remains needs a signing key or a screen (M10).**

**Found by hand after M25: the speed filter still stood still, and the
cause was under everything.** The tile protocol was registered with Tauri's
*synchronous* scheme handler, which runs on the main thread — the webview's
own — so every tile evaluation froze the interface for as long as it took.
A viewport of imported-field tiles is a second or more on the CPU (§13),
and each slider tick asked for one, so the slider stood still while its
last tick rendered under it; M25's one-write-in-flight was correct and
beside the point. The handler is asynchronous now, doing its work on the
runtime's blocking pool and answering when it is done. This had been true
since M2 and showed on nothing until a layer whose tiles cost tens of
milliseconds met a control that wrote on every pointer report. The same
rule held for commands: a plain `#[tauri::command]` runs inline on the main
thread, so a GRIB import, an export or a two-second capture bake froze the
window for its whole run and the new spinner would have had nothing to
spin over. The fourteen commands the spinner names are
`#[tauri::command(async)]` now and run on Tauri's thread pool. Not
measured: it needs the app on a screen, and the number is the tile cost,
which is known.

**M23–M26 complete (2026-09-05).** All thirty findings are delivered, one
commit per milestone; each section below records what was measured and
what was left. Every finding that turned out to be a bug in a rule now has
the rule stated once: one clipboard, one creation layer, one cursor table,
one hint line, one preview scene.

**Re-planned again on 2026-09-05, from the running app.** With every
milestone delivered, the user worked through the app and handed over thirty
numbered findings — bugs, missing affordances and a redesign of macro
recording. They are sequenced below as **M23–M26**, bugs first: selection,
clipboard and deletion (M23); every cursor and outline the map shows (M24);
the chrome — the top row, the status bar, collapsible panels, the timeline's
right-hand side, autosave and the speed filter (M25); and macro recording as
keyframes with a preview mode (M26). The findings that turned out to be one
bug are recorded as one: an object's keyframes were not lost on paste, the
object was never copied at all, because a capture once taken kept the
clipboard for the rest of the session; and a `Cmd`-click that moved the whole
selection was a move that carried the *centroid* to the pointer rather than
the point that was pressed. Section §2 has the milestones and §5 the decisions
they raise, D65–D72.

**Unplanned, after M7: GRIB import** (spec §4.8, D44). A GRIB2 file becomes a
layer — two, when it holds both wind and currents — whose lattice is sampled
by both kernels beneath the layer's own objects. The project keeps the path
and nothing else. The decoder is new and hand-written: template 3.0 in every
scanning mode, 4.0/4.1/4.2/4.8/4.11/4.12/4.15, 5.0/5.2/5.3, bitmaps; JPEG 2000
is refused by name with the repacking command. Measured on the dense tile
with a global 0.25° lattice beneath it: 37.9 ms standard on the CPU against
31.4 without, 3.98 ms exact on the GPU against 3.83. The fidelity suite now
generates raster scenes and finds a worst error of 0.0015 m/s and 0.042°
over 30,662 samples. The GPU device now asks for six storage buffers per
stage rather than downlevel's four; an adapter without them falls back to the
CPU as one without a GPU does. Also **Open from GRIB** on the start screen, which derives a project's
settings from a file (spec §4.8) and imports it. Found by hand afterwards:
the object list showed "empty" under a GRIB layer, whose field *is* its
content; and the visibility toggle's two ring glyphs did not read as one, so
it is an eye now. **Not verified by hand**: no real forecast file has been
imported in the running app, only files the writer produced.

**Unplanned, after M7: the tracks say what happens between the keys** (spec
§9.3). A dot on every step a segment interpolates through, so a moving segment
is distinguishable from a held one without reading the easing off a
right-click menu; and a per-track value graph behind a disclosure, drawn on
the ruler's scale under its track. The graph's samples come from the model
(`track_samples`) rather than from a second interpolation in the frontend —
the easings, the shortest arc and the great circle stay in one place — and the
frontend converts them the way the inspector does, in knots and in the
project's direction convention, unwrapping a degree series so a turn across
the seam draws as the short arc it is. **Not verified by hand**: the graph has
been tested as arithmetic and as samples, not looked at in the running app.

**More from the running app.** The pink edge was drawn by *stroking* the
footprint, which is a union of stamps that `Path2D` cannot union — so it traced
every stamp's circle and left a chain of rings. It is a band now: grow the
footprint by half the band's width, knock out a copy shrunk by the same, and
what remains is the union's boundary exactly, centred on it. Reported again
afterwards as "too small, inset from the perimeter", which was a second bug and
an older one: the conversion from a backend outline to a footprint *halved* the
radius, so every outline — the drag preview's included, since M6 — was drawn at
half the size of the object it outlined. `radius_km` is a radius. It has its own
function and its own test now (`footprintOfOutline`). The brush hovers too, and the outline request is
bounded by the tool in hand rather than by the project. The warp is **pulled**
— shift-drag grabs the one under the pointer and drags its field where it
should go — which replaced its distance and bearing with a `push_to` position,
so both ends of the push are keyframable, and made it unmergeable in both
modes, since both are now measured from the anchor. A GRIB layer takes a
**speed band**, applied in both kernels where a missing sample is applied, so a
forecast can be read one band at a time. And every numeric field in the app now
goes through one component: the old ones could not be *cleared*, because an
empty box parsed as `NaN`, committed a fallback and refilled under the cursor —
a size of 500 could be edited in the middle but never replaced.

**Also from the running app: the timeline says nothing about coverage, the
modifiers were stamps, and the mask's icon was an eraser.** A GRIB layer's row
now marks the steps its file has a message for, which is the other half of
D48 — a field that comes and goes needs the timeline to say why. `←` and `→`
step the playhead (clamped, where playback loops), beside the `Space` that
already played and stopped. The four modifiers are painted rather than
click-placed, so a swathe can be treated in one gesture and two strokes of one
merge like two of a brush — except the two whose field is measured from their
own anchor, which cannot absorb another without moving the centre they
radiate from or turn about, the same rule that keeps two clone strokes apart.
Their edges highlight in pink under the pointer as a mask's do, from one
`operator_outlines` command that covers both. Migration to schema version 9
turns an existing modifier stamp into the one-point stroke it would have been.

**Found by hand, after the GRIB import: an imported field outstayed its
file** (spec §4.8, D48). Every step past the last message showed that message,
unchanged — a six-hour file standing in for a whole timeline and looking like
data. A step the file has no message for now shows no imported field at all.
The rule this replaces was chosen for consistency with keyframes, which was the
wrong analogy: a keyframe is an instruction and holds; a message is a
measurement and does not. `frame_at` returns an `Option` now, which is what
made the change small — the two callers either have a lattice for this step or
have nothing to composite.

**Unplanned, after M7: the mask, and taking a vector off the map** (spec §6.1,
§6.2, D46). The eraser is the **mask**, renamed through the document with a
migration, since the tool's name is stored on every object it drew. It gains
`invert`, which turns its coverage inside out — cull included, which is the
whole of the implementation subtlety: the spherical cap that makes every other
object cheap by rejecting a distant cell has to *accept* it here. A mask and
its inverse are exact complements, asserted through the field, which is the
only place a coverage weight is observable. Its edge is drawn on the map now,
selected or under the pointer — a mask paints calm and had nothing to show
where it was — and the hit test is `isPointInStroke` against the very path the
overlay draws, so what highlights is what covers.

And the **eyedropper**: on the brush, the shape fill and the curve, where the
tool paints a single vector, a click takes the speed and direction from the
field itself. Declared in the schema like everything else a tool offers, so the
option bar renders it from a description rather than from a case per tool, and
it is inert wherever either property it writes is — in a gradient mode, or on a
curve aiming relative to its own path. That last mode's `absolute` is now
`constant`, the name every other tool uses for the same thing. **Not verified by
hand**: none of this has been used in the running app.

**Unplanned, after M7: the field modifiers** (spec §6.3, D45). Four tools that
change the field beneath them instead of adding one — intensify/reduce,
diverge/converge, rotate flow, and warp/liquify — each a click-placed disc with
the common placement, an amount, and a feather, animatable like everything
else. They cost one branch in each compositing loop, because the loop already
held exactly "everything below this object" at the moment it applied one: the
guarantee the clone stamp was built on. The warp is the exception that reads
elsewhere, so it inherits the clone stamp's recursion, its depth cap and its
exclusion from the GPU. The evaluation is asserted against hand-computed
vectors — 20 m/s east plus 100% outward at a cell due north of the anchor is
28.28 m/s on 045 — rather than against what the evaluator happens to produce.
Fidelity after: worst speed error 0.0030 m/s and direction 0.019°, against
0.25 and 2°; before the modifiers it was 0.0009 and 0.004°, and the difference
is the anchor-relative term D38 had removed. **Not verified by hand**: no
modifier has been placed in the running app.

**A pre-existing gap the new coverage found.** Adding modifiers to the fidelity
suite's generator shifted its random stream, and the new scenes caught a
disagreement that had nothing to do with them: at the edge of an imported grid
the CPU allows a millionth of a cell of slack and the shader a thousandth, so
a sample in between reads the edge row on one backend and calm on the other.
The band is a few hundred metres wide at the edge of a 2° grid and the
generator aims half a cell outside the grid deliberately, so it was only a
matter of which seed. The suite now exempts that band and says so, like the
path-tangent tie it already exempts; the slacks themselves cannot be equalised,
since the shader's exists to absorb `f32` index error the CPU does not have.

**Also found by hand: the eraser trailed rings.** Its gesture outlined the
swept region, and a footprint is a union of stamps that `Path2D` cannot union
— so stroking one traced every stamp's own circle and a drag left a chain of
overlapping rings behind the pointer. The eraser now draws nothing but its
nib, the stamp under the pointer, held through the drag; the mask the map is
already drawn through is the whole of the preview (D37). The clone stamp had
the same bug and takes the same fix — the map shows what either operator does,
and an outline over that is redundant as well as wrong. The rule is one
function, `overlayPlan`, rather than a condition per tool at the call site. Raised at the same time and **declined**: making the
eraser destructive. Objects are parametric — a circle has no representation
for "with a bite out of it", and a stroke could be cut only to within its own
width — so the erase object stands as spec §6.2 and §7.4 have it, and D37 with
it.

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

Tool breadth (M6) and the timeline (M7) are comparatively low-risk once the
spine holds — and both bore that out.

```
M0 ─ M1 ─ M2 ─ M3 ─ M4 ══ walking skeleton complete
                    │
                    ├─ M5 ─ M6 ── tools & editing                     ✓
                    ├─ M7 ─────── timeline & animation                 ✓
                    │
                    ├─ M12 ────── decoder: CCSDS and JPEG 2000 import      ✓
                    ├─ M20 ────── GRIB frames copied between steps         ✓
                    ├─ M21 ────── ICON's icosahedral grid                  ✓
                    ├─ M22 ────── every grid definition the centres ship   ✓
                    ├─ M13 ────── motion vectors, linked objects        ✓
                    ├─ M14 ────── region selection, fill, copy/paste      ✓
                    ├─ M15 ────── settings and shortcuts               ✓
                    ├─ M16 ────── macros                                ✓
                    ├─ M17 ────── warp and liquify, two tools            ✓
                    ├─ M18 ────── image layers                           ✓
                    ├─ M19 ────── export precision                        ✓
                    ├─ M11 ────── projections                            ✓ (cylindrical tier)
                    ├─ M8 ─────── measurement                            ✓
                                        │
                                       M10 ── hardening & release            ✓ (less signing)
                                        │
                    ├─ M23 ────── selection, clipboard, deletion (bugs)     ✓
                    ├─ M24 ────── cursors, hover and outlines             ✓
                    ├─ M25 ────── chrome: top row, status bar, panels, timeline ✓
                    └─ M26 ────── macro recording as keyframes, with a preview ✓
```

**M23–M26 (2026-09-05) go bugs first, then what each later one needs.** M23
is where the data is wrong — a move that jumps, a copy that never happens, an
object landing in a GRIB layer — and every fix in it is a rule the later
milestones build on (one clipboard, one creation target, one selection). M24
is the map's feedback layer and gives M26 its cursor rule and its hover
outline. M25 builds the status-bar hint area and the panel dimming M26's
recording mode shows through. M26 is last because it is the one redesign in
the list and touches the backend session, the tile protocol and the timeline
together.

**The order of M12–M19 is dependency first, then risk.** M12 is small, fully
specified by ten files on disk, and lets a real forecast be imported by hand
for the first time, so it goes first, and M20 — a GRIB layer's frames copied
between steps — follows it because it is the other thing a real forecast
file needs and touches the same code. M13 is model and kernel work with no UI
prerequisite. M14 builds the region gesture, the coverage bit and the capture
container that M16 needs, and M15 the settings surface M16's library lives
in, so both precede it. M17, M18 and M19 are independent of each other and of
the rest. M11 comes before M8 because M14's map-space regions and M18's image
quads are the last two things that will assume equirectangular, and it is
better to find out what they cost under a curved projection with those fresh
than after M8 has added a third. M10 is last as always.

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
D39–D43.

**Deliverables**

- Timeline dock: ruler with forecast-hour and UTC labels, scrub, step selection.
- Collapsible layer → object → property track tree.
- Object `active_range` bars with draggable ends.
- Keyframe editing: add, move, delete, box-select, per-segment interpolation.
- Auto-key mode and explicit per-property key buttons.
- Playback gated on GPU residency, with the last frame held and dimmed under a
  frame still landing (D43); one evaluation per tile across the map's own
  requests and the pool (single-flight in the render cache); each step
  flattened, hashed and planned once per revision rather than once per tile.
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

### M8 — Measurement tools · **complete**

**Goal:** spec §10, complete.

**Delivered as specified**, as one tool with three modes rather than three
tools: they share a gesture — click points on the map — a handle, an overlay
and a clear action, so three palette entries would have been three ways of
pointing at the same thing, and D54 allocated one key (`T`) for exactly this.

- **Dividers**: a chain measured leg by leg, each leg a great circle, with a
  running total. The chain stays open until `Enter`, `Escape` or a tool change,
  which is the polygon's gesture and the same code path's reasoning: a shape
  built up click by click has no pointer-up to end it.
- **Passage** (the great circle / rhumb pair): both paths between two points,
  the great circle solid and the rhumb dashed, labelled `GC` and `RL`. The
  great circle carries its *initial* bearing and the rhumb its constant one —
  which is the difference between them, said in the label.
- **Range rings**: N geodesic circles about a draggable centre. The interval
  and count are typed in the bar and edit the set most recently placed or
  touched, since neither is a position and neither has a handle.
- Per-mode clear, global clear-all, and alt-click on any handle to remove that
  one measurement — which is what "individually clearable" turned out to mean.

**One command, not six.** Every edit — place, drag, extend, set an interval,
clear a mode, clear everything — is `SetAnnotations { before, after }`,
replacing the whole list. The list is a handful of measurements of a handful of
points each, against a document whose objects carry property maps and
keyframes; six commands and six inverses would have bought nothing but a
smaller history entry nobody can see. It also gives a drag its coalescing for
free, so one undo returns a dragged point to where the drag began rather than
one pointer report back.

**A measurement does not bump the tile revision**, deliberately. The revision
addresses rendered tiles and a measurement changes no pixel of the field;
bumping it would discard the whole cache because someone dropped a pair of
dividers on the map. The cost is that the frontend re-reads the measurements
whenever the document summary changes rather than watching a number — one short
call under one lock, off the render path.

**Acceptance**

- **Distances against independent references.** Not against a second copy of
  the same formula: a quarter of the equator and a pole-to-pole meridian are
  closed forms; every vertex of a great-circle path is checked by the property
  that defines the arc (the distances to the two ends sum to the whole); every
  vertex of a ring is measured back to its centre; and the **rhumb path is
  checked against walking its own bearing** — twenty thousand short
  `destination` steps at the constant course, which is a numerical integration
  of the defining property and a different computation from the closed form.
  They agree to 134 m over 5 758 km, and the error halves when the step count
  doubles, so what is left is the walk's and not the formula's. Antimeridian
  and near-polar pairs are in every one of these.
- **The two paths visibly diverge** on a long high-latitude passage: New York
  to Amsterdam parts by more than a hundred kilometres in the middle, and the
  great circle is both the shorter path and the poleward one.
- **The rhumb line is straight in Mercator**, checked in the isometric latitude
  against the chord between the endpoints — and the same path is shown to bow
  a fifth of a degree off a straight line in *plain* latitude, so the test says
  which projection the property belongs to.
- **Annotations survive a save and reopen**, to the file's own nine-decimal
  quantisation, and are in the hostile-float round trip.
- **They never reach an export**, asserted on the bytes: the same project
  exports byte-identically with a chain, a passage and four range rings laid
  over it as with none. Export is deterministic (invariant 4), so a byte match
  is a complete statement that the measurements were not consulted — and it
  stays true if someone later gives `FlatScene` a field it should not have.

**One correction to this plan.** The acceptance list said the rhumb line
"renders as a straight line in equirectangular projection". It does not, and
cannot: a rhumb line is straight in the *isometric* latitude, which is
Mercator's `y` — in a flat map it bows. The property was worth keeping and is
now testable for the first time, M11 having shipped Mercator, so the criterion
is corrected rather than dropped.

---

### M10 — Hardening and release · **complete, less what needs a signing key**

**Goal:** shippable.

**Delivered**

- **Crash recovery, which did not exist.** The application had an `autosave`
  directory from M0 and nothing had ever written to it, so "verified by
  killing the process mid-edit" would have verified nothing. A thread now
  snapshots the open project every 60 s or 50 history entries, whichever
  comes first — the rule spec §4.2 promised — only while it is dirty and has
  changed, from a clone taken outside the session lock. A clean save, a close
  or an agreed replacement removes the snapshot; the start screen offers what
  a crash leaves under **Recovered work**, and recovering opens the project
  dirty at its old path. Tested by dropping one `AppState` mid-edit and
  standing up another over the same directories, which is what a new process
  sees.
- **The large-project stress**, `tests/stress.rs`: 5,000 objects across 240
  steps, saved, opened, flattened at every step and rendered as a tile,
  numbers printed in every build and the §13 open budget (≤ 500 ms) asserted
  in release. Measured on this machine in release: save 61 ms to a 173 KB
  archive; **open 202 ms against the 500 ms budget**; flatten 5.2 ms per step
  (1.2 million flat objects over the timeline); one CPU-fallback tile of the
  whole 5,000-object scene 331 ms — reported against §13's 40 ms for scale
  and not asserted, since that budget is stated for 200 objects and this
  scene is twenty-five times it. `tile_cost` was already the tile budget at
  the budget's own scale; between them these are the §13 rows that can be
  measured without a screen.
- **User documentation** (`docs/USER-GUIDE.md`) and **three sample projects**
  (`assets/samples/`): trade winds, a cyclone built from modifiers over a
  stroke, and an ocean gyre — written by `examples/make_samples.rs` through
  the real pipeline, so each is exactly the file the application produces,
  and deterministic, so a change in the samples is a change in the
  application.
- **A release workflow** (`.github/workflows/release.yml`): on a version tag,
  bundles for macOS universal, Windows and Linux through `tauri-action`, with
  the offline check run on the bundle that ships. Signing and notarisation
  are wired to secrets the repository does not hold; without them the macOS
  bundle is unsigned and otherwise identical.

**Not done, and why**

- **Signed and notarised bundles**, and **smoke tests on clean machines**:
  both need an Apple developer identity and machines this session does not
  have. The workflow is ready for the secrets.
- **The socket-level offline test** over the packaged app. It needs a packaged
  app and a way to observe its sockets, which is a CI job with a VM and not a
  test in this repository; the static check runs on every build and on the
  release bundle. Invariant 5 remains enforced by the CSP at runtime and by
  the absence of any networking dependency, which `cargo tree` shows.
- **The remaining §13 rows** — frame rate, pointer latency, playback rate,
  peak RSS — are interactive measurements that need the app on a screen.
  Not renegotiated, because nothing measured suggests they miss: they are
  simply not measured here.
- **Memory and leak passes** likewise need a running app under a profiler.

---

### M22 — Every grid the centres ship · **complete**

**Goal:** a regional forecast file imports, wherever its model runs. Asked for
after M21, from a directory of 1,735 sample files.

Almost every regional model runs on a projection rather than on lat/lon, and
the decoder refused all of them by name. Five grid definitions were added —
rotated lat/lon (3.1), Mercator (3.10), polar stereographic (3.20), Lambert
conformal (3.30) and NCEP's rotated Arakawa non-E staggered grid (3.32769) —
which with 3.0 and 3.101 is **every grid definition in the sample set**: 1,653
GRIB2 messages, all of them now parsed.

**Deliverables**

- `projection.rs`: the four projections, ellipsoidal (Snyder), reducing
  exactly to spherical at zero eccentricity so code table 3.2's shapes cost
  one code path. `Prepared` holds a projection's derived constants, because a
  Lambert cone costs two logarithms and two powers to describe and a resample
  evaluates it millions of times.
- `resample.rs`: a projected grid onto the **span of the project's lattice it
  covers**, bilinear, with a pole-enclosure test the boundary walk cannot
  make for itself.
- **The grid convergence**, which is the part that is easy to get silently
  wrong: flag table 3.5 bit 5 resolves `u` and `v` along the grid's axes on
  1,004 of the sample messages, and reading those as eastward and northward
  is 30° of error across a Lambert CONUS grid.
- `Scan`, one type for the sixteen scanning modes, shared with the lat/lon
  path it was extracted from.

**Acceptance — met**

- Every grid definition in the sample directory parses: 902 lat/lon, 374
  Lambert, 203 stereographic, 104 Mercator, 54 unstructured, 16 rotated.
- Templates 3.10 and 3.32769 state a **last** grid point that the projection
  does not consume; walking to it lands within **0.725 cells** worst case
  across all 110 such files, and exactly on it for 3.32769.
- The fixtures are section 3 of eight real messages, and the assertions are
  geography: each grid covers the places its model covers and excludes the
  ones it does not. A projection with its cone constant or hemisphere wrong
  still produces plausible numbers; it does not put Alaska over Alaska.
- HRRR CONUS resamples onto 0.1° in 26 ms, HRDPS continental in 77 ms
  (`resample_cost.rs`).

**Not done, and named rather than left to be discovered**

- **GRIB edition 1**, 82 files. An edition, not a grid definition: different
  sections, tables and packing. All 82 are derived duplicates whose GRIB2
  originals are not in the set.
- **JPEG 2000 codestreams wider than 60,000 px**, 55 files. Météo-France
  writes AROME's codestream as a single row of 4,160,515 pixels and
  `hayro-jpeg2000` caps a dimension at 60,000. A packing limit, not a grid
  one — those files are on plain lat/lon grids.
- **Product templates 4.5, 4.6, 4.9 and 4.10**, 70 files. Probability and
  percentile products, which carry no `u`/`v` and so would not import as a
  field even once read.

---

### M11 — Alternative map projections · **complete, cylindrical tier**

**Goal:** the map can be drawn in projections other than equirectangular,
including a globe.

**Delivered:** the cylindrical tier — equirectangular, **Mercator** and
**Miller** — complete, plus the `stamp_space: projected` rule the plan required
before any of this shipped. The pseudo-cylindrical tier and the globe are not
done; the reasons are below and in spec §5.1, which now states the family and
what excludes the rest.

Nothing below the view is coupled to the projection — objects are stored in
geodesic AEQD frames, the evaluator works in lat/lon and true bearings, and the
GRIB writer has its own grid — so this cannot affect a stored project or an
exported file (invariant 3). There is no migration and no fidelity question;
the cost is entirely in the frontend, and it was a day.

**The projection rides on the camera.** `Camera.projection` rather than a
parameter beside it: it is the same kind of thing as the centre and the scale —
part of how the map is looking at the world — and the camera is already threaded
through every one of the thirty-odd call sites that needed it, from the renderer
to the pointer to the overlay's footprints. A camera built without one is
equirectangular, so every test fixture and every dev-capture scenario keeps the
numbers it had. The alternative, an extra argument on nine functions, would have
had the same effect at the cost of every caller having to remember it — and the
failure mode of forgetting is a stroke that lands somewhere other than the
cursor, which is exactly what this milestone exists to prevent.

**The four coupling points, and what each turned out to be**

| | was | is |
|---|---|---|
| `geoToScreen` in the shaders | one linear transform, shared by four programs | `latToY` in the shared GLSL prelude, on a `uProjection` uniform; one branch, taken identically by every vertex of a draw |
| `project` / `unproject` in `camera.ts` | a closed-form inverse pair | still a closed-form inverse pair — the whole reason the family stops where it does |
| tile quads in the renderer | a lat/lon rectangle is a screen rectangle | still is. The corners are exact; only the *interior* is not, so the raster fragment recovers its latitude from the pixel through the inverse rather than interpolating the corners — exact everywhere, and equirectangular takes the interpolated value untouched |
| `worldOffsets` | the world repeats horizontally | unchanged: longitude is linear in `x` in every cylindrical projection |

Three more turned up that the plan had not listed, all of them the same mistake
in different clothes — a screen distance treated as a span of latitude:
`visibleBounds` (so tile culling asked for the wrong rows), the glyph pass's
own off-screen test, and `regionOfView`, which is what `Cmd`-`A` selects.

**Two places the projection is genuinely visible in the geometry**

- **A footprint's vertical radius** passes through `dy/dlat` at its centre.
  Under Mercator that makes a *geodesic* footprint a true circle at every
  latitude rather than an ellipse — which is the projection being conformal,
  not a special case in the overlay. Using the derivative at the centre keeps
  the ellipse symmetric and is exact to second order; a stamp big enough for
  that to show is bigger than any brush.
- **The clone stamp's source camera** shifts in the projection's vertical
  coordinate and is anchored at the brush, so the preview is exact where the
  user is drawing and drifts away from it. A constant offset in degrees is a
  constant offset in pixels only under equirectangular, and there is no single
  translation that is right everywhere else. This is the same bargain the
  `Fixed` offset mode already makes, and it is a preview.

**The `stamp_space: projected` rule** (D63, spec §5.1, and the plan's open
question above): **projected means the *project's* map space — equirectangular lon/lat
degrees — never the view's.** The plan's "cheapest rule" was the opposite, that
a px size or a region be converted through the view's projection at creation so
a shape drawn round on a Mercator screen is stored as the equirectangular shape
occupying the same ground. That is rejected for two reasons. The first is that
it makes a document depend on a view setting: the same px number would produce
a different object depending on which projection happened to be showing, and
nothing on screen would say so. The second is that it is not expressible — a
projected stamp has *one* size, `size_km`, and a round-on-Mercator shape is an
ellipse in equirectangular space, so there is nothing to convert it into
without adding an aspect property to every sized tool. The visible consequence
of the rule as chosen is that a projected stamp is round only under
equirectangular and draws taller further from the equator elsewhere, exactly as
a lat/lon rectangle does. It behaves like the map it is defined on, which is
the answer that needs no warning.

**Acceptance**

- Switching projection never alters a project or an export — structurally, not
  by test: the projection reaches nothing but `ui/src/map`, and the setting
  lives in `AppSettings` rather than in the document.
- Painting is accurate in each: `unproject` then `project` returns the same
  screen point to nine figures in all three, and zoom keeps the point under the
  cursor under the cursor.
- **The shader's formulas and the pointer's are checked against each other**,
  by lifting `latToY` and `yToLat` out of the shipped GLSL source and evaluating
  them as JavaScript against `projection.ts` at every 2.5° of latitude. They are
  the same numbers to nine places. This is the drift that would show as arrows
  sitting slightly off the colour they describe — easy to look at and not see.
- The dateline and both poles are unchanged, being what the world-copy offsets
  and the latitude clamp already handle.

**Not done, with reasons**

- **Pseudo-cylindrical (Mollweide, Robinson, Winkel Tripel).** Lat/lon
  rectangles become curved quads, so tile quads and basemap triangles both need
  subdivision; Robinson and Winkel have no closed-form inverse, and `unproject`
  runs on every pointer move *and* every painted point, so the iteration has to
  be fast enough for painting rather than for a readout.
- **Globe (orthographic).** All of that, plus back-face culling, tile selection
  over a spherical cap instead of a lat/lon box, an interaction model that
  rotates rather than pans, and a glyph lattice whose on-screen spacing varies
  across the disc — which makes the "pick a step near 40 px" rule meaningless
  as written.
- **Basemap edge subdivision**, in the builder: Antarctica's ring runs to 180°,
  drops to 90°S and crosses the pole edge to −180°. Under any *cylindrical*
  projection that edge is still a straight line along the bottom and renders
  correctly, which is why it is not needed yet. It is needed by the first
  curved projection, and by nothing before it.

None of the three is blocked; each is a real feature that begins where this one
stops, and spec §5.1 says so rather than leaving the family looking arbitrary.

### M12 — Every packing the forecast centres ship · **complete**

**Goal:** every file in the reference set imports, with no C library.

**Delivered as planned, plus one thing nobody was looking for.** The spike
came first and both candidate crates were bit-exact on the real files, so the
codec choice took an afternoon rather than a milestone: `rust-aec` for 5.42,
`hayro-jpeg2000` for 5.40 with its default features off, which leaves it with
no dependencies at all. `pure_jpeg2k` was the named fallback and was not
needed. What the reference set found instead was a live bug in template 5.3 —
see the status notes above — which is the reason this milestone was worth
running before any of the feature work.

**What the reference set needs.** The ten sample files in
`/Users/jon/temp_test_gribs` — GFS, GEFS, NCEP's two AI models, ECMWF
deterministic and ensemble, AIFS, ARPEGE, GEM deterministic and ensemble — are
all regular lat/lon (template 3.0), 10 m `u`/`v` on product templates 4.0 and
4.1, without bitmaps. They differ only in packing:

| Packing | Template | Files | Today |
|---|---|---|---|
| Complex with spatial differencing | 5.3 | GFS, GEFS, AI-GFS, AI-GEFS | decodes |
| CCSDS adaptive entropy coding | 5.42 | ECMWF ×2, AIFS, ARPEGE | refused as "template 5.42" |
| JPEG 2000 | 5.40 | GEM ×2 (0.15°, scanning mode 64; 0.5°) | refused by name |

So the work is two packings, under one constraint: invariant 5 and the build.
Pure Rust only, no `-sys` crate and no `cc` build script, so every target the
app builds for gets the decoder with nothing installed.

**Deliverables**

- **Template 5.42.** `rust-aec` (MIT, pure Rust, written for GRIB2 5.42 —
  `flags_from_grib2_ccsds_flags` maps octet 22 directly) or `oxiarc-szip`
  (Apache-2.0). The output is the packed integers; the 5.0 formula
  `packing.rs` already applies does the rest. Samples are `ceil(bits / 8)`
  bytes each, big-endian, per the template.
- **Template 5.40.** A pure-Rust JPEG 2000 decoder on the raw codestream —
  GRIB carries J2K with no JP2 wrapper — one component, the reversible 5/3
  wavelet, precision equal to the bits per value. Candidates:
  `hayro-jpeg2000` (Apache/MIT, MSRV 1.92, the best maintained; its default
  features pull `image` and `moxcms` and are switched off), `pure_jpeg2k`
  (lossless Part 1, integer output), `dicom-toolkit-jpeg2000` (native bit
  depth). **The choice is a spike, not a decision:** `hayro` is image-oriented
  and normalises components towards a colour space, so integer-exact recovery
  of a 16-bit component has to be shown against ecCodes on the GEM files
  before it is picked, with `pure_jpeg2k` the fallback. Whichever is chosen,
  `cargo tree` and its `unsafe` count are read, because it runs on every
  imported file.
- The two deferred lines in spec §4.8 and §14 ("JPEG 2000 … needs a decoder
  the project does not carry") are struck, with the "repack it with ecCodes"
  message.
- **Fixtures, small.** Not the 17 MB sample set. The writer's own output
  re-packed by ecCodes — `grib_set -s packingType=grid_jpeg` and
  `grid_ccsds`, each with and without a bitmap — is a few kB per message and
  its values are known because the writer wrote them. The full sample set
  runs as an `#[ignore]` test keyed on the directory's presence, against
  `grib_get_data`.
- **Export is untouched.** Template 5.0 simple packing only (spec §12.3). This
  milestone changes the reader and nothing else.

**Acceptance**

- All ten sample files import, and every decoded value matches ecCodes to
  within the packing's own step (`2^E · 10^−D`); `u` and `v` at ten spot
  cells — both poles and the seam among them — match `grib_get -l`. **Met**:
  200 spot values across ten files, and every message decoded bit-exactly
  against a full ecCodes dump during development.
- A 0.15° GEM message (2,882,400 values) decodes in under a second in
  release, measured: a pure-Rust JPEG 2000 decoder may be an order slower than
  OpenJPEG, and a file has one message per component per step. **Met**: 283 ms.
- ~~`cargo tree` shows no `-sys` crate and no `cc`.~~ **Corrected**: no `-sys`
  crate, and neither codec adds a `cc`. `blake3` has had one since M3 for its
  SIMD assembly, which the criterion overlooked.
- A bitmapped 5.42 and 5.40 message each decode with the bitmap honoured.
  **Met**, as fixtures.

**Risks:** decoder maturity. A 16-bit single-component reversible codestream
is a corner the PDF-oriented crates may never have been run on; the spike
exists to find that out in an afternoon rather than a milestone. Decode time
on 0.15° files is the second, and is measured rather than assumed.

---

### M13 — Motion in the field, and objects that follow · **complete**

**Goal:** an object that moves can put its own motion into the wind, and an
object can be tied to another so it moves with it.

Both are model work of spec §9.3's kind. Only the first reaches the kernels.

**Motion vectors.**

- On the brush, circle, shape fill, clone stamp and curve — the tools that
  paint a vector — the `position`, `rotation_deg` and `scale_pct` rows in the
  timeline each carry a checkbox, **motion**. While it is on, the object's own
  movement between steps is added to the vector it paints. A stroke
  travelling east at 15 m/s adds 15 m/s eastward to whatever it paints, so a
  tailwind strengthens, a headwind weakens and a crosswind turns — one vector
  addition per cell, in the cell's own east/north frame, which is what makes
  all three cases the same rule and the addition correct at every relative
  angle.
- Stored as three booleans on the object (`motion: {position, rotation,
  scale}`, `#[serde(default)]`, no migration). The checkbox sits on the track
  row because the track is where the movement is, one per row and not one
  per object (D57), so a spinning object can add its spin without its travel.
- **The velocity is taken from the track the field already uses.** At step
  `k` it is the central difference of the property's sampled value at `k−1`
  and `k+1` over `2·Δt`, one-sided at the ends, `Δt` from `step_hours` — read
  from the same `track_samples` the graph draws, never a second
  interpolation. A `Step` segment contributes nothing: a value that jumps is
  a teleport, and a 500 km jump in an hour is not a 140 m/s wind. A property
  with no keys has no motion.
- **A translation and a rotation are each one angular velocity.** A position
  segment is a great-circle slerp — a rotation of the sphere — so the
  translation velocity at every cell of the footprint is exactly `Ω × p` for
  one 3-vector `Ω`; a rotation about the anchor is `ω·â` for the anchor's unit
  vector. Both sum into one `[f32; 3]` in the flat object, and a cell's
  velocity is a cross product and a projection to east/north. That is exact
  everywhere, poles and seam included, where "the anchor's speed and bearing,
  applied uniformly" would be wrong by the frame's own drift a few thousand
  km out (CLAUDE.md, directions). Scale adds a radial velocity of `ṡ/s · r`
  along the frame's radial bearing, which the modifiers already compute: one
  more `f32`.
- Both kernels add it to the object's own vector *before* feather and edge
  mode, so the edge fades the sum; the clone stamp adds it to the cloned
  sample. The recipe applies in full: `FlatObject` and the WGSL struct,
  `CpuEvaluator`, the kernel, the fidelity generator (keyed positions across
  the pole and the seam, keyed rotations), the cache hash — the velocity is in
  the flat object, so the hash follows.
- The gesture preview shows no motion (a fresh stroke has no keys); the tiles
  do.

**Linked objects.**

- In the timeline, `Cmd`-drag from the `position` or `rotation_deg` row of one
  object and drop it on the same row of another: the first now **follows** the
  second. Those two properties only; a drop on another property, on any other
  row or on itself is refused, and the cursor says so. The row then shows a
  link glyph and the primary's name, with an unlink button; `Cmd`-clicking the
  glyph unlinks too.
- **A follower keeps its offset in the primary's frame.** At link time the
  follower's position is recorded as a distance and a bearing from the
  primary's anchor, relative to the primary's rotation; a rotation link records
  the difference of the two rotations. At every step the follower's value is
  derived: the primary's position moved that distance along that bearing
  turned by the primary's rotation, so a follower orbits a turning primary as a
  rigid part of it. The primary's motion therefore propagates: a follower with
  `motion` on takes its velocity from the derived track.
- The follower's own keys for the property are kept but dormant (greyed on the
  track). Unlinking reactivates them and, so nothing jumps, writes the derived
  value at the current step under D42's rule. Deleting the primary, or pasting
  the follower without it, unlinks the same way. A cycle is refused at the
  command; chains resolve in dependency order when a step is flattened. Link
  and unlink are history entries.
- Model: `Animatable<T>` gains `follow: Option<Follow>` (`primary: Id`, the
  offset), resolved in `ve-core` when a property is sampled, so the kernels
  never see it. Serde default covers old files; no migration.

**Acceptance**

- An eastward 20 m/s stroke on the equator, keyed 300 km east over one 3-hour
  step (27.8 m/s), reads 47.8 m/s east with motion on and 20 without; the
  same stroke painted westward reads 7.8 m/s *east*; painted northward it
  reads 34.2 m/s on 054°. Hand-computed, not read off the evaluator.
- A circle turning 30° per 3-hour step adds 9.7 m/s at 200 km from its
  anchor, tangential and clockwise, the same on both backends at cells due
  north and due east of the anchor and across the seam.
- A `Step` segment adds nothing, an unkeyed property adds nothing, and a
  document with every `motion` off exports byte-identically to before.
- The fidelity suite's motion scenes sit inside the standing tolerances.
- A follower linked 500 km east of a primary at 45°N stays 500 km from it on
  the primary's bearing through a primary that travels 3,000 km and turns 90°;
  unlinking at step 5 leaves it where it was at step 5; a cycle is refused;
  deleting the primary leaves the follower where it stood.

---

### M14 — Region selection, fill, copy and paste · **complete**

**Goal:** a region of the map can be selected, filled with a vector, and
copied as a field.

This is not the object selection §8.2 has — that picks *objects*; this picks
*ground*. The two coexist: the select tool is a tool like the brush, so in it
a plain drag draws the region, and the hand tool keeps plain drag as pan
(D26).

**Deliverables**

- **Select tool** (`M`, the marquee's conventional key; the unbuilt measure
  tool moves to `T`, D54). Three modes on its bar: rectangle, circle
  (dragged from the centre, as the shape fill's presets are, D33) and lasso
  (freehand, closed on release). A region is a `Shape` — rect, disc, polygon —
  **in map space**: it is drawn on the map, `Cmd`-`A`'s "the view" is only
  meaningful as a map rectangle, and `Cmd`-`Shift`-`A`'s "the map" is the
  rectangle that spans it. So anything made from a region takes `stamp_space:
  projected`, exactly as a px-sized stamp does (D28, D55).
- The region is session state: not document, not history. It is drawn as a
  moving outline on the overlay (through `requestOverlay`, like every other
  camera-positioned thing). **Deselect** on the bar and `Cmd`-`D` clear it;
  `Cmd`-`A` enters the select tool with the view selected; `Cmd`-`Shift`-`A`
  the whole map. Neither key is bound today. With a region active, `Cmd`-`C`
  copies the region; with objects selected and no region, it copies the
  objects as it does now.
- **Fill tool** (`G`, the paint bucket's key). It acts on the current region:
  a click creates a **shape fill** whose geometry is the region's shape. The
  options are the shape fill's own — constant speed and direction with the
  eyedropper (D47), gradient with its two speeds and two directions, and the
  aimed modes `TowardPoint` and `AwayFromPoint` — so the bar is the shape
  fill's bar and the object is a shape fill: the seven shared-rule tests cover
  it with no new case. With no region the tool is inert and the bar says so.
- **Copy and paste of a region.** `Cmd`-`C` captures the visible composite
  inside the region at the current step — speed and direction per cell on the
  project's grid lattice, over the region's bounding box — and `Cmd`-`V`
  creates a **patch**: an object whose geometry is the region's shape and
  whose field is the captured samples, pasted at the original place with the
  small offset objects get (§8.5) or, with the pointer over the map, under
  it. It moves, rotates, scales, keys, hovers, feathers and edge-modes like
  any object, and takes M13's motion. **Zero and undefined are distinct**: a
  cell no object or raster wrote, or one a mask removed, is undefined, and an
  undefined cell writes nothing when the patch is composited, so what is
  beneath shows through (D58). This needs a coverage bit per cell in the CPU
  composite, which the capture reads; the capture is evaluated with
  `CpuEvaluator`, like an export, in a worker.
- **Where the samples live** (D52). A patch is a raster, and invariants 1 and
  2 as written forbid one in the project; the user has decided to reword
  them. The `.veproj` ZIP gains a `captures/<hash>.vecap` binary entry, the
  JSON keeps the hash and nothing else, and a *rendered* raster — a cache
  product, reproducible from the objects — stays forbidden while a
  *captured* one — user content, not reproducible once its sources change —
  travels with the project as an entry, never as JSON. **The rewording of
  invariants 1 and 2 in spec §1.5 and CLAUDE.md lands in the commit that
  first writes an entry**, not before: until then the invariants as written
  still hold. M16 uses the same entry format.
- **The `.vecap` container.** A header (magic, version, field kind, lattice
  spacing, extent in the region's frame, frame count, seconds per frame,
  whether it moves), the region `Shape`, then per frame an offset and the
  `u`/`v` pairs as `f32` with NaN for undefined, compressed with `lz4_flex`
  (pure Rust) — chosen for decode speed, since it is read on open. One frame
  for a patch, many for a macro.
- **Both kernels sample a patch** through the raster sampler the GRIB layers
  use (`RasterGrid::sample`, `sample_raster`), in the object's frame rather
  than lat/lon — the transform is the frame's, the sampler is shared, so GPU
  support arrives as the raster layers' did. The cache hash takes the
  capture's hash and the frame.

**Acceptance**

- A circle region is round on the map at 0°, 45° and 70°, and `Cmd`-`A`'s
  region has the viewport's corners.
- Fill on a lasso makes a shape fill whose footprint is the lasso's polygon
  exactly, and it passes the catalogue's shared-rule tests unchanged.
- A region over a 20 m/s eastward stroke and open water, pasted 1,000 km away
  over another field, reads 20 m/s east where the stroke was and that other
  field where the water was — zero and undefined stay distinct through the
  container, both kernels and an export.
- A patch round-trips through save and load byte-identically, entry included;
  both kernels agree on it within §7.9.
- `Cmd`-`D` clears; with objects selected and no region, `Cmd`-`C` still
  copies objects.

---

### M15 — Application settings and shortcuts · **complete**

**Goal:** the app has a preferences surface, and the keys are rebindable.

**Deliverables**

- A settings file in the config directory — `paths.rs` has the directory and
  `session::Settings` already holds the recent list there, tolerant of a
  broken file; it grows rather than being replaced — and a Settings dialog on
  `Cmd`-`,` with three sections: Shortcuts, Macros (filled by M16), Display.
- **Shortcuts.** One bindings table, read by the palette's tooltips, the
  timeline's keys and the map's handlers alike; today `Space`, the arrows and
  the palette keys are wired by hand in `Timeline.tsx` and `MapView.tsx`, and
  they move to it. Rebindable: play/pause, step left and right, every tool,
  map pan (`Shift`-arrows by default, since the bare arrows are the
  timeline's) and zoom in and out. A binding that collides with another, or
  with a reserved key the webview or the OS owns, is refused on entry; reset
  to defaults is one button.
- **Legend colour scales.** The scale is a project setting (it reaches the
  map through `ProjectSummary`, spec §4.1), so Display offers the default for
  *new* projects and edits the *open* project's scale in place — a document
  write, undoable. A few bundled ramps plus editable stops, in knots per field
  kind. Tiles carry speed, not colour, so a scale change costs no tile.
- **Macros:** the library's directory (a picker; default under the app data
  directory) and **Delete all macros**, with the library's count and size
  beside it. The controls land here; the library they act on is M16's.

**Acceptance**

- A rebound tool key selects the tool and appears in its tooltip; a colliding
  binding is refused; settings survive a restart and a broken file falls back
  to defaults.
- A colour-scale edit changes the legend and the map without a tile being
  re-rendered, measured by the cache's entry count.

---

### M16 — Macros · **complete**

**Goal:** a region of the field, over a run of frames, can be captured under a
name, kept across projects, and put back down anywhere.

Two tools and a library.

**The library.** A directory (M15's setting) of `.vemacro` files, one per
macro: M14's `.vecap` container with a name, a creation time and the source
project's field kind and step size. Listed by the insert tool and by the
settings dialog — name, frames, duration, size — and deleted one at a time or
all at once. Nothing in it is project data. A project that uses a macro
carries its own copy of the frames (M14's decision), so clearing the library
breaks nothing already inserted.

**Capture tool** (`K`). Options: region mode — rectangle, circle, lasso, the
select tool's own gestures — and **Static / Record movement**. The flow, as
requested:

1. At any step, draw the region and press **Start capture** on the bar. The
   timeline enters **capture mode**: only clicking the ruler and `←`/`→` move
   the playhead; play, key drags, range bars, box-select, the inspector's
   writes and every map tool are disabled; **Cancel** and **Finish** appear in
   the transport. The region cannot be redrawn, reshaped or deselected while
   capturing. `Esc` cancels.
2. At each frame the region may be dragged. **Each frame holds its own
   position**, initialised to where the region was drawn: moving it at frame
   3 moves frame 3 and no other. These positions are capture state — not
   keyframes, not the document, not history — and the timeline draws none of
   them as keys.
3. **Finish** asks for a name and bakes: for each frame, the visible composite
   inside that frame's region is evaluated with `CpuEvaluator` at the
   project's grid spacing, undefined kept distinct from calm (M14's coverage
   bit), in a worker with progress and cancel, like an export. **Record
   movement** also stores each frame's displacement from the first, in the
   region's frame. **Static** stores none: every frame is written as if at
   the first frame's place, so a region dragged to follow a moving system
   yields a macro of that system standing still. Then the region is
   deselected, the timeline re-enabled, and **no object is created**.

**Insert tool** (`N`). The bar lists the library; pick one, click the map, and
a **macro object** lands with its region centred on the click. It is a
creation object with a field: `ToolKind::Macro`, geometry the captured
region's shape, the common properties, `edge_mode`, `feather`, hover and
selection outline on the region's edge, keyable position, rotation and
scale, and M13's motion checkbox. Its field at a step is the capture's frame,
sampled through M14's patch sampler in the object's frame and shifted by that
frame's recorded displacement when the capture moved — motion is relative, so
it begins wherever the click put it. The object holds a copy of the frames in
the project (`captures/` entry, by hash; two inserts of one macro share one
entry).

**Time.** Frame `f` is at macro time `f·Δt_m`; the object's step `s` is at
`(s − start)·Δt_p`. Equal steps map one to one. When they differ, the
object's `resample` option decides: **hold** (the frame at or before the
step's time — a macro is a thing the user placed and holds like a keyframe;
D48 is about a measurement and does not apply) or **interpolate** (blend the
two nearest frames, `u`/`v` linearly, undefined where either is; the
displacement likewise). Steps past the last frame show nothing unless `loop`
is set. The bar says what will happen before the click — "3-hourly capture,
hourly project: two steps in three interpolated" — and the option is per
object afterwards, so the temporal steps are always consistent by
construction rather than by the user's arithmetic.

**Kinds.** A wind macro in a current project, or the reverse, is allowed, as
showing a GRIB layer of the other kind is (spec §4.8); the list marks the kind.

**Acceptance**

- A moving 3-step stroke captured **static** with the region dragged to follow
  it inserts as three frames of the same stroke standing still; captured with
  **record movement** and the region left alone, the insert moves as the
  original did. Both checked against the stroke's known field.
- Zero and undefined: a capture over a stroke and open water, inserted over
  another field, shows that field where the water was.
- In capture mode `Space` does not play, a key drag is refused and the
  inspector is read-only; cancel restores everything and writes nothing;
  finish creates no object and exactly one library file.
- A 6-hourly capture in a 3-hourly project holds or interpolates per the
  option, with one interpolated cell computed by hand; an hourly capture in a
  6-hourly project reads every sixth frame.
- The macro object passes the catalogue's shared-rule tests, and both kernels
  agree on it.
- Across projects: capture in A, open B, insert; delete all in settings; B
  still renders.
- Capturing a 2,000 × 2,000 km region at 0.25° over 40 steps completes in a
  measured time with the map interactive throughout.

**Risks:** a capture is an export-shaped job, as slow as the region times the
steps, so it runs off the UI thread with a size estimate like the export's.
Capture mode is a cross-cutting lockout: it is one flag in the session that
every *backend* write path checks, not a set of disabled controls in the UI —
a control that was missed is a write during capture.

---

### M17 — Warp and liquify, two tools · **complete**

**Goal:** the placed warp and the painted smear as two tools (D56).

**Delivered.** `Warp` keeps the pull and the twist on a placed region; the
pull's line now reads out its length in km beside its head. **`Liquify`**
(`L`) is new: a stroke of stamps where each stamp carries the pointer's own
movement into it, scaled by `strength` and stored *in the geometry* — a new
`Geometry::Smear` of `SmearPoint { x, y, dx, dy }`. At a cell the displacement
is the sum of the deltas of the stamps that cover it, each faded by that
stamp's own feather ramp, and the field is read from behind the sum, as a push
reads from behind its push.

**Less of the geometry recipe than the plan expected, and deliberately.** The
smear's *footprint* is the capsule every swept tool already has: the deltas
ride beside it on `FlatObject::smear`, parallel to the capsule's chains, so
`sdf.rs`, coverage, the outlines, the cull and the WGSL learn nothing new. What
changed is a `Modifier::Smear` with no payload, the CPU's re-read
(`smear_source_position`), the GPU decline, the cache hash (the deltas are
hashed with the modifier — they are what the stroke *does*), and the anchor
re-expression, which moves the stamps and leaves the deltas alone as it does
a push's local displacement. Strength is frozen at creation because the
deltas are stored scaled; animating it would mean storing the unscaled stroke
and re-scaling per step, for an option nobody keyframes.

**Acceptance**

- **A liquify stroke east over a northward field reads from west of each
  cell**: a field stepping from 5 to 15 m/s at the meridian, dragged east ten
  degrees at full strength, reads 5 m/s three degrees east of the step and is
  untouched fifteen degrees out either way. Half strength does not reach.
- **Hand-computed displacement**: two stamps on a line, radius 50 km, feather
  0.5, delta 30 km each — a cell at the second stamp's centre reads
  `30·1.0 + 30·smoothstep(0, 25, 20)` km behind, and a cell at the second
  stamp's feather rim reads from where it is. Both written out in
  `cpu.rs`'s test, against `feather_weight`'s own ramp.
- **The GPU declines it**, in the same test that declines a warp, and the
  three modifiers that transform in place are still accepted.
- **A warp from before the liquify opens unchanged**: same kind, same mode,
  and it does not acquire a strength. The schema version did not move — the
  liquify is a new kind, not a reinterpretation.
- **Both tools pass the shared-rule tests** through the catalogue entry, and a
  liquify never merges — its deltas are its own.

**Not done, and why.** The twist's rotate handle on the pink edge. The
selection's own transform already offers a rotate handle on every object,
and giving the twist a second one on the edge band is a gesture-conflict
question — which rotates the *frame* and which the *field* — that deserves
its own decision rather than a guess in a milestone about something else. The
twist stays a number in the bar. The operate-on-the-map preview (D37) for the
liquify is the footprint outline every other modifier shows: a smear's
displacement varies along the stroke, so a shifted-camera preview of the kind
the clone stamp has would be wrong everywhere but one point, and a true one is
a per-pixel warp of the drawn map — a WebGL pass, not an overlay.

---

### M18 — Image layers · **complete**

**Goal:** a georeferenced image shows under the field.

**Delivered as planned.** `LayerSource::Image { path, placement, opacity }`,
display only — never composited, never evaluated, never exported, no field of
its own. The project keeps the path and six numbers; the pixels reach the
screen and nowhere else (invariant 2).

**Georeferenced on import** where the file says: a GeoTIFF's `ModelTiepoint`
and `ModelPixelScale`, or a world file beside a PNG, a JPEG or a TIFF. Both
reduce to exactly the six numbers a placement holds, so there is no conversion
— only a half-pixel, because a world file names the *centre* of the top-left
pixel and the placement names its corner. Half a pixel on a coarse chart is
tens of kilometres, so it has a test of its own. A **projected** GeoTIFF is
refused by name, saying what to do about it: reprojecting a raster is a
different piece of work from placing one, and metres read as degrees put an
image somewhere plausible and wrong.

**Placed by hand otherwise**, by three control points — top-left, top-right,
bottom-left. Three because three determine an affine exactly; a fourth handle
would let the user ask for a shape no affine can make. A plain drag shears
(the three-point case); shift keeps the image north-up and its aspect true (the
two-point case), which is what a chart scan almost always wants. The handles
belong to the **active layer** only, or a project with several charts under it
would stack handles from all of them on one corner.

**Rendering.** Decoded in Rust by `png`, `jpeg-decoder` and `tiff` — pure Rust,
no `-sys` crate between them, the same rule the GRIB codecs live under.
Downsampled to the caller's own `MAX_TEXTURE_SIZE` and re-encoded as a lossless
PNG, served through the tile scheme at `image/<revision>/<layer>/<max edge>`:
the revision makes the address immutable exactly as it does for a tile. **The
file on disk is never touched.** Drawn as a 16×16 subdivided quad between the
land and the field tiles, with world copies at ±360° — subdivided because the
placement is affine in *degrees*, and a straight line in lon/lat is not straight
on the map once the projection is not the flat one (M11 having just shipped two
that are not).

**Acceptance**

- **A GeoTIFF of known extent lands with its corners at the right lon/lat**,
  and so does one whose tiepoint is not the corner — the walk back to the
  corner is its own test. The fixtures are written by the test, byte by byte,
  including the IFD: a committed GeoTIFF whose provenance is "it decoded"
  asserts nothing about the tags being read right.
- **An image across the antimeridian keeps its longitudes unwrapped.** A
  60°-wide image starting at 150°E has its right edge at 210°, not at −150°;
  normalising it would put the right edge to the left of the left one and fold
  the picture in half.
- **A hand-placed image's control points survive a save and a reopen**, to the
  file's nine decimal places, and the layer says it was placed by hand rather
  than by the file.
- **A project with an image layer exports byte-identically** to the same
  project without it — asserted on the bytes, with the image laid directly over
  the painted stroke so a leak would reach the values compared.
- **The `.veproj` contains no pixel data**, asserted by looking for the
  fixture's own byte run in the saved file.
- Opacity is clamped, a drag of a control point is one undo, and three points
  on a line are refused rather than stored.

**One thing the plan did not mention and the code needed.** `Path` already has
an inherent `with_extension`, so the extension trait written to find `MAP.TFW`
beside `MAP.TIF` was shadowed at every call site and never ran — clippy caught
it as dead code. It is a free function now, which cannot be shadowed.

---

### M19 — Export precision · **complete**

**Goal:** the packing width as a visible choice against the file size (D53).

**Delivered.** The export dialog offers **8, 12, 16 or 24 bits per value**,
default 16 — which is what every export wrote before this was a choice, so an
old caller that sends nothing gets the file it always got. Beside the choice:
the step the width implies, in knots over a nominal ±60 m/s (half a knot at 8
bits, 0.004 at 16), and the estimated size, which now scales with the width
rather than assuming two bytes. Template 5.0 stays the only packing written;
the width is refused before a file is opened if it is not one of the four.

**Acceptance.** Every offered width writes a message that reads back within
its own step, through the writer's test verifier *and* through the import
decoder — the one that meets other centres' files and must meet ours at every
width. Section 5 records the width. The file shrinks in exactly the
proportion the width says, headers being equal. Every width is
byte-reproducible (invariant 4). The 8-bit step over ±60 m/s is `2^E` for the
`E` the packer chose, which is the number the dialog shows.

**Not done.** ecCodes decoding each width is a CI step, as the other external
decodes are (§12.5); it is not run locally. The in-repo import decoder standing
in for it is a weaker witness only for our own output — it is the decoder that
has been checked against ecCodes on ten centres' files.

---

### M20 — A GRIB layer's frames, copied between steps · **complete**

**Goal:** a step of an imported layer can be copied to another step of the
same layer, without creating an object and without copying a sample.

A forecast file rarely lines up with a timeline: a 6-hourly file in a
3-hourly project shows a message on every other step and nothing between
(spec §4.8, D48), a file ends before the timeline does, and a message is
sometimes simply bad. Today the only answers are another file or the hold
rule D48 removed for good reason. This gives the user the choice a keyframe
gives — *this* step shows *that* message — as an instruction rather than a
measurement.

**Deliverables**

- **The model.** `Layer` gains `frame_overrides`, a sorted `Vec<(step,
  Option<step>)>` — `#[serde(default)]`, no migration — mapping a step to the
  step whose message it shows, or to nothing. `RasterSequence::frame_at`
  consults it first: an overridden step returns the source step's frame,
  wherever the file put it; `None` hides the file's own message at that step.
  Everything else is untouched — the frame served is one the file already
  holds, so the flat scene's raster hash follows the choice and the render
  cache, readiness and both kernels are correct without a line changed
  (D59). **The project stores a step number, never a sample**: invariants 1
  and 2 stand as written, and `Layer::raster` stays `#[serde(skip)]`.
- **The gesture, in the timeline.** The GRIB layer's row already marks the
  steps its file has a message for (spec §9.2). Those marks become
  selectable: click one, `Shift`-click a run of them, `Cmd`-`C`, move the
  playhead, `Cmd`-`V`. A run keeps its spacing — steps 0 to 3 pasted at 6 land
  on 6 to 9. **The paste goes to the layer the copy came from**, whatever
  layer is active: the clipboard entry names it, and a GRIB frame pasted into
  another layer or into a project with a different file would be a copy of
  samples by another route. A step past the end of the timeline is dropped
  from the paste, not clamped.
- **A pasted mark is drawn as one** — the file's own marks stay solid, an
  override is an outline with the source step in its tooltip — and `Delete`
  on it restores the file's own message at that step, or the gap. `Delete` on
  a file's own mark hides that message at that step: the same override with
  `None`, which is what a bad message needs and costs nothing extra.
- Every change is a `Command` with an inverse, coalesced per paste, so one
  undo returns a run. The layer's speed band and visibility apply to a pasted
  frame as they do to any other.
- **The file may change under it.** Overrides reference steps, and a step's
  message is decided by time alignment on open (spec §4.8), so if the file at
  the path is replaced by one that has no message at a source step, the
  pasted step shows nothing and its mark is drawn empty, like a layer whose
  file has gone. The user's objects and the other overrides are unaffected.
- Precedence with M14: `Cmd`-`C` copies a frame when the timeline holds the
  focus and a mark is selected; the map's region and the object selection are
  untouched by it.

**Acceptance**

- A 6-hourly file in a 3-hourly project: copy step 0 to step 3, and step 3
  samples exactly what step 0 does at ten spot cells on both backends; the
  cache holds one frame for the two steps, since they hash alike.
- Copy steps 0 to 2 and paste at 4: steps 4, 5 and 6 show 0, 1 and 2; paste
  at the last step drops the two that fall off the end; one undo returns all
  three.
- An override on a step the file has a message for shows the pasted frame
  until `Delete` restores the file's own; `Delete` on a file mark hides it and
  the layer's painted objects stand alone there.
- Save and load round-trips the overrides byte-identically and the `.veproj`
  gains no sample data; the file replaced by one lacking the source message
  leaves the pasted step empty and marked.
- Pasting with another layer active still lands on the source layer, and the
  paste creates no object in any layer.

**Risks:** small. The one trap is a paste that copies the lattice rather than
the step number — a `RasterGrid` in the document — which the round-trip test
and invariant 2 forbid together.

---

### M21 — ICON's icosahedral grid · **complete**

**Goal:** an ICON global GRIB imports with no files but the forecast itself.

Asked for after M12 and built the same day. Unplanned, and out of the M12–M20
sequence.

**What it turned out to be.** Three things, none of them the decoder:

- **Template 3.101 in `ve-grib::decode`.** Thirty-five octets of which one is
  geometry — the shape of the earth — and the rest names the grid. `Header`'s
  grid becomes an enum, which was a four-line change at four call sites
  because nothing outside `decode` and `import` had ever touched it.
- **The bundled positions** (`ve-grib::icon`, `tools/icon-grid-builder`,
  `assets/icon_grids.bin`). ICON lists its cells in a space-filling order, so
  a delta between neighbours is small; stored on the lattice the source
  messages were packed on and deflated, the global mesh is 2.4 MB against
  11.8 MB of raw coordinates.
- **The resampling** (`ve-core::regrid`). A bucket index whose longitude count
  scales with `cos(lat)`, so a search ring covers the same ground at the pole
  as at the equator, and a ring that widens until the third-best distance is
  inside the ground it is *known* to cover — the answer does not depend on the
  bucket size being lucky.

**Acceptance**

- Every value matches an independent brute-force interpolation to six
  decimals, at eight points including both poles and both sides of the
  antimeridian. **Met.**
- A reopened project's neighbour set produces a field with the same content
  hash as a fresh search. **Met**, and at 48 ms against 477.
- No file but the forecast, and no network. **Met**: the positions are
  bundled and matched by UUID; an unbundled mesh is refused by name rather
  than placed on a guess.
- The asset costs less than the basemap it sits beside. **Missed, knowingly**:
  3.5 MB for two meshes against the basemap's 1.7. A proper LZMA encoder would
  reach 0.4 MB, but the only pure-Rust one loses to deflate, and a C library
  is out (D60's reasoning).

---

### M23 — Selection, clipboard and deletion · **complete**

**Goal:** the things the user found *wrong* rather than missing. Every item
here is a rule with one owner, so the later milestones can rely on it.

**Delivered 2026-09-05.** Everything below, with these notes. The
mutual-exclusion rule is two lines — `selectObjects` in `App` drops the
region through `MapHandle.clearRegion`, `selectRegion` in the map drops the
objects — and a pure reducer over them would have asserted only that the
two lines exist, so the promised `selection.test.ts` was not written; the
rule is stated in spec §8.2 instead. The multi-frame region copy measured
**1.97 s for a whole-map `Cmd`-`Shift`-`A` at 0.25° over 24 steps** of a
scene that changes every step — 24 frames of 1441 × 721 on the CPU, all of
it in the bake — against **0.10 s for one frame** of the same region
(`whole_map_animated_copy_cost` in `tests/capture.rs`, release). A still
scene bakes one frame and costs what it did. It runs on the command thread;
a 0.1° whole-map copy of a long animated timeline would be tens of seconds,
and if that turns out to be wanted the bake moves behind a progress event. The
timeline's own `Delete` (keys, GRIB frames) still comes first, through a
second callback beside `onFramesSelected` rather than a listener-order
assumption. The settings dialog's shortcut rows were also brought up to the
catalogue: it listed a fill tool that no longer exists and none of liquify,
measure, capture or insert.

The findings, in the user's numbering: 1, 3, 4, 5, 6, 7, 9 (the half that is
a bug), 25, 26, 30.

**What was actually broken, from reading the code**

- **A copied object's keyframes were not lost; the object was never copied
  (3, 4).** `App` stands down from `Cmd`-`C`/`V` whenever the map reports a
  region *or a held capture*, and `captured` is set when a region is copied
  and never cleared — so after one region copy every later `Cmd`-`C` did
  nothing and every `Cmd`-`V` pasted the same patch. The clipboard code
  (`ve_core::clipboard`) carries keyframes and always did.
- **`Cmd`-click moved the selection (5)** because two things line up. The
  hand tool's pointer-down starts a *move* on any click on a selected object
  once the hit test comes back, modifier or not; and a move is
  `SphereRotation::carrying(pivot, pointer)` — it carries the selection's
  **centroid** to the pointer, so a click anywhere on a member away from the
  centroid jumps the whole group so its centroid lands under the click. The
  same rotation is right for the centre handle, whose press *is* the pivot,
  which is how it went unnoticed.
- **A placed macro that did not appear (9)** is time, not drawing: a macro's
  frames run from the object's first active step, which is 0 for every new
  object, so a five-frame macro placed at step 12 has already ended unless
  loop is on. It also lands in the *top* layer, not the active one, as does a
  pasted patch (25).
- **Nothing refuses a GRIB layer (26):** creation, paste, insert, duplicate
  and the panel's drag-drop all take whatever layer they are given.

**Deliverables**

- **Object selection and region selection are mutually exclusive (1).**
  Selecting an object — a map click, a marquee, a panel row — clears the
  region; closing a region clears the selection. The region stays the map's
  state; `App` wraps `setSelection` and asks the map to drop its region
  through `MapHandle`, and the map calls `onSelect([])` when a region lands.
  One rule, tested in `selection.test.ts` as a pure reducer over the two.
- **One clipboard (3, 4).** The backend session owns the exclusivity:
  `copy_objects` drops the held capture and `capture_region` drops the object
  clipboard, and a `clipboard_kind` query says which is held. `Cmd`-`C` copies
  the region if one is selected, otherwise the objects; `Cmd`-`V` asks the
  backend what is held and pastes that. The `regionActive` and `captured`
  refs go. Acceptance: copy an object with keys at 5 and 9, paste at 12, the
  copy has keys at 12 and 16 (already asserted in `clipboard.rs`; now asserted
  end to end through the command layer, which is where it failed).
- **A region copy takes a run of frames (3).** `capture_region` bakes every
  step from the copy step to the last one, exactly as `capture_finish` does
  for a macro, and the pasted patch's active range **starts at the paste
  step**, which `capture_of` already uses as the run's origin — so a field
  copied at step 5 and pasted at 12 shows at 12 what the source showed at 5
  and carries on from there, not in sync with its source and not meant to be.
  Frames are **deduplicated by scene hash**: a still scene bakes one frame
  and holds it, so the common case costs what it costs today. The bake runs
  where the export does, on the CPU, and the whole-map `Cmd`-`Shift`-`A` case
  is bounded by the timeline length; the cost at 0.25° over 24 steps is
  measured and reported in this section when the milestone closes (D65).
- **A move carries the press point (5).** `motion_of` for `Move` becomes
  `carrying(gesture.pointer, pointer)`; the centre handle is unchanged, since
  there the press is the pivot. And a modifier click is a selection edit, so
  the hand tool never starts a move while `Cmd`/`Ctrl` is held. Acceptance: a
  transform test that presses 300 km from a two-object centroid, previews at
  the same point and asserts no member moved; and one that drags the body
  100 km east and asserts each member moved 100 km east.
- **`Delete` and `Backspace` remove the selection (6)**, from the map or the
  panel, through one new `remove_objects` command that is a `Command::Batch`
  so one undo returns all of them; a follower freed by a deleted primary is
  in the same entry, as `remove_object` already does. The timeline's own
  `Delete` wins while keys or GRIB frames are selected there, and a text
  field never loses the key.
- **Panel rows do not select text (7):** `user-select: none` on `.object` and
  `.layer-header`, and the rename field keeps its own.
- **Every path that adds an object takes the active layer, and none of them
  takes a GRIB layer (9, 25, 26).** `paste_capture`, `insert_macro`,
  `paste_objects` and `duplicate_object` gain the same `layer: Option<u64>`
  `create_object` has; `move_object` and the panel's drop refuse a layer
  whose `source` is a file. When the active layer is a GRIB layer, the
  creation is **refused** and the hint area says why (M25 gives it the
  place; until then the error reaches the status bar). The rule is one
  function, `document::creation_layer`, and the seven shared-rule tests gain
  an eighth: no tool, paste or insert can land in a raster layer (D66).
- **A placed macro begins at the step it was placed (9).** `insert_macro`
  takes `step` and sets the object's active range to start there, which is
  what `capture_of` measures from. Acceptance: a three-frame macro inserted
  at step 12 is visible at 12, 13 and 14 and not at 11.
- **`Shift`+arrows nudge (30).** With objects selected, one step of the
  selection by a screen distance — a `begin_transform`/`drag_transform`/
  `end_gesture` round trip at the centroid, so it is the same rigid move a
  drag is and one undo per press; with a region selected, the region moves
  instead. The map's pan, which held `Shift`+arrows, moves to `Alt`+arrows;
  the bindings table gains an `alt` modifier so it stays one table (D67).

**Acceptance**

- The reproductions above, as tests: the clipboard sequence (region copy,
  then object copy, then paste yields the object); the centroid press; the
  step-12 macro; a paste with a GRIB layer active landing in the painted
  layer above it.
- `Delete` on a three-object selection is one history entry; undo returns
  three objects and their followers' links.
- A region copy of a still scene bakes one frame; of a scene animated over
  steps 5–8 bakes four distinct frames and holds the last.

**Risks:** the multi-frame region copy is the one item with a cost, and it
is bounded by the same lattice arithmetic the macro bake already runs. If the
whole-map case at 0.1° exceeds a second, it is reported rather than hidden,
and the bake moves off the command thread behind a progress event.

---

### M24 — Cursors, hover and outlines · **complete**

**Goal:** the pointer says what a click will do, and a selected or dragged
object is outlined by its perimeter.

**Delivered 2026-09-05.** Everything below. `cursorFor` in
`ui/src/map/cursor.ts` is the table, tested for every tool and both
overrides; the map writes it to the element per pointer report rather than
rendering a class, for the reason the readout store exists. The eyedropper's
magnifier reads the readout store's sample and the store now carries the
stored azimuth beside the displayed direction, so the glyph is drawn from
the same number the click will take. The curve's hover needed one change in
the footprint builder — its stamp's size is `WidthKm`, not `SizeKm` — and
the palette's `has_hover` flag; spec §6.2's "none, deliberately" for the
curve is reversed and says why. `MacroEntry` gained `outline` and `track`,
read from the file's shape and frames by the library scan that already
opens each file. The drag outline goes through `drawEdgeBand` and the
static selected band no longer excludes the brush; while a drag is live the
static band stands down so the moving one is the only edge. The `VE_CAPTURE`
check of the magnifier was not run: it needs the running app, and the
numbers it would log are the readout's, which the readout already logs.

The findings: 2, 10, 11, 15, 16, 17, 18, 19, 20, 21.

Today the canvas has two cursors: `grab` everywhere and `crosshair` for the
brush and a pick. Everything else the map shows about the tool in hand is
drawn on the overlay. This milestone makes the cursor a rule with one owner —
`cursorFor(tool, context)` in `ui/src/map/cursor.ts`, pure and tested — and
the overlay's hover indicators a second table beside it.

**Deliverables**

- **The cursor table (16, 17, 19, 20).** Hand: an open hand, closed while
  panning; the palette icon is redrawn as a full open hand rather than the
  present palm-and-fingers. Select and capture: the arrow. Shape fill,
  circle, the modifiers and the measurement tools: a crosshair whose centre
  is the click. The curve: **the brush nib** — the tool's footprint at the
  pointer, drawn by the hover path the brush already uses (`schema.hover`),
  since a curve is painted with a stamp and the nib is what says how wide.
  Every custom cursor is an inline SVG data URI in CSS — no file, no fetch
  (invariant 5).
- **The paint bucket (2).** With a region selected and the brush or one of
  the four operators in hand, the pointer *inside* the region is a bucket,
  and outside it is the tool's own cursor. It is the same predicate the click
  uses (`editsRegion(tool) && regionContains(region, …)`), evaluated on
  pointer move and nowhere else.
- **The eyedropper (15, 21).** The option bar's "⌖ Sample field" becomes an
  eyedropper icon. While it is armed the overlay draws, at the pointer, a
  magnifier ring with a plus at its centre, the field's vector as the
  project's glyph — barb or arrow — and the speed and direction in the
  project's units and convention. The numbers come from the readout store the
  cursor readout already keeps, one sample in flight, so arming the
  eyedropper costs no second IPC stream. After the sample the tool's own
  cursor returns.
- **The insert hover (18).** The macro's region outline follows the pointer
  before the click, and a macro that recorded movement draws its track —
  the per-frame displacement as a polyline from the outline's centre, with a
  dot per frame. `MacroEntry` gains the region's `shape` and its `track`;
  both are read from the file's header and lattice, which the library scan
  already opens.
- **A dragged stroke is outlined by its perimeter (10).** `drawDragOutlines`
  strokes each stamp of a swept chain today, which is the ring-trail bug the
  edge band fixed for the static outline in a different function. The drag
  outline goes through `buildFootprintPath` and `drawEdgeBand` like the
  static one, so the union's boundary is what moves. `footprint.test.ts`
  gains the case: a two-stamp chain's drag outline has one boundary.
- **The brush is outlined when selected (11).** `drawOverlay` excludes
  `tool === "brush"` from the selected band on the reasoning that its field
  shows where it is; the user wants the edge, and every other tool has it.
  The exclusion goes, and the hover band on the brush stays as it is.

**Acceptance**

- `cursor.test.ts` covers the table for every `ActiveTool`, the bucket
  predicate inside and outside a region, and the eyedropper's precedence over
  the tool's cursor.
- The eyedropper overlay is checked the way the glyphs were: `VE_CAPTURE=1`
  renders it over a known field and the log carries the sampled numbers.
- A swept chain of 200 stamps dragged draws one band; a selected brush stroke
  draws one yellow band.

---

### M25 — Chrome: the top row, the status bar, the panels and the timeline · **complete**

**Goal:** the controls that are not about painting move out of the painting
toolbar, every panel can be put away, and two things that felt slow or
unreachable are fixed.

**Delivered 2026-09-05.** Everything below, with D69 as settled: the
start-time field and its clear button are gone and nothing replaces them.
**The speed filter's slowness was the frontend, measured**: one coalesced
`set_layer_speed_range` write costs under a microsecond on the backend
(`speed_filter_write_cost`, release: 200 writes in less than a tenth of a
millisecond), so the cost per slider tick was the round trip, the
`ProjectSummary` re-render of every panel, and the invalidation of every
visible tile of the imported field — 38 ms a tile on the CPU (§13), a
second or more per tick at a full viewport, with the slider's value held
back until the document returned. The thumb follows the hand now and
**nothing is written until the pointer comes up**: the release writes the
band once, one history entry per release. A first attempt kept one write in
flight during the drag, latest wins; the user found the thumb still
lagging, because every write that did go out re-rendered the panels and
the tiles under it, so the drag now writes nothing at all. Found by hand
after that: the two thumbs were tied together near each other, and the thumb
jumped back at release. Each slider showed the *ordered* band, so a thumb
pushed past the other was pinned there and the other one followed the
pointer; and the local band was dropped at release, a round trip before the
document had the new one. A dragged thumb stops at the other now, and the
released band is shown until the panel has read it back (`SpeedFilter.tsx`,
with the first component test in the frontend — under happy-dom, since
jsdom 27 cannot load on the Node 21 this is developed on). The first cut of
that held the band only until the write's promise settled, and the user
still saw the jump: the band the panel shows comes from the layer tree,
fetched *after* the summary, so the thumb fell back for one more round
trip. A four-lens fan-out over the release path then found the guard
against a no-op release was dead as well — the document's f32 m/s never
equals the slider's integer knots — so every release wrote and the sliders
held values a step-1 input snaps away from. The held band is keyed to the
revision the write returns now and let go when the tree fetched at that
revision lands; the stored band is rounded to the slider's step; the tree
fetch drops a response a newer fetch has overtaken; and the test fixture
goes through `Math.fround`, which is what let the old fixture pass. The
release-to-last-tile time was not measured: it is the tile cost times the
viewport, unchanged by this milestone. The
offline check gained one allowance, the SVG XML namespace, which the M24
cursors need and which no parser fetches. The history and properties
panels' own headers moved into the shell, where the fold lives.

The findings: 8, 12, 13, 14, 22, 23, 24, 27, 28.

**Deliverables**

- **The top row (8).** The title bar gains a centred slot holding, from the
  map's toolbar: the measure tool, the projection menu, the glyph menu, the
  graticule switch, and undo and redo as icons. The **capture** tool moves to
  the title bar beside the project buttons; the **insert** tool stays in the
  palette. "Step n / m" leaves the toolbar; the timeline already says it. The
  state stays where it is — `tool`, the glyph style and the graticule are the
  map's — and the controls reach the title bar through a React portal into a
  slot `App` owns, so nothing about the map is lifted for the sake of where a
  button sits (D68).
- **The timeline's right-hand side (12, 13, 14).** The start-time field and
  its clear button go, and nothing replaces them: a start time exists only
  where a file supplied one — a project opened from a GRIB or with one
  imported — and is otherwise asked for at export, where it is needed
  (D69). The ruler labels forecast hours until a file gives it a clock. The
  loop button is drawn larger, as an icon.
- **Collapsible panels (22).** The left sidebar, the right sidebar and the
  timeline dock each collapse to a strip with a toggle; inside the right one,
  properties and history collapse separately; and each layer's object list
  folds under its header. What is open is remembered in `localStorage` — a
  viewer's convenience, not a project's fact — and a collapsed map neighbour
  changes the map's size, which goes through the existing resize path.
- **Autosave is a setting (23).** `AppSettings.autosave` is one of `off`,
  `recovery` (today's behaviour: a snapshot every 60 s or 50 edits, offered
  back on the start screen) and `save` (the project file written in place on
  the same cadence when it has a path, a recovery snapshot when it does not).
  The thread reads the setting each tick. Default `recovery`, so nothing
  changes for an existing install (D70).
- **The speed filter follows the hand (24).** The slider's value is local
  while the thumb is down, and the write is one in flight, latest wins — the
  drag preview's pattern — with the release writing the final band under the
  same coalescing key, so a drag is still one undo. Measured before and after
  on the 0.25° reference file: reports per second the slider survives, and
  the time from release to the last tile.
- **The project is renamed in place (27).** Clicking the name in the title
  bar edits it; `Command::SetProjectName` exists in `ve-core` and is exposed
  as `rename_project`, undoable like a layer rename.
- **A hint area (28).** The status bar's middle carries one line: the tool's
  hint, or the last error, whichever is newer, from a store every panel and
  the map write to (`createHintStore`, the readout store's shape). The capture
  bar's "Draw a region first" moves there, and the map's, the timeline's and
  the panels' error text with it.

**Acceptance**

- Every control that moved is reachable and does what it did: the
  reachability check from the milestone workflow, plus `toolbar.test.ts`
  asserting the slot's contents.
- The speed-filter numbers, in this section, before and after.
- Autosave `save` writes the file and leaves `dirty` false; `off` writes
  nothing in a session of 200 edits; `recovery` is byte-for-byte today's.
- A renamed project round-trips and undoes.

---

### M26 — Macro recording as keyframes, with a preview · **complete**

**Goal:** recording a macro is editing a position track, and what was
recorded can be seen before it is kept.

**Delivered 2026-09-05.** Everything below. Two things the build found.
The map had never moved the region to a step's position on scrub — each
frame "held its own position" in the session and the map drew the region
where it last was — so the keys would have been invisible; visiting a step
now keys it *and* shows it there, through one command. And a preview
revision seeded from the clock in the same millisecond as the project's
*was* the project's revision, which the test caught on its first run; the
preview's revisions carry a bit no document revision reaches. The preview's
playback treats every frame as ready by the backend's account and asks the
map alone, since the readiness strip is the document's. What was not done:
the fifty-object stress and the capture suite were not run against the
preview scene, and the interpolation is a great circle rather than the
easing curves an object's track offers — a capture's keys have no easing
menu, and none was asked for.

The finding: 29, with the remainder of 18 and the cursor rule from M24.

Today a capture's positions are a map from step to place, held wherever the
user did not drag, with no way to see or remove one, and finishing writes the
file at once. This makes the positions **keyframes** in everything but the
document, and adds a **preview** phase between recording and saving.

**Deliverables**

- **Positions are keys (29).** Every step the playhead visits while recording
  gets a key at the region's current position; dragging the region at a step
  sets that key; a key can be deleted, and the position **interpolates**
  across the gap by great circle, which is what "jump forward and change the
  position" asks for. `capture_finish` and `capture_mode(step)` both read
  through one `position_at(step)`, so the bake and the map agree. A new
  `unplace_capture(step)` removes a key; the first step's cannot be removed.
- **The timeline shows the track.** While a capture runs a temporary row,
  **selection position**, sits under the ruler with a diamond per key and the
  dotted interpolated steps every other track has; `Delete` or `Alt`-click
  removes a key. Steps **before the first step are dimmed and refused** —
  the ruler will not scrub into them and the arrows stop at the first step.
  The "● Recording" label stays through recording and leaves for the
  preview. The position keys are session state and nothing else: not the
  document, not history, not a `.veproj`.
- **A preview phase (29).** Finishing a recording bakes the capture **into
  the session** rather than to disk and enters preview: the map shows the
  basemap and nothing else, the macro region highlighted in green, "Macro
  Preview" in the map's bottom-right, tool selection disabled. A click stamps
  the macro at the pointer and the timeline loops over its frames. The
  buttons are **Save macro**, which writes the file and ends the session,
  **Edit macro**, which returns to recording with the keys intact, and
  **Cancel**. The preview is rendered by the tile pipeline from a **preview
  scene** held in the session — the baked capture as one object at the
  stamped anchor, on an empty project — served by `protocol::serve` under a
  fresh revision, so the document is never written and nothing is hidden by
  editing visibility while the history is locked (D71). Steps before the
  first are dimmed in preview as in recording.
- **The panels dim (29).** During recording and preview the left and right
  sidebars are dimmed and dead to the pointer instead of showing refusal
  text; the history lock is unchanged and still the thing that makes this
  safe.
- **The cursor during recording** is the selection cursor from M24's table.

**Acceptance**

- Keys at 3 and 7 with the region moved 5° east at 7: `position_at(5)` is on
  the great circle between them; delete the key at 5 after placing one and
  the position at 5 is interpolated again. A bake over 3–7 stores the
  interpolated displacement per frame when movement is recorded and none when
  static.
- A preview never writes: the history's entry count before recording equals
  the count after cancel, after edit, and after save; the document hash is
  unchanged through all three.
- Preview tiles are served without a document write: the project's revision
  is unchanged while the preview's revision differs, and leaving the preview
  restores the map without a re-render of a tile that was cached before it.
- Scrubbing below the first step during recording leaves the playhead at the
  first step.

**Risks:** the preview scene is a second source of tiles, and every rule
about tiles — one path through `protocol::serve`, content-hashed keys,
readiness by cache probe — has to hold for it. It is kept small by being an
ordinary `Project` value with one layer and one object, flattened by the same
`flatten`, so the only new thing is which project the session hands the
protocol.

---

### M27 — Polish from the second pass · **complete**

**Goal:** the thirteen findings the user handed over on 2026-09-06 after
using the app with M23–M26 in it, delivered in order, one commit each, with
anything not done recorded here with its reason.

**Delivered 2026-09-06**, thirteen commits `M27.1`–`M27.13` on main, plus a
follow-up to 7 for the stylesheet test that asserted the old insets. The
reachability sweep — every `api.*` method in `ui/src/ipc.ts` with no caller
— finds only `setStartTime` (retired by D69) and three state queries that
`clipboardKind` superseded; everything added here is reachable from the
title bar, the map or a chord. **Not clicked through in the running app**:
item 5 in particular is a fix for the one cause the code shows, and item 12
was measured by its tests and not by eye.

**Follow-up, 2026-09-06.** The stamp hover's track was a dot per frame; the
user asked for the motion shown as a box at each keyframe with the path
between. A saved macro holds no keys, so `trackKeyframes`
(`ui/src/map/macroTrack.ts`, tested) reads them off the track — the frames
where the step changes, which under great-circle interpolation is exactly
where a key was placed, a pause included — and `drawMacroFootprint` draws
the outline at each, fainter, behind the one at the pointer.

1. **Hover indicator stands down inside a selected region.** Delivered: the
   footprint was drawn under the paint bucket, so a click that would fill
   the region looked as though it would paint a spot. `showsHoverIndicator`
   (`ui/src/map/hover.ts`) is the rule, pure and tested beside `cursorFor`,
   and takes the same inside-the-region predicate the cursor and the click
   use; every filling tool goes through it.
2. **`Ctrl`-`Shift`-`V` pastes a still.** Delivered: `PasteTiming::Still` in
   `ve-core` freezes every property at the copy step (`value_at`), drops
   keys, motion and links and gives the copy the whole timeline; the app's
   two paste commands take a `still` flag, and a captured region pasted
   still is rebuilt as a one-frame capture with its own hash. The chord is
   `Ctrl`-`Shift`-`V` on a Mac and `Ctrl`-`Alt`-`Shift`-`V` elsewhere,
   where `Ctrl`-`Shift`-`V` already means absolute timing.
3. **Shift-click in the layer panel selects no text.** Delivered: the
   layer and object lists are `user-select: none`, the rename field alone
   keeps text selection.
4. **The macro stamp's hover outline.** Delivered: the outline and the
   track were drawn (M24) but *behind* the overlay's `!schema` return, and
   the insert tool has no schema, so the code was never reached. The block
   now runs before that return. The preview's own stamp hover is item 13.
5. **The shape fill's click neither magnifies nor samples.** Delivered as
   far as the code shows a cause: the one thing on the map that magnifies
   and shows the wind at a click is the eyedropper's magnifier, and it
   stayed armed through a polygon's vertices. `showsMagnifier` now stands it
   down — no magnifier drawn, no click sampled — while a polygon or a curve
   is being built. **Not reproduced from the code**: no other path in the
   polygon's click magnifies anything; if the symptom survives, it is
   something else and needs the exact gesture.
6. **The magnifier hides the tool's hover glyphs.** Delivered:
   `showsHoverIndicator` takes `magnifying`, so no tool's footprint or
   glyph is drawn under the magnifier.
7. **The map is full-window behind the panels.** Delivered: a positioned
   `.stage` holds the map absolutely, edge to edge, with the workspace row
   and the timeline floating over it (`pointer-events: none` on the row,
   back on for the panels), and the map's chrome is inset by `--dock-left`,
   `--dock-right` and `--dock-bottom`, which `App` sets from the panel
   state. A fold no longer resizes the canvas, so nothing re-renders.
8. **Smaller `Shift`-arrow nudges.** Delivered: 3 CSS px a press, from 8.
9. **Capture, measure, undo and redo buttons alike and aligned.**
   Delivered: the view controls' icon buttons are one fixed box, 30 by 26,
   icon centred, and the dividers stand the same height, so an SVG's own
   height no longer sets a button's and the row lines up.
10. **Panel toggles on the border, still.** Delivered: `DockToggle`, one
    tab per panel on the stage's edge, placed by the same dock variables
    the map's chrome uses, centred along its edge; a closed panel renders
    nothing, and the timeline's own collapse button and collapsed strip are
    gone.
11. **The rendering count moves to the status bar.** Delivered: the hint
    store gained an `activity` slot the map sets from its pending-tile
    count, and `StatusHint` leads its line with it; the title bar's span is
    gone.
12. **Auto scale.** Delivered: `AppSettings.auto_scale` and
    `set_auto_scale`; each tile's speed range is read from its bytes at
    upload (`tileSpeedRange`, tested), the renderer accumulates the range of
    the tiles it drew and returns it from `render`, and the map applies it
    to the next frame's ramp — `uRampMin` joined `uRampMax` in the raster
    shader and `rampCss` — when it has moved by more than 2% or 0.1 m/s. The
    toggle is in the title bar beside the graticule; the legend's ends
    follow and say *auto*. Found on the way: `draw` is built once and read
    `rampMax` by closure, so a colour scale changed in the settings reached
    the map only when something else rebuilt the callback; both ends go
    through a ref now, with a redraw when either moves.
13. **The macro preview: frame, lockout, stamp cursor, autoplay, badge.**
    Delivered: a green `.map-preview-frame` round the visible map, inside
    the docks, in place of the ring at the stamp; the overlay draws nothing
    of the document while previewing — only the stamp hover, the macro's
    region outline and track at the pointer, through the same
    `drawMacroFootprint` the insert tool uses, with the track carried on
    `CaptureMode.track`; the timeline stays mounted when put away, so the
    preview's loop plays with it hidden; the badge sits at the bottom
    centre, off the legend.

---

### M28 — Polish from the third pass · **complete**

**Goal:** the five findings the user handed over on 2026-09-06 after the
M27 build, in order, one commit each.

**Delivered 2026-09-06**, five commits `M28.1`–`M28.5`. The reachability
sweep finds the same four uncalled methods as M27's and nothing new; the
stamp-only preview command is gone with its caller. Not clicked through:
the eraser's sweep and the loupe in particular want a look in the app.

1. **A real eraser.** Delivered: `ERASE`, a frontend-only tool with the
   others of its kind, a palette button, `X`, an icon and a crosshair. It
   sees every object of the creation layer through `object_outlines`'
   new `all_in_layer` mode, highlights the one under the pointer in pink,
   marks everything a drag passes over — coalesced positions included — and
   on release calls `erase_objects`: `objects_remove` for every frame, or
   with `Ctrl` a `SetProperty` batch keying each object's `Enabled` off at
   the step alone (`hide_at`, with keys either side so the switch holds
   where it was). Three tests in `editing.rs`.
2. **The preview places on click and hides the document at once.**
   Delivered: `place_preview` replaces the stamp-only command — the baked
   capture becomes a macro object in the document through `macro_object`,
   the builder the library insert now shares, pushed through the lock
   (unlocked for one push, relocked whatever happened) with the stamp moved
   to the click; and the map holds a previous frame's tiles only from the
   same scene, preview or document, so the layers vanish on entering the
   preview rather than at the first click. The layers stayed because the
   document's last frame was *held*, dimmed, under the preview until the
   preview's own first frame had landed and been replaced. Test in
   `macros.rs`.
3. **Shift-click in the layer panel still selected text.** Delivered: M27's
   rule was the unprefixed `user-select`, which WebKit does not read; the
   prefixed one is beside it now, and the rows refuse a shift-press's
   default as well.
4. **The magnifier magnifies.** Delivered: the ring holds the GL canvas
   under the pointer at 2.5×, clipped to the circle, copied in the same
   frame as the GL pass; a pointer move with the eyedropper armed requests
   a whole frame, since the drawing buffer is not preserved between frames
   and an overlay-only redraw would copy a cleared one.
5. **Undo and redo sit level with capture and measure.** Delivered: the
   history group is `display: contents`, so its two buttons are items of
   the same flex line as the other two and the line centres all four alike.

---

### M29 — The fourth pass · **complete**

**Goal:** the fifteen findings the user handed over on 2026-09-06 after the
M28 build, in order, one commit each, with anything scoped down recorded
here with its reason. Two of them reopen recorded decisions on the user's
instruction: 13 restores a start time to the timeline (D69), and 8 makes
the field kind a property of the layer rather than of the project (§4.1).

**Delivered 2026-09-06**, fifteen commits `M29.1`–`M29.15`. Two decisions
were reopened on the user's instruction: D69 (13) and the project-level
field kind of §4.1 (8). Not clicked through in the app; 8, 14 and 15 most
want it.

1. **Layers drag above and below other layers.** Delivered: a drop names
   the half of the row it landed on, `layerDropIndex` (tested against the
   order the panel would then show) turns that into the document index
   `MoveLayer` wants, the row shows a line on the edge the drop will take,
   and each row has a grip and WebKit's `-webkit-user-drag: element`, which
   is what lets a row with no selectable text be picked up at all.
2. **The dividers read out while the second point is chosen.** Delivered:
   `preview_measurement` builds the leg as `add_measurement` would and
   returns its view without storing it; the map asks for it one-in-flight
   as the pointer moves and draws it with the placed measurements. Tested
   equal to the placed view.
3. **The timeline lists layers and objects in the panel's order.**
   Delivered: both loops reverse the document's bottom-first order, as the
   layer panel does.
4. **A new object is the selection.** Delivered: `create_object` returns
   `Created` — the summary and the object it made or merged into — and the
   map selects it as the gesture lands.
5. **The follow button does something on a click.** Delivered: it was a
   drag source only. A click now arms the link — the matching rows light,
   the status bar says what to click — and the next click on another
   object's row of the same property completes it; the drag still works.
6. **Divergence strokes with the same settings merge.** Delivered — by
   changing what a divergence radiates *from*: the anchor made a merge
   change the field, so it was excluded. It radiates from the nearest point
   of the stroke's own centreline now, on both backends, tapered to nothing
   within 100 m of the line where a bearing is ill-conditioned, and joins
   the merge list. The scene hash gained `EVALUATOR_VERSION`, since a
   kernel's meaning changed with no byte of the scene changing. Two tests
   in `merging.rs` against geometry: north of an east–west line, out is
   north.
7. **The motion button's help text.** Delivered: "Add velocity / rotational
   / scale vectors to the vector data." by track, and "Remove …" when on.
8. **Field kind per layer, and a two-field export.** Delivered, and the
   largest change of the pass. `Layer.parameter` (a `FieldKind`, serde
   default wind; a GRIB layer's is its file's; an image has none), schema
   11 with a migration that gives every layer the project's kind and turns
   the one colour scale into the wind/current pair; `flatten_kind` beside
   `flatten`; `Project::kinds_present`. The app's tile address gained a
   kind segment, the scene cache and the render pool prepare per kind, and
   the readout, eyedropper, region copy and macro capture take the kind on
   show; the export writes a u/v pair per present kind per step, wind
   first. `set_layer_parameter` and a per-kind `set_colour_scale`. In the
   frontend a *Show* menu in the title bar, following the active layer, the
   layer panel's field row, two scale fields in the settings, and the
   new-project dialog no longer asks. `settings.field_kind` stays as the
   default a new layer takes. Tests: the migration, the layer command and
   its undo, and a two-kind export decoded message by message.
9. **The intensity slider.** Delivered: `PropSpec.slider` declares a
   centred slider — named ends, an optional reversed sense — carried on the
   option spec and the property view, so the option bar and the inspector
   both render `CentredSlider` for it without knowing the tool. The
   intensity's amount runs −100% to +200% with 0 in the middle and as its
   default; the track is piecewise linear about zero (`sliderMath`,
   tested), and the inspector commits on release.
10. **The divergence slider.** Delivered: the same declaration, reversed,
    with *Divergence* and *Convergence* as its ends and 0 as the default;
    the readout names the side the value is on ("50% Divergence"), tested.
    The stored sign is untouched, so no migration and no kernel change.
11. **The turn tool's direction and amount.** Delivered: `TurnSense`
    (clockwise, counter-clockwise) and `TurnAmountDeg` (0–180) replace the
    signed `TurnDeg`; the evaluator turns by the signed product, so no
    kernel changed. Schema 12's migration splits every saved turn, keys
    and all — a stepped sense key wherever the sign was — and is tested.
12. **The warp does something.** Delivered: a freshly drawn push warp's
    target was its own anchor — nowhere — until it was re-aimed with Shift,
    so a drag did nothing at all. The target is the stroke's last point now,
    so the field under where the drag began is dragged to where it ended;
    a click still pushes nowhere and a named target still wins. Tested.
13. **A start time on the timeline.** Delivered, superseding D69 on the
    user's instruction: *Add start time* on the Time row opens an inline
    UTC date-and-hour editor that calls the `set_start_time` command M25
    left in place; the step readout shows the frame's UTC time beside its
    forecast hour; × removes it. The export dialog pre-sets its start to the
    project's, else to the current UTC hour rounded to the nearest. The
    draft and rounding helpers are tested.
14. **The macro preview places into a scene of its own.** Delivered,
    replacing M28.2's document write, which put the copy in a layer the
    preview hid and left it behind afterwards. A click now adds a stamp to
    the active capture and the preview scene is rebuilt with a copy of the
    macro at each, looping with the original; nothing is written, the lock
    stands, Edit clears the copies and Cancel leaves no trace. Tested. The
    animation not showing at all is **not reproduced**: the backend's
    preview scene samples the macro at the stamp (the tests say so), so the
    remaining cause is in the running app and needs a look there — most
    likely playback waiting on tiles that take seconds a step on the CPU.
15. **The eraser erases what it covers.** Delivered, replacing M28.1's
    whole-object eraser. `Object.erased` holds swept stamps in the object's
    frame and `Layer.erased` holds them in geographic space for a GRIB
    layer; the CPU multiplies coverage by what they leave and drops a
    covered lattice node; the GPU declines a scene holding one, the patch's
    fallback; the scene hash covers them. `erase_stroke` decides per thing
    under the brush: an erasure on a painted object, deleted when a sampled
    lattice over its footprint finds nothing left; a rewritten capture with
    its own hash for a patch or macro; a raster stamp for an imported layer.
    One batch per sweep. The map's eraser has the brush's bar — size in km
    or px, shape, feather — a footprint preview, and `Shift` for one frame.
    Tests in `evaluation.rs`, `fidelity.rs`, `editing.rs` and `capture.rs`.
    **Not done as asked**: a GRIB layer's samples are not overwritten in the
    file; the stamp is kept beside the path and applied on read, which is
    what invariants 1 and 2 allow, and the user sees no object for it.

### M30 — Both kinds on the map · **complete**

**Goal:** the user's instruction of 2026-09-06 after the M29 build: remove
the title bar's 10 m wind / surface current selector, always show both
kinds, always draw wind as barbs and currents as arrows.

**Delivered 2026-09-06**, one commit. The renderer takes a list of field
passes instead of one frame: each pass names its tiles, its held frame,
its ramp and its glyph style, and the raster and glyph stages run once per
pass, wind first and currents above. `MapView` builds one pass per kind in
`kinds_present` — the preview's one kind while a macro preview is shown,
which `CaptureMode.kind` now says — holds the last complete frame per kind,
keeps the auto-scale range per kind, and shows a legend entry per kind
with its own scale. `warm` prefetches every kind and reports resident only
when all are; the render pool's `request` and `readiness` with no kind
named cover every present kind, a step's total being tiles × kinds. The
*Show* menu is gone, and the glyph menu is a switch: the kind chooses the
style, and a gesture's preview draws the active layer's. The active layer's
kind still steers the readout, the eyedropper, a region copy and a macro
capture. Spec §5.3 and §5.5 rewritten. Not clicked through in the app: the
composition of two rasters and two glyph sets over one another is the part
most worth a look. **Superseded by M31 the same day**: the two passes are
one composite again.

### M31 — Layers composite on their own, and a tile carries what was written · **complete**

**Goal:** the user's four findings of 2026-09-06 after the M30 build: every
edit seemed to redraw the whole map; edit objects must apply only in their
own layer, and GRIB layers must take them; changing a GRIB layer's speed
filter re-rendered the whole map; and in blending, zero must not be
transparent — only undefined — with the layers rendered in order and the
layer beneath shown only where the one above is undefined, never a defined
wind and a defined current together.

**Delivered 2026-09-06.** Three commits.

1. **The composite.** `flatten` takes every visible layer of every kind in
   stack order and tags each object and raster with its layer and kind;
   `flatten_kind` is the export's and a capture's one-kind view of the same
   walk. Both kernels composite each layer on its own — a modifier, a clone
   and a warp read their own layer beneath them and nothing lower — and
   stack the layers by coverage, premultiplied, the kind of a cell being the
   topmost layer's that covers half of it. The evaluators return a `Sample`
   (field, coverage, kind); the export still takes the field alone. The
   tile is one 32-bit word a texel — speed 14 bits, azimuth 12, coverage 5,
   kind 1 — and the shaders draw alpha from the coverage, the ramp and the
   glyph from the kind, on one lattice. The address lost its kind segment,
   the pool and the readiness probe prepare one scene a step again, the
   readout samples the composite and says *no field* where nothing wrote,
   and `EVALUATOR_VERSION` is 3. A warp carries the coverage of where it
   read from, or a patch dragged onto open water vanished. Spec §5.3, §6.3,
   §7.6 and §7.7 rewritten. The tile-cost benchmark is at parity with the
   code before it (dense standard 44–48 ms against 47–54 ms), and both are
   over the 40 ms CPU-fallback budget on the development machine today;
   the GPU path is at 5–8 ms. Not reduced here.
2. **Edit tools on a GRIB layer.** `creation_layer` takes what is being
   placed: a field or an edit. An imported layer takes an edit — a
   modifier, the mask, the clone stamp, and the eraser it already took —
   and refuses a field, with the hint saying so; a paste is an edit only if
   every object in it is; a move and a duplicate ask about the object
   moved. With the composite of 1, a modifier in a GRIB layer edits the
   file's field and reaches nothing beneath. D66 reopened in part, on the
   user's instruction. Test: an intensity stroke on an imported layer
   doubles its field where it lands and leaves it alone elsewhere, while a
   brush there is still refused.
3. **A tile is keyed by what reaches it.** `ve_render::cull` digests every
   object once per frame and gives each tile the sub-scene whose caps touch
   it — inverted masks always, a clone, a warp or a liquify with its whole
   layer beneath — hashed from the digests and rendered from the sub-scene,
   which the tests hold to render identically to the whole. The protocol's
   `Frame` and the pool's `Prepared` key per tile, the readiness probe caches
   keys per step, and the backend is chosen per sub-scene, so the eraser or
   a clone stamp sends only the tiles it reaches to the CPU. The map's
   `TileCache` keeps textures by key: a draw asks `tile_keys` for the tiles
   in view once, reuses every texture whose key it holds and fetches the
   rest. An edit therefore redraws the tiles it reaches and leaves the rest
   undimmed; a step change on a still scene fetches nothing. Spec §7.10.
   **On the speed filter (finding 3):** a filter change re-keys every tile
   the imported field reaches — a global forecast is every tile — so the
   whole map does re-render on release, as it must; what changed since it
   was cheap is that M30 rendered two kinds' tiles and M31 renders one
   again, and the GRIB's tiles go to the CPU only where an eraser or a
   clone reaches them. Not clicked through in the app. **Superseded in part
   by M33.1**, which culls the imported field per tile too.

### M33 — The fifth pass · **complete**

**Goal:** the eight findings the user handed over on 2026-09-06 after the
M32 build, in order, one commit each, with anything scoped down recorded
here with its reason.

**Delivered 2026-09-06**, seven commits `M33.1`–`M33.7`; findings 2 and 6
are one item. Two of them had been reported before and not found: the
macro preview (9) had never worked because a u64 revision cannot survive
a JavaScript number, and the speed filter (4) is now local except at a
band's low end. Nothing clicked through in the app; the reorder and the
preview want it most.

1. **An imported field is culled per tile** (findings 1 and 4). An erase
   and a change to a speed filter re-rendered the whole map because every
   tile kept every raster whole, erasures and band included, so any edit to
   one re-keyed every tile. `RasterGrid::window` and `max_speed_in` answer
   what a tile can read of a lattice; `cull::tile_raster` drops a raster the
   tile lies outside, keeps only the erasures whose stroke reaches it, and
   clamps the band's top to the fastest node the tile holds — a band over
   everything the tile has keys it as no band at all. The per-tile maximum
   is memoised by lattice hash and tile. What is *not* fixed: dragging the
   low end of a band across the speeds a global forecast holds under a tile
   does re-render that tile, because it changes what the tile shows. Tests
   in `cull.rs`.
2. **The eraser draws no chain of circles** (findings 2 and 6). Its swept
   footprint was stroked, and a swept footprint is a union of stamps that
   `Path2D` cannot take, so the stroke traced every stamp's own circle. The
   sweep is filled and never stroked now; the dashed nib is one stamp,
   whose outline is its own silhouette. The band the outlines use would
   also do, but it is `destination-out` and has to run before anything else
   on the frame, and the stroke already shows what it does — the field
   leaves it as the pointer moves (M32).
3. **Smooth edges, and glyphs that stay their own size** (finding 3, and
   the screenshot). A swept preview stamped every half radius, which leaves
   a scallop of a thirty-second of the radius — invisible on a small brush
   and a three-pixel bite out of a large one, the arcs the user could see.
   The spacing now bounds the scallop at 0.4 px, so it grows with the
   square root of the radius and a small brush costs *fewer* stamps than
   before. Separately, the glyph ladder's rungs were up to two and a half
   times apart and a glyph is sized from the spacing a rung gives, so
   zoomed in one barb covered the stamp it described: the ladder is now
   half-step rungs and `glyphSizeScale` holds a glyph to the size its style
   asked for wherever the lattice is wider than that, on the map and in a
   gesture's preview alike.
4. **A macro reaches as far as its frames** (finding 5). An inserted macro
   took a range to the end of the timeline while drawing nothing past its
   last frame (§8.7, D65), so the timeline showed it spanning steps it
   painted none of. The range now ends at the frame span in project steps;
   turning `loop` on afterwards is what the range handle is for. A still
   capture is one frame at hour zero and so covers one step, which is what
   it already drew.
5. **An edge follows what the eraser took** (finding 7). `OperatorOutline`
   carries the object's erasures at the step, lifted into geographic terms
   through the same frame its outline is; the band is cut where a piece is
   gone and the piece's own rim drawn inside the object in its place. Tests
   in `editing.rs`.
6. **Layer and object reordering is a pointer drag** (finding 8). The rows
   were `draggable` with HTML5 drop handlers, and WebKit will not start a
   native drag from an element it considers unselectable — which the rows
   are, so that a shift-click extends the selection rather than selecting
   text across them (M28). `-webkit-user-drag: element` did not lift it.
   The panel now presses, moves and releases with pointer events, resolving
   the row under the pointer with `elementFromPoint`; the index arithmetic
   and the choice of side are `reorder.ts`, tested. The DOM hit test itself
   is not unit-tested — a fake `elementFromPoint` asserts nothing about
   WebKit — so this one wants a click-through.
7. **The macro preview shows the macro** (finding 9). A preview's revision
   was the clock with bit 62 set, and a revision crosses IPC as a number:
   above 2^53 JavaScript keeps only a double, so the address the map built
   was not the revision, every tile of the preview was refused as stale, and
   the preview had never shown anything at all — the "not reproduced" note
   under M29's item 15 and again under M31. The bit is 2^52 now, which a
   millisecond clock reaches in a hundred and forty thousand years, and a
   test holds the revision under 2^53 and exact through a double. Beside it:
   the legend shows the preview's one kind, `CaptureMode.kind` being back for
   it, and a press in the preview pans, placing a copy only if it barely
   moved.

### M32 — Edit tools show their effect as the pointer moves · **complete**

**Goal:** the user's instruction of 2026-09-06: the mask, the intensity,
the divergence, the turn, the warp, the liquify and the eraser must show
their effect live as the brush paints, not at the release.

**Delivered 2026-09-06**, one commit. The live-gesture mask of D37 became
an *operator*: the mask texture plus the tool's kind and settings, uploaded
per pointer report, and the raster and glyph programs apply it per pixel —
`remove` for the mask and the eraser, `keep` for a clone's and a push's
shifted source, and a modifier's own change for the gain, the turn and the
divergence, the last radiating from the nearest point of the stroke's
centreline carried in a uniform array of up to 64 points. A liquify is the
one displacement that varies from pixel to pixel: the field is drawn alone
into a viewport-sized texture and read back displaced through the stamps'
deltas, seam-free, with glyphs reading the tile texel at the source. The
palette's `Outline` preview kind is gone; each edit tool names its own, and
`operatorOf` in the frontend turns the tool's state into the operator with
a unit test per tool. The eraser rasterises its own stroke into the mask,
and every operator's held preview is silent on the overlay. A twist has no
live preview. Spec §6.3. Not clicked through in the app: the direction of a
divergence and the sign of a turn on screen are the two things most worth
a look.

**Follow-up, 2026-09-06 — intensity strokes not merging.** The backend
merges identical intensity strokes in every case that could be built — km
and px space, clicks and strokes, over a brush stroke — with the exact
option set the tool bar sends. The one way found to stop them is a size in
**px**: it is resolved to km at the zoom of each stroke (§3.5) and the wheel
zooms continuously, so two strokes of one pixel brush a scroll apart never
agreed to the metre, and two modifier strokes that do not merge compound
where they overlap, which is what makes it show. `merge.rs` now takes two
px-sized strokes within a tenth as the same brush, at the target's width.
If the strokes were sized in km the cause is elsewhere and the sequence
that produces it is needed.

### M34 — Macros take what is under them · **complete**

**Goal:** the three findings the user handed over on 2026-09-07 after the
M33 build, in order, one commit each.

**Delivered 2026-09-07**, three commits `M34.1`–`M34.3`. The second
reopens M29's rule that a capture takes the kind on show, on the user's
correction: it takes every kind under it. Not clicked through in the app.

1. **A macro's timeline extent** — reported again. The insert path was
   fixed in M33.4 and is verified end to end: a three-frame macro placed at
   step 1 of a six-step project has an active range of 1–3 in the document
   *and* in the tree the timeline draws from. Nothing more was found to
   fix, so this commit adds the tree-level assertion and no behaviour. What
   can still look full-length: a macro placed before M33.4, whose range is
   what it was created with and is dragged by its end handle; a macro whose
   frames really do span the project; and a pasted region, which is a patch
   and holds its last frame to the end by D65.

   **Reported a third time with a screenshot, and answered (2026-09-07).**
   The screenshot's macro is *Macro 0 · 24 frames · 69 h* in a 24-step,
   three-hourly project: it has a frame for every step of the timeline, so a
   bar over every step is exactly as wide as its frames.
   `a_macro_is_as_wide_as_its_frames_even_when_that_is_everything` is that
   case end to end — 24 frames from step 0 gives 0–23, from step 13 gives
   13–23. What makes a macro that long is the **run**: every step the
   playhead visits while recording becomes a key (D72) and the run ends
   where the playhead is when *Preview* is pressed, so scrubbing to the end
   to watch the field records to the end. Two ways to make macros shorter,
   neither taken here because both change a settled decision and the user
   should choose: end the run at the last step the region was *dragged* at,
   ignoring steps merely scrubbed through (reverses D72's "visiting keys");
   or drop trailing frames whose field and position repeat the one before
   (changes what a still macro paints past its last change).
2. **A capture takes every kind under it** (finding 2, the user correcting
   the M29 rule). `Capture.kind` became `kinds`, one plane of samples per
   kind, container version 3 — versions 1 and 2 still read as the one kind
   they name. The region copy and the macro bake both sample every kind
   `kinds_present` reports; `capture_of` resolves the plane from the
   layer's kind and paints nothing where the capture holds none; the
   preview builds a layer per kind, so the legend shows a scale for each;
   and an insert places one object per kind, in the layer asked for where
   it is of that kind and the topmost visible painted layer of that kind
   otherwise. Tests in `macros.rs`.
3. **The preview shows only what was placed** (finding 3). The copy at the
   recorded position is gone: the preview opens empty and fills with what
   is clicked, so what the user placed can be told from what was already
   there. `preview_stamp` no longer rebuilds the scene under a new
   revision — there is no object at the stamp to move, and doing so threw
   away every tile on screen whenever the pointer moved.
### M35 — The macro library says what it holds · **complete**

**Goal:** the two findings the user handed over on 2026-09-07 after the M34
build: deleted macros still offered by the stamp tool, and no confirmation
before deleting them all.

**Delivered 2026-09-07**, one commit.

1. **The stamp tool re-reads the library.** It read the library when the
   tool was picked up and kept what it read, so emptying the library from
   the settings left it offering macros that were no longer on disk — and
   the click that placed one refused. The settings dialog reports a library
   change, the shell counts them, and the map re-reads on the count as well
   as on the tool.
2. **Deleting every macro is confirmed.** The button opens a confirmation
   naming how many macros and how much disk go, and saying what makes it
   safe: a project that used a macro keeps its own copy of the frames
   (D52). Nothing is deleted until it is answered — which is what
   `SettingsDialog.test.tsx` holds.

### M37 — A picture is addressed by its opening · **complete**

**Goal:** the user's instruction of 2026-09-07: a dragged image must render
as it moves, rather than appearing when the pointer is released.

**Delivered 2026-09-07**, one commit. An image's address carried the
document's revision, which every edit bumps — so each report of a drag
made a new address: the old texture was released, the whole chart was
decoded again on the backend, and nothing was drawn until one landed,
which is why the picture arrived at the release. The address now carries a
token fixed for the opening (`OpenProject::image_token`, published on
`ProjectSummary`), so the texture stays put and the picture moves because
its placement did. It cost every edit a re-decode of every image, not only
a drag. Test in `image_layers.rs`.

### M36 — An image is dragged by its picture · **complete**

**Goal:** the user's instruction of 2026-09-07: an imported image must be
draggable once selected.

**Delivered 2026-09-07**, one commit. The three control points placed an
image but nothing moved it, so a chart could only be walked across the map
corner by corner. With the hand in hand, a drag inside the active image
layer's outline now carries every control point by the same delta —
`insideImage` is the crossing test over the quad the outline draws, and
`movedCorners` holds the latitude back at the poles so the shape stays
rigid. It goes through the same `set_image_corners` command, the same
one-in-flight rule and the same coalescing key as a corner drag, so a move
is one undo entry and the picture follows the hand. The cursor over the
picture is `move` rather than the hand's usual `grab`, since a drag there
does not pan. Tests in `place.test.ts` and `cursor.test.ts`.

### M43 — Panels that do not overlap, and a gradient renamed

**The overlap.** The right sidebar is a flex column and each panel is one
item in it. A flex item shrinks below its content by default and nothing
there clipped, so once the history grew past the room left for it the
properties above were squeezed and their rows were painted straight over
the history's — two panels' text on the same lines, both unreadable. A
section is `flex: 0 0 auto` now, so it is never squeezed, and the sidebar
takes up the slack by scrolling, which is what it is for. The history is
the panel that grows without bound, so it also bounds its own height and
scrolls inside it rather than pushing everything else off the top.

Asserted against the stylesheet, as the toolbar's wrapping rule is:
happy-dom has no layout engine and could not measure an overlap, but it
can fail when the declarations that prevent one are removed.

**The rename.** `speed` is labelled "Green". The identifier stays as it
was — that is what project files hold, so changing it is a migration, and
every file naming the old one would fall back to the default and lose the
choice its author made. The label costs nothing to change, and
`Gradient::label` now says so beside itself.

### M42 — Colour gradients, and an eraser that respects a hidden layer

**Goal:** the user's instructions of 2026-09-07: erasing on a hidden
layer should do nothing, and the colour gradient should be a setting
with presets for wind and for current, taking Panoply for inspiration.

**The eraser.** `stroke_erase` checked that a layer was not locked and
never that it was visible, so a stroke over a hidden layer took pieces
out of it — an edit with no visible effect, found later with nothing to
say what made it. Refused now, with a message, rather than dropped
silently: the pointer went down on purpose. The live preview already
did the right thing since M40, because what fills the hole is the stack
without that layer and a hidden layer is not in the stack to begin with.

**The gradients.** Nine, in `ve_core::colour`: the application's own,
the three perceptually uniform Matplotlib palettes, the NCAR blue-to-red
much of the published meteorology is drawn with, GMT's Haxby, cmocean's
speed and thermal, and a greyscale for print. They are approximations,
sampled at the anchor points those palettes are usually quoted at — a
ramp is read as "faster is hotter" rather than as a measurement, and the
difference between eight stops and two hundred does not survive being
stretched over a colour bar an inch tall.

The table is one table: identifier, label, note and stops together,
served to the frontend rather than written there. A list of names in
Rust and a list of colours in TypeScript is the arrangement that drifts,
and the drift would be silent — the map painting one palette while the
legend drew another.

The shader takes the stops as a uniform array instead of carrying them
as constants, so choosing a gradient is a redraw and never a re-render:
the tiles hold speed, not colour. `ramp.ts` takes a gradient as an
argument for the legend, the brush previews and the stroke previews,
which all have to agree with the pixels beside them.

**Two things the design turns on.** The identifier is what the project
file stores, and a name this build does not know is drawn with the
default and *saved back unchanged*, so a file from a later version can
go home intact. And `ProjectSettings` is no longer `Copy` — a gradient
is named by a `String`, and the alternative, an enumeration of the ones
this build happens to have, could not carry that name back out.

**The picker shows the gradients.** A native `select` renders nothing but
text in its options, so the first version could only name them — which
asks the reader to remember what "Haxby" looks like, the very thing they
opened the settings to find out. A flat radio group was tried and was
wrong for the panel: nine rows always open, laid out across the width,
pushed the rest of the settings away. `GradientPicker` is a dropdown of
the application's own — closed it shows the colours in use, open it shows
them all — with Escape, click-away and the arrow keys, and the
unknown-gradient case listed but unpickable.

**Not done.** There is no application-level default for new projects, as
there is for the scale; a new project takes the built-in pair. Adding
one is another two preferences and another two fields, and nothing has
asked for it yet.

### M41 — Why a history import looked like a hang

**The report:** the import never finished and showed no data, while
GribHistory fetched the same archives quickly.

**What it was not.** The app's log showed three imports starting and
none finishing, with no error. Two candidates were ruled out by
measurement rather than by reasoning. `reqwest`'s blocking client
documents a panic when built inside an async runtime, which would leave
a Tauri task dead and its promise unresolved — the exact shape of the
symptom — but a probe on a Tauri runtime worker opened both archives and
read four steps without complaint. And driving `history_import` directly,
with a real `AppState` and no Tauri at all, imported eight steps from
both archives in 41 s and built the layers. The path works.

**What it was.** It is slow, and it was silent. Timed against the
archives from this connection: ERA5 costs about 2.5 s per step and
GlobCurrent about 0.8 s, opening the two stores costs another ten,
and a step is two chunks of a few megabytes. Eight steps from both
archives is the better part of a minute; the three abandoned attempts
were given 50 s, 108 s and 132 s. Nothing was written until an entire
archive was done, so the log went quiet for the whole of it and there
was no way to tell a slow fetch from a stopped one — which is exactly
what the report says.

**What changed.**

1. **A line per step in the log**, with the archive, the step and the
   elapsed time, plus one when a store opens and one when the import
   ends. A log that says an import began and nothing more cannot be
   told from one that hung.
2. **Failures are logged, not only returned.** The only report of a
   failure was a line in the status bar that the next hint replaces.
3. **Four steps at once**, streamed to the file through a bounded
   channel. Measured, the link is already saturated by one request —
   four chunks sequentially took 7 s and four at once took 8 — so this
   is for the latency, not the throughput, and it is four rather than
   GribHistory's eight because eight made every fetch land at the same
   moment and turned the wait into one silent jump. The bound on the
   channel is what keeps a 240-step import out of the heap: it used to
   build the whole file in memory, which is over a gigabyte at the cap.
4. **The read-back happens outside the session lock.** Decoding is
   hundreds of megabytes at the cap and was freezing every edit and
   every tile for the length of it.
5. **The fetch runs on a plain `std::thread`.** Not because the runtime
   was the cause — it was not — but because `reqwest` documents that its
   blocking client must not be built inside an async runtime, and
   relying on it tolerating one is not a thing to leave in place.
   GribHistory does the same, for the same reason.

**Still true.** A 24-step import from both archives is about two minutes
at this link's ~1.9 MB/s, and both archives are ticked by default, so it
fetches twice what one would.

### M40 — An erase preview takes one layer, not the stack

**Goal:** the user's report of 2026-09-07: erasing on a layer that is not
the top one visibly erased every layer above it until the mouse came up.

**Cause.** The map draws the composite of every visible layer as one
tile, and the live preview is a screen-space mask applied to that tile
(M32). "Remove where the gesture covers" therefore removed the whole
stack. The commit was always right — `erase_stroke` writes to exactly
one layer — so the release put everything back, which is what made it
read as a flash rather than as damage.

**Fix.** Fill the hole with the same frame minus the layer being erased.
`flatten_where` already took a layer predicate, so the scene side is
`flatten_without`, one line. The frame is addressed as
`without/<layer>/<revision>/<step>`, a prefix rather than a query because
the tile cache treats everything before the `z/x/y` as one opaque token
and a prefix keeps that true. `SceneCache` holds two frames now instead
of one: an erase draws from both on every frame, and a single slot would
re-flatten the project twice per redraw at pointer rate.

The renderer already had the shape for this — the clone stamp draws the
field twice, `remove` then `keep` — so the change there is a second
source of tiles rather than a second camera. `frameToken.ts` owns the
token's shape for both the builder and the resolver, since two copies
would drift and the failure is silent: the tiles of one scene, correctly
cached, under an address that names another.

**Why it costs little.** A texture is kept by the content hash of what
reaches it, so a tile the erased layer does not touch hashes the same
with the layer left out as with it and is already resident. Only the
tiles that layer actually covers are rendered. The frame is warmed while
the eraser is in hand rather than at the first pointer-down, so the round
trip happens in the pause before the stroke.

**Not done.** The modifiers — intensify, rotate, diverge, liquify — still
preview against the whole composite. They change the field in place
rather than taking it away, so blending two stacks does not express what
they do; scoping them needs the active layer's own contribution as a
separate texture, which is a third frame and a shader that composites.
Their previews are wrong in the same way on a lower layer, and less
visibly, since they alter the field rather than removing it.

### M39 — The eraser's stroke carries no colour of its own

**Goal:** the user's report of 2026-09-07: the eraser's stroke had a
purple tint before the button came up, and the tint had to go from both
the brush circle and the swept stroke.

The tint was pink at 12% over a dark blue map, which reads purple, and it
sat on top of the field the tool was in the middle of taking away — a
wash over the result, right where the eye is looking. It made sense when
it was written: the erase only landed on release, so the tint was the
only thing saying which ground the stroke had covered. M32 made the erase
live, and from that moment the field leaving under the pointer said it
better, and the tint said nothing the map was not already showing.

Both fills are gone, from the sweep and from the nib. What is left is the
nib's dashed outline, which is a brush cursor doing a brush cursor's job:
where the tool bites and how wide. The sweep is still never stroked, for
M33's reason — a swept footprint is a union and a stroked union traces
every stamp's own circle — and now there is nothing left for a sweep
outline to say either.

The pink stays everywhere else it is used: the highlight on the object
under the pointer, and the rim an erasure leaves in an outline. Those
mark things that are *on* the map rather than tinting them.

### M38 — Importing from the record · **done**

**Goal:** the user's instruction of 2026-09-07: import history from the ERA5
and GlobCurrent archives over a date range, ERA5 wind in one layer and
GlobCurrent in another, both behaving exactly as GRIB layers do, of layer
type zarr, reached by a calendar button beside *Import image*.

**This reopens invariant 5**, on the user's instruction: the application
fetched nothing, and a history import fetches. The invariant is now
"nothing is fetched that the user did not ask for" — no CDN, no tiles, no
telemetry, no update check, the webview's CSP still `'self'`-only — and the
one exception is confined to `ve-zarr`, which nothing calls but the import.
`check-offline.sh` enforces exactly that: a URL anywhere else in Rust is
still a finding, and in `ve-zarr` only the two archives are allowed.

1. **The reader.** `ve-zarr` reads ERA5 10 m wind (ARCO-ERA5 on Google
   Cloud) and GlobCurrent total surface current (Copernicus Marine), both
   public and anonymous, both Zarr V2 read with `zarrs`. Carried over from
   the GribHistory application with its tests — the archives, their quirks
   and the reasoning about provisional hours are the same problem — with
   two changes: the blosc codec stays the pure-Rust one, and the HTTP store
   is written here rather than taken from `zarrs_http`, which pulls
   `reqwest` with `native-tls` and so `openssl-sys`, a system C library the
   three-platform build forbids. Rustls links none. Both sources hand back
   fields on the ERA5 0.25 degree grid, GlobCurrent regridded onto it, so a
   field goes into a GRIB2 message as it is.

2. **A bitmap in the writer.** `writer::message_masked` writes the points a
   field has no value for as absent, through a section 6 bitmap, rather than
   packing a NaN or a zero. GlobCurrent has no current over land, and a
   land value of zero is not "no current" but "dead calm" — the distinction
   D58 exists to keep. A field with no holes still goes through the plain
   `message`, so nothing about existing output changes; a field with
   nothing anywhere is refused. Round-tripped through the decoder, which
   reads the masked nodes back as missing.

3. **The pipeline.** `ve_app::history` opens each archive, walks the hours
   in the range, and writes them as GRIB2 into the application's data
   directory — one file per archive, named for the archive and the range.
   The file is then read by the *ordinary* importer, so a history layer
   holds exactly what a forecast layer holds and there is no second
   decoding path to keep in step, and so the alignment rule is §4.8's
   unchanged: the file's first hour is its own reference, so it lands on the
   project's first step. A project with no start time takes that hour as its
   own, in the same history entry as the layers, so one undo takes the whole
   import back. `Layer::from_history` is
   `Layer::from_grib` plus the provenance, built as one so the two cannot
   drift. Ten days is the cap, refused at the dialog with its length rather
   than abandoned halfway. The document keeps the file's path, the archive
   and the hours, and never a sample (invariants 1 and 2).

4. **The dialog.** A calendar button beside *Import image* — dates, not a
   file, which is what distinguishes it — opens a start, an end and a
   checkbox per archive. `historyRange.ts` holds every rule about the
   times, tested against arithmetic rather than against a second copy of
   its own formula: both ends inclusive, the cap at its boundary, a date
   that does not exist refused rather than rolled into the next month, and
   the value read as **UTC** and never as local time, which would shift
   every fetched hour by the reader's own offset. The default range is a
   day ending a week back, because both archives trail real time and a
   range ending now is one neither has yet.

5. **Only what the project can show, and a bar while it is fetched.** The
   first version read every hour of the range, which on a three-hourly
   project was three times what any step could draw: an imported message
   reaches a step only at that step's own forecast hour (§4.8, D48), so
   the rest was minutes of transfer for data the app throws away.
   `wanted_hours` is now the plan — the range's start and every
   `step_hours` after it, at most `step_count` of them — and the fetch
   walks that and nothing else. The cap moved with it, from a span in
   hours to a count of reads, which is what the wait is actually
   proportional to: 240 reads is ten days hourly and two months
   six-hourly. The dialog counts downloads rather than hours, and the
   status bar carries a determinate bar fed by `history://progress`,
   since the count is known before the first byte moves. The bar is
   bounded by the busy store rather than by its own last event, so a
   failed import clears it the same way a finished one does.

   Measured before the change, on a domestic connection: ERA5 wind cost
   2.5 s an hour and GlobCurrent 0.83 s, so 24 hourly steps from both was
   about 90 s and 160 MB. The same day on a three-hourly project is now
   nine steps, about 30 s.

6. **The dialog's own presentation.** Three defects the first version
   shipped, all found by using it. `.modal input` is 6.5 rem wide, which
   cut the hour off a `datetime-local` control, and `.modal label` is a
   column, which stacked each archive's checkbox above its own text: both
   are right for the export and new-project forms they were written for,
   so the history dialog now overrides them under its own class rather
   than changing them. And it is rendered through a portal to the body —
   the layer panel is a stacking context, so a modal rendered inside it
   painted *under* the timeline whatever its `z-index` said.

**Not done, and why.** There is still no way to cancel a fetch part-way;
that wants the read to run on a worker with a cancellation flag, the way
the export does, rather than inside the command. Nothing tests the two
archives end to end either — a test that reaches the network would put
the one exception to invariant 5 into CI, where no user asked for it —
so the seam is tested instead: `ve-zarr` against its own store fixtures,
and the encode-and-read-back join against hand-made fields.

**Known upstream gap.** ERA5's root attributes advertise data about two
days past what is actually written: on 2026-09-07 they claimed hours
through 2026-09-01T23:00Z while the last written chunk was
2026-08-30T23:00Z. `Era5Store` takes the attributes at their word, so a
range inside that gap fails on the first missing hour — quickly, and
with a message naming the chunk, but it fails. The dialog's default was
a week back, which landed in it; it is the project's own timeline now,
and this day a year ago when the project has no date, which clears the
lag and the overstatement together. Clamping the *coverage* to what is
written would need a probe per range and is not done: the refusal is
already fast and says which chunk is missing.

---

## 3. Testing strategy

| Layer | Approach |
|---|---|
| Document model | Property-based (`proptest`): save/load round-trip, undo/redo inverse, interpolation edge cases. |
| Geodesy | Comparison against reference values, with deliberate antimeridian and polar cases. |
| Evaluation | GPU/CPU parity on randomised scenes; golden raster snapshots at 1° for regression. |
| GRIB | Self-round-trip via the in-test reader; external decode by `wgrib2`/ecCodes in CI; committed byte-level golden files. |
| Frontend | Component tests for the timeline and layer panel; Playwright smoke over the packaged app. |
| Captures (M14, M16) | Hand-computed fields through capture → container → insert; the container round-trips byte-identically; undefined and zero asserted distinct through both kernels and an export. |
| Decoder (M12) | ecCodes-repacked fixtures with known values; the full sample set as an ignored test keyed on the directory. |
| Resampling (M21) | A brute-force nearest-three interpolation over the whole mesh as the reference; both poles and the antimeridian among the sampled points; a reopened neighbour set held to the same content hash as a fresh search. |
| GRIB frame overrides (M20) | Spot-cell equality of the pasted step with its source on both backends; round-trip with no sample data in the file; undo of a run. |
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
| Pure-Rust JPEG 2000 decoder immature or slow (M12) | GEM files still refused, or seconds per message | Spike against ecCodes on the GEM files before choosing; a named fallback crate; decode time measured |
| Motion vector mis-signed or mis-framed (M13) | Silently wrong GRIBs — the same class as an inverted direction convention | Hand-computed acceptance vectors; both backends; M4's independent-viewer check repeated with a moving object |
| Captured rasters in the project (M14, M16) | Invariants 1 and 2 change; a project can grow large | Settled as D52; compressed entries shared by hash; a size shown at capture as the export shows one |
| Capture-mode lockout leaks (M16) | A write during capture corrupts the document or the capture | One session flag checked by every backend write path, not disabled controls in the UI |
| Rebound shortcuts collide with keys the webview or OS owns (M15) | A key silently does nothing | A reserved-key list; collisions refused on entry; reset to defaults |
| A bundled mesh goes stale or a new ICON grid appears (M21) | An ICON file is refused although the app "supports ICON" | The refusal names the UUID rather than guessing; adding a mesh is one run of `icon-grid-builder` and no code |

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
| D11 | ~~GRIB reader lives only in tests~~ — superseded by D44. The writer's *verifier* stays test-only | Invariant 5 — no runtime dependency on ecCodes or any external decoder. The decoder that shipped with D44 is hand-written Rust and depends on nothing, which is what the rule was protecting |
| D50 | A warp is pulled to a place, not typed as a distance and a bearing | Aiming a liquify by typing two numbers is guesswork, and the numbers cannot be keyframed into anything meaningful — an animated bearing sweeps the field round the compass. A `push_to` position makes the gesture the tool is named for possible (shift-drag grabs the warp under the pointer and drags its field where it goes) and makes both ends of the push ordinary animatable positions, so a warp that travels is two keyframed points. `PushTo` is its own property id and not a share of `Target`, which is what an *aimed direction mode* points at: two positions answering different questions (compare D30's `brush_shape`). The cost is that a push is now measured from the anchor like a twist, so a warp merges in neither mode (spec §6.3, schema version 10) |
| D51 | Every numeric field is one component, and can be cleared | `value={n}` with `onChange={commit(Number(raw) \|\| fallback)}` cannot be emptied: the empty string parses as `NaN`, the fallback is committed, and the box refills under the cursor before the next key arrives — so a size of 500 could be edited in the middle but never replaced, and a minimum of 1 turned an attempt to type 40 into 1 and then 14. The field keeps a *draft* while it is being edited and commits only what parses; bounds are applied when it is left, not as it is typed, since a bound applied mid-entry is the same trap in another costume. Found by hand, across the tool bar, the inspector, the timeline and both dialogs |
| D52 | A captured field travels inside the `.veproj` as a compressed binary entry, referenced by hash; invariants 1 and 2 are reworded to forbid a *rendered* raster and admit a *captured* one | A pasted patch and an inserted macro are rasters, and the alternative — keeping them only in the macro library and referencing them by path, D44's rule for a GRIB — makes every pasted patch in every project go blank when the library is cleared, which the user will do from a settings button. A rendered raster is a cache product, reproducible from the objects, and stays forbidden; a captured one is user content that cannot be reproduced once its sources change, which is the property that made a GRIB's samples worth not copying and a capture's worth keeping. Never in the JSON; the wording changes in spec §1.5 and CLAUDE.md in the commit that first writes an entry (M14, M16). Settled with the user 2026-09-04 |
| D53 | "Export as float16" is a bits-per-value option under simple packing | GRIB2 has no 16-bit float template (5.4 offers 32, 64 and 128) and the export already packs 16 bits per value; what the request wanted is the precision as a visible choice against the file size. Template 5.0 stays the only packing written (M19). Settled with the user 2026-09-04 |
| D54 | New palette defaults: select `M`, ~~fill `G`~~ (withdrawn by D64), capture `K`, insert `N`, liquify `L`; measure moves from `M` to `T` | `M` is the marquee's key in every paint program, and the measure tool has no code yet so moving it costs nothing. All rebindable after M15, so only the defaults were at stake (M14–M17). Settled with the user 2026-09-04 |
| D55 | A region is a map-space shape | It is drawn on the map, and `Cmd`-`A` selects the *view*, which is only a rectangle there; a circle region is round on screen at every latitude. What a region makes — a fill, a patch, a macro's footprint — therefore takes `stamp_space: projected`, exactly as a px-sized stamp does (D28). A geodesic region would be D28's ellipse in reverse (M14). Settled with the user 2026-09-04 |
| D56 | Warp keeps the placed pull and twist; liquify is the painted smear | The two are different gestures on different geometry: a warp moves a placed region's field as one block, a liquify drags the field along a stroke with each stamp carrying the pointer's own movement. Splitting `warp_mode` into two tools by rename would have left the smear unbuilt (M17). Settled with the user 2026-09-04 |
| D57 | The motion checkbox is per track row — position, rotation, scale — not per object | A rotating system that also travels may want its spin in the field and not its translation, or the reverse; one switch per object cannot say which. Three booleans, each beside the track it reads (M13). Settled with the user 2026-09-04 |
| D58 | A cell a mask removed is *undefined* in a capture, not calm | The mask exists to let what is beneath show through; a capture that turned that into a real zero would overwrite whatever lies beneath the paste. Undefined writes nothing when composited, so the paste is transparent exactly where the source was (M14, M16). Settled with the user 2026-09-04 |
| D59 | A GRIB frame copied to another step is a step number in the layer, never a copy of the lattice | The project keeps a file's path and nothing of its samples (D44); a pasted frame that copied the grid would be a raster in the document by another route. Mapping the step to the source step gives the same picture from the same file, and because the flat scene already carries the served frame's hash, the render cache, readiness and both kernels are right by construction. It is an instruction, so unlike a message it stays where it was put, one step at a time, and D48's rule against holding a measurement forward is not touched (M20) |
| D60 | The two compressed GRIB2 packings are decoded by crates, not by us | 5.40 is a JPEG 2000 codestream and 5.42 a CCSDS entropy coder; hand-writing either would be thousands of lines of wavelet and adaptive-coding work to no product end, and the risk is not that they are hard but that they are subtly wrong on files nobody has. `hayro-jpeg2000` (default features off, which leaves it with *no* dependencies) and `rust-aec` were both shown bit-exact against ecCodes on every real file in the reference set before being chosen, and neither pulls a `-sys` crate: invariant 5 and the three-platform build both forbid a C library, which is what ruled out OpenJPEG and libaec (M12). The three integer packings stay hand-written, since they are a bit reader and a formula |
| D61 | An unstructured grid is resampled onto the project's grid at import, its cell positions are bundled, and only the neighbour indices are kept | Three decisions that stand together. **Resampling** rather than sampling the mesh directly, because `RasterGrid` is what the render cache, both kernels and the exporter all speak — teaching the WGSL kernel to search a point cloud would have touched everything, where resampling touches the import alone. **Bundling** the positions, because an ICON message names its grid by UUID and carries no geometry, so without them the file cannot be placed on the earth at all; fetching them is out (invariant 5) and asking the user for two more files is a poor trade against 2.4 MB of assets. **Keeping the indices, not the weights**: the search is 494 ms at 0.1° and the weights recomputed from the coordinates are ~30, so storing the weights would triple the size for nothing — 1.9 MB against 155. The value at a node is a point sample, matching the convention the evaluator and exporter already use, and `u`/`v` interpolate as components because averaging bearings turns two opposing vectors into a fast one pointing nowhere (M21, spec §4.8) |
| D62 | Every projected grid resamples onto a **window** of the project's lattice, and grid-resolved components rotate at the source | Four decisions that stand together. **Resampling** for D61's reason: `RasterGrid` is what the render cache, both kernels and the exporter speak, and a Lambert lattice is not one. **A window** rather than the globe, because a 2.5 km regional model reaches a few percent of the earth and a global 0.1° lattice of it is 52 MB of mostly nothing; the window's nodes are the project's own nodes, so an imported regional field still lines up with what the project exports. **Rotating at the source**, because near a stereographic grid's pole the convergence turns through a full circle in a few cells and an interpolated grid-relative vector there means nothing — and because reading grid-resolved components as eastward and northward is a 30° error across a Lambert CONUS grid that looks entirely plausible on screen. **Ellipsoidal formulas throughout**, since they reduce to the spherical ones exactly at zero eccentricity, so honouring code table 3.2 costs one code path rather than two. Two departures from the octets, both forced by real files and both matching ecCodes and wgrib2: NCEP's template 3.32769 states increments in no unit that reproduces its own corners, so they are derived from the first point, the last point and the centre it also states; and the Mercator orientation field, which two NCEP blend grids fill with 200° and 295°, is ignored, their stated corners being where an unrotated grid puts them to a few parts per million (M22, spec §4.8) |
| D63 | `stamp_space: projected` means the *project's* map space — equirectangular lon/lat degrees — and never the view's | M11 required this rule, or its replacement, before any alternative projection shipped, and its own proposal was the opposite: convert a px size or a region through the view's projection at creation, so a shape drawn round on a Mercator screen is stored as the equirectangular shape occupying the same ground. Rejected twice over. It makes a document depend on a view setting — the same px number producing a different object depending on which projection happened to be showing, with nothing on screen saying so — where D9's px-to-km conversion depends only on latitude and zoom, both of which the user can see. And it is not expressible: a projected stamp has *one* size, `size_km`, and a round-on-Mercator shape is an ellipse in equirectangular space, so there is nothing to convert it into without adding an aspect property to every sized tool. The px number is therefore measured on the horizontal axis, where every projection the view offers is linear, so what a px size resolves to in km is the same in all of them. The visible consequence is that a projected stamp is round only under equirectangular and draws taller further from the equator elsewhere, exactly as a lat/lon rectangle does — it behaves like the map it is defined on, which is the answer that needs no warning. D28 and D55 are unchanged in meaning; this says what their word meant once there was more than one map (M11, spec §3.5, §5.1) |
| D64 | A selection is a footprint for the brush and the four operators, on a click inside it; the fill tool is withdrawn | The fill tool (M14) turned the current region into a shape fill. It is gone: with a region selected, a click inside it with the brush, the mask, the intensity, the divergence or the turn makes that kind of object from the region's boundary — a polygon, or a disc — as a projected stamp (D55), and a brush applied that way *is* the fill, with the brush's own bar. One button fewer for a gesture every painting tool now has. The evaluator keys on geometry rather than tool and the merge rule accepts only swept strokes, so this is routing, not engine work: a region-made brush paints a polygon, a region-made mask masks one, and neither merges into a painted stroke. *Inside* the region, not anywhere on the map, or a region left selected would turn every stroke into a region. The clone stamp and the warp are left out because both are measured from their anchor to somewhere *else*, and a region does not say where; a rectangle goes as its four corners and only a circle as an extent, because these tools have no `shape_source` to say what an extent means (spec §8.2). The shape fill in the palette is untouched: it is drawn with its own presets, not applied. Requested by the user 2026-09-04, in two steps: the operators first, then the brush in place of the fill |
| D49 | The modifiers are painted, and merge — except the two measured from their own anchor | They were click-placed discs, which made a swathe of intensification a row of stamps and gave them none of the merging the brush and the mask have. Painting them is the same gesture, geometry and merge rule the other swept tools already use, so it is subtraction rather than addition. The exception is the rule §6.1 already states: a merge re-expresses the new chain under the *target's* anchor, and a divergence radiates from its anchor while a twisting warp turns about it, so absorbing one would change what it paints. Intensify, rotate and a pushing warp refer to no anchor and merge freely. The edge highlight and the selection outline are the mask's, generalised: one `operator_outlines` command for every object that has no field of its own (spec §6.3, schema version 9) |
| D48 | A step the file has no message for shows no imported field — reverses the hold half of D44 | Holding the last message forward draws a forecast for a time it was never made for, and does it most misleadingly past the end of a short file, where a six-hour file stood in for a ten-day timeline unchanged and looking like data. Found by hand. A keyframe holds because it is an instruction the user gave, and between two of them the document still means something; a message is a measurement, and between two of them the file means nothing. The consequence is that a step size that does not divide the message times hides most of the file, so "Open from GRIB" now derives the largest offered step that *divides* every message's offset rather than the largest no wider than the gap — a 4-hourly file takes hourly steps and blanks three in four, where before it took 3-hourly steps and showed one message in four (spec §4.8) |
| D44 | A GRIB2 file imports as a layer that keeps the file's *path*, never its samples; one layer per field kind, the other kind hidden; ~~each step shows the last message at or before its forecast hour~~ — the hold rule is reversed by D48 | Invariants 1 and 2 forbid a raster in the project, and copying forecast data into every project that references it would have been the cost of relaxing them. The lattice lives in memory beside the layer and is read back on open; a missing file leaves an empty, marked layer rather than refusing the project. Hold-previous was chosen because it is the rule keyframes already follow (spec §4.5); D48 reverses it, a message not being a keyframe. Both kernels sample the lattice, so the preview and the export agree on it as they do on everything else (spec §4.8) |
| D45 | Four modifier tools that read the composite beneath them, transform it and write it back — and a warp that reads it at a displaced position | The field-shaping D38 removed comes back as objects rather than as properties. What made `divergence` and `curl` bad was that they were per-tool properties that changed a vector after that object's own direction mode had produced it, from an anchor nothing showed: invisible, and impossible to reason about across a stack. A modifier is placed, sized, feathered, z-ordered and named for what it does, and it acts on the composite. It follows that it has no `edge_mode` (nothing to replace with), no speed and no direction of its own, and no colour to preview as — its footprint outline is the honest preview. The compositing loop already held "everything below this object" at the moment each object is applied, so three of the four are a branch in one loop on each backend; the warp reads elsewhere and takes the clone stamp's recursion, its depth cap, and its exclusion from the GPU. Measured: the fidelity suite's worst speed error went from 0.0009 m/s to 0.0030 and direction from 0.004° to 0.019°, against budgets of 0.25 and 2° — the anchor-relative term is back, bounded to the modifier's own footprint (spec §6.3, §7.5, §7.6, §7.8) |
| D46 | The eraser is the mask, and gains `invert` | The tool was named for the gesture and not for what it makes: a first-class object that decides where the field beneath it shows. `invert` is what makes that plain — cover everything *but* the footprint, which is how a global flow is confined to a basin rather than cut out of one — and it is the mask's alone. On a tool with a direction it would be unsafe: an inverted object is evaluated at every cell on the globe including its own antipode, where the bearing from its anchor is ill-conditioned and two backends legitimately disagree; a mask writes calm and has no direction to disagree about. The rename is a migration, because the tool is stored on every object it drew (schema version 8), and object *names* are left alone: "Erase 3" was the user's. The edge is drawn on the map because a mask is otherwise invisible — selected, or pink under the pointer with the tool in hand (spec §6.2) |
| D47 | A tool that paints a single vector can take it off the map | Aiming a wind by typing two numbers is guesswork next to pointing at one that is already there. The eyedropper is declared in the schema — two property names and the conditions that make them meaningful — so the option bar renders it generically and it is inert exactly where what it writes is: a gradient has two speeds and two bearings and no single answer, and a curve in `relative_to_path` holds an offset rather than a direction. Sampled through the evaluator rather than decoded from the tile under the cursor, so the number that lands in the document is the field's own and not the map's 16-bit quantisation of it; only visible layers contribute, which is what the map is showing anyway (spec §6.1) |
| D12 | `edge_mode` defaults to `Blend` | Identical to `Replace` over calm areas, so the default only governs soft edges over existing data — where fading to calm is never wanted (spec §7.4) |
| D13 | Reducing `step_count` deletes keyframes, behind a quantified confirmation | Keeps the document free of invisible state; the confirmation carries the cost (spec §4.1) |
| ~~D14, D15~~ | **Withdrawn with the sailboat route feature.** Both settled how a route solver should behave — the `max_tws` default and merge-on-re-solve — and neither survives the feature's removal. The row stays so the numbering around it does not shift |
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
| D43 | Playback advances only into a step whose tiles are resident on the GPU; a frame not yet on screen holds the last one, dimmed | The backend's readiness said a step was rendered, and the map advanced into it before the webview had fetched a single tile of it — a blank or dimmed map for a frame, every step, which the user saw as flicker and a fade. Rendered and shown are two different facts; the map is the only one that knows the second. Holding the last frame replaces the blank; the dim keeps it honest (spec §9.4) |
| D40 | "Stale" is the frontend's memory | The backend reports what the cache holds now; "was solid at an earlier revision, is not at this one" needs the earlier revision, which only the timeline saw. Playback treats stale as not ready: the tiles on screen for it belong to a revision that no longer exists (spec §9.4, §9.5) |
| D39 | The render pool renders through `protocol::serve`, the function that answers the map's own tile requests | The key, the backend, the quality and the encoding are decided once, so a tile rendered ahead is byte for byte the tile the map will fetch and "ready" means "will be a cache hit". A pool that chose any of those differently would fill the cache with tiles nobody asks for and report frames ready that are not (spec §9.5) |
| D38 | No tool has `divergence` or `curl`; an object's direction mode is the whole of its direction | Reverses the half of D30 that kept them on the tools with a centre. What it costs is the spiral: a radial component is what makes a low converge rather than merely turn, so a cyclone is now built from more than one object — a circle for the rotation, a larger one aimed at its centre for the inflow. What it buys is that a direction is decided in one place, and that nothing in the evaluator depends on an object's *anchor* rather than its geometry any more. Measured: GPU–CPU agreement tightened from 0.108 m/s to 0.0009 and from 0.186° to 0.004°, the radial terms having been the main source of `f32` disagreement between the kernels. Removing a property from a tool is a migration, never only a table edit (schema version 7, spec §7.5) |
| D37 | The eraser and the clone stamp are previewed by operating on the map, not by drawing over it | Both are defined against what is already beneath them, so a coloured wash on the overlay would show something neither tool does — and an overlay cannot show a removal at all, being a canvas above the field that can add pixels and never take them away. A gesture with either one becomes a screen-space mask and the map is drawn through it: the eraser's region loses its field, the clone's loses it and gains the source's, through a camera shifted so the source lands under the brush. The basemap is never masked, since something has to be left to see. A tool declares *how* it previews (`PreviewKind`) and supplies a footprint; nothing else about it is per tool (spec §6.1) |
| D36 | Values are quantised where they enter the document, not where they leave it | D17 quantises at the serialisation boundary, which covers a value the user typed and misses one the application computed — a polygon's centroid, a dragged anchor. `PropValue::canonical` now applies in `Animatable`'s writers, which is one place and off the evaluator's per-sample path. Doing it in `LonLat::new` instead would have put a rounding on the clone stamp's inner loop (`ve-core::canonical`) |
| D17 | Document `f64` values are quantised at the serialisation boundary | `serde_json``s parser is one ULP off on ~10% of `f64` values, so raw floats do not round-trip and a project would not equal itself across save/load. Chosen precisions are far finer than anything observable; `f32` is unaffected (`ve-core::canonical`) |
| D65 | A region copy captures a run of frames from the copy step, deduplicated by scene hash, and a pasted patch's time runs from its paste step | One frame made a copied animation a still; capturing the whole timeline from step 0 would put the wrong frame under the paste. The bake is the macro's, and `capture_of` already measures from the object's first active step, so the patch needs no new sampler. Dedupe is the "steps that look the same share their tiles" property applied to frames, so a still scene costs one frame (M23). Proposed 2026-09-05 |
| D66 | One creation target, and never a GRIB layer for a *field*: a creation aimed at one is refused with a hint. Reopened in part by M31 on the user's instruction (2026-09-06): an imported layer takes edit objects — a modifier, a mask, a clone, the eraser — and refuses only objects that paint a field of their own | Creation, paste, insert, duplicate and drag-drop each chose a layer their own way — the top of the stack, the active one, whatever was given — and none refused an imported layer. One function decides, refusing a layer whose source is a file and saying so in the hint area, rather than quietly redirecting to another layer the user did not choose (M23). Settled with the user 2026-09-05 |
| D67 | `Shift`+arrows nudge the selection; the map's pan moves to `Alt`+arrows, and the bindings table gains an `alt` modifier | The user asked for `Shift`+arrows by name, and it is the convention. Pan had the chord; rather than drop pan from the keyboard, the table's chord spelling grows one modifier so it stays one table with one collision rule (M23). Settled with the user 2026-09-05 |
| D68 | The view controls reach the title bar through a portal, not by lifting the map's state | The tool, the glyph style and the graticule are the map's state and read by its draw loop; moving them to `App` for the sake of where a button sits would re-render the shell on every tool change, which is the class of bug the readout store was made to end (M25). Proposed 2026-09-05 |
| D69 | ~~A project has no start time of its own~~ Superseded by M29.13 on the user's instruction (2026-09-06): a start time is optional and set from the Time row; the export still asks, pre-set to it | Until a forecast file gives the timeline a clock there is nothing to set — step 0 is "now" and the ruler counts hours. Opening a project from a GRIB or importing one derives the start from the file (§4.8), and the export dialog asks for the one the GRIB needs. The timeline's field and its clear button go, and no settings field replaces them (M25). Settled with the user 2026-09-05 |
| D70 | Autosave is a three-way setting: off, recovery snapshots, or the file written in place; default recovery | "Auto-save" can mean the snapshot the app already takes or the project file itself. Offering both, with today's behaviour as the default, changes nothing for an existing install and lets a user who wants the file kept current have it (M25). Settled with the user 2026-09-05 |
| D71 | A macro preview is a preview scene in the session, served by the tile pipeline under its own revision, never a document write | Hiding every layer to show the macro alone is a document write, and the history is locked while a capture runs for exactly the reason it must stay locked. A one-object project flattened by the same `flatten` and served by the same `protocol::serve` keeps every tile rule — one path, content-hashed keys, readiness by cache probe — and leaves the document untouched (M26). Proposed 2026-09-05 |
| D72 | A capture's positions are keyframes in the session: visited steps are keyed, gaps interpolate by great circle, keys can be deleted | Holding the last position wherever the user did not drag made a moving system that was placed at 3 and 9 stand still until 9 and jump; keys with interpolation are what every other track does and what "jump forward and change the position" means. They stay session state — not document, not history — so a placement is still not an edit (M26). Proposed 2026-09-05 |

---

## 6. What to settle before coding starts

The original questions are resolved (spec §15, D12–D17). The re-plan of
2026-09-04 raised seven more, and all seven were settled with the user the
same day: D52–D58 in §5. The one that changes an invariant — captured fields
inside the project, D52 — takes effect in the commit that first writes a
capture entry, and until then invariants 1 and 2 stand as written.

Nothing blocks M12.

**The 2026-09-05 re-plan (M23–M26) raised eight more, D65–D72.** Four were
readings of a finding that could be read two ways and were put to the user
the same day: a creation aimed at a GRIB layer is refused with a hint (D66);
pan moves to `Alt`+arrows (D67); the start time exists only where a GRIB
supplied one and is otherwise asked at export (D69); autosave is a
three-way option (D70). The other four (D65, D68, D71, D72) are
implementation choices with one sensible answer, recorded so they are not
re-derived. Nothing blocks M23.

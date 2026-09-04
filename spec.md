# VectorEffects — Specification

**Status:** Draft v0.1 · **Date:** 2026-09-02

A self-contained desktop application for authoring global wind and ocean-current
fields with painting, layering, and keyframe animation tools, and exporting them
as GRIB2 files.

---

## 1. Purpose and scope

### 1.1 Problem

Testing weather-routing engines, navigation software, and forecast-consuming
tools requires GRIB files with *known, deliberately shaped* vector fields. Real
forecast data is uncontrolled: you cannot ask NOAA for "a 40-knot cyclone
parked at 40°N 30°W that decays over 18 hours" or for "a wind field in which
exactly three sailing routes are viable." VectorEffects lets a user draw the
field they need and bake it into a standards-compliant GRIB2.

### 1.2 Product summary

A Tauri desktop app. Rust owns the entire domain — document model, field
evaluation, caching, GRIB encoding. The webview is a view layer. The user
paints vector data onto a global map with brush/stamp/shape/curve tools,
organises those into layers, animates every tool property across forecast time
steps, and exports GRIB2.

### 1.3 In scope

- Global regular lat/lon grids only, at 1°, 0.5°, 0.25°, 0.1°.
- Two field kinds: 10 m wind, or ocean surface current. One kind per project.
- GRIB2 output only, simple packing, custom Rust writer.
- Fully offline. No network access at runtime, ever.

### 1.4 Out of scope (v1)

- Regional/sub-global, rotated or non-lat/lon grids **as the project's own
  grid**. What a project paints on and exports is always a global regular
  lat/lon lattice at one of the four resolutions. Reading such a grid is a
  different matter and is in scope: §4.8's import decodes every grid
  definition the forecast centres ship and resamples the ones that are not
  lat/lon onto the project's lattice.
- GRIB1, NetCDF, or any other output format.
- Importing real forecast data *as objects*. A GRIB2 file can be imported as
  a layer (§4.8), but its samples are a lattice, not geometry: nothing traces
  it into strokes.
- Vertical levels beyond the single surface level per field kind.
- Multi-user, cloud sync, or collaborative editing.
- Scalar fields (pressure, temperature, wave height).

### 1.5 Non-negotiable invariants

These are the load-bearing rules of the architecture. Violating any one of them
is a design regression, not a trade-off.

1. **No raster is ever persisted as project data.** Rasters exist only as (a) a
   transient in-memory buffer, (b) an evictable on-disk render cache keyed by
   content hash, or (c) the bytes inside an exported GRIB2 file. Deleting the
   entire render cache must be lossless.
2. **The project file stores geometry and parameters, never pixels.** A brush
   stroke is a polyline plus a radius, not a bitmap.
3. **The view is a proxy, never a source.** The preview renders from the
   objects at whatever resolution is fast; the GRIB is baked from the same
   objects at the project's full grid resolution. **No pixel from the preview
   ever reaches the export.** The two paths therefore need to agree
   *perceptually*, not numerically (§7.9), and the preview is free to evaluate
   coarsely and interpolate (§7.7).
4. **Export is deterministic.** The same project exported twice on any two
   machines produces byte-identical GRIB2 output.
5. **Zero runtime network access.** The basemap and every other asset are
   bundled.
6. **Interaction stays fast; export is allowed to be slow.** Never trade frame
   rate for export throughput.

---

## 2. Glossary

| Term | Meaning |
|---|---|
| **Field** | The global u/v vector field at one time step. |
| **Time step** | One forecast hour offset. Indexed `0..step_count-1`. |
| **Object** | A single instance of a vector-creation tool: one brush stroke, one circle stamp, one curve — or several strokes that merged because nothing distinguished them (§6.1). |
| **Layer** | An ordered group of objects. Persists across all time steps. |
| **Scene** | The flattened, fully-evaluated set of objects for one time step, in z-order. |
| **Frame** | The rendered field for one time step. |
| **Frame hash** | Content hash of a scene; the render cache key. |
| **Coverage** | Whether an object writes to a given grid cell at all. Binary. |
| **Feather** | A speed falloff applied near an object's edge. Does *not* affect coverage. |
| **Azimuth-toward** | Direction a vector points, degrees clockwise from true north. The internal storage convention. |
| **AEQD frame** | An object-local azimuthal-equidistant coordinate system centred on the object's anchor. |

---

## 3. Conventions

These are pervasive and getting them wrong is the most likely source of silent,
hard-to-detect bugs. They are restated in `CLAUDE.md`.

### 3.1 Earth model

Single constant, used everywhere including GRIB encoding:

```
EARTH_RADIUS_M = 6_371_229.0
```

This matches GRIB2 shape-of-earth code `6` (spherical, radius 6,371,229 m), so
the geometry the user draws and the geometry declared in the exported file are
the same sphere. All geodesic math (distance, bearing, destination point) uses
spherical formulae on this radius. No ellipsoid.

### 3.2 Coordinates

- Longitude ∈ `[-180, 180)`, normalised on entry. Internally always this range;
  only the GRIB writer converts to `[0, 360)`.
- Latitude ∈ `[-90, 90]`.
- Angles in the public API are degrees; radians only inside math kernels.
- The antimeridian and the poles are never special-cased in geometry code. All
  rasterisation is done by geodesic distance and bearing from an anchor
  (§7.3), which is continuous across both.

### 3.3 Direction

**Every vector is stored internally as azimuth-toward** (`az`), degrees
clockwise from true north, ∈ `[0, 360)`, paired with a speed in m/s.

Export components:

```
u = speed * sin(az_rad)     // eastward,  m/s
v = speed * cos(az_rad)     // northward, m/s
```

Display and tool input differ by convention and are a *presentation* concern:

| Project kind | Default UI convention | Relationship |
|---|---|---|
| Wind | Meteorological — direction wind comes **from** | `display = (az + 180) mod 360` |
| Current | Oceanographic — direction current flows **toward** | `display = az` |

The project carries a `direction_convention` setting; every direction input in
the UI is labelled explicitly (`Direction (from)` / `Direction (toward)`) so the
active convention is never ambiguous. Conversion happens exactly once, at the
UI boundary. **No domain code below the IPC layer ever sees a "from" bearing.**

**Every direction input means every one**, the inspector included. A property
panel showing the stored azimuth where the tool that set it took a "from"
bearing reports the reciprocal of what the user just painted — 270 entered,
90 displayed — and the two disagree with no indication which is which.

So the schema marks *which angles are flow directions*: `Unit::Direction` for a
direction the field flows in, `Unit::Degrees` for a geometric bearing — an
object's rotation, a gradient's axis. Only the former is converted, and only it
carries the convention's label. Converting every angle would flip every object's
rotation by half a turn; converting none is the bug above. An angle that is a
flow direction in one mode and a relative offset in another (the curve's
`direction`) cannot be marked either way, and is resolved by *mode*, below.

### 3.4 Units

| Quantity | Storage | Notes |
|---|---|---|
| Speed | m/s (f32) | **Always displayed in knots.** Not configurable — see below. |
| Distance (geometry) | metres (f64) | |
| Distance (UI) | km and nautical miles | Both shown on measurement tools. |
| Sizes in tool options | km | See §3.5. |
| Time | UTC only | No local time anywhere. |

**Speed is stored in m/s and shown in knots, always.** m/s is what GRIB2
encodes, so it is what the document holds and what the evaluator works in.
Knots is what this tool's audience reads: a sailing forecast and a wind barb
are both in knots, and a barb is *defined* in 5-knot increments. An
earlier draft made the display unit a project setting; that bought nothing —
nobody wants half their speeds in km/h — and cost a branch at every display site
plus a field in the file format. Conversion happens at the UI boundary and
nowhere else, exactly as with the direction convention.

### 3.5 Pixel-valued options resolve to km at creation time

Several tools offer sizes "in px or km." Pixels are an **input convenience
only**. At the moment the object is created, a px value is converted to km using
the map scale at the cursor's latitude and the current zoom. **The stored
property is always km.** Consequence: zooming the map afterwards never changes
an existing object's footprint, which is required for objects to stay pinned to
the earth (§6.6).

#### px asks for a shape on the map, so it paints one

A ground circle projects to an ellipse (§5.1), `1 / cos(lat)` wider than it is
tall — at 60° twice as wide. There is therefore **no single "size in pixels"**
for a ground shape, and a px input measured against either axis produces a
footprint that visibly deforms as it moves away from the equator.

So a size given in px does not merely convert: it also chooses the space the
stamp is a shape in.

| Unit | Stamp space | Footprint |
|---|---|---|
| km | `geodesic` | A shape on the **ground**: a disc of `size_km` at any latitude, drawn as an ellipse that widens with latitude. |
| px | `projected` | A shape on the **map**: a circle of exactly that many pixels, at any latitude and any zoom. On the ground it is an ellipse, narrowed east-west by `cos(lat)`. |

`km/px` for a projected stamp is therefore `KM_PER_DEGREE / pxPerDeg`, with no
cosine: a degree of latitude is a fixed number of pixels everywhere, so the
conversion is latitude-independent and the footprint comes out the requested
number of pixels **across and tall**. `size_km` means the same thing in both
spaces — the footprint's north-south ground extent, the one axis the projection
leaves alone — so switching units changes the shape's width, never its height.

The space is a property of the object, frozen at creation like every other tool
option (§6.1), and it survives in the file. It is not a display setting: a
projected stamp paints a different field and exports a different GRIB, which is
why the two are separate objects and never merge.

**One unit per tool, not per field.** `stamp_space` is one property of the
object, so a circle with a diameter in px and a ring width in km would be
asking for a shape that is on the map in one of its measurements and on the
ground in the other. There is no such shape. The option bar therefore offers
the unit once, beside the tool's first size, and it governs every size that
tool has.

**The unit is the only control.** `stamp_space` is never offered as an option
of its own, because the unit already asks exactly that question and a tool that
offered both would have two controls for one property — with the space as the
one that did nothing, since it is the unit the gesture freezes. So the bar shows
`km`/`px` and the object stores `geodesic`/`projected`, and the mapping between
them is the whole of the rule.

It follows that a tool has a unit exactly when it has a `stamp_space`, and that
the unit inherits that property's dependencies: the shape fill's freehand
polygon places its vertices geographically one by one, so it has no size, and
is offered no unit to measure one in. A tool whose sizes are *dragged out*
rather than typed — the shape fill's presets — still has the unit, because a
drag is still a measurement and is still made either on the ground or on the
map; it simply has no number beside it.

**A projected object's local frame is map space, not AEQD** (§7.2). That is the
one exception to the frame rule, and it is a definitional one rather than a
concession: a shape defined on the map has to be measured on the map. It gives
up ground fidelity — a local metre is north-equivalent, worth `cos(lat)` metres
of ground when it points east — and gives up nothing at the antimeridian or the
poles, where it is if anything better behaved: there is no cosine to divide by,
longitude deltas normalise as they do everywhere else, and local north is true
north by construction rather than only at the anchor. Directions are unaffected:
they still come from geographic bearings, never from a frame angle (§7.2).

---

## 4. Project model

### 4.1 Project settings

Set at creation. Grid resolution, field kind, and time-step size are
**immutable after creation** — changing them would invalidate every object's
relationship to the grid and every rendered frame. `step_count` may be
increased or decreased after creation.

| Setting | Values | Immutable? |
|---|---|---|
| `field_kind` | `Wind` \| `Current` | Yes |
| `resolution` | `Deg1` \| `Deg0_5` \| `Deg0_25` \| `Deg0_1` | Yes |
| `step_hours` | `1` \| `3` \| `6` \| `24` | Yes |
| `step_count` | `1..=240` | No |
| `direction_convention` | `From` \| `Toward` | No (display only) |

If a user needs a different resolution, they create a new project. A "duplicate
project at new resolution" action is a possible future addition (§14).

**Reducing `step_count` deletes keyframes beyond the new end.** The action is
gated behind a confirmation dialog that states the exact count and lists the
affected objects by name, e.g. *"Reducing to 24 steps will delete 47 keyframes
across 6 objects."* Objects whose `active_range` extends past the new end have
it clamped. The deletion is a normal history entry and is undoable within the
session, but is permanent once the project is saved. This keeps the document
free of invisible state at the cost of making the shrink operation genuinely
destructive — hence the explicit, quantified confirmation.

### 4.2 Grid geometry

Global, regular lat/lon, cell **centres** on exact multiples of the resolution.
Origin at `(lon 0, lat 90)`, scanning west→east then north→south.

| Resolution | Ni (lon) | Nj (lat) | Points | u+v @16-bit per step |
|---|---|---|---|---|
| 1° | 360 | 181 | 65,160 | 0.26 MB |
| 0.5° | 720 | 361 | 259,920 | 1.04 MB |
| 0.25° | 1440 | 721 | 1,038,240 | 4.15 MB |
| 0.1° | 3600 | 1801 | 6,483,600 | 25.9 MB |

There is no duplicated column at longitude 360. Rows exist at both poles.

**Grid resolution does not affect the preview.** It governs the exported file
and nothing else: preview tiles are sized by zoom, so a 0.1° project pans and
zooms exactly as fast as a 1° one. Choosing the finest grid costs export time
and file size, never interactivity.

### 4.3 Document structure

```rust
Project {
    schema_version: u32,
    id: Id,
    name: String,
    settings: ProjectSettings,
    layers: Vec<Layer>,          // index 0 = bottom-most
    annotations: Annotations,    // measurement overlays (§8) — never affect the field
    view: ViewState,             // last camera + current time step; UI convenience
}

Layer {
    id: Id,
    name: String,
    visible: bool,
    locked: bool,
    objects: Vec<Object>,        // index 0 = bottom-most within the layer
    source: LayerSource,         // Painted, or Grib { path, field } (§4.8)
    raster: Option<Arc<RasterSequence>>,  // in memory only, NEVER serialised
}

Object {
    id: Id,
    name: String,                // user-editable
    kind: ToolKind,
    geometry: Geometry,          // object-local AEQD metres, NOT animatable
    active_range: (u32, u32),    // inclusive time-step range
    props: PropertyMap,          // every entry is Animatable
}
```

**Z-order** is layer order first, then object order within the layer. The
basemap is not a layer and is always beneath everything (§5.2).

### 4.4 Animatable properties

Every property of every object is animatable — including the common transform
properties, and including enums and booleans.

```rust
Animatable<T> {
    base: T,                     // value when no keyframes exist
    keys: Vec<Keyframe<T>>,      // sorted by step, unique steps
}

Keyframe<T> {
    step: u32,
    value: T,
    interp: Interpolation,       // governs the segment *leaving* this key
}
```

Every object carries these four common properties in addition to its
tool-specific ones:

| Property | Type | Default | Notes |
|---|---|---|---|
| `position` | `LonLat` | creation point | The AEQD anchor. |
| `scale_pct` | `f32` | 100.0 | Scales geometry radii in metres. |
| `rotation_deg` | `Angle` | 0.0 | True bearing rotation about the anchor. |
| `enabled` | `bool` | true | Step interpolation only. |

`edge_mode` (§7.4) is carried alongside these by every tool that paints a
field. A **modifier** (§6.3) does not carry it: its output is whatever was
beneath it, changed, so there is nothing for "replace" to name.

### 4.5 Interpolation

| Value type | Allowed interpolations | Rule |
|---|---|---|
| `f32` | Step, Linear, EaseIn, EaseOut, EaseInOut, Bezier(cubic) | Standard. |
| `Angle` | same | **Shortest-arc**; wraps through 0/360 correctly. |
| `LonLat` | same | Great-circle slerp between keys, easing applied to the arc parameter. |
| `bool`, enum | Step only | UI hides other options. |
| `Geometry` | none | Not animatable. Move/scale/rotate the object instead. |

Evaluation at a step outside the keyframe range holds the nearest key's value
(no extrapolation). With zero keys, `base` is used at every step. **This holds
for holding kinds too**: a lone `enabled = false` key at step 3 switches the
object off from step 0, because before the first key it is the first key that
holds, not the base. The timeline marks where a value is held rather than
keyed so this is visible.

A new key takes `Linear` wherever the kind allows it and `Step` otherwise. A
keyed speed or position is nearly always meant to move between its keys.

### 4.6 Active range

Each object has an inclusive `active_range`. Outside it the object contributes
nothing, regardless of `enabled`. This is what the timeline's per-object bar
edits (§9.4). `enabled` is the animatable per-step switch; `active_range` is the
coarse lifetime.

### 4.7 File format

Single-file container, extension `.veproj`. A ZIP archive (deflate):

```
project.json          canonical JSON, stable key order, pretty-printed
META-INF/version      schema_version, for fast pre-parse rejection
```

- JSON is chosen over a binary format for diffability and recoverability. A
  5,000-object project is a few MB; parse budget is in §12.
- **No rasters, no thumbnails, no cache.** Invariant 1. Enforced by a test that
  fails on any unexpected archive entry.
- **Canonical numeric precision.** `f64` values are quantised at the
  serialisation boundary — distances to the millimetre, angles and coordinates
  to 1e-9°, ratios to 1e-6. This is not a stylistic choice: `serde_json`'s
  parser lands one ULP away from the correct value for roughly 10% of `f64`s
  (measured: 19,953 failures in 200,000 samples), so an unquantised project does
  not equal itself across a save and load. The chosen precisions are orders of
  magnitude finer than the 11 km finest grid cell, so nothing observable is
  lost, and `f32` — which every scalar property uses — is unaffected.
- **Deterministic archive.** ZIP entries carry a fixed timestamp, so saving an
  unchanged project twice produces identical bytes.
- **A revision is unique to one opening of one project**, seeded from the wall
  clock rather than starting at 1. Tile URLs carry it and tiles are served
  `immutable` for a year (§7.7), so two documents that both counted from 1 would
  share tile addresses: the webview then answers the new project's requests out
  of the old project's cache, and the previous document's strokes appear on the
  new map. Nothing depends on a revision being reproducible — it is a cache key,
  and never reaches a project file or an export.
- `schema_version` gates a migration chain in `ve-core::migrate`. Opening a
  newer-schema file is refused with a clear message; older schemas migrate
  forward on open and are written back at the current version on save. The
  chain is currently: **1 → 2**, a stroke's single `points` polyline becoming a
  `chains` list of one, for merged strokes (§6.1); **2 → 3**, the brush's
  circle-or-square choice moving off `fill_mode` — the id the circle stamp uses
  for filled/perimeter/gradient — and onto a `brush_shape` of its own. The step
  drops the old key rather than renaming it: it was never settable, so every
  version-2 brush holds the default, which is the same default `brush_shape`
  backfills to.

**Import** means opening a `.veproj` produced elsewhere — the same path as open,
with migration. The one foreign format that can be imported is GRIB2, as a
layer rather than as a project (§4.8).

**Autosave** writes a recovery copy to the app data directory every 60 s and on
every 50 history entries, whichever comes first. On launch, an unclean shutdown
offers recovery.

**Replacing the open project.** New and Open each replace what is open, as does
`close_project` — which the app no longer offers a button for, the window's own
close being the way out; the command stays because opening and creating are
built on it. When it has unsaved changes the user is asked first, with three
answers — **Save**, **Don't save**, **Cancel** — not the two a plain confirm can
offer: making saving the thing the user has to think of *before* reaching for
the action is how work gets lost. Cancel is the default, taken by Escape and by
clicking outside, because it is the only answer that cannot lose anything.
Choosing Save and then cancelling the destination dialog cancels the whole
operation; it never falls through to discarding.

**"Don't save" is carried, not acted on.** The answer travels as an argument to
the command that replaces the project — `new_project(request,
discard_unsaved)`, `open_project(path, discard_unsaved)` — so nothing is dropped
until that command runs. Discarding at the moment the question is answered would
mean a user who says "don't save" and then cancels the file dialog has lost the
project to an operation that never happened.

The prompt is the frontend's, but the refusal is not: both commands return
`unsaved-changes` rather than replacing a dirty project when not told to
discard, so no caller can drop a user's work by forgetting to ask.
`close_project` is the one command whose purpose is to discard, and it does so
without complaint.

### 4.8 Imported GRIB layers

A GRIB2 file can be imported as a layer, from the **Import GRIB** button
beside the layer panel's `+`. The layer carries the file's `u`/`v` field as a
lattice — a *raster*, in the sense of invariant 1 — beneath any objects
painted on it, and composites like any other layer: where the lattice has a
value it overwrites what is beneath; where it has none (outside a regional
grid, under a bitmap's gaps) it leaves the field beneath alone. It has no
feather, since a lattice has no edge.

**What the project keeps is the path.** Invariants 1 and 2 stand: the
`.veproj` stores `LayerSource::Grib { path, field }` and nothing else, the
decoded lattice lives in memory beside the layer, and the file is read again
when the project opens. A file that has gone leaves its layer in place,
empty and marked in the panel; the user's own objects are unaffected. The
project is therefore portable only with its GRIB, which is the price of not
copying forecast data into every project that references it.

**Open from GRIB.** The start screen can also build a project *from* a
file: the kind is wind when the file has wind and current otherwise; the
grid is the app resolution nearest the file's spacing; the time step is the
largest the app offers that **divides** every message's offset, so every
message lands on a step and none of them is invisible; the count covers the
file's span; step 0 takes the first message's valid time as the project's start
time; and the name is the file's.
The file then imports as it would into any project, and the history starts
empty — the import is what the project is, not an edit to it.

**One layer per field kind.** A file holding both wind and currents imports
as two layers, wind above currents. The layer whose kind matches the project's
is shown; the other is imported *hidden*, because the map composites every
visible layer into one field and a current drawn over a wind is not a wind.
It can be shown, and it is then treated as a field of the project's kind —
which is what the user asked for by showing it.

**Time alignment.** The file's earliest valid time is aligned with the
project's step 0, whatever the file's reference time. Each step then shows the
message valid **at** that step's forecast hour, and **a step the file has no
message for shows no imported field at all** — the layer is simply not there,
and whatever the user painted on it stands alone.

So a 3-hourly file in an hourly project shows its 0 h message at hour 0 and
nothing at hours 1 and 2; an hourly file in a 3-hourly project is read at 0, 3,
6 and its other messages are never shown; and past the file's last message
there is nothing, for the rest of the timeline however long it is.

**A step can be told which message to show.** The rule above is right and it
leaves the user no way to say otherwise, so a step of an imported layer can be
given one: click a mark on the layer's timeline row, `Shift`-click a run of
them, `Cmd`-`C`, move the playhead, `Cmd`-`V`. The run keeps its spacing —
steps 0 to 3 pasted at 6 land on 6 to 9 — and a step past the end of the
timeline is **dropped, not clamped**, since clamping would pile the tail of a
run onto the last step and leave one frame where four were asked for.
`Delete` on a pasted mark restores the file's own message there; `Delete` on
the file's own hides it, which is what a bad message needs.

**The paste goes to the layer the copy came from**, whatever layer is active:
the clipboard entry names it, and a frame pasted into a layer reading a
different file would be a copy of samples by another route.

**What the project stores is a step number, never a sample.** Invariants 1 and
2 stand: the frame that reaches the map is one the file already holds, so the
flat scene's raster hash follows the choice by itself and the render cache,
the readiness probe and both kernels are correct without knowing the feature
exists. Two steps showing one message hash alike, so the cache holds one frame
for the pair. An override references a *step*, and which message a step gets
is decided by time alignment on open — so if the file at the path is replaced
by one with nothing at the source step, the pasted step shows nothing and its
mark is drawn empty, like a layer whose file has gone.

The file's own marks are drawn solid and a pasted one as an outline, so the
timeline still says which times the file actually covers after the gaps have
been filled.

This is deliberately *not* the hold rule a keyframe follows (§4.5), and the
difference is what the two kinds of value are. A keyframe is an instruction the
user gave: between two of them the document still means something, and holding
the earlier one is the meaning. A message is a measurement, made for one time;
holding it forward draws a forecast for a time it was never made for. Past the
end of a short file that is at its worst — a six-hour file standing in for a
ten-day timeline, unchanging and looking like data.

A time with only one of `u` and `v`, or whose two components sit on different
grids, is dropped from the sequence, and is therefore a time with no message
like any other.

**Speed filter.** A GRIB layer can be limited to a band of speeds: a low end
and a high end, in knots on screen and m/s in the document, set by a slider or
typed. A sample outside the band is dropped exactly as a missing one is, so
whatever is beneath shows through — including the layer's own painted objects,
which is what makes this a filter on the *import* rather than on the layer. A
forecast is far easier to read one band at a time: the calms, the gale, the
jet.

It is a property of the layer and not of the lattice — a choice about what to
show, not a fact about the file — so it costs two numbers in the project file,
survives the file being re-read on open, and is undoable like any other edit.
Both kernels apply it, in the same place: after the lattice is sampled and
before it is written into the buffer. The scale the slider runs to is the file's
own fastest sample, since a filter is set by looking at the field.

**Levels.** A file often carries wind at several heights. The 10 m wind
(surface type 103 at 10 m) is taken when present, other heights only when it
is not; currents prefer the surface (type 160 at 0 m). Speed is m/s on the
wire and, as everywhere, knots on the screen (§3.4). Wind barbs and arrows
come from the same tiles as the painted field, so the imported field gets
them for free.

**Both kernels sample it.** The lattice is sampled bilinearly, missing corners
left out of the blend, on the CPU and in the WGSL kernel alike, so the
preview and the export agree on it to the tolerances of §7.9 — the fidelity
suite generates raster scenes. On the GPU a scene's rasters share one storage
binding, so a scene with more than 128 MB of lattice (two global 0.1° grids)
falls back to the CPU like a clone-stamp scene does. The render cache keys on
the lattice's content hash and its place in the stack (§7.10).

**Decoder.** Pure Rust, in `ve-grib`: **every grid definition the forecast
centres put a model on** — regular lat/lon (3.0), rotated lat/lon (3.1),
Mercator (3.10), polar stereographic (3.20), Lambert conformal (3.30),
NCEP's rotated Arakawa non-E staggered grid (3.32769) and the unstructured
one (3.101) — in any scanning mode, the common product templates, and
**every packing they ship** — simple (5.0), complex with and without spatial
differencing (5.2, 5.3), JPEG 2000 (5.40) and CCSDS adaptive entropy coding
(5.42) — with bitmaps throughout. PNG packing (5.41), Gaussian and thinned
grids, a rotated grid turned about its own pole, and GRIB edition 1 are
refused by name, as is a CCSDS stream of signed samples, which the shared 5.0
scaling has no meaning for.

The section walking and the three integer packings are hand-written; the two
compressed ones are `hayro-jpeg2000` and `rust-aec`. Both are pure Rust with
no C library behind them, so invariant 5 holds — no external decoder, no
network — and the three-platform build needs nothing installed. Every one of
them yields the same packed integers 5.0 stores in the clear, so one scaling
formula serves all five.

**Projected grids.** Almost every regional model runs on a projection rather
than on lat/lon: HRRR and the air-quality models on Lambert conformal, the
Alaska and arctic grids on polar stereographic, the Hawaii and Puerto Rico
ones on Mercator, HRDPS and RAP on a sphere whose pole has been moved. Each
is a regular lattice — evenly spaced rows and columns — but its rows are not
parallels and its columns are not meridians, so where a node sits comes from
inverting a projection. The formulas are Snyder's, ellipsoidal, and reduce to
the spherical ones exactly when the eccentricity is zero, so the shape of
earth a file names (code table 3.2) is honoured rather than assumed: four
messages in a 1,653-message corpus are on WGS 84 and the rest are on spheres
of four different radii — only one of which is the 6 371 229 m the app itself
works on (§3.1).

**A projected field is resampled onto the project's own grid at import**, for
the same reason an unstructured one is: nothing downstream speaks anything
but a lat/lon lattice. It is resampled onto **the span of that grid it
covers**, not onto the globe — a 2.5 km regional model reaches a few percent
of the earth, and a global 0.1° lattice of it would be 52 MB of mostly
nothing. The window's nodes *are* the project's nodes, so an imported
regional field lines up with what the project exports; outside it the layer
has no value, which is what a regional grid has always meant here. The value
at a node is a bilinear point sample of the four source nodes around it, a
missing corner left out of the blend, exactly as `RasterGrid::sample` treats
the lattice it produces.

**Components resolved along the grid are rotated onto east and north, at the
source.** GRIB flag table 3.5 lets a message resolve `u` and `v` along the
grid's own axes rather than along east and north, and most regional models
do. Reading those as eastward and northward is not a small error: it is 30°
or more across a Lambert CONUS grid, and it looks entirely plausible on
screen. The angle is the grid convergence — `n·(λ - λ₀)` for the conic and
stereographic projections, the angle between the two poles' directions for a
rotated one, zero for Mercator. Each **source** node is rotated before the
blend, not each target node after it: near a stereographic grid's pole the
convergence turns through a full circle in a few cells, and an interpolated
grid-relative vector there means nothing at all.

A grid that encircles a pole covers every longitude, and no walk of its
boundary can discover that — so the pole is projected into the grid's own
plane and the grid asked whether it holds it. Such a field comes out wrapping
the earth and reaching the pole itself, rather than stopping at wherever the
boundary walk happened to start.

Two departures from what the octets literally say, both forced by real files
and both noted where they are made. NCEP's template 3.32769 states
increments in no unit that reproduces its own corners; since it also states
its first point, its last point and its centre as true coordinates, the
increments are derived from those instead, and the two corners then sit
symmetrically about the stated centre to nine digits. And the Mercator
orientation field, which two of NCEP's blend grids fill with 200° and 295°,
is read and ignored: the corners those files state are where an unrotated
grid puts them to a few parts per million of their own spacing. ecCodes and
wgrib2 make the same two calls.

**Unstructured grids.** Not every model runs on a lat/lon grid. ICON's is
icosahedral: 2,949,120 cells of roughly equal area, and a message
(template 3.101) carries a bare run of values, the cell count, and a **UUID
naming the grid** — nothing about where any cell is. The positions are a
separate thing entirely, which DWD publishes as `CLAT`/`CLON` messages on
that same grid.

Those positions are **bundled**, converted at build time by
`tools/icon-grid-builder` into `assets/icon_grids.bin` and matched by UUID, so
an ICON import needs no files but the forecast and reaches no network. A file
whose UUID nothing bundled matches is refused by name rather than placed on a
guess. The asset holds ICON global R03B07 and the ICON-EPS global R02B06 mesh;
it is grid *geometry*, time-invariant and shared by every file ever issued on
that grid, which is the kind of thing §1.5 says a project may hold.

**An unstructured field is resampled onto the project's own grid at import**
and is an ordinary raster from then on — the render cache, both kernels and
the exporter never learn that such files exist. The value at a target node is
a **point sample**, interpolated inverse-distance from the three nearest
cells, because that is the convention everything else already works in: the
evaluator computes each cell's vector *at* the cell, and averaging here would
make an imported field mean something different from a painted one at the same
resolution. `u` and `v` are interpolated as components, never as speed and
bearing, which would turn two opposing vectors into a fast one pointing
nowhere. A missing cell is skipped rather than blended, and a node with
nothing present is missing.

The search that finds those three cells is the expensive half — half a second
for a global 0.1° grid — and depends only on the mesh and the project's
resolution, never on the values. It is therefore computed once and **kept in
the project**, as its own binary archive entry keyed by mesh and grid. It is
derived state: losing it costs a rebuild and nothing else, and a set that no
longer matches the project's grid is dropped on open rather than trusted.

For a file that states no spacing, the resolution a new project gets comes
from the mesh itself: `n` roughly equal cells over the sphere are about
`sqrt(4π/n)` radians across, which for ICON global is 0.118° and picks the
0.1° grid. A projected file states metres, and a metre is a fixed fraction of
a degree of latitude wherever the grid is, so it reaches the same answer.

A regional model is usually finer than the finest grid the app offers, and
resampling it onto that grid is a real loss: a 2.5 km field on a 0.1° lattice
keeps about one node in four. That is the price of one lattice for the whole
app, the same price §4.8 already pays for ICON, and it is paid once at import
rather than at every frame.

A message that cannot be read is skipped and logged rather than failing the
file; a file with no usable `u`/`v` pair is refused with the reasons.

---

## 5. Map view

### 5.1 Projection

**Equirectangular (Plate Carrée).** Chosen because the grid is global lat/lon:
the map is 1:1 with the data grid, both poles are visible (Mercator cannot show
them, and a global grib has rows at ±90), and there is no zoom-dependent
distortion of the editing surface.

Distortion near the poles is inherent and visible. Tools that care expose an
explicit choice (§6.2, circle tool) between a shape that is circular *on screen*
and one that is circular *on the globe*.

Other projections, including a globe, are planned as M11. They are a view
concern only: object geometry is stored in geodesic frames and the export has
its own grid, so a projection cannot affect a saved project or an exported file
(invariant 3).

- Pan: unbounded in longitude (wraps seamlessly), clamped in latitude.
- Zoom: continuous, from whole-world to roughly 1 grid cell ≈ 8 px.
- The camera state lives in the frontend and is mirrored into `ViewState` on
  save.

### 5.2 Basemap

Bundled **Natural Earth** (public domain) land polygons and coastlines at 1:110m
and 1:50m, converted at build time into a compact binary of pre-tessellated
triangles plus coastline line strings. Rendered as WebGL geometry, LOD switched
by zoom. Graticule drawn procedurally.

The basemap is **not a layer** and does not appear in the layer panel. It is
always beneath all layers and cannot be reordered, hidden, or edited.

### 5.3 Field display

The map **always** shows speed and direction. `u` and `v` are never displayed
anywhere in the UI.

- **Speed** as a colour ramp raster, sampled from the render tiles (§7.7), with
  a legend and an auto/manual scale control.
- **Direction** as instanced glyphs on a lattice anchored to the globe — points
  sit at whole-degree multiples chosen so their on-screen spacing stays near a
  target, and the step snaps to a fixed ladder so it changes only at discrete
  zoom thresholds. Glyphs are generated in the shader by sampling the tile
  texture, so glyph layout costs no CPU round-trip.

  A screen-anchored lattice was tried first and abandoned. Anchored per tile it
  leaves a gap of `tile width mod spacing` at every tile edge, which reads as
  glyphs clustered into blocks; anchored to the viewport it makes glyphs slide
  through the field as the map pans. Anchoring to the globe is continuous across
  tile boundaries by construction and keeps each glyph on its own geographic
  point. In equirectangular projection a whole-degree lattice is uniform on
  screen at every latitude, so nothing is lost by it.
- **Two glyph styles ship in v1**, switchable from display settings:
  - **Arrows** — uniform instanced geometry, length optionally scaled by speed.
    Available for both project kinds.
  - **Wind barbs** — the meteorological idiom, quantised to the conventional
    5 kt half-barb / 10 kt full barb / 50 kt pennant. Wind projects only;
    hidden for current projects. Requires variable per-instance flag geometry
    (see M2 in `plan.md`).
  - Speed is always also carried by the colour ramp, continuously and without
    quantisation, so barbs never become the only speed reference.
- Cursor readout: lon/lat, speed in the display unit, direction in the display
  convention, and the grid cell index under the cursor.

### 5.4 Stale-tile behaviour

While a frame is re-rendering, previously-rendered tiles remain on screen,
dimmed slightly, so pan and zoom never blank out. A small activity indicator
shows rendering is in flight.

### 5.5 Map chrome

Everything drawn over the map — the active tool's options, the colour legend,
the cursor readout, selection handles, gesture previews — obeys three rules.

**The tool option bar spans the map view and wraps within it.** It is the width
of the map, not of its contents. A tool's options grow as the tool gains them,
and a bar that sizes to its contents puts the last ones past the right-hand edge
where they cannot be reached or even seen. Wrapping to a second row is the cost
of that, and it is the right cost: a control the user cannot reach is worse than
one a row lower. Nothing else occupies the map's top edge.

**Chrome does not overlap chrome.** The legend sits bottom right, the readout
bottom left, the option bar along the top; each is placed so that another
growing cannot cover it. A panel that appears over a control is indistinguishable
from a broken one.

**Chrome that names a place moves with the map.** Handles, aim markers, gesture
previews and footprint outlines are positioned by the camera, so they are drawn
in the same frame as the field beneath them — never on a schedule of their own.
Objects are pinned to the earth (§8.3) and so is everything drawn to point at
them: a handle that lags a pan by even a frame reads as the selection coming
loose from the object.

---

## 6. Vector-creation tools

### 6.1 Shared behaviour

All tools produce **objects**. Common rules:

- **One gesture, one object — unless it merges.** A brush stroke from
  pointer-down to pointer-up commits exactly one object, and a stamp click
  commits one object. The exception is a stroke that lands on an existing one it
  is indistinguishable from: it is absorbed as another *chain* of that object
  instead. Going over an area repeatedly then leaves one thing to select, move
  and animate, rather than a pile of identical objects.

  Merging is only allowed where it cannot change what the layer looks like, so
  all of the following must hold:

  - Same tool and same `active_range`.
  - Every property equal, keyframes included, except `Position` — which the two
    necessarily differ in, and which must not be animated on the target (an
    animated position moves the frame the absorbed stroke would be expressed
    in).
  - Footprints actually overlap, measured segment-to-segment (§7.3).
  - Nothing between them in z-order overlaps the new stroke. Merging pulls a
    stroke down to the target's place in the stack, and an overlapping object in
    between would end up covering it.
  - No point of the absorbed stroke lands more than a quarter of the earth's
    circumference from the target's anchor, which keeps the object's local frame
    well conditioned (§7.2).

  A merge is a geometry change on the existing object, so undo restores the
  previous geometry rather than removing an object.
- **Options freeze at creation.** The tool's current option values are copied
  onto the object as its `base` property values. Changing a tool option
  afterwards affects only *subsequently created* objects, never existing ones.
  (Editing an existing object's properties is done through the inspector.)

  Because they freeze, the inspector is where an existing object is corrected,
  so it has to be able to edit every kind of value a property can hold.

  **Some properties are fixed once the object exists.** They describe what the
  object *is* rather than a parameter of it, and changing one afterwards re-makes
  it into something the user did not draw: the brush's `brush_shape` is part of
  the geometry the stroke painted, and its `stamp_space` is the space that
  geometry was measured in. Getting a different one means painting a different
  stroke. Such a property is marked `creation_only` in the schema, and the rule
  is enforced at the write path as well as in the panel — a rule the document
  does not enforce is decorative, and the timeline would walk straight through
  it. It follows that a creation-only property is not animatable: a keyframe is
  an edit spread over time.

  The corollary is that a creation-only property must be settable *at* creation,
  which means the tool's own options. One that is neither editable nor offered
  by its tool cannot be set to anything but its default, and does not belong on
  the tool at all — which is why a property no tool offers is removed from the
  model rather than left frozen at its default (§7.5).

  **A property its object's own mode makes inert is not listed.** A brush aimed
  at a point never reads its constant bearing; one on a constant bearing never
  reads its target. Offering either invites editing a value and watching nothing
  happen, which reads as a broken control. The dependencies are declared beside
  the property tables (`schema::dependencies`) and resolved where the values
  are, so the frontend needs no knowledge of what any mode means.

  Hidden is not deleted: the value stays in the document, so switching the mode
  back brings it back as it was. Nothing else may read a property this rule
  hides — if the evaluator reads it, the rule is wrong. **A position
  property is edited by pointing at it as well as by typing it**: alongside its
  coordinate fields, each one offers to take the next map click. A coordinate
  pair is exact but useless for a point chosen by where it is — an aim point, an
  anchor, a clone source — and those are exactly the properties a user needs to
  adjust after seeing the result. The picker is driven by the value's kind, so
  it appears for every position property without the inspector knowing what any
  of them means, and an armed pick takes the click ahead of every tool.
- Objects persist across all time steps within their `active_range` and belong
  to exactly one layer — the layer selected when they were created.
- A layer holds an unlimited number of objects.
- All objects support: rename, delete, duplicate, copy/paste, select and
  multi-select, enable/disable, move, rotate, scale.
- All objects are geospatial polygons or polygon-generating geometry. Nothing is
  defined in screen space after creation (§3.5).
- Where a tool specifies a hover indicator, it previews the exact footprint that
  a click would produce, transformed to the current camera.
- **A gesture in progress previews the field it will paint, not just its
  outline.** The swept region is filled in the speed colour of §5.3's ramp, and
  direction glyphs are drawn over it in the current style, on the same
  globe-anchored lattice the map uses. What the user is aiming is a wind, so
  what the tool shows while aiming is that wind, in the terms the map already
  displays it in.

  **The two operators are previewed on the map itself.** The mask writes calm
  and the clone stamp reads the composite beneath it, so what either one paints
  is defined by what is already there — and a coloured wash would be showing
  something neither tool does. Nor could an overlay show a removal at all: it is
  a canvas stacked above the field, and it can add pixels, never take them away.

  So a gesture with either one is applied to the field *while the pointer is
  down*: the swept footprint becomes a screen-space mask, and the map is drawn
  through it. The mask's covered region loses its field, leaving the basemap —
  which is never masked, since something has to be left to see. The clone's
  loses it and gains the field from the source instead, drawn through a camera
  shifted so the source lands under the brush; a shift is enough because the
  projection is equirectangular, where a constant offset in degrees is a
  constant offset in pixels at every latitude (§5.1).

  **An operator's overlay draws nothing but its nib**: the outline of the stamp
  under the pointer, held through the drag. Not the swept region — a footprint
  is a union of stamps and a stroked path is not a union, so outlining a sweep
  traces every stamp's own circle and leaves a chain of rings trailing the
  pointer, which is neither what either tool does nor what it looks like. The
  mask is the preview; the nib only says where the tool is, which is what
  matters where there is nothing beneath to operate on and so nothing else
  changes.

  The mask is the same footprint the overlay would have drawn, so a tool
  declares how it previews and inherits the rest. It is rasterised at half the
  framebuffer's resolution: it is uploaded on every pointer report, and the
  softened edge is a preview's to give away where the frame budget is not
  (§7.9, §13).

  This is overlay drawing, not evaluation: the colour comes from the tool's own
  options rather than from a tile, so §7.9's fidelity tolerances do not apply to
  it and nothing is evaluated on the pointer path. It is deliberately not the
  composited answer — the stroke has not been committed, and what is drawn is
  what the *stroke* carries, not what the stack under it will resolve to. A
  calm stroke keeps a floor under its opacity, where the field itself would fade
  to nothing, so that an aimable outline never disappears.

  Placement follows §5.3's lattice, so a brush narrower than the lattice step
  can cover no lattice point at all. One glyph is then drawn at the head of the
  stroke instead: off-lattice, but a preview showing no direction at all is
  worse.
- **The preview is held until the field it previews is on screen.** Committing
  is an IPC round trip and a tile render, so the gesture ending is not the
  moment the paint appears. Dropping the preview at pointer-up leaves a window
  in which the stroke is in the document and nothing on screen shows it, and the
  paint visibly blinks out and back.

  §5.4's dimmed stale tiles do not cover this: an edit bumps the revision, tile
  URLs carry the revision, so an edit re-addresses every tile and there is no
  previous tile at the new address to hold over. The preview is what covers it,
  and it is dropped once the revision it produced has been drawn with no tile
  fetches outstanding — with a timeout as a backstop, so a failed fetch cannot
  leave a preview painted on the overlay indefinitely.

  A held preview draws from the values the stroke froze at creation, not from
  the tool's current options: the two can differ by the time it is dropped.

#### What a tool inherits

Everything in §6.1 is **shared behaviour, not the brush's**. The brush is only
the tool that happens to exist first, and each of these rules was written after
the same class of mistake — a behaviour built for one tool, and the next one
built differently. A new tool gets them all, and the checklist below is what
"done" means for one.

| Inherited | Rule | Where |
|---|---|---|
| Option bar | Spans the map view and wraps within it. A tool's options grow with the tool, and a bar that sizes to its contents puts the last ones off screen where they cannot be reached. Nothing else may occupy the map's top edge. | §5.5 |
| Sizes in px | A size given in px selects `stamp_space: projected` and one in km selects `geodesic`, for **every** tool with a size. px means a shape on the map, at any latitude and zoom; km means one on the ground. One vocabulary, one property — and one control: the unit, never the space beside it. | §3.5 |
| Gesture preview | Every gesture previews the field it will paint — speed colour and direction glyphs, not an outline — and the preview is held after release until the new revision's tiles are drawn. A tool that *operates* on the field rather than adding one is previewed by applying the operation to the map, live, and that too is held until the commit lands. | §6.1 |
| Hover indicator | Where a tool has one, it is the exact footprint a click would produce, in the same colour and with the same glyph as the gesture preview. | §6.1 |
| Chrome | Handles, markers and previews are drawn in the same frame as the map, and never overlap each other. | §5.5 |
| Position options | Every `LonLat` option is placeable by pointing: on the tool while setting it up, and on an existing object through the inspector. Typing coordinates is the alternative, never the only way. | §6.1 |
| Eyedropper | A tool that paints a *single* vector can take that vector from the map: arm it, click, and the speed and direction come from the field at that point. Offered exactly where the two properties it writes are both live — never in a gradient mode, which has two of each, and never where a bearing is an offset rather than a direction. | §6.1 |
| Direction modes | `Constant`, `TowardPoint` and `AwayFromPoint` mean the same thing for every tool that has a `target`, and are the same property with the same variant indices. | §6.2 |
| Direction display | Flow directions are shown in the project's convention wherever they appear, the inspector included; geometric bearings are not converted. | §3.3 |
| Inert options | An option the object's own mode never reads is not shown. | §6.1 |
| Creation-only options | An option that defines *what the object is* — the stamp's shape, the space it is defined in — is fixed once the object exists, refused at the write path and not merely hidden. An option that can be neither set at creation nor edited afterwards does not belong on the tool. | §6.1 |
| Transform | Move, rotate, scale and re-anchor behave identically for every geometry, because they act on the object's frame rather than on its shape. A drag previews and writes once, on release. | §8.2 |
| Merging | Two gestures of the same tool with identical properties and overlapping footprints merge into one object, subject to §6.1's conditions. Two that differ in *any* property — including the stamp's shape or space — never do. | §6.1 |

**The eyedropper samples what is visible.** The value it takes is the composite
the evaluator produces at that point (§7.6) — every visible layer beneath the
pointer, and no hidden one, which is exactly what the map is drawing. It is
sampled through the evaluator and not read back from the tile under the cursor:
a tile carries the 16-bit quantisation the map draws with (§7.7), and this
number goes into the document. It is written in stored units, m/s and an
azimuth-toward, and converted for display like any other value (§3.3).

The one rule that is genuinely per tool is **whether a hover indicator exists**;
§6.2 states it for each, and "none" is a decision to be made deliberately rather
than an omission.

**These are inherited by construction, not by discipline.** A checklist that has
to be worked through by hand is a checklist that will be missed, so the shared
behaviour is shared code and a tool supplies only what is genuinely its own:

- **One creation command** for the whole catalogue. A tool sends a *gesture* —
  the geometry the pointer drew — and its *options*, which are property values
  keyed by the same ids the inspector uses. The aim-mode check, the anchor, the
  frame, the layer choice and the merge test are written once. The mask and
  the clone stamp are brush-like because all three send the same gesture, not
  because three code paths were written to resemble each other.
- **Five gestures, not one per tool**: a stroke, a click, a centre-out drag, a
  ring of placed vertices, a path of nodes. Two tools that draw the same way
  share the gesture, so anything true of one is true of the other.
- **One option bar**, rendered from what the schema says a tool has — the same
  question the inspector asks, so the two cannot disagree about which options a
  mode makes inert.
- **One footprint type** — swept, disc, ring, rectangle, polygon — which the
  gesture preview, the hover indicator and the held preview all draw. A tool
  supplies its footprint and inherits all three; a tool that gets it wrong is
  wrong in all three at once, which is the failure worth having because it
  shows up immediately rather than in whichever of the three nobody tried.

The two facts about a tool that are *not* properties — which gesture drives it,
and whether it has a hover indicator — are declared alongside its schema, so
they cannot be forgotten either.

### 6.2 Tool catalogue

Property types: `f32` unless noted. All are animatable per §4.4.

#### Brush

Freehand painting. Geometry is a polyline of pointer positions; footprint is the
swept capsule chain of the brush shape along that polyline.

| Option | Type | Notes |
|---|---|---|
| `brush_shape` | enum `Circle` \| `Square` | **Fixed at creation** (§6.1). The stamp swept along the polyline; the square is axis-aligned in the object's frame, so rotating the object turns it. |
| `size_km` | f32 | Entered as px or km; stored km (§3.5). The disc's **diameter** and the square's **side**, so the two shapes agree across the flats and differ only at the corners. Measured north-south, which is the axis both stamp spaces share. |
| `stamp_space` | enum `Geodesic` \| `Projected` | **Fixed at creation** (§6.1), being the stamp's geometry like `brush_shape`. Whether the stamp is a shape on the ground or a shape on the map (§3.5). **Not a control of its own**: the size's unit chooses it, km paints geodesic and px paints projected. Two strokes differing in it never merge — the footprints are different shapes. |
| `speed` | f32 m/s | |
| `direction_mode` | enum `Constant` \| `TowardPoint` \| `AwayFromPoint` | Step-interpolated. `AwayFromPoint` is the reciprocal of `TowardPoint` at every cell, so a field radiating out of a low and one converging on it are the same stroke with one option flipped. |
| `direction` | Angle | Used when `Constant`. |
| `target` | LonLat | Used by both aimed modes: each cell's azimuth is the initial great-circle bearing from that cell to the target, and `AwayFromPoint` adds 180° to it — the outward tangent to the same great circle. **Not** the bearing measured at the target: that differs from the reciprocal by the meridian convergence between the two points, which is tens of degrees for a distant target. Set three ways — typing coordinates, arming **Pick on map** and clicking, or dragging the marker itself. The marker is a handle and takes precedence over painting under it, on the same rule as §8.1's transform handles: without that, a placed target could never be adjusted on the map, only retyped or re-picked. |
| `feather` | f32 0–1 | Fraction of the radius over which speed falls to zero at the edge. |

**No tool has `divergence` or `curl`** (§7.5). An object's direction mode is
the whole of its direction; a field that converges as well as turns is built
from more than one object.


Hover: yes — outline of the brush footprint at the cursor, filled in the speed
colour with a single direction glyph at the tip (§6.1).

#### Circle (stamp)

Click to place; no drag.

| Option | Type | Notes |
|---|---|---|
| `fill_mode` | enum `Filled` \| `Perimeter` \| `FilledGradient` | |
| `ring_width_km` | f32 | `Perimeter` only. |
| `speed` | f32 | Non-gradient modes. |
| `speed_min`, `speed_max` | f32 | `FilledGradient`: radial ramp, centre→edge. |
| `rotation_sense` | enum `CW` \| `CCW` | Tangential flow direction. |
| `diameter_km` | f32 | px or km input; stored km (§3.5). |
| `stamp_space` | enum `Geodesic` \| `Projected` | **Fixed at creation**, and set by the diameter's unit rather than by a control of its own — px paints `projected`, a circle on the map at any latitude; km paints `geodesic`, a constant-radius cap that appears stretched near the poles. The unit governs the ring width too: one space per object means one unit per tool (§3.5). It was once `circle_space`, with its own `ScreenCircular`/`GeodesicCircular` vocabulary; one question deserves one name (§6.1). |
| `feather` | f32 0–1 | |

Hover: yes — the disc a click would place, in the speed colour with its glyph.

#### Shape fill

Draw a polygon (click vertices, click the first one again to close), or place a
preset (square, rectangle, circle) **by dragging out from its centre**.

**All three presets are one press-drag-release gesture**, not three
interactions. They differ only in how the drag is read into a footprint, which
is what `shape_source` decides — so a preset cannot acquire an interaction of
its own by accident.

Centre-out rather than corner-to-corner so that all three presets have the same
anchor as the shape they produce — the pivot their handles turn them about
(§8.2). A
square takes the larger of the drag's two reaches, so a drag that is mostly
sideways produces the square it looks like it is producing.

A press and release **at one point** commits nothing: it describes a shape of no
size, and a press with a pixel of hand tremor describes an object the user
cannot see and did not ask for. The drag must exceed the same few pixels of
slack that tell a click from a pan (§8.1), measured on screen so that the same
hand movement means the same thing at any zoom.

A preset's size is **geometry, not a property**: it is part of what the user
drew, like a polygon's vertices, and it is resized afterwards by the same scale
handle every other object uses. The circle stamp is the other way round — its
diameter is typed, so it is an animatable property — and `Geometry::Disc`
carries an optional radius to say which of the two a given disc is.

| Option | Type | Notes |
|---|---|---|
| `shape_source` | enum `Polygon` \| `Square` \| `Rectangle` \| `Circle` | **Fixed at creation**: it decides which geometry the object *is*. |
| `stamp_space` | enum `Geodesic` \| `Projected` | **Fixed at creation**, and set by the tool's unit rather than by a control of its own. Applies to the dragged-out presets: px gives a shape that keeps its proportions on the map, km one that keeps them on the ground. The presets type no number, so the unit stands alone — a drag is still a measurement. A freehand polygon has no size at all, its vertices being placed geographically one by one, so the question does not arise, the property is inert for it, and the unit is not offered (§6.1). |
| `vector_mode` | enum `Constant` \| `Gradient` | |
| `speed` | f32 | `Constant`. |
| `speed_start`, `speed_end` | f32 | `Gradient`. |
| `direction_start`, `direction_end` | Angle | `Gradient`; shortest-arc across the ramp. |
| `gradient_axis` | Angle | Bearing along which the gradient ramps. |
| `direction_mode` | enum `Constant` \| `TowardPoint` \| `AwayFromPoint` | `vector_mode: Constant` only. The same three the brush has, meaning the same things (§6.1). |
| `direction` | Angle | Used when `direction_mode` is `Constant`. |
| `target` | LonLat | Used by both aimed modes. Placeable by pointing, on the tool and on the object (§6.1). |
| `feather` | f32 0–1 | |

Hover: **none**, deliberately. A polygon is built vertex by vertex and a preset
is dragged out, so there is nothing a *click* would produce to preview; the
in-progress gesture previews itself instead, like every other tool (§6.1).

#### Mask

Identical interaction to the brush; writes covered cells with speed 0. It is a
first-class object, not a deletion — it can be moved, animated, and disabled,
restoring what was underneath. **Called the mask** and not the eraser, because
that is what it is: an object that decides where the field beneath it shows,
which `invert` makes plain.

| Option | Type |
|---|---|
| `brush_shape` | enum `Circle` \| `Square` — **fixed at creation**, as on the brush |
| `size_km` | f32 (px or km input) |
| `stamp_space` | enum `Geodesic` \| `Projected` — **fixed at creation**; set by the size's unit, px selecting `projected`, as on the brush (§3.5) |
| `feather` | f32 0–1 — ramps *toward* 0 speed, i.e. blends back toward the underlying field's speed at the edge |
| `invert` | bool — cover everything **except** the footprint |

**`invert` turns the coverage inside out, cull included.** An inverted mask
covers the whole globe but its own footprint, which is how a field is confined
to a region rather than cut out of one: paint a global flow, draw a mask over
the basin you want, invert it, and nothing outside the basin remains. The two
sides are exact complements — at every cell the two weights sum to one, feather
included, so a mask and its inverse leave no seam along their shared edge.

The spherical-cap cull (§7.3) is what makes every other object cheap by
rejecting a distant cell before any SDF work; for an inverted object that same
test *accepts* the cell, at full weight and still without touching the SDF.
`invert` is therefore the mask's alone: it is safe on a tool that writes calm,
where there is no direction to compute, and it would not be on one whose
direction is measured from an anchor — a bearing to a point on the far side of
the globe is ill-conditioned, and near the antipode two backends cannot agree
on it (§7.9).

Overlapping mask strokes with identical options merge into one object, exactly
as brush strokes do (§6.1) — `invert` is one of the options that must match, so
a mask and its inverse never merge.

Hover: yes — the same footprint outline the brush shows, since it sweeps the
same stamp along the same kind of polyline. Like the clone stamp, and unlike
every tool that paints a field, it keeps that outline *through* the drag and
draws nothing else while masking (§6.1).

**A mask's edge is drawn on the map**, because a mask paints calm and is
otherwise invisible: there is nothing to say where one is, which side of it is
covered, or that a click landed on it.

- A **selected** mask has its footprint outlined.
- With a tool in hand that draws swept strokes — the brush, the mask, or a
  modifier (§6.3) — the edge of the object under the pointer is highlighted in
  **pink**, a colour used for nothing else on the map, so it cannot be read as a
  selection or a preview. The brush's strokes are visible already; what the
  highlight adds there is *which* stroke, and where it ends. Only the objects of
  the tool in hand are offered, so the work is bounded by what that tool has
  drawn rather than by the size of the project.

- **An edge is the outline of the whole footprint**, not of the stamps it is
  swept from, and it sits **on** the perimeter. A footprint is a *union*, and a
  stroked path is not a union: it traces every stamp's own circle and leaves a
  chain of rings along the stroke. The band is drawn by filling the footprint
  grown by half the band's width and knocking out a copy shrunk by the same,
  which is the union's boundary exactly — for a swept chain, a square stamp or
  a ring alike — and centred on it, where a stroke of that width would be.
- An **inverted** mask is drawn with a wide, faint band on top of its edge, since
  an outline alone cannot say which side is covered.

The outlines come from the same footprint the evaluator paints, so what is
highlighted is what is covered. Only what is visible is listed: a hidden layer's
masks are not on the map and are not offered to the pointer either.

**"Identical interaction to the brush" is binding.** Anything true of a brush
stroke's gesture, stamp, sizing, preview or transform is true here: the two
share their geometry (`Stroke`), their swept-stamp SDFs and their preview path,
and a difference between them is a bug in one of them rather than a choice.

#### Clone stamp

Paints by sampling the composite of everything **strictly below it in z-order**,
at a fixed geodesic offset. See §7.6 for the evaluation rule and recursion cap.
The GPU declines a scene containing one and routes it to the CPU (§7.8), so
this is the one tool whose evaluation has a single implementation.

| Option | Type | Notes |
|---|---|---|
| `brush_shape` | enum `Circle` \| `Square` | **Fixed at creation**, as on the brush. |
| `size_km` | f32 | px or km input. |
| `stamp_space` | enum `Geodesic` \| `Projected` | **Fixed at creation**; set by the size's unit, px selecting `projected` (§3.5). |
| `source_point` | LonLat | Placeable by pointing, on the tool and on the object, like every position option (§6.1) — "a dedicated pick action" is that shared affordance, not a bespoke one. |
| `offset_mode` | enum `Aligned` \| `Fixed` | `Aligned`: the source moves with the brush, preserving the offset the gesture began with, so a long stroke copies a correspondingly long band. `Fixed`: the source stays where it was put, so every stamp along the stroke reads the same neighbourhood of it and a long stroke repeats one patch. The displacement is measured from the object's anchor in the first case and from the nearest point of the stroke's own skeleton — the stamp centre for that cell — in the second. |
| `feather` | f32 0–1 | |

Hover: yes — footprint at the cursor, plus a crosshair at the current source
location, drawn the way every position marker is: a handle that can be dragged,
not a printed mark (§6.1).

**Two clone strokes never merge**, which is the one exception to §6.1's merging
rule. A merge re-expresses the absorbed stroke in the target's frame, and a
clone stamp samples at a displacement off its own anchor — a different anchor is
a different patch of the field. Absorbing one would change what it paints, which
is the thing a merge may never do.

#### Curve

A polyline or cubic-Bézier path with a vector field along it.

| Option | Type | Notes |
|---|---|---|
| `curve_kind` | enum `Polyline` \| `Bezier` | **Fixed at creation.** A node's handles are what make a segment a Bézier, so the kind is implied by the geometry — but it is also what the *tool* was set to when the path was drawn, which is what the option bar has to remember. |
| `stamp_space` | enum `Geodesic` \| `Projected` | **Fixed at creation**; set by the width's unit. The corridor has a width, so it asks the same question every sized tool asks (§3.5). |
| `speed` | f32 | |
| `direction_mode` | enum `Constant` \| `RelativeToPath` | **Its own property**, not the shared `direction_mode`: a curve aims along its path, where the shared modes aim at a point. A tool may add modes of its own; it may not redefine a shared one. `Constant` is named for the fixed bearing every other tool calls by that name — it was `Absolute`, which meant the same thing in different words and made two option bars read as though they described different things. The variant is renamed in place, not reordered: the stored value is the index. |
| `direction` | Angle | `Constant`: fixed bearing, a flow direction shown in the project's convention (§3.3). `RelativeToPath`: an *offset* added to the path's local tangent, so 0 = along the path and 90 = across it — an offset is not an azimuth and must not be converted. The two readings of one property are why it is displayed by mode, not by unit, and why the eyedropper (§6.1) is offered in `Constant` mode only. |
| `width_km` | f32 | Corridor half-width; px or km input, so `stamp_space` applies (§3.5). |
| `feather` | f32 0–1 | Across the corridor width. |

Hover: **none**, deliberately: a curve is built node by node, so a single click
produces no footprint to preview.

### 6.3 Field modifiers

Four tools that do not paint a field. Each reads the composite **beneath it in
z-order**, transforms it, and writes the result back inside its own footprint —
so what a modifier produces is always a function of what was already there.
Over calm water every one of them leaves calm water. That is the line between a
modifier and a creation tool, and it is what makes them safe to stack: a
modifier cannot invent a wind, only change one.

| Tool | Key | What it does |
|---|---|---|
| **Intensify / reduce** | `I` | Scales the speed by `1 + amount`, leaving the direction alone. `+100%` doubles it, `-100%` takes it to calm. |
| **Diverge / converge** | `D` | Adds a radial component of `amount × local speed`, outward from the anchor when positive and inward when negative. |
| **Rotate flow** | `R` | Turns every vector by a fixed angle, clockwise for a positive amount. The speed is untouched. |
| **Warp / liquify** | `W` | Reads the field from a displaced position: `push` drags it along a bearing, `twist` rotates it about the anchor. Nothing about the vectors changes, only where they are read from. |

All four are **painted**, like the brush and the mask: a stamp swept along a
polyline, sized in px or km (§3.5), with the common `position`, `scale_pct`,
`rotation_deg` and `enabled` (§4.4). A swathe of field is therefore intensified
or turned in one gesture, and **two strokes of one modifier with identical
settings and overlapping footprints merge into a single object**, exactly as two
brush strokes do (§6.1) — with the exception below. All four animate like
everything else: keyframe the amount and the field ramps; keyframe the position
and the modifier sweeps across the map. None carries `edge_mode`.

**A modifier measured from its own anchor never absorbs another.** A merge
re-expresses the new chain in the target's frame, and therefore under the
target's anchor; a tool whose field is measured from that anchor would paint
something different afterwards, which is the one thing a merge may never do
(§6.1). That is the diverge/converge tool, which radiates from its anchor, and
the warp, which both twists about it and pushes from it — the same rule that
keeps two clone strokes apart. Intensify and rotate refer to no anchor at all
and merge freely.

| Option | Type | Tool |
|---|---|---|
| `size_km` | f32 | all four; the swept stamp's diameter, px or km, so `stamp_space` applies as everywhere else |
| `feather` | f32 0–1 | all four |
| `gain` | f32 % (−100…400) | intensify / reduce |
| `radial` | f32 % (−400…400) | diverge / converge |
| `turn_deg` | f32 ° (−180…180) | rotate flow. A signed *amount*, not a bearing: it is not converted into the project's direction convention (§3.3), and animating it from −170 to 170 unwinds through zero rather than taking the short way round as a bearing would. |
| `warp_mode` | enum `Push` \| `Twist` | warp |
| `push_to` | LonLat | warp, `Push`: where the field under the anchor is dragged to |
| `twist_deg` | f32 ° (−360…360) | warp, `Twist` |

**The feather is the whole of the edge.** A modifier fades from what was there
to what it makes of it, by the same coverage weight every tool uses (§7.4), so
at the rim of its footprint the field is exactly what it was and a modifier has
no visible outline of its own. A warp's *displacement* is faded by the same
weight, which is what makes it a warp rather than a translation that tears the
field along its own edge.

**Direction is measured from the anchor**, for the one tool that needs a
direction at all: "outward" at a cell is the frame's radial bearing, the same
one the circle's rotation is a quarter turn off (§7.5). At the anchor itself
that bearing is undefined, exactly as a circle's tangent is; the value stays
finite and the cell is one cell.

**A warp is pulled, not typed.** Hold `Shift` with the warp tool and the
pointer stops painting: it grabs the warp under it and drags the field where it
should go. With no warp under it, `Shift` does nothing at all — it does not
paint. The modifier says "act on the warp that is there", and one that drew a
new object when it missed would be a way to paint one by accident in the middle
of aiming another. A warp therefore starts pushing *nowhere* — `push_to` is its own
anchor until it is pulled — and the option bar does not offer it at all, since
a push is measured from an anchor no gesture has placed yet. It is the one
property of any tool that cannot be set before the object exists. The object's own `position` is where the field comes from and
`push_to` is where it lands — two positions, so **both ends are keyframable**
like any other property (§9.3), and a warp that travels or grows over time is
two animated points and nothing else. A distance and a bearing said the same
thing in numbers nobody could aim; this is the gesture the tool is named for.
The pull previews as a line from the anchor to the pointer and writes once on
release, like every other drag (§8.2).

It follows that **a warp never merges**, in either mode: a merge re-expresses a
stroke under the target's anchor, and both a twist and a push are measured from
that anchor, so absorbing one would move what it turns about or push it
somewhere else.

**A cyclone is two modifiers over a stroke.** Converge bends the flow inward
and rotate turns it; the pair is the spiral §7.5 gave up, built from objects
that say what they do rather than from properties hidden inside every tool.

Hover: yes — the footprint outline at the cursor, and nothing inside it (§6.1).
There is no colour that stands for "the same wind, half as fast", so the honest
preview of a modifier is where it will land, and the field itself answers when
the stroke lands.

**A modifier's edge is drawn on the map**, on exactly the terms a mask's is
(§6.2): a selected one is outlined, and with any operator tool in hand the edge
under the pointer is highlighted in pink. A modifier is invisible for the same
reason a mask is — what it produces is what was already there, changed — so the
same affordance answers the same question.

---

## 7. Field evaluation

This is the heart of the application. It is a pure function:

```
evaluate(scene: &FlatScene, points: &[SamplePoint]) -> Vec<(f32 /*u*/, f32 /*v*/)>
```

The same function serves screen-tile preview (points = tile pixel centres) and
GRIB export (points = every global grid cell). There is no second code path for
export.

### 7.1 Scene flattening

For a given time step `t`:

1. Walk layers bottom→top, objects bottom→top.
2. Skip invisible layers, objects outside `active_range`, and objects whose
   `enabled` evaluates false at `t`.
3. For each surviving object, evaluate every animatable property at `t` to a
   concrete value.
4. Emit a `FlatObject` — resolved parameters plus geometry plus the AEQD frame
   — into a flat, ordered buffer.

The `FlatScene` buffer layout is shared verbatim between the CPU evaluator and
the WGSL shader. This is what makes parity tractable: both backends consume the
identical bytes.

### 7.2 Object-local AEQD frame

Each object defines an azimuthal-equidistant frame centred on its evaluated
`position`. Geometry is stored in this frame, in metres.

To test a grid cell against an object:

```
d = geodesic_distance(anchor, cell)                  // metres
θ = initial_bearing(anchor, cell)                    // degrees
x = d * sin(θ - rotation_deg) / (scale_pct / 100)
y = d * cos(θ - rotation_deg) / (scale_pct / 100)
```

Then `(x, y)` is tested against the object's local geometry.

This single construction gives, for free:

- Correct behaviour across the antimeridian and at both poles — no wrapping
  special cases, because distance and bearing are continuous there.
- Rotation as a true bearing rotation, not a distortion of lat/lon space.
- Scale in true metres, so a "500 km" brush is 500 km at any latitude.

#### The projected frame

An object whose `stamp_space` is `Projected` (§3.5) is defined on the map rather
than on the ground, so its frame is map space:

```
x = (normalize_lon(cell.lon - anchor.lon) · M_PER_DEGREE) / (scale_pct / 100)
y = ((cell.lat - anchor.lat)              · M_PER_DEGREE) / (scale_pct / 100)
        then rotated by rotation_deg, as above
```

`M_PER_DEGREE` is metres per degree of *latitude*, derived from
`EARTH_RADIUS_M`, so a local metre is north-equivalent: exact due north, and
worth `cos(lat)` metres of ground due east. A circle in this frame is therefore
a circle on an equirectangular map at every latitude, which is the whole point,
and an ellipse on the ground.

Everything downstream is unchanged. The SDFs (§7.3) are the same functions on
the same numbers; feather is a fraction of the shape's own extent (§7.4), so it
is scale-free; and the cull radius still bounds the reach, because a projected
metre is never *more* than a ground metre — the bound errs conservatively, which
is the only direction a cull may err in. Directions never come from the frame in
either space (above), so a circle's rotation and every aiming mode are
untouched.

The antimeridian and the poles stay ordinary here too, for a different reason:
the longitude delta is normalised, and nothing divides by `cos(lat)`, so there
is no singularity to approach. Latitude clamps at ±90 rather than wrapping over
the pole, which is what the shape looks like on the map.

### 7.3 Signed distance and coverage

Every geometry kind implements a signed distance function in its local frame,
returning metres (negative inside):

| Geometry | SDF |
|---|---|
| Capsule chains (round brush, mask, clone) | min distance to any segment of any chain, minus radius |
| Swept square (square brush) | min **Chebyshev** distance to any segment of any chain, minus half the side |
| Disc / annulus (circle) | \|p\| − r, or \|\|p\| − r\| − w/2 |
| Polygon (shape fill) | winding-number sign × min edge distance |
| Corridor (curve) | distance to path − half-width |

**The square brush is measured in the Chebyshev metric**, whose unit ball is
the stamp. The footprint of a square dragged along a path is exactly the set of
points within half a side of it in that metric, so coverage is exact rather
than a polygon approximation of a square that the two backends would have to
agree on independently. Inside the footprint the value is also the true
distance to the boundary — the inward offset of a square is a smaller square —
which is what the feather band needs (§7.4). Outside, it under-reports along
the diagonals, and nothing reads it there: coverage stops at zero, and the cull
uses the corner radius `half_side · √2` rather than the edge.

The per-segment minimum is exact rather than searched for. Chebyshev distance
to a segment is the minimum of a convex piecewise-linear function of the
parameter, so it is attained at one of the two ends or at one of four
crossings; both backends evaluate the same six candidates and land on the same
number.

**Coverage is binary:** `sdf <= 0`. Feather does not change coverage — it
modulates speed inside the covered region (§7.4). This is what makes
"overwrite" semantics well-defined.

A capsule holds **several** polylines, not one. A merged stroke (§6.1) is
several gestures in one object, and concatenating them into a single polyline
would sweep the brush along the segment joining one gesture's end to the next
one's start — painting a line nobody drew. Taking the minimum across chains
instead leaves the gaps uncovered. The distance between two chain sets, which
is what decides whether two strokes overlap, is measured segment-to-segment
rather than vertex-to-vertex: two long strokes can cross in the middle of a
span with every vertex far from the crossing.

Both backends must agree on this. The GPU uploads a swept shape — round or
square — as independent *segment pairs* rather than as polylines, so the shader
needs no chain boundaries: it walks pairs and takes the nearest, and a
single-point chain is a degenerate pair the segment distance already handles.
The two shapes share that layout and differ only in the metric the shader
measures the pairs in.

**Culling:** each `FlatObject` carries a bounding radius (max local geometry
extent × scale). A spherical cap test rejects cells before any SDF work. Caps
are converted to lat/lon row ranges for the full-grid pass, with correct pole
and antimeridian clamping.

### 7.4 Feather

Feather produces a weight `w` over the covered region, ramping from 0 at the
object's edge to 1 inside it:

```
w = smoothstep(0, feather * R, -sdf)     // R = object's characteristic radius
```

Every object carries an `edge_mode` property governing how `w` is applied:

| Mode | Rule | Effect |
|---|---|---|
| **`Blend`** (default) | `out = lerp(under, obj, w)` | The edge fades into whatever is beneath it. |
| `Replace` | `out = obj_with_speed_scaled_by(w)` | The edge fades toward calm, always overwriting. |

**Why `Blend` is the default.** When the underlying field is calm the two modes
are mathematically identical — `lerp((0,0), V, w)` equals `V·w`. They diverge
only when painting over existing data, and there `Replace` produces a ring of
near-calm between the new object and the surrounding flow, which is essentially
never wanted. Feather is also opt-in: at `feather = 0` both modes are a hard
overwrite. So the default governs exactly one situation — a soft edge over
existing data — and `Blend` is the right answer there. `Replace` remains
available per object for when an exact, uncontaminated footprint is needed.

Coverage is unaffected by either mode: `w` modulates the *value* written, never
whether a cell is written.

**Blending interpolates `u` and `v` components**, not speed and angle
separately. Component lerp is the physically sensible superposition of two
flows. Its one consequence is that where two roughly opposing flows meet, the
blend passes through low speed — which is a correct depiction of a convergence
zone, not an artefact.

The mask is a natural consequence of this model rather than a special case: it
is an object whose speed is 0, so its feathered edge blends the underlying field
back toward calm.

### 7.5 No tool shapes its flow with divergence or curl

Earlier versions gave the tools that place a centre a radial and a tangential
component, added to the base direction as multiples of the speed. Nothing has
them now. **An object's direction mode is the whole of its direction.**

This still holds with the modifiers of §6.3 in the catalogue, and it is worth
saying why they are not the same thing coming back. What was removed was a
*property* on every tool that changed a vector after that object's own
direction mode had produced it, measured from that object's anchor — invisible
in the panel unless you knew to look, and impossible to reason about across a
stack. A modifier is an object: it has its own place, size, feather and z, it
says in its name what it does, and it acts on the composite rather than on one
object's own output. What it costs is that an anchor-relative term is back in
the evaluator, and with it some of the `f32` disagreement between the kernels
that removing the properties bought — bounded now to the modifier's own
footprint, and measured in the fidelity suite like everything else.

What this costs is the spiral. A radial component is what makes a low converge
rather than merely turn, so a cyclone painted with the circle tool is a pure
rotation and the inflow has to be built from other objects — a second, larger
circle aimed at the centre, or a shape fill on `toward_point`. The aimed
direction modes reach the same fields; they take more than one object to do it.

What it buys is that a direction is decided in exactly one place. The components
were the last thing that could modify a vector after its mode had produced it,
and they were the last thing in the evaluator whose result depended on the
object's *anchor* rather than on its geometry — which is why removing them
tightened GPU–CPU agreement by two orders of magnitude, from 0.108 m/s to 0.0009.

The circle tool's `rotation_sense` was never sugar over curl and is unaffected.
Curl *added* to a base direction, and the circle has no direction property of
its own, so a curl-based rotation defaulted that base to north and produced a
circle that drifted northward while it turned. Rotation is its own direction
mode — flow tangential to the anchor at the object's `speed` — and now the only
thing a circle does.

Removing a property from a tool is a migration, never only a table edit: a
`PropertyMap` returns whatever it holds and falls back to the schema only when
the key is absent, so a leftover entry goes on bending a flow that no panel
shows and no tool defines (§4.7, schema version 7).

### 7.6 Compositing

Iterate the flat scene in z-order, writing into an accumulation buffer:

```
for obj in scene:                     // bottom → top
    for cell in obj.cap_cells:
        if obj.sdf(cell) <= 0:
            buffer[cell] = obj.vector_at(cell)   // OVERWRITE
```

Cells never touched by any object are calm — `(0, 0)`.

**A modifier reads the buffer and writes it back** (§6.3):

```
for obj in scene:                     // bottom → top
    if obj.is_modifier:
        for cell in obj.cap_cells:
            if obj.sdf(cell) <= 0:
                w = coverage_and_feather(obj, cell)
                buffer[cell] = lerp(buffer[cell], obj.transform(buffer[cell]), w)
```

Because objects are processed in z-order, the buffer at that moment holds
exactly "everything below this object" — the same guarantee the clone stamp
relies on, and the reason a modifier needs no sub-scene of its own. The one
exception is the **warp**, whose read is at another position and which
therefore takes the clone stamp's path below, sharing its depth cap.

**Clone stamp** is the one creation tool that reads the buffer:

- It samples the buffer at the offset source location. Because objects are
  processed in z-order, the buffer at that moment contains exactly "everything
  below this object," which is the specified semantic.
- Source samples may fall outside the tile currently being rendered. The
  evaluator handles this by running a **nested evaluation of the sub-scene**
  (objects strictly below the clone stamp) at the required source points.
- **Recursion is capped at depth 4.** A clone stamp sampling a region containing
  other clone stamps — or warps — beyond that depth reads calm. This is a performance guard,
  not a correctness one — the z-order dependency graph is acyclic by
  construction.
- Nested evaluation is the main performance hazard in the engine. It is
  measured explicitly in the M3 benchmark.

### 7.7 Render tiles

Preview rendering is tiled, in an equirectangular pyramid: `z=0` is 2×1 tiles of
256², covering 360°×180°; each level doubles.

Tiles are served to the webview through a Tauri custom URI scheme, not through
IPC message passing — raw bytes, no JSON, no base64:

```
ve-tile://<frame_hash>/<z>/<x>/<y>
```

Payload is RGBA8, 256×256, 256 KB per tile:

| Channels | Content |
|---|---|
| R,G | speed as u16 LE, scaled by the project's display max |
| B,A | azimuth-toward as u16 LE, `az/360 * 65535` |

Because the frame hash is in the URL, the browser's own cache, our disk cache,
and invalidation all key off the same identity, and stale tiles remain
addressable while new ones render.

**The preview may evaluate coarsely and interpolate.** A tile need not evaluate
the field at every one of its 65,536 pixels: it may sample a coarser lattice and
interpolate between samples, and it may drop to a coarser lattice still while
the camera is moving. This is sound precisely because the view is a proxy
(invariant 3) — the only requirement is that it looks right. The export path has
no such licence and always evaluates every grid cell exactly once.

**Tiles must be sampled with nearest-neighbour filtering.** Both values are
16-bit quantities split across byte pairs, so hardware bilinear filtering would
interpolate the high and low bytes independently and yield meaningless numbers.
The renderer smooths the speed raster itself, by decoding four neighbouring
texels and interpolating the decoded speeds. Direction is point-sampled, since
averaging bearings across the 0/360 wrap is wrong anyway.

### 7.8 Backends

Two implementations of one trait:

```rust
trait FieldEvaluator {
    fn evaluate(&self, scene: &FlatScene, points: &[SamplePoint]) -> Vec<Vec2>;
}
```

| Backend | Used for | Rationale |
|---|---|---|
| **`GpuEvaluator`** (wgpu compute, WGSL) | Preview tiles, playback prerender | Fast enough for interactive painting with deep object stacks, and free to approximate (§7.9). |
| **`CpuEvaluator`** (rayon) | **GRIB export**, parity reference, headless tests, CI | Bit-reproducible across machines. Invariant 4. |

**Why export uses the CPU path.** GPU floating-point results are not
bit-identical across vendors and drivers — FMA contraction and transcendental
precision differ legitimately. Since the exported file's bytes are the product,
and export is explicitly permitted to be slow, the CPU kernel is authoritative
for export. A `fast_export` setting (default **off**) allows GPU export, with a
persistent warning that output bytes become machine-dependent.

**Backend selection and fallback.** At startup, request a wgpu adapter with the
required compute limits. Fall back to `CpuEvaluator` for preview when: no
adapter is available; the adapter lacks required limits; device creation fails;
or `VE_FORCE_CPU=1` is set. The active backend and the reason for any fallback
are shown in About and written to the log. wgpu covers Metal (Intel and Apple
Silicon Macs), DX12 and Vulkan (Windows), and Vulkan (Linux); the fallback
covers machines with none of those, plus CI and headless runs.

**A scene may also be declined for what is in it**, and falls back the same
way. Two things need recursion a compute shader has not got: the clone stamp,
and the warp modifier (§6.3), both of which read the composite at a position
other than the cell being written. The other three modifiers transform the
vector where it already is and stay on the GPU. Declining is per scene and not
per object: a preview that quietly dropped the one object it could not render
would be a proxy for a scene the user does not have.

### 7.9 Preview fidelity

The preview and the export are produced by the same object model but not by the
same numbers, and they are not required to match numerically.

| Path | Requirement |
|---|---|
| Export (CPU) | Authoritative. Byte-reproducible across machines (invariant 4). |
| Preview (GPU, or CPU fallback) | Must look like the export. Speed within the greater of 0.25 m/s and 2% of the value; direction within 2°. |

**Why this is loose on purpose.** An earlier draft required the two backends to
agree within 1e-3 m/s. That is far below anything visible — the tile encoding
already quantises direction to 0.0055° and a 2° bearing error moves an arrow
tip by a fraction of a pixel — and it imposed a real cost: the WGSL kernel would
have had to reproduce the CPU's exact trigonometry, forbidding fast approximate
`atan2`, reduced precision, and coarse-lattice evaluation. Since no preview
pixel can ever reach an exported file (invariant 3), that precision bought
nothing. The tolerance is now set where a difference would actually be *seen*.

What still has to hold, and is tested:

- Both backends produce the same *structures*: a cyclone rotates the same way,
  bands sit at the same latitudes, an object's footprint has the same extent.
- Direction is right to within 2°, so glyphs never mislead.
- Neither backend produces a non-finite value anywhere on the globe.

### 7.10 Render cache

- **Key:** BLAKE3 of the canonicalised `FlatScene` for a time step, plus grid
  parameters, plus tile coordinates. Any edit that changes what a frame looks
  like changes the hash; any edit that does not (renaming a layer, moving the
  camera) does not. Invalidation is therefore automatic and exact — there is no
  hand-maintained dirty-tracking to get wrong.
- **Location:** OS cache directory, never inside the project file.
- **Eviction:** LRU with a configurable disk cap, default 4 GB.
- **Safety:** deleting the cache directory while the app is closed is always
  safe and always lossless.

---

## 8. Editing behaviour

### 8.1 Tools palette

Left-hand vertical palette, keyboard-shortcut per tool:

| | Tool | Key |
|---|---|---|
| ✋ | Hand / select — pan the map, click objects to select | `V` |
| 🖌 | Brush | `P` |
| ⭕ | Circle stamp | `C` |
| ⬟ | Shape fill | `F` |
| ⧉ | Clone stamp | `S` |
| 〰 | Curve | `B` |
| ⌫ | Mask | `E` |
| ⊕ | Intensify / reduce | `I` |
| ✳ | Diverge / converge | `D` |
| ↻ | Rotate flow | `R` |
| ≈ | Warp / liquify | `W` |
| 📏 | Measure (dividers / great circle / range rings) | `M` |

`P` for the brush and `B` for the curve follow the conventions of other paint
applications — the brush is the *pen* tool and the curve the *Bézier* — rather
than the tools' initials, so hands that already know those keys need not
relearn them. The four modifiers (§6.3) take their initials, which were free.
The mask keeps `E`, the eraser's key: `M` belongs to the measure tool, and a
rename is not a reason to move a key out from under the hands that know it.

The palette is in three groups: the tools that lay a field down, then the
mask, which takes one away, then the modifiers, which change one. The order
is not decoration — neither the mask nor a modifier does anything until there
is something under it.

Each button carries **a mark rather than a word**: seven names across the top of
the map is a row of text where a row of shapes is quicker to find, and the
option bar beside it needs the room. The marks are inline SVG drawn in the
frontend — invariant 5 rules out an icon font or a sprite fetched at runtime,
and a bundled icon package would be a dependency carried for seven glyphs. They
are stroked in the button's own colour, so the active state reaches them without
the set knowing anything about the theme.

**The name is unwritten, not gone.** Each button keeps it as its accessible
label and, with the shortcut, in its tooltip. Replacing a word with a picture
must not take the word away from anyone reading the page with something other
than their eyes.

The hand tool is the prominent default and the one the app returns to on
`Escape`. In it, dragging empty map pans; dragging a *selected* object moves it;
dragging a handle rotates, scales, or repositions the anchor. Whether a press
landed on a selected object takes a hit test, and a hit test is a round trip, so
panning begins immediately and converts to a move if the answer arrives before
the pointer has actually travelled — waiting for it would put IPC latency in
front of every pan.

### 8.2 Selection

- Click to select, `Cmd`/`Ctrl`-click to add or remove from the selection.
- **`Shift`-drag on empty map** for a rubber band; add `Cmd`/`Ctrl` to reach
  across layers. The modifier is what reconciles this with §8.1: a plain drag on
  empty map has to keep panning, so the band needs one of its own. A band adds
  to the selection rather than replacing it.
- Selection is scoped to the **active layer** — the last layer or object clicked
  in the layer panel — and the cross-layer modifier lifts that. The active layer
  is also the one new objects join (§6.1).
- A marquee catches an object when its footprint *meets* the rectangle, not only
  when it is wholly inside. The test is the object's bounding cap against the
  rectangle, which errs towards selecting: a band that misses what it visibly
  enclosed is worse than one that catches a near miss.
- Multi-selection transforms operate about the **collective centroid**, averaged
  as unit vectors rather than as degrees — the mean of 179° and -179° is 180°,
  and that is where the objects are.
- Selected objects draw handles: the body moves, a rotate handle, a scale
  handle, and the anchor point at the centre (draggable independently, so you
  can change the pivot). Moving the anchor re-expresses the geometry in the new
  frame, so the shape stays exactly where it was painted; for a disc or a
  rectangle, which *are* centred on the anchor, there is nothing to leave behind
  and the object moves with it. A multi-selection has no anchor of its own, so
  its centre handle moves the group.
- **A group transform is one rigid operation, not a per-object edit.** A move is
  a rotation of the sphere carrying the pivot to the pointer; a rotate turns
  every member about the pivot *and* adds the same angle to each member's own
  rotation; a scale multiplies both each member's distance from the pivot and
  its own scale. Distances within the selection survive all three. Offsetting
  each anchor by a fixed number of degrees would not: near a pole the same
  offset is a different distance for every member, and the group shears.
- **A drag is computed from a baseline captured at pointer-down**, held by the
  backend for the life of the gesture. The pointer reports an absolute position
  many times a second and each report must give the same answer as if it were
  the first; a delta applied to a value that already contains it is how a drag
  runs away. It also makes the whole drag one `Command::Batch`, so one undo
  returns every member.
- **A drag writes nothing until the pointer comes up.** What follows the pointer
  is a *preview*: the backend answers "where would this leave the selection"
  without touching the document, and the map draws that outline over the field.

  Applying the drag on every report instead is what makes a drag unusable. Every
  write bumps the revision, the revision addresses every tile (§4.7), so each
  report invalidates the whole visible field and asks for a re-render costing
  tens to hundreds of milliseconds (§13). The renders never finish before the
  next one replaces them, and the object appears to move only once the drag
  ends. A preview evaluates nothing and re-renders nothing, so the outline keeps
  up with the pointer; the field catches up once, on release.

  **The preview and the write are computed from the same placement**, so what is
  drawn under the pointer is where the object lands — not a second calculation
  of it that could drift. As with a painted stroke (§6.1), the preview is held
  after the release until the new revision's tiles are on screen, so the object
  does not snap back to where it started while they render.

  The field itself is not previewed. A drag shows the footprint moving over a
  field that has not moved yet, which is the same bargain §1.5's third invariant
  makes everywhere: the view is a proxy, and evaluating one at pointer rate is
  the thing that cannot be afforded.
- **Selection changes are not undo history entries** (standard editor
  convention).

### 8.3 Objects are pinned to the earth

During pan and zoom, every object stays locked to its geographic position and
scale — the map moves under them. Objects change position, rotation, or scale
only through explicit user manipulation or keyframe animation. This falls out of
§3.5 (px resolved to km at creation) and §7.2 (geometry in metres in a geodesic
frame).

### 8.4 History

Command-pattern undo/redo with explicit inverses.

- Default depth 200 entries, configurable.
- **Coalescing:** a continuous drag, a slider scrub, or a text-field edit
  collapses into a single entry on commit.
- One shared history per project (not per layer), so undo order matches what the
  user did.
- Everything mutating is undoable: object creation, edits, deletion, layer
  reorder/rename/visibility, keyframe add/move/delete/retype, active-range
  changes.
- The history panel lists entries and supports jumping to any point.

### 8.5 Copy and paste

The clipboard holds fully serialised objects, including all keyframes.

- Copy/cut/paste/duplicate work within a layer, between layers, between time
  steps, and between open projects.
- **Keyframe remapping on paste:** default is *relative* — keyframes are shifted
  so the copied object's `active_range` starts at the paste-time step. An
  "Paste at original times" alternative preserves absolute step numbers.
- Pasting into the same location offsets by a small delta so the copy is
  visible.
- Object `Id`s are always regenerated on paste; names get a `copy` suffix.

---

## 9. Timeline and animation

### 9.1 Layout

A bottom dock, resizable, with two coordinated regions:

- **Ruler:** one tick per time step, labelled with the forecast hour and, once a
  start time is set, the absolute UTC time. Click or scrub to change the current
  step.
- **Tracks:** a collapsible tree — layers, then objects, then one row per
  animatable property, with keyframe diamonds on each.

### 9.2 Object bars

Each object row shows a bar spanning its `active_range`, draggable at either end
to change start and end. Layers have **no** bars — the range is an object-level
concept per the requirements.

**A GRIB layer's row marks the steps its file has a message for** (§4.8). A step
the file says nothing about shows no imported field at all, so without this the
user is left to infer which times a file covers from a field that appears and
disappears. It is drawn on the layer's own row because it is a property of the
layer and not of any object in it — and it is the one thing a layer row shows,
which is why it is not a bar.

### 9.3 Keyframe editing

- Add a key at the current step by changing a property while auto-key is on, or
  explicitly via the diamond button beside any property in the inspector.
- **Where a change goes.** A change to a property that has no keys edits its
  base, unless auto-key is on. A change to a property that already has keys
  always keys the current step — auto-key or not — because its base is
  unreachable once a key exists (the nearest key holds outside the keyed range,
  §4.5), so writing it would change nothing visible. The map's move, rotate,
  scale and anchor drags are property changes and follow the same rule; a drag
  at step 6 of an object keyed at 2 and 10 adds a key at 6 and leaves the other
  two alone. An edit at a keyed step replaces that key's value and keeps its
  interpolation; a new key takes the kind's default (linear where allowed).
- Select, move, delete, and box-select keys. Move constrained to integer steps.
- Right-click a key to set the interpolation of the segment leaving it.
- Properties with no keys show `base` and no diamonds.
- A property row indicates when the current step's value is interpolated rather
  than keyed.
- **The steps a segment animates through are dotted.** Between two keys, every
  step whose value is interpolated carries a small dot on the track, so a
  segment that moves is distinguishable at a glance from one that does not. A
  holding key blends nothing until the next one and its segment has no dots;
  neither do the held steps outside the first and last key (§4.5).
- **A track opens into a value graph.** A property with a magnitude — a scalar,
  an angle, a position — has a disclosure beside its name that opens a graph of
  its value at every step, drawn under the track and sharing the ruler's
  horizontal scale, with its keys marked and the current step read out. A
  boolean or a choice has no graph: it holds rather than blends, and a line
  through discriminants would say nothing the diamonds do not.

  The samples are the **model's**, taken through the same evaluation the field
  uses (`track_samples`), never a second interpolation of the same keys in the
  frontend: the easing curves, the shortest-arc angle and the great-circle
  position live in one place. What the frontend does is convert — a speed is
  sampled in m/s and graphed in knots, a flow direction is graphed in the
  project's convention (§3.3) — and unwrap a degree series so a turn through
  the 360° seam draws as the short arc it is rather than a full-height fall. A
  position graphs as two series, longitude and latitude, on a shared degree
  axis.

**Motion in the field.** The `position`, `rotation` and `scale` rows of a
tool that paints a vector each carry a **motion** switch. While it is on, the
object's own movement between steps is added to the vector it paints: a stroke
travelling east at 15 m/s adds 15 m/s eastward, so a tailwind strengthens, a
headwind weakens and a crosswind turns. One vector addition per cell, in the
cell's own east/north frame, which is what makes all three the same rule and
the sum right at every relative angle.

One switch per **track**, not per object: a rotating system that also travels
may want its spin in the wind and not its translation, and one switch cannot
say which. A modifier and a mask have no switch at all — a modifier writes what
it read and a mask writes calm, so neither has a field of its own for a
velocity to be added to (§6.3).

**The velocity comes from the track the field already uses.** At step `k` it is
the central difference of the property's own sampled value at `k−1` and `k+1`,
one-sided at the ends, over the elapsed time — never a second interpolation of
the keys. A held segment contributes nothing: a value that jumps is a teleport,
and a 500 km jump in an hour is not a 140 m/s wind. A property with no keys has
no motion.

**A translation and a rotation are each one angular velocity.** A position
segment is a great-circle slerp — a rotation of the sphere — so the translation
velocity at every cell of a footprint is exactly `Ω × p` for one 3-vector, and
a turn about the anchor is `ω` about the anchor's own unit vector. The two add
into one vector per object, and a cell's velocity is then a cross product and a
projection onto east and north: exact everywhere, at the poles and across the
seam alike, where "the anchor's speed and bearing applied uniformly" would be
wrong by the frame's own drift a few thousand kilometres out (§7.2). Scale adds
a radial velocity of `ṡ/s · r` along the frame's radial bearing.

Both kernels add it to the object's own vector **before** the feather and the
edge mode, so the edge fades the sum; a moving clone stamp adds it to what it
copied. The velocity is part of the flat object, so the render cache keys on it
by construction (§7.10). The gesture preview shows no motion — a fresh stroke
has no keys — and the tiles do.

**Objects that follow objects.** `Cmd`-drag from one object's `position` or
`rotation_deg` row onto the same row of another, and the first **follows** the
second. Those two properties only: a speed that followed another object's speed
would be a different feature, and no other property is a place in the world for
an offset to be kept in. A drop on another property, on any other row, or on
itself is refused. The row then shows a link glyph and the primary's name;
clicking it unlinks.

**A follower is a rigid part of its primary.** At link time the offset is read
from where the two objects already are — so linking never moves anything — and
a position link stores it as a distance and a bearing **measured against the
primary's own rotation**. At every step the follower's anchor is the primary's,
moved that distance along that bearing turned by the primary's rotation, so a
follower orbits a turning primary rather than sliding beside it. A rotation
link keeps the difference of the two rotations. The primary's motion therefore
propagates: a follower with **motion** on takes its velocity from the value it
derives, not from the keys it is ignoring.

The follower's own keys are **kept but dormant**, greyed on the track.
Unlinking wakes them and, so nothing jumps, writes the derived value at the
current step under §4.5's rule — a base write would be invisible once a
property has keys, and those are exactly what unlinking brings back. Deleting
the primary frees its followers the same way, in the same history entry, so one
undo puts the whole arrangement back; a follower copied without its primary
stands alone, and a pair copied together keeps its link between the copies.

**A cycle is refused when it is made.** A loop has no meaning — every object in
it would be defined by the others — and a user who made one by accident would
see a set of objects stop responding with nothing to point at. Chains resolve
in dependency order, once per step for the whole project, so the field, the
map's outlines and hit testing all place a follower in the same place. A link
to an object that is no longer there is inert rather than fatal: the property
falls back to the follower's own keys.

### 9.4 Playback

- Play, pause, stop, and loop. Playback advances one **time step** per display
  frame at a user-set rate (default 8 steps/s), not real time.
- Playback only advances into frames that are ready; if the next frame is not
  cached, playback holds and shows a buffering state rather than stuttering.
  "Ready" means **solid** in §9.5's terms — a stale frame is not ready, since
  the tiles on screen for it belong to a revision that no longer exists — *and*
  resident: every tile of the step is on the GPU, by the map's own account.
  The backend holding a tile rendered is not enough; advancing on that alone
  draws the previous step under the new one for a frame, which is the flicker.
  While playing, the map keeps the next two steps' tiles fetched ahead of the
  playhead.
- **A frame that is not yet on screen draws its missing tiles from the last
  frame that was, dimmed.** A step change or an edit re-addresses every tile;
  blanking the map until the new ones land is a flicker on every scrub and a
  flash on every stroke. The dim says the tile belongs to another frame.
  Playback never shows a dimmed tile, by the rule above.
- A stall of several intervals — a slow render, a suspended window — resumes
  at the *next* step. Advancing as many steps as the clock says would turn a
  long render into a skip.
- `Space` plays and pauses; `←` and `→` step one time step back and forward,
  stopping playback first. The arrows **clamp** at the ends where playback
  loops: an arrow is for reaching a particular time, and wrapping round to the
  other end of the timeline is a jump nobody asked for.

### 9.5 Background rendering and readiness

A worker pool renders frames ahead of the playhead.

- Priority: current step → steps adjacent to the playhead → the rest of the
  range, in order.
- Work unit is a tile, at the current zoom, for the current viewport.
- The ruler shows per-step readiness: **empty** (not rendered), **partial**
  (rendering, with progress), **solid** (all viewport tiles cached), and a
  distinct **stale** state when an edit has invalidated a previously-ready
  frame.
- Editing cancels in-flight work for frames whose hash changed and requeues.
  Because keys are content hashes (§7.10), frames unaffected by an edit stay
  ready — editing an object with a narrow `active_range` does not invalidate the
  whole timeline.
- Changing zoom or panning far requeues, but previously rendered tiles at other
  zooms remain cached.
- **The pool renders through the same function that serves the map's own tile
  requests.** The key, the backend, the quality and the encoding are decided
  once, so a tile rendered ahead is byte for byte the tile the map will fetch,
  and "ready" means "will be a cache hit". Readiness is a probe of the cache
  index by content hash; it reads no tile and reorders no eviction.
- Nothing is cancelled. A unit in flight when an edit lands finishes into a key
  that is merely unreachable — one tile of wasted work, and no bookkeeping that
  could be wrong.
- **A tile is evaluated once.** The map's own request for a tile and the pool
  working ahead on the same step meet in the cache: a request for a key already
  being rendered waits for that render and reads what it stored. No frame is
  rendered again unless its content hash changes.
- Each step is flattened, hashed and planned once per document revision, not
  once per tile or per readiness probe.
- **Stale** is the frontend's memory: the backend reports what the cache holds,
  and "was solid at an earlier revision, is not at this one" is the timeline
  remembering. It clears when the frame is solid again.
- Steps that look the same share their tiles: a still scene is one frame,
  rendered once, and solid at every step at once.

---

## 10. Measurement and annotation tools

These are overlays. They never contribute to the field and never appear in the
GRIB output. They are saved in `Project.annotations` so a measurement survives
save/reopen.

| Tool | Behaviour |
|---|---|
| **Dividers** | Click a chain of points. Shows each segment's great-circle distance and initial bearing, plus a running total. Points are draggable. km and nm shown together. |
| **Great circle / rhumb line** | Pick two points; draws both paths simultaneously in distinguishable styles, labelled with each distance and the constant rhumb bearing. Individually clearable. |
| **Range rings** | Pick a centre; draws N geodesic circles at a set interval, labelled. Interval, count, and centre are editable. Clearable. |

Each has an explicit clear action, and there is a global "clear all
measurements."

---

## 11. Sailboat route definition — *withdrawn*

This section specified a sailboat route feature: bundled boat polars, a
route tree drawn on the map, and a solver that inverted the polar to find
the wind each leg needed. It was withdrawn before any of it was built.

The app makes forecast fields; what a routing engine then does with one is
that engine's business, and building a solver here meant owning a polar
format, an under-determined inversion, and a merge-on-re-solve model for
objects the user could also edit by hand — a second product inside the
first.

**The number is retired rather than reused.** Thirty-odd references to §12
through §15 are spread through the code's doc comments, and renumbering to
close a gap would risk pointing several of them somewhere wrong for no
gain. There is no §11.

## 12. GRIB2 export

### 12.1 Output

A single `.grib2` file containing concatenated messages: **one message per
component per time step**, ordered by time step then u, v.

Parameters by field kind:

| Field kind | Discipline | Category | u | v | Level |
|---|---|---|---|---|---|
| Wind | 0 (meteorological) | 2 (momentum) | 2 (UGRD) | 3 (VGRD) | Type 103, 10 m above ground |
| Current | 10 (oceanographic) | 1 (currents) | 2 (UOGRD) | 3 (VOGRD) | Type 160, depth 0 m below sea surface |

Units are m/s in both cases.

### 12.2 Writer design

A hand-written Rust encoder in `ve-grib`. It builds each message from a template
of fixed section bytes, patching only what varies: the reference time, the
forecast time offset, the parameter number, the packing header, and the data
array. This is what the requirements ask for, and it keeps the encoder small and
auditable.

Section layout per message:

| Section | Contents | Length |
|---|---|---|
| 0 | `GRIB`, discipline, edition 2, total length | 16 |
| 1 | Identification: centre, tables versions, reference time, production status, type of data | 21 |
| 2 | Local use — **omitted** | — |
| 3 | Grid definition, template 3.0 (regular lat/lon) | 72 |
| 4 | Product definition, template 4.0 (analysis/forecast at a horizontal level) | 34 |
| 5 | Data representation, template 5.0 (simple packing) | 21 |
| 6 | Bitmap indicator 255 (no bitmap) | 6 |
| 7 | Packed data | 5 + ⌈N·bits/8⌉ |
| 8 | `7777` | 4 |

Key field values:

- **Shape of earth** = 6 (spherical, radius 6,371,229 m); radius scale factor
  and value are set missing, as the code implies the radius. Consistent with
  §3.1.
- **Basic angle** = 0, subdivisions = missing ⇒ coordinates in units of 10⁻⁶
  degrees.
- `La1` = 90 000 000, `Lo1` = 0, `La2` = −90 000 000, `Lo2` = 360 000 000 − Di.
- **Resolution and component flags** = `0x30` — i and j increments given, u/v
  resolved relative to easterly/northerly directions (earth-relative, not
  grid-relative). This matters: consumers will otherwise rotate the vectors.
- **Scanning mode** = `0x00` — +i (W→E), −j (N→S), i consecutive.
- `Di`, `Dj` in micro-degrees: 1 000 000 / 500 000 / 250 000 / 100 000.
- **Significance of reference time** = 1 (start of forecast); **type of data** =
  1 (forecast).
- **Indicator of unit of time range** = 1 (hour); **forecast time** =
  `step_index × step_hours`.
- **Centre** defaults to 255 (missing) and is configurable, since some consumers
  treat unknown centres poorly; setting it to 7 (NCEP) is offered as a
  compatibility option and clearly labelled as a fiction.

### 12.3 Simple packing (template 5.0)

```
Y * 10^D = R + X * 2^E
```

For each field: `D = 0`; `R = min(Y)`; `bits = 16`;
`E = ceil(log2((max - min) / (2^bits - 1)))`, clamped to ≥ a floor that avoids
denormal blow-up; `X = round((Y * 10^D - R) / 2^E)`.

16 bits gives roughly 0.001 m/s resolution over a ±60 m/s range — far finer than
anything the tools can express. A constant field (max == min) is encoded with
`bits = 0` and no data octets, which is legal and compact.

### 12.4 Export flow

1. User invokes export and supplies: **forecast start time (UTC)**, output
   **file name**, and **output location**. Optionally centre code and the
   `fast_export` toggle.
2. A pre-flight panel shows the estimated file size (`Ni × Nj × 2 bytes × 2
   fields × step_count`, plus headers) and the estimated duration. At 0.1° with
   120 steps this is ≈ 6.2 GB, which the user should see before starting.
3. Export runs on a background worker, streaming to disk one message at a time —
   the whole field set is never held in memory.
4. Progress is per-step, with a cancel button. Cancelling deletes the partial
   file.
5. The UI stays fully interactive throughout.
6. Output is written to a temporary file and atomically renamed on completion,
   so a crashed or cancelled export never leaves a plausible-looking truncated
   `.grib2` behind.

### 12.5 Verification

- **Round-trip:** a minimal GRIB2 *reader* lives in the test suite (not in the
  shipped binary). Every writer test decodes its own output and asserts the
  values match the source field within packing tolerance.
- **External validation in CI only:** `wgrib2` and/or ecCodes decode the golden
  files and their output is compared against expected inventories. These are CI
  dev-dependencies and never runtime dependencies — invariant 5 holds.
- **Golden files:** small (1°) fixtures are committed and byte-compared, which
  is what enforces determinism (invariant 4).

---

## 13. Performance budgets

Targets, measured on a mid-range machine (8 cores, integrated or better GPU),
with a scene of 200 objects unless noted. These are enforced by benchmarks in
CI, not aspirations.

| Operation | Target |
|---|---|
| Map pan / zoom | 60 fps sustained; zero evaluation on the UI thread |
| Tile render, GPU (256²) | p50 ≤ 8 ms, p95 ≤ 25 ms |
| Tile render, CPU fallback | p50 ≤ 40 ms |
| Cold viewport (~24 tiles) | ≤ 250 ms to fully resolved |
| Time-step switch, warm cache | ≤ 50 ms |
| Playback | ≥ 8 steps/s at 0.25° with frames pre-rendered |
| Brush stroke feedback | ≤ 16 ms from pointer move to preview update |
| Transform drag feedback | ≤ 16 ms from pointer move to outline update; **no document write and no tile invalidation until the pointer comes up** (§8.2) |
| Undo / redo | ≤ 16 ms |
| Project open, 5,000 objects | ≤ 500 ms |
| Full-grid CPU evaluation, 0.1° | ≤ 1.5 s per step |
| Export, 0.1° × 120 steps | ≤ 5 min total |
| Peak RSS | ≤ 1.5 GB |
| Render cache on disk | ≤ 4 GB default cap, LRU |

---

## 14. Deferred (post-v1)

Recorded so they are not accidentally designed out:

- Duplicate a project at a different grid resolution.
- Additional GRIB parameters (pressure, temperature, wave fields).
- Region-limited grids.
- Tracing an imported forecast into objects, and interpolating between an
  imported file's time steps rather than holding (§4.8).
- Scriptable/batch export.
- Additional interpolation curve editing (full graph editor).
- Presets and object libraries shareable between projects.

---

## 15. Resolved questions

All questions raised during specification have been settled. Recorded here with
their reasoning so they are not reopened by accident.

| # | Question | Resolution | Section |
|---|---|---|---|
| Q1 | Feather over existing data — fade to calm, or blend? | **`Blend` by default**, `Replace` available per object. The two are identical over calm areas, so the default only governs the case `Replace` handles badly. | §7.4 |
| Q2 | Keyframes beyond a reduced `step_count`? | **Deleted, behind a confirmation** stating the exact count and affected objects. No invisible state. | §4.1 |
| Q5 | Arrows or wind barbs? | **Both ship in v1**, switchable. Barbs are wind-only. The colour ramp always carries unquantised speed regardless. | §5.3 |

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

- Regional/sub-global grids, rotated or non-lat/lon grids.
- GRIB1, NetCDF, or any other output format.
- Importing real forecast data as a starting point.
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
5. **Zero runtime network access.** Basemap, polars, and all assets are bundled.
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
Knots is what this tool's audience reads: a sailing forecast, a boat polar and a
wind barb are all in knots, and a barb is *defined* in 5-knot increments. An
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
    polars: Vec<Polar>,          // embedded boat polars (wind projects)
    routes: Option<RouteTree>,   // sailboat route feature (§10)
    view: ViewState,             // last camera + current time step; UI convenience
}

Layer {
    id: Id,
    name: String,
    visible: bool,
    locked: bool,
    objects: Vec<Object>,        // index 0 = bottom-most within the layer
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

### 4.5 Interpolation

| Value type | Allowed interpolations | Rule |
|---|---|---|
| `f32` | Step, Linear, EaseIn, EaseOut, EaseInOut, Bezier(cubic) | Standard. |
| `Angle` | same | **Shortest-arc**; wraps through 0/360 correctly. |
| `LonLat` | same | Great-circle slerp between keys, easing applied to the arc parameter. |
| `bool`, enum | Step only | UI hides other options. |
| `Geometry` | none | Not animatable. Move/scale/rotate the object instead. |

Evaluation at a step outside the keyframe range holds the nearest key's value
(no extrapolation). With zero keys, `base` is used at every step.

### 4.6 Active range

Each object has an inclusive `active_range`. Outside it the object contributes
nothing, regardless of `enabled`. This is what the timeline's per-object bar
edits (§9.4). `enabled` is the animatable per-step switch; `active_range` is the
coarse lifetime.

### 4.7 File format

Single-file container, extension `.veproj`. A ZIP archive (deflate):

```
project.json          canonical JSON, stable key order, pretty-printed
polars/<id>.csv       embedded boat polars, verbatim as imported
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
with migration. There is no import of foreign formats in v1.

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

  **The two operators are previewed on the map itself.** The eraser writes calm
  and the clone stamp reads the composite beneath it, so what either one paints
  is defined by what is already there — and a coloured wash would be showing
  something neither tool does. Nor could an overlay show a removal at all: it is
  a canvas stacked above the field, and it can add pixels, never take them away.

  So a gesture with either one is applied to the field *while the pointer is
  down*: the swept footprint becomes a screen-space mask, and the map is drawn
  through it. The eraser's covered region loses its field, leaving the basemap —
  which is never masked, since something has to be left to see. The clone's
  loses it and gains the field from the source instead, drawn through a camera
  shifted so the source lands under the brush; a shift is enough because the
  projection is equirectangular, where a constant offset in degrees is a
  constant offset in pixels at every latitude (§5.1). The overlay draws the
  footprint's outline and nothing else.

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
| Direction modes | `Constant`, `TowardPoint` and `AwayFromPoint` mean the same thing for every tool that has a `target`, and are the same property with the same variant indices. | §6.2 |
| Direction display | Flow directions are shown in the project's convention wherever they appear, the inspector included; geometric bearings are not converted. | §3.3 |
| Inert options | An option the object's own mode never reads is not shown. | §6.1 |
| Creation-only options | An option that defines *what the object is* — the stamp's shape, the space it is defined in — is fixed once the object exists, refused at the write path and not merely hidden. An option that can be neither set at creation nor edited afterwards does not belong on the tool. | §6.1 |
| Transform | Move, rotate, scale and re-anchor behave identically for every geometry, because they act on the object's frame rather than on its shape. A drag previews and writes once, on release. | §8.2 |
| Merging | Two gestures of the same tool with identical properties and overlapping footprints merge into one object, subject to §6.1's conditions. Two that differ in *any* property — including the stamp's shape or space — never do. | §6.1 |

The one rule that is genuinely per tool is **whether a hover indicator exists**;
§6.2 states it for each, and "none" is a decision to be made deliberately rather
than an omission.

**These are inherited by construction, not by discipline.** A checklist that has
to be worked through by hand is a checklist that will be missed, so the shared
behaviour is shared code and a tool supplies only what is genuinely its own:

- **One creation command** for the whole catalogue. A tool sends a *gesture* —
  the geometry the pointer drew — and its *options*, which are property values
  keyed by the same ids the inspector uses. The aim-mode check, the anchor, the
  frame, the layer choice and the merge test are written once. The eraser and
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

#### Eraser

Identical interaction to the brush; writes covered cells with speed 0. It is a
first-class object, not a deletion — it can be moved, animated, and disabled,
restoring what was underneath.

| Option | Type |
|---|---|
| `brush_shape` | enum `Circle` \| `Square` — **fixed at creation**, as on the brush |
| `size_km` | f32 (px or km input) |
| `stamp_space` | enum `Geodesic` \| `Projected` — **fixed at creation**; set by the size's unit, px selecting `projected`, as on the brush (§3.5) |
| `feather` | f32 0–1 — ramps *toward* 0 speed, i.e. blends back toward the underlying field's speed at the edge |

Hover: yes — the same footprint outline the brush shows, since it sweeps the
same stamp along the same kind of polyline.

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
| `direction_mode` | enum `Absolute` \| `RelativeToPath` | **Its own property**, not the shared `direction_mode`: a curve aims along its path, where the shared modes aim at a point. A tool may add modes of its own; it may not redefine a shared one. |
| `direction` | Angle | `Absolute`: fixed bearing, a flow direction shown in the project's convention (§3.3). `RelativeToPath`: an *offset* added to the path's local tangent, so 0 = along the path and 90 = across it — an offset is not an azimuth and must not be converted. The two readings of one property are why it is displayed by mode, not by unit. |
| `width_km` | f32 | Corridor half-width; px or km input, so `stamp_space` applies (§3.5). |
| `feather` | f32 0–1 | Across the corridor width. |

Hover: **none**, deliberately: a curve is built node by node, so a single click
produces no footprint to preview.

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
| Capsule chains (round brush, eraser, clone) | min distance to any segment of any chain, minus radius |
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

The eraser is a natural consequence of this model rather than a special case: it
is an object whose speed is 0, so its feathered edge blends the underlying field
back toward calm.

### 7.5 No tool shapes its flow with divergence or curl

Earlier versions gave the tools that place a centre a radial and a tangential
component, added to the base direction as multiples of the speed. Nothing has
them now. **An object's direction mode is the whole of its direction.**

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

**Clone stamp** is the one operator that reads the buffer:

- It samples the buffer at the offset source location. Because objects are
  processed in z-order, the buffer at that moment contains exactly "everything
  below this object," which is the specified semantic.
- Source samples may fall outside the tile currently being rendered. The
  evaluator handles this by running a **nested evaluation of the sub-scene**
  (objects strictly below the clone stamp) at the required source points.
- **Recursion is capped at depth 4.** A clone stamp sampling a region containing
  other clone stamps beyond that depth reads calm. This is a performance guard,
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
| 🖌 | Brush | `B` |
| ⭕ | Circle stamp | `C` |
| ⬟ | Shape fill | `U` |
| ⌫ | Eraser | `E` |
| ⧉ | Clone stamp | `S` |
| 〰 | Curve | `P` |
| 📏 | Measure (dividers / great circle / range rings) | `M` |

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
  changes, route-tree edits.
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

### 9.3 Keyframe editing

- Add a key at the current step by changing a property while auto-key is on, or
  explicitly via the diamond button beside any property in the inspector.
- Select, move, delete, and box-select keys. Move constrained to integer steps.
- Right-click a key to set the interpolation of the segment leaving it.
- Properties with no keys show `base` and no diamonds.
- A property row indicates when the current step's value is interpolated rather
  than keyed.

### 9.4 Playback

- Play, pause, stop, and loop. Playback advances one **time step** per display
  frame at a user-set rate (default 8 steps/s), not real time.
- Playback only advances into frames that are ready; if the next frame is not
  cached, playback holds and shows a buffering state rather than stuttering.

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

## 11. Sailboat route definition (wind projects only)

### 11.1 Purpose

Produce a GRIB in which a small, known set of sailing routes is viable — the
inverse of routing. Instead of "given wind, find the route," this is "given the
routes, synthesise wind that makes them work," so a routing engine can be tested
against a known-correct answer.

The menu entry is hidden for current projects.

### 11.2 Boat polars

A polar is a table of boat speed (kt) indexed by true wind angle (0–180°) and
true wind speed (0–40 kt), bilinearly interpolated.

- Import from CSV / `.pol` (the common ORC-style tab- or semicolon-separated
  grid: first row TWS, first column TWA).
- Two or three sample polars ship with the app so the feature works out of the
  box, offline.
- Imported polars are embedded in the project file (§4.7), so a project is
  portable without its source files.

### 11.3 Mode flow

A guided, modal state machine. The map switches to route-editing interaction;
painting tools are unavailable inside the mode.

1. **Enter mode.**
2. **Select a polar** from the embedded list or import one.
3. **Set departure:** choose the departure time step, then click the departure
   location. This creates the root node.
4. **Step forward.** For each subsequent time step, every node from the previous
   step is a candidate parent. The user selects a parent and clicks to add a
   child. Unlimited children per parent.
   - A **reachability disc** is drawn around the selected parent: a geodesic
     circle of radius `max_boat_speed(polar, ≤ max_tws) × step_hours`. Clicks
     outside it are rejected with an explanation.
   - `max_tws` caps how strong a wind the app is willing to invent. **It
     defaults to the selected polar's own maximum defined TWS**, so the solver
     uses every row of real data available and never extrapolates beyond the
     table. It is user-adjustable (5–60 kt) and the effective value is displayed
     in the route panel.
   - Because the solver already selects the *minimum* wind that satisfies each
     leg (§11.4), a high cap does not produce generally stronger fields — it
     only makes aggressive legs feasible that would otherwise be flagged. Legs
     solved above 35 kt are noted in the route report as a heads-up, without
     blocking.
5. **Finish adding routes.** Explicit button. The tree is then solved.

Nodes and edges remain editable after solving; re-solving is idempotent.

### 11.4 Solving

For each leg (parent → child at consecutive steps):

1. Required course = initial great-circle bearing parent→child. Required boat
   speed = geodesic distance / `step_hours`.
2. Apply a safety margin: the solved wind must yield ≥ `required_speed × 1.05`.
3. **Invert the polar.** The problem is under-determined — a family of
   `(TWD, TWS)` pairs satisfies any achievable speed. Resolve it with an
   explicit, documented selection rule:
   - Restrict TWA to `[35°, 170°]` (out of the no-go zone, off a dead run).
   - Among feasible solutions choose **minimum TWS** — the least extreme wind
     that does the job.
   - Break ties toward a TWA of 90° (beam reach), the most forgiving point of
     sail for a routing engine to reproduce.
   - The selection strategy is a named enum so alternatives can be added later.
4. If no feasible solution exists (the leg is unreachable within `max_tws`), the
   leg is flagged in the UI and skipped rather than silently approximated.

### 11.5 Generated objects

Each solved leg produces a **curve-tool object** in a dedicated, auto-created
layer named `Routes`:

- Geometry: the leg's great-circle path.
- `width_km`: user setting, default 200 km corridor.
- `speed` and `direction` from the solved wind, with generous `feather`.
- `active_range` set to that leg's single time step.

Generated objects are **ordinary, fully editable objects**. They are not locked.
They carry a route-link tag recording which leg produced them, which drives the
merge behaviour below.

### 11.6 Re-solving and merge rules

Re-solving preserves the user's manual edits. Two mechanisms make this
predictable rather than fragile.

**Leg identity is structural, not positional.** A leg is identified by the pair
`(parent_node_id, child_node_id)`. Route-tree node `Id`s are stable across every
edit that does not delete the node — including moving it. So:

| Tree change | Effect on the leg's object |
|---|---|
| Node moved | Identity preserved; solver-owned properties recomputed, user edits kept |
| Node added | New object created with defaults |
| Node deleted | Its legs' objects are deleted, with a confirmation naming any that carry user edits |
| Polar changed | All identities preserved; all solver-owned properties recomputed |

This is what defuses the usual fragility of merge-on-regenerate: topology changes
never silently reassign one leg's edits to a different leg, because identity was
never derived from ordering or position in the first place.

**Property ownership is explicit.** Each route-generated object tracks a
per-property user-modified flag, set the first time the user changes that
property.

| Ownership | Properties | On re-solve |
|---|---|---|
| Solver-owned | `speed`, `direction`, geometry (the leg path), `active_range` | Always recomputed |
| User-owned | `width_km`, `feather`, `edge_mode`, `name`, `enabled` | Preserved once modified; otherwise follow the solver's default |

If the user manually edited a solver-owned property, re-solving overwrites it and
lists the overwrites in the route report — a notice, not a prompt.

**Escape hatch.** "Detach from route" (per object, or for the whole `Routes`
layer) drops the route-link tag. Detached objects become entirely ordinary and
are never touched by re-solving again. This is the path for a user who wants to
hand-tune a wind field that started from a solved route.

### 11.7 Conflicts

Two legs at the same time step whose corridors overlap with materially different
solved winds are a genuine conflict. The app:

- Detects overlaps and flags them in a route report.
- Resolves by z-order (later leg wins), the same rule as everywhere else.
- Reports affected legs so the user can widen the time step, move a node, or
  accept the overlap.

---

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
- Importing a real forecast as a background layer to trace over.
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
| Q3 | Default `max_tws` for route solving? | **The polar's own maximum defined TWS.** Uses all real data, never extrapolates. Adjustable; legs above 35 kt noted in the report. | §11.3 |
| Q4 | Are route-generated objects editable? | **Fully editable, with merge on re-solve.** Leg identity is the node-`Id` pair; property ownership is explicit; "detach from route" is the escape hatch. | §11.6 |
| Q5 | Arrows or wind barbs? | **Both ship in v1**, switchable. Barbs are wind-only. The colour ramp always carries unquantised speed regardless. | §5.3 |

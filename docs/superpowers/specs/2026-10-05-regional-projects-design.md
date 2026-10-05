# Regional projects — design

Requested 2026-10-05: when a project is created the user chooses **Global** or
**Regional**. A regional project has a lat/lon rectangle that may cross the
antimeridian and may reach either pole. The map cannot zoom or pan out beyond
it. All imported and downloaded data is clipped to it, and the clipping is
real: less is held, written and exported, not just less drawn.

## What is stored today, and what "smaller" can mean

The `.veproj` file stores almost no field data already (invariants 1 and 2).
It holds paths, objects, captures and the ICON neighbour sets. The global
bulk lives in three places:

| Where | What | Global size today | Regional effect |
|---|---|---|---|
| `<data>/history/*.grib2` | History and NRT fetches (ERA5, GlobCurrent, ten NRT products, SST) | 1440×721 per step, ≈4.2 MB per step per vector product, up to ≈1 GB per import | **Written regional.** A 20°×20° region at 0.25° is 81×81 nodes: ≈0.6 % of the global file |
| `.veproj` `regrid/<key>.bin` | The 3-neighbour set for an ICON mesh onto the project lattice | one entry per lattice node: 6.5 M nodes at 0.1° | **Built for the region's lattice only**, so the entry shrinks with it |
| Memory (`RasterGrid`) | Every imported or fetched lattice, re-read on open | a file's full extent | **Cropped on read** to the region plus a one-node margin |
| Exported `.grib2` / Zarr | The baked field | global grid | **Regional grid** (GRIB); only the region's chunks (Zarr) |

A user's own GRIB or Zarr file stays where they put it and is referenced by
path, as it is now. The project never stored it, so there is nothing to
shrink. It is cropped in memory on every read. Writing a clipped copy would
add storage, not remove it (decision R4).

## Decisions

**R1. The region is a project setting, immutable after creation.** It is
the same kind of setting as the resolution: it defines the lattice every
import is cropped to and every export is written on. Changing it would
silently change what the stored fetches cover. It goes in spec §4.1's table
as *Immutable: Yes*. Growing a region is "duplicate project at new extent",
which is out of scope (§14), like the resolution.

**R2. Stored as integers on the project lattice.** `Region { west_udeg,
span_udeg, north_udeg, south_udeg }` in micro-degrees (`i32`).
- `west_udeg` is in `[-180e6, 180e6)`.
- `span_udeg` is the eastward arc from the west edge, in `(0, 360e6]`. A
  span of `360e6` means every longitude.
- Every value is a multiple of the resolution's micro-degrees. Edges are
  **node** positions, inclusive, matching spec §4.2's cell centres.

Integers need no `canonical::*_field` helper, cannot drift a ULP, and make
the snap to the lattice explicit. A west/east pair cannot express a full
circle (`west == east` is ambiguous), and a polar cap needs one; a span can.
The region lives in `ProjectSettings.region: Option<Region>`. `None` means
global, so every existing project opens unchanged and no migration is needed.

**R3. What "crosses the date line" and "polar" mean.**
- *Antimeridian:* `west + span > 180°`. For example, west 160°E with span
  40° covers 160°E to 160°W. Every lattice already measures longitude as
  `(lon − lon0).rem_euclid(360)` (`RasterGrid::sample`), so a lattice whose
  `lon0` is 160 and whose columns run past 180 needs no new sampling code.
- *Polar cap:* `span = 360°` and `north = 90°` (or `south = −90°`). For
  example, the Arctic north of 60°N is `{west −180, span 360, north 90, south
  60}`. The lattice then wraps in longitude (`ni = 360/d`, no duplicated
  column) and includes the pole row, exactly like the global grid.
- *Polar wedge:* a span under 360° that reaches a pole is legal. It is an
  ordinary lat/lon rectangle that happens to touch the pole row.
- Global is `None`, never a region of span 360° from 90 to −90. The region
  editor refuses that and offers Global instead, so there is one spelling.

**R4. Local files are cropped in memory and never copied.** See the table
above. The decoders crop before building the `RasterGrid`, so a 0.1° global
GRIB in a 10°×10° project holds about 10,000 nodes, not 6.5 M.

**R5. Fetches write a regional GRIB.** `GridSpec` gains a first point
(`la1_udeg`, `lo1_udeg`) and section 3 writes it. Where a source can
subset at the server, it does: the Copernicus ARCO stores through
`ArraySubset` windows, and ERDDAP through index subscripts (two requests when
a `LonPM180` dataset's window crosses its seam). ERA5 and GlobCurrent store
one whole-globe chunk per step, so they still download the globe and crop
before writing. The bandwidth stays the same; the disk use does not. The
same goes for the whole-file products (Seawinds, CMC, ASCAT granules). The
file name carries the region, so a global and a regional fetch of the same
hours never share a file.

**R6. The GRIB export is a regional grid.** Template 3.0 with La1/Lo1 at the
region's north-west node, Ni/Nj the region's. GRIB longitudes are
`[0, 360)` (only `ve-grib` converts), so a region crossing 180° is contiguous
there (Lo1 160e6, Lo2 200e6). A region crossing 0° gives Lo2 < Lo1, which
GRIB allows and our decoder already reads (`decode.rs` `rem_euclid`). It has
to be checked with wgrib2 and grib_dump before it ships. A polar cap has
`Ni = 360/d` and Lo2 = Lo1 − d mod 360. A global project's output is
byte-identical to today: `the_encoding_is_identical_on_every_platform` must
not move.

**R7. The Zarr export keeps the routing store's global shape.** The routing
consumer (`~/SampleRoutingData/chunk_loader`) wraps longitude over the whole
axis and expects longitude ascending from −180. A store with regional axes
would be silently misread by it. So a regional project writes the same
global-shaped store, evaluates only its region's nodes, leaves the rest NaN,
and the writer's existing rule (an all-NaN chunk is never written, spec
§12.3) makes the store the size of the region. `routing_layout.rs`'s text
equality stays true.

**R8. The map is bounded by the region in every projection.** The region
rides on the `Camera`, as the projection does, so every path through
`clampCamera` sees it. "Can't zoom out beyond" means the minimum zoom is the
zoom at which the region fits the window, and panning keeps the window over
the region on any axis where the region is larger than the window. On an
axis where it is smaller, the region is centred.
- *Cylindrical:* longitude is clamped in the region's unwrapped arc, unless
  the span is 360°, which keeps today's free wrap. Latitude is clamped in
  the projection's y. A region past Mercator's 85.05° stops at the
  projection's own edge.
- *Movable (globe, azimuthals):* the centre is clamped into the region, and
  the minimum zoom fits the region's projected outline around that centre.
  A polar cap under a polar stereographic or the globe centres on the pole.
- *Fixed general maps:* the region's outline projected in the plane gives a
  box, treated like the cylindrical case in plane coordinates.

The exterior that a window of a different aspect still shows is dimmed (an
even-odd fill in the overlay, over the region's subdivided outline). A
gesture that **starts** outside the region is refused with a hint, the M68
pattern. An object that straddles the edge is allowed; what lies outside
reaches no export.

**R9. Choosing the region.** The new-project form gains *Extent: Global |
Regional*. Regional shows:
- a picker: a small equirectangular canvas drawing the basemap's coarsest
  coastline, centred on whichever longitude the user drags (so a box can be
  dragged across 180°). Drag direction decides the wrap, as `marqueeBounds`
  does;
- four numeric fields (N, S, W, E) as the source of truth, in `NumberField`,
  snapped outward to the lattice on commit;
- *Arctic* and *Antarctic* buttons that fill a 360° cap from 60° to the pole
  (editable afterwards), and a *Full circle* check that sets span 360°;
- the export-size estimate, recomputed for the region.

From an open project, *New project* also offers *Use current view*. *Open
from GRIB / Zarr* makes a regional project when the file is not global,
using the file's extent snapped outward, and a global one otherwise. The MCP
`project_new` tool takes an optional `region {west, east, south, north,
full_circle}`.

**R10. Imports that miss the region are refused.** A file whose lattice
does not intersect the region is refused, naming the file and the region,
rather than adding an empty layer. A file that partly covers it imports the
overlap, and the rest of the region reads missing, as a regional GRIB does
today.

## Out of scope

- Changing a project's region after creation (R1).
- Clipping display-only layers (images, GIS, charts, OSM). They store no
  field data and are cheap. They are drawn under the dimmed exterior as now.
- Non-rectangular regions.
- Shrinking the bandwidth of whole-globe-chunk sources (ERA5, GlobCurrent).

## Invariants checked

1–3: no raster joins the project file. The preview and the export both read
the same cropped lattices. 4: the export is still `CpuEvaluator` on a
deterministic point list, and the global output is byte-identical. 5: no new
host; ERDDAP and ARCO subsetting talk to the same servers already named. 6:
cropping makes reads and tiles cheaper, never dearer.

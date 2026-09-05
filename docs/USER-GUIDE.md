# VectorEffects — user guide

VectorEffects paints global **wind** and **ocean-current** fields and exports
them as GRIB2. Everything you draw is an *object* on a *layer*; the map is a
live preview of the field those objects make, and the export bakes the same
objects at the project's grid resolution. The preview never reaches the file.

## Projects

A project fixes four things at creation and never after: the field kind
(wind or current), the grid resolution, the time step and the number of
steps. Everything else is editable. **New…** starts one; **Open from GRIB…**
starts one shaped by a forecast file — its kind, grid, step and span come
from the file, and the file becomes the first layer.

Save with `Cmd`-`S`. A `.veproj` holds geometry and parameters, never pixels;
a GRIB or image layer keeps the file's *path*. If the application stops
unexpectedly, the start screen offers the last snapshot of any unsaved work
under **Recovered work** — a snapshot is taken every thirty seconds while a
project has unsaved changes and removed when it is saved or closed.

## The map

Drag with the hand tool (`V`) to pan, wheel to zoom. **Projection** in the
map's toolbar switches between equirectangular (the default — the map is 1:1
with the grid), Mercator (a bearing drawn on it is the bearing sailed) and
Miller. A projection is a view setting: it changes nothing stored or exported.

Speed is the colour ramp; direction is the glyph. **Glyphs** offers arrows or
wind barbs, and the readout at the bottom gives the field under the cursor in
knots, in the project's direction convention.

## Painting

Every tool has a size, in **km** on the ground or **px** on the map — the unit
is the only control, and it decides whether the shape is a circle on the globe
or a circle on the screen.

| Key | Tool | What it makes |
|---|---|---|
| `P` | Brush | A stroke of wind or current at a speed and direction |
| `C` | Circle | A rotating stamp: a gyre or a cyclone |
| `F` | Shape fill | A polygon or preset shape filled with a field |
| `B` | Curve | A field along a path |
| `E` | Mask | Removes the field under it — hold the underlying layer back |
| `S` | Clone stamp | Copies the field from elsewhere |
| `I` | Intensify / reduce | Scales the speed of what is beneath |
| `D` | Diverge / converge | Bends the field outward or inward |
| `R` | Rotate flow | Turns the field by an angle |
| `W` | Warp | Pushes or twists the field under a placed region |
| `L` | Liquify | Drags the field along the stroke, like a smear |

The four modifiers and the warp act on the field *beneath* them; they make
nothing on empty ocean.

**Selections.** The select tool (`M`) draws a region — rectangle, circle or
lasso. With a region selected, clicking inside it with the brush, mask,
intensify, diverge or rotate tool makes that object from the region's
boundary; `Cmd`-`C` copies the field inside it and `Cmd`-`V` pastes it back
down as a patch.

Objects merge when two strokes of the same tool with the same settings
overlap, so a swathe painted in three passes is one object. Undo is `Cmd`-`Z`.

## Time

The timeline runs along the bottom. Every property is animatable: change a
value at a step with **Auto-key** on and it becomes a keyframe; the field ramps
between keyframes. Position keyframes move an object along a great circle, and
**Motion** on a track row puts that movement into the field itself. An object
can **follow** another.

A GRIB layer's frames show on the timeline as marks; select marks and
`Cmd`-`C`/`Cmd`-`V` to copy a frame to another step.

## Macros

**Capture** (`K`) records a region of the field over a run of frames into
the macro library: draw a region, press **Start capture**, scrub the ruler,
drag the region into place at each frame, then **Finish** and name it.
**Insert** (`N`) puts a macro back down anywhere, in any project. The
library's location is in Settings.

## Measurement

**Measure** (`T`) offers dividers (a chain measured leg by leg), a passage
(great circle and rhumb line between two points, both drawn and labelled) and
range rings. Distances are shown in nautical miles and kilometres; bearings
are courses, not wind directions.

## Image layers

**Import image** in the layer panel lays a GeoTIFF, or a PNG/JPEG with a world
file, under the field where the file says. An image without a georeference
lands on the view; drag its three corner handles into place (shift keeps it
north-up). Opacity is per layer. Images are never exported.

## Export

**Export GRIB…** asks for the forecast start time, an originating centre, and
the **precision** — 8, 12, 16 or 24 bits per value, with the step each implies
and the file size beside it. Sixteen is the default. The export runs in the
background, can be cancelled, and never leaves a truncated file behind.

## Settings

`Cmd`-`,` or the ⚙ button: every shortcut is rebindable, the default colour
scales for new projects, and the macro library's directory.

The application makes no network requests at any time.

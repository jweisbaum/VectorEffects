# Image control-point warp design

**Date:** 2026-09-20. **Status:** approved in discussion, awaiting spec review.
**Companion to** `spec.md` §4.9 (image layers), §5.1 (projection), §3.5
(stamp space), and invariants 1, 2 and 3.

## 1. What this is

A way to georeference a picture by pointing at things. The user opens the
mode from the image layer's own controls, then clicks a feature *in the
picture* and the place it belongs *on the map*, as many times as they like,
and presses Enter. The image is then warped so that every pair lands exactly
where it was put, bending between the pairs.

It replaces the three corner handles, which could only place an image by an
affine and so could never correct a scanned sheet's own distortion.

Three decisions were taken in discussion and are fixed here:

1. **Warped means bending.** Every pair lands exactly; the picture deforms
   between them.
2. **The source may be anything** — a scanned paper chart, a photograph of
   one, a screenshot, a satellite image — so the fit must be the most
   correct one available rather than the cheapest.
3. **The corner handles are replaced**, not kept alongside.

## 2. The transform

A **homography base with a thin-plate-spline residual**, chosen by how many
pairs there are:

| Pairs | Fit | Exact |
|---|---|---|
| 0 | the stored `Placement` — today's behaviour | — |
| 1 | translation | yes |
| 2 | similarity: translate, rotate, uniform scale | yes |
| 3 | affine | yes |
| 4 | homography | yes |
| 5–50 | homography, then TPS on the residual | yes |

**Fifty pairs is the cap**, and it is a real limit rather than a guess at
what anyone would place: the fit solves an `n x n` dense system and the mesh
is re-evaluated against every pair, so the cost is quadratic in one place and
linear in a hot one. A fifty-first click is refused with a hint rather than
accepted and quietly dropped.

Neither half is sufficient alone, which is why the composition is the design
and not an implementation detail:

- A **homography** maps straight lines to straight lines. It corrects a
  picture taken at an angle, which no affine can, but it never bends, so it
  cannot answer the request.
- A **thin-plate spline** bends, but its own linear part is only affine. Used
  alone on a perspective source it must spend control points correcting
  distortion that a homography gets for free, and it converges to the right
  answer more slowly and less stably.

Composed, four pairs give a pure perspective correction (the TPS residual is
identically zero), and each pair after that buys local bending where it is
placed. This is also what the established georeferencers do; it is not novel.

**Interpolating, not smoothing.** The TPS regularisation λ is 0, so every
pair is honoured exactly. A smoothing parameter is deliberately *not* exposed
in this work; if measurement error later makes it wanted, it is one number
added to the fit and one control in the panel.

**Extrapolation is the known weakness.** A thin-plate spline's radial term
`r² log r` *grows* with distance, so image corners well outside the convex
hull of the control points can fly away. Two mitigations, both required:

- The homography base carries the global behaviour, so the extrapolated
  region degrades to a sensible projective map rather than to nothing.
- The radial term is damped outside the hull, so a distant corner tends to
  the homography rather than to the spline's divergence.

The panel reports the residual at the image's four corners, so a warp that is
misbehaving outside the points is visible rather than discovered later.

## 3. What is stored

`ve_core::document::LayerSource::Image` gains:

```rust
/// The pairs the user placed: a point in the picture, and where on the
/// earth it belongs. The warp is computed from these and never stored —
/// it is derived state, like a render (invariants 1 and 2).
#[serde(default)]
control_points: Vec<ControlPoint>,
```

with

```rust
pub struct ControlPoint {
    /// Across the image, in pixels from the left edge.
    u: f64,
    /// Down the image, in pixels from the top edge.
    v: f64,
    /// Where it belongs: longitude in [-180, 180), degrees.
    lon: f64,
    /// Where it belongs: latitude, degrees.
    lat: f64,
}
```

Four rules follow from the conventions and are not optional:

- **Every one of those `f64`s needs a `canonical::*_field` serde helper**, or
  the values will not survive a save and load and only the round-trip test
  will notice. `degrees_field` for `lon` and `lat`; `ratio_field` for `u` and
  `v`, whose six decimal places on a pixel coordinate are far finer than a
  pixel and well inside what an `f64` holds even for a hundred-megapixel
  scan. No new helper is needed.
  `io::tests::hostile_floats_survive_a_round_trip` is extended with the new
  field.
- **`u` and `v` are image pixels, not degrees.** That is what makes a pair
  independent of the placement in force when it was made, so pairs can be
  accumulated and re-fitted without drift.
- **A new field needs no migration**, per the recipe: `#[serde(default)]`
  means an older project opens with an empty list and behaves exactly as it
  does today. `SCHEMA_VERSION` is *not* bumped.
- **`placement` stays** and keeps its present meaning: the file's own
  georeference, or where the picture landed when it was dropped. It is the
  base the fit starts from, it is what zero control points uses, and it is
  what *Reset place* returns to. Deleting every pair therefore puts the image
  back where it started, which is the behaviour that makes the mode safe to
  experiment in.

The warp itself — the homography's nine numbers and the spline's weights — is
recomputed from the pairs whenever they change and on open. It is never
serialised.

## 4. Rendering

This is the part that makes "all projections" true, and it is mostly a
rearrangement of what is already there.

An image is already drawn as a subdivided quad whose `aCell` attribute is a
**static** grid in the image's own 0..1 space. Today `IMAGE_VERT` derives each
vertex's lon/lat from the affine uniforms `uPlaceLon`/`uPlaceLat`.

**The change:** for a warped image, Rust evaluates the warp at every mesh
vertex when the pairs change — *not* per camera — and uploads lon/lat as a
vertex buffer. The vertex shader then only projects what it is handed. Because
projection happens strictly after the warp, every projection works without a
special case: the globe, the azimuthals, the 270 general presets and the flat
map all go through the paths they already use.

**The one real constraint.** `IMAGE_FRAG`'s globe path (`uProjection >= 15`)
finds the texel by *inverting* the placement per pixel, which is an explicit
2×2 affine inverse. A thin-plate spline has no closed-form inverse, and
inverting it numerically per pixel would mean tens of radial evaluations per
Newton step per pixel, which is not affordable.

So the rule is:

> **A warped image is drawn through the mesh in every projection. An unwarped
> image keeps the existing per-pixel inverse on the globe.**

That split is justified rather than convenient. The per-pixel path exists
because an image can span the earth and a mesh fine enough to follow a sphere
at every zoom would be rebuilt every frame — the thing the globe work removed.
A rubber-sheeted image is by its nature local: a harbour, an approach, a
scanned sheet. **A test asserts that assumption** rather than leaving it
implied, and the mesh path is chosen by "has control points", not by size.

**Mesh fineness.** The quad is subdivided more finely than today so the mesh
follows the bending rather than chording across it. At 64×64 that is 4,096
vertices; with the 50-pair maximum that is about 205,000 radial evaluations
per rebuild, on a change only. Cheap, but it is **measured and reported, not
assumed**, against the budgets in `spec.md` §13.

## 5. The interaction

**Entering.** A button beside Opacity in `ImageControls`. It puts the map into
an alignment mode for that layer and nothing else; no other tool changes.

**Placing.** The mode alternates, starting on the picture:

1. Click a feature **in the picture**. The screen point is unprojected to
   lon/lat by the camera, then carried back through the *current* warp to an
   image pixel `(u, v)`. Doing it this way is what lets pairs accumulate
   against a stable image space however many have already been placed.
2. Click where it belongs **on the map**. That point unprojects to the target
   lon/lat.

**Finishing.** `Enter` applies, `Escape` cancels the whole session leaving the
document untouched, `Backspace` drops the last pair. A half-finished pair — a
picture click with no map click yet — is discarded.

**Feedback.** The overlay numbers each pair and draws a line from the picture
point to its target. Once there are enough pairs to fit, the residual is shown
per pair and at the image's four corners.

**Existing machinery this follows**, rather than inventing its own:

- `measure.ts`'s click-to-place precedent for collecting points and for
  Enter/Escape.
- `hint.ts` for the prompt ("click the same place on the map"), because no
  panel renders an error line of its own.
- `cursorFor` in `map/cursor.ts` for the cursor, written per pointer report
  and never as a CSS class.
- The overlay is drawn inside `draw()`, through `requestOverlay`, never by a
  synchronous `drawOverlay` after a camera change.

**Why this works in every projection**: every click goes through `unproject`,
which already reads the projection off the camera, and every stored number is
either an image pixel or a lon/lat. Neither is a screen or projection
quantity, so nothing in the stored result depends on how the map was being
looked at when the pairs were placed.

**One write, one undo.** Applying is a single `Command` that replaces the
whole control-point list, the shape `Command::SetAnnotations` already uses —
which is what makes the inverse right by construction.

## 6. What is removed

Per the decision that the handles are replaced:

- `ve_app::image::set_image_corners` and `corners_set`.
- `ve_core::document::Placement::from_corners`.
- `imageDrag` and `placing.corners` in `MapView.tsx`, and the corner handles
  drawn on the overlay.

**A consequence to state plainly:** nudging an image is no longer a drag. It
is one control point, which the table in §2 makes a pure translation. That is
a smaller gesture than a drag, not a larger one, but it is a change in how the
coarse placement is done and it is recorded here as deliberate.

*Reset place* stays, and now clears the control points as well as restoring
the placement, since the two together are "where the image sits".

## 7. Testing

Against independent references, never against a second copy of the formula:

- **The fit.** Hand-built pairs whose answer is known: a pure translation, a
  90° rotation, a known perspective quad. Then the defining property — for
  any set of pairs, evaluating the fitted warp at each `(u, v)` returns its
  `(lon, lat)` to within a tolerance far tighter than a pixel. That property
  is what "perfectly" means and it is the test that would catch a wrong
  spline.
- **Degradation by count.** One pair is a translation and moves nothing else;
  two preserve the aspect; three give the unique affine through those three
  points, checked against that affine worked by hand. Three points determine
  an affine exactly, so the expected answer is arithmetic and not a second
  copy of the fit — and it is the same answer `from_corners` gave before this
  work removed it, which makes it a check on continuity as well.
- **Antimeridian and poles**, as every geodesy change needs: pairs that
  straddle 180°, and an image placed against a pole.
- **The projection claim.** The same pairs, fitted once, produce the same
  lon/lat per mesh vertex regardless of the camera — asserted by fitting with
  several projections active and comparing the vertex buffer, which is the
  claim in §4 stated as a test.
- **The locality assumption** behind the mesh/per-pixel split, asserted
  rather than assumed.
- **Round trip.** Save and load with control points, including
  `hostile_floats_survive_a_round_trip`.
- **Undo.** Applying and undoing returns the exact previous list.
- **Cost.** The mesh rebuild measured, before and after, against §13.

## 8. What this does not do

Deliberately out of scope, listed so they are choices rather than omissions:

- **No smoothing parameter.** λ is 0; every pair is exact. §2 says what it
  would take to add.
- **No control-point editing after the fact** in this work: pairs are placed
  and applied. Moving or deleting an individual pair later is a natural
  follow-up and the stored shape already supports it.
- **No automatic feature matching.** The user points at things.
- **No change to what an image *is*.** It stays display only: it reaches no
  scene, no field, no render-cache key and no export (spec.md §4.9). A warp
  changes where a picture is drawn and nothing else.

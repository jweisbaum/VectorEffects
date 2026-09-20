/**
 * Brush footprint geometry for the tool overlay.
 *
 * Extracted from the drawing code so the path construction can be tested: the
 * bug it exists to prevent is invisible to a type checker and only shows up as
 * a strange shape on screen.
 */

import type { BrushShape } from "../generated/BrushShape";
import type { StampSpace } from "../generated/StampSpace";
import { type Camera, type Viewport, normalizeLon, project, projectionFor, unproject, validGeo } from "./camera";
import { destination, EARTH_RADIUS_M } from "./geo";
import { projectionOf, type ProjectionId } from "./projection";

/** The projection frozen into a pixel footprint; ground previews use latitude. */
export function stampProjection(space: StampSpace) {
  return projectionOf(space === "geodesic" || space === "projected" ? "equirectangular" : space as ProjectionId);
}

/**
 * Kilometres per degree of latitude — a ground measure, so it is the same
 * everywhere and in every projection. What varies is how many *pixels* a
 * degree of latitude is worth, which is `Projection.scaleAt`.
 *
 * Derived from the one earth radius rather than written out: this is the same
 * conversion `ve_render::aeqd::M_PER_DEGREE` makes, and a preview that measures
 * the map on a different earth than the field is drawn on is a preview of
 * something else.
 */
export const KM_PER_DEGREE = (EARTH_RADIUS_M * Math.PI) / 180 / 1000;

/**
 * Floor on the latitude cosine.
 *
 * Every scale here divides by it, and at the pole it is zero. Shared by the
 * screen-size and the pixel-to-kilometre conversions so the two stay exact
 * inverses of each other: a brush asked for in pixels must draw at exactly that
 * many pixels wide.
 */
const MIN_COS_LAT = 0.02;

export function cosLat(lat: number): number {
  return Math.max(Math.cos((lat * Math.PI) / 180), MIN_COS_LAT);
}

/**
 * The part of `CanvasRenderingContext2D` this module needs.
 *
 * `lineTo` and `closePath` are here rather than in a second interface because a
 * swept *square* needs them (M58): the connector between two stamps is a
 * hexagon, so every footprint this module builds may draw a polyline.
 */
export interface PathSink {
  moveTo(x: number, y: number): void;
  lineTo(x: number, y: number): void;
  closePath(): void;
  ellipse(
    x: number,
    y: number,
    radiusX: number,
    radiusY: number,
    rotation: number,
    startAngle: number,
    endAngle: number,
  ): void;
  rect(x: number, y: number, width: number, height: number): void;
}

/**
 * Screen dimensions of a stamp of `radiusKm`.
 *
 * `radiusKm` is the north-south ground extent in both spaces; the spaces differ
 * only in how far the stamp reaches east-west.
 *
 * The vertical radius passes through the map projection (M11), because a span
 * of latitude is not a fixed number of pixels once `y` is not the latitude.
 * Under Mercator that makes a geodesic footprint a true circle at every
 * latitude rather than an ellipse — which is the projection being conformal,
 * not a special case here.
 */
export function footprintRadii(
  camera: Camera,
  lat: number,
  radiusKm: number,
  space: StampSpace = "geodesic",
  lon = camera.centerLon,
): { rx: number; ry: number } {
  if (projectionFor(camera).general) {
    if (space !== "geodesic") {
      const px = radiusKm / KM_PER_DEGREE * camera.pxPerDeg;
      return { rx: px, ry: px };
    }
    const view = {width: 0, height: 0}, centre = {lon, lat};
    const origin = project(camera, view, centre);
    const east = project(camera, view, destination(centre, 90, radiusKm * 1000));
    const north = project(camera, view, destination(centre, 0, radiusKm * 1000));
    return {rx: Math.hypot(east.x-origin.x,east.y-origin.y), ry: Math.hypot(north.x-origin.x,north.y-origin.y)};
  }
  const halfDeg = radiusKm / KM_PER_DEGREE;
  const source = stampProjection(space);
  const viewProjection = projectionFor(camera);
  const ratio = space !== "geodesic" && source.id === viewProjection.id ? 1
    : viewProjection.scaleAt(lat) / Math.max(1e-8, space === "geodesic" ? 1 : source.scaleAt(lat));
  const ry = halfDeg * ratio * camera.pxPerDeg;
  // A circle on the ground spans more longitude the further from the equator,
  // so it draws as an ellipse. A projected stamp is defined on the map instead
  // and spans the same degrees both ways — which is what a size in pixels is
  // asking for (spec.md 3.5).
  // Clamped so a geodesic footprint near a pole stays finite rather than
  // filling the map.
  const rx = (space !== "geodesic" ? halfDeg : halfDeg / cosLat(lat)) * camera.pxPerDeg;
  return { rx, ry };
}

/**
 * Ground size in kilometres of a span given in screen pixels.
 *
 * For a **projected** stamp there is nothing to correct: a degree of latitude
 * is a fixed number of pixels at any latitude, so `pixels` converts straight
 * through and the footprint comes out that many pixels across *and* tall. That
 * is what a size in pixels asks for, and why px implies a projected stamp.
 *
 * For a **geodesic** stamp the map scale depends on latitude as well as zoom: a
 * degree of longitude covers `cos(lat)` of the ground a degree of latitude
 * does, so the same pixel count buys less ground the further north the cursor
 * is. Measuring against the horizontal scale makes the footprint exactly that
 * many pixels *wide* — it is `cos(lat)` as many tall, because a ground circle
 * is an ellipse on this map and no single number is its size in pixels.
 *
 * The answer is resolved once, when the object is created, and stored in
 * kilometres (spec.md 3.5). Zooming afterwards never resizes an existing
 * object; that is what keeps objects pinned to the earth.
 */
export function kmFromPixels(
  camera: Camera,
  lat: number,
  pixels: number,
  space: StampSpace = "geodesic",
  lon = camera.centerLon,
): number {
  if (projectionFor(camera).general) {
    if (space !== "geodesic") return (pixels / camera.pxPerDeg) * KM_PER_DEGREE;
    return pixels / Math.max(1e-8, footprintRadii(camera,lat,1,"geodesic",lon).rx);
  }
  const scale = space !== "geodesic" ? 1 : cosLat(lat);
  return (pixels / camera.pxPerDeg) * KM_PER_DEGREE * scale;
}

/** The inverse of {@link kmFromPixels}, for showing the current size in px. */
export function pixelsFromKm(
  camera: Camera,
  lat: number,
  km: number,
  space: StampSpace = "geodesic",
  lon = camera.centerLon,
): number {
  if (projectionFor(camera).general) {
    if (space !== "geodesic") return (km / KM_PER_DEGREE) * camera.pxPerDeg;
    return km * footprintRadii(camera,lat,1,"geodesic",lon).rx;
  }
  const scale = space !== "geodesic" ? 1 : cosLat(lat);
  return (km / (KM_PER_DEGREE * scale)) * camera.pxPerDeg;
}

/**
 * Adds one footprint to a path, round or square (spec.md 6.2).
 *
 * For the round stamp the `moveTo` is not optional: `ellipse` continues the
 * current subpath, so without it each footprint is joined to the previous one
 * by a straight line and a chain of stamps fills as one enormous polygon.
 * `rect` opens and closes its own subpath, which is why the square case needs
 * no such guard — an asymmetry worth naming rather than rediscovering.
 */
export function addFootprint(
  sink: PathSink,
  camera: Camera,
  view: Viewport,
  lon: number,
  lat: number,
  radiusKm: number,
  shape: BrushShape = "circle",
  space: StampSpace = "geodesic",
  insetPx = 0,
): void {
  if (projectionFor(camera).general) {
    projectedStamp(sink,camera,view,lon,lat,radiusKm,radiusKm,shape,space,insetPx);
    return;
  }
  const { x, y, rx, ry } = stampBox(camera, view, lon, lat, radiusKm, space, insetPx);
  if (shape === "square") {
    sink.rect(x - rx, y - ry, rx * 2, ry * 2);
    return;
  }
  sink.moveTo(x + rx, y);
  sink.ellipse(x, y, rx, ry, 0, 0, Math.PI * 2);
}

/** One stamp's box on screen: where its centre is and how far it reaches. */
interface StampBox {
  x: number;
  y: number;
  rx: number;
  ry: number;
}

/** Where a stamp sits on screen, and how far it reaches from there. */
function stampBox(
  camera: Camera,
  view: Viewport,
  lon: number,
  lat: number,
  radiusKm: number,
  space: StampSpace,
  insetPx: number,
): StampBox {
  const point = project(camera, view, { lon, lat });
  const full = footprintRadii(camera, lat, radiusKm, space);
  // Inset in *screen* pixels, not in kilometres: it exists to draw a band of a
  // constant width along a footprint's edge, and a band is a screen measure.
  // Never past nothing — a stamp smaller than the inset shrinks to a point
  // rather than turning inside out.
  return {
    x: point.x,
    y: point.y,
    rx: Math.max(0, full.rx - insetPx),
    ry: Math.max(0, full.ry - insetPx),
  };
}

/**
 * Joins two square stamps with the region the stamp sweeps between them (M58).
 *
 * A round stamp's union along a stroke is scalloped, and the scallop shrinks as
 * the square of the spacing — which is what {@link SCALLOP_PX} bounds. A
 * *square* stamp's union is not: two axis-aligned squares a few pixels apart on
 * a diagonal overlap only near their corners, so the edge comes out as a
 * staircase whose step is the whole spacing. Stamping thickly enough to hide it
 * would need a stamp every half pixel.
 *
 * So the gap is filled rather than sampled away. The region a rectangle sweeps
 * along a segment is the convex hull of the rectangle at each end — a hexagon —
 * and drawing that makes the preview's edge exact between the samples instead
 * of merely dense. It is also what the field itself does: `swept_square_distance`
 * measures the Chebyshev distance to the *segment*, whose unit ball is the
 * stamp, so the object has always had the straight edge the preview lacked.
 *
 * The two end stamps are drawn as well, so a hull that clips a corner when the
 * two boxes differ in size — they do, slightly, since a geodesic stamp is wider
 * further from the equator — cannot uncover any part of either square.
 */
function addSquareJoin(sink: PathSink, from: StampBox, to: StampBox): void {
  const sx = to.x >= from.x ? 1 : -1;
  const sy = to.y >= from.y ? 1 : -1;
  const corners: Array<[number, number]> = [
    [from.x - sx * from.rx, from.y - sy * from.ry],
    [from.x + sx * from.rx, from.y - sy * from.ry],
    [to.x + sx * to.rx, to.y - sy * to.ry],
    [to.x + sx * to.rx, to.y + sy * to.ry],
    [to.x - sx * to.rx, to.y + sy * to.ry],
    [from.x - sx * from.rx, from.y + sy * from.ry],
  ];

  // Wound the way `rect` winds, always. The subpaths of one footprint are
  // filled together under the non-zero rule, so a piece traced the other way
  // round would cancel against the stamps it overlaps and punch a hole through
  // the stroke. Whether this order comes out clockwise depends on which way the
  // segment runs, so it is measured rather than reasoned about: the shoelace
  // sum is positive for a clockwise ring in screen coordinates, where y grows
  // downward.
  let area = 0;
  for (let i = 0; i < corners.length; i++) {
    const [ax, ay] = corners[i]!;
    const [bx, by] = corners[(i + 1) % corners.length]!;
    area += ax * by - bx * ay;
  }
  if (area < 0) corners.reverse();

  const [first, ...rest] = corners;
  if (first === undefined) return;
  sink.moveTo(first[0], first[1]);
  for (const [x, y] of rest) sink.lineTo(x, y);
  sink.closePath();
}

/** Most footprints one stroke preview will draw. */
const MAX_FOOTPRINTS = 4000;

/**
 * How far the edge of a swept footprint may fall inside the true silhouette,
 * in screen pixels (M33).
 *
 * The union of discs along a stroke is scalloped by however far apart their
 * centres are; under half a pixel the scallop is beneath what the screen can
 * show, and the stamps stay few enough that a long stroke is still cheap.
 */
const SCALLOP_PX = 0.4;

/**
 * How far along a stroke its swept path has been built.
 *
 * A stroke in progress gains a point per pointer report, and rebuilding the
 * whole swept path each time is quadratic in the stroke's length — a thousand
 * points is a million stamps over the drag. The path is built by extension
 * instead: the stamps already added stay, and only the new segment is walked.
 * This is the state that makes the extension exact.
 */
export interface SweptPathProgress {
  /** How many of the stroke's points have been walked. */
  done: number;
  /** Stamps added so far, against the cap. */
  drawn: number;
}

/** The state of a swept path with nothing built yet. */
export function freshSweptPath(): SweptPathProgress {
  return { done: 0, drawn: 0 };
}

/**
 * Builds the swept region of a stroke.
 *
 * Every stamp is added as its own subpath, so filling once merges them into a
 * single silhouette rather than drawing each outline over the others.
 *
 * **Footprints are interpolated along each segment.** The stored stroke is
 * thinned — a capsule chain sweeps continuously between its points, so the
 * *field* needs no more than that. The preview does: stamping only the recorded
 * points leaves visible gaps whenever a drag outruns the pointer sample rate,
 * which reads as a broken brush even though what gets painted is solid.
 *
 * The whole stroke from nothing. {@link extendStrokePath} is the same walk from
 * wherever it left off, and this is that walk started at zero — so the two
 * cannot disagree about what a stroke looks like.
 */
export function buildStrokePath(
  sink: PathSink,
  camera: Camera,
  view: Viewport,
  points: ReadonlyArray<readonly [number, number]>,
  radiusKm: number,
  shape: BrushShape = "circle",
  space: StampSpace = "geodesic",
  insetPx = 0,
): void {
  extendStrokePath(
    sink,
    camera,
    view,
    points,
    freshSweptPath(),
    radiusKm,
    shape,
    space,
    insetPx,
  );
}

/**
 * Adds to a swept path the stamps for the points walked since `progress`.
 *
 * Exact, not approximate: a stamp depends only on the segment it lies on, and
 * the cap on stamps is carried in `progress`, so extending point by point adds
 * the same stamps in the same order as building from scratch. The caller keeps
 * `sink` and `progress` together and passes the same stroke back, longer.
 */
export function extendStrokePath(
  sink: PathSink,
  camera: Camera,
  view: Viewport,
  points: ReadonlyArray<readonly [number, number]>,
  progress: SweptPathProgress,
  radiusKm: number,
  shape: BrushShape = "circle",
  space: StampSpace = "geodesic",
  insetPx = 0,
): void {
  // A square stamp is joined to the one before it rather than relying on the
  // two overlapping (M58); a round one needs no join, its scallop being what
  // the spacing already bounds.
  const joins = shape === "square" && !projectionFor(camera).general;
  const stamp = (lon: number, lat: number, previous?: readonly [number, number]) => {
    if (progress.drawn >= MAX_FOOTPRINTS) return;
    if (joins && previous !== undefined) {
      addSquareJoin(
        sink,
        stampBox(camera, view, previous[0], previous[1], radiusKm, space, insetPx),
        stampBox(camera, view, lon, lat, radiusKm, space, insetPx),
      );
    }
    addFootprint(sink, camera, view, lon, lat, radiusKm, shape, space, insetPx);
    progress.drawn += 1;
  };

  if (progress.done === 0) {
    const first = points[0];
    if (first === undefined) return;
    stamp(first[0], first[1]);
    progress.done = 1;
  }

  for (let i = progress.done; i < points.length; i++) {
    const from = points[i - 1]!;
    const to = points[i]!;

    // Longitude is stepped by the shorter way round, so a stroke across the
    // dateline interpolates through it rather than back across the world.
    const dLon = normalizeLon(to[0] - from[0]);
    const dLat = to[1] - from[1];

    // Stamps close enough that the scallop between them is under half a
    // pixel (M33). Half a radius apart — what this was — leaves a sagitta of
    // r/32, which is invisible on a small brush and a three-pixel bite out of
    // the edge of a large one: the arcs the user could see along a stroke.
    // The spacing that bounds the sagitta at `s` is 2*sqrt(2rs - s^2), so it
    // grows with the square root of the radius rather than with the radius.
    //
    // A square stamp needs none of this — its samples are joined exactly
    // (M58) — but it keeps the same walk, since the spacing also bounds how
    // far the interpolated polyline departs from the projected curve, which
    // both stamps care about.
    const { ry } = footprintRadii(camera, from[1], radiusKm);
    const a = project(camera, view, {lon:from[0],lat:from[1]}), b = project(camera,view,{lon:to[0],lat:to[1]});
    const spanPx = projectionFor(camera).general && Number.isFinite(a.x) && Number.isFinite(b.x)
      ? Math.hypot(a.x-b.x,a.y-b.y) : Math.hypot(dLon * camera.pxPerDeg, dLat * camera.pxPerDeg);
    const stride = Math.max(1, 2 * Math.sqrt(Math.max(2 * ry * SCALLOP_PX - SCALLOP_PX ** 2, 0)));
    const steps = Math.max(1, Math.ceil(spanPx / stride));

    let previous = from;
    for (let step = 1; step <= steps; step++) {
      const t = step / steps;
      const at: readonly [number, number] = [
        normalizeLon(from[0] + dLon * t),
        from[1] + dLat * t,
      ];
      stamp(at[0], at[1], previous);
      previous = at;
    }
    progress.done = i + 1;
  }
}

/**
 * The region a gesture will paint, in geographic terms.
 *
 * The preview, the hover indicator and the drag outline all draw *this* rather
 * than each knowing what any tool is: a new tool supplies its footprint and
 * inherits the whole preview path, which is what spec 6.1 means by the gesture
 * previewing the field. The variants are geometries, not tools — the brush, the
 * mask and the clone stamp all produce a `swept` one, and that is precisely
 * why they cannot drift apart.
 *
 * Rotation is absent on purpose: a footprint is only ever previewed at the
 * moment it is drawn, and an object's rotation starts at zero. A committed
 * object's outline comes from the backend (`ObjectOutline`), which does carry
 * the frame.
 */
export type Footprint =
  | {
      kind: "swept";
      /** The polyline the stamp is swept along, as `[lon, lat]`. */
      points: ReadonlyArray<readonly [number, number]>;
      /** Half the stamp's size: a disc's radius or a square's half-side. */
      radiusKm: number;
      shape: BrushShape;
      space: StampSpace;
    }
  | {
      kind: "disc";
      centre: readonly [number, number];
      radiusKm: number;
      space: StampSpace;
    }
  | {
      kind: "ring";
      centre: readonly [number, number];
      /** Radius of the ring's centreline. */
      radiusKm: number;
      /** Half the ring's thickness. */
      halfWidthKm: number;
      space: StampSpace;
    }
  | {
      kind: "rect";
      centre: readonly [number, number];
      halfWidthKm: number;
      halfHeightKm: number;
      space: StampSpace;
    }
  | {
      kind: "polygon";
      /** Vertices in order, as `[lon, lat]`. The closing edge is implied. */
      points: ReadonlyArray<readonly [number, number]>;
    };

/**
 * A backend outline as a footprint the map can draw.
 *
 * `ObjectOutline` is what the evaluator's own shape looks like lifted into
 * geographic coordinates, and its `radius_km` is the stamp's **radius** — the
 * same half-extent `Footprint` carries, and the same one a square stamp
 * measures from its centre. It is passed through, not halved: halving it drew
 * every outline at half the size of the object it was outlining, which reads as
 * an outline sitting well inside the paint it belongs to.
 *
 * Its own function so that the two places that draw one — the selected and
 * hovered edges, and a drag's outlines — cannot come to disagree about that,
 * and so the conversion can be tested against the backend's contract.
 */
export function footprintOfOutline(outline: ObjectOutlineShape): Footprint[] {
  if (outline.kind === "contours") return outline.rings.map((points) => ({ kind: "polygon", points }));
  if (outline.kind === "ring") {
    return [{ kind: "polygon", points: outline.points }];
  }
  return outline.chains.map((chain) => ({
    kind: "swept",
    points: chain,
    radiusKm: outline.radius_km,
    shape: outline.square ? "square" : "circle",
    space: outline.space,
  }));
}

/**
 * The shape of a backend outline, as much of it as this needs.
 *
 * Structural rather than the generated type, so `footprint.ts` stays free of
 * the IPC bindings and can be tested without them.
 */
export type ObjectOutlineShape =
  | {
      kind: "swept";
      chains: ReadonlyArray<ReadonlyArray<readonly [number, number]>>;
      radius_km: number;
      square: boolean;
      space: StampSpace;
    }
  | { kind: "contours"; rings: ReadonlyArray<ReadonlyArray<readonly [number, number]>> }
  | { kind: "ring"; points: ReadonlyArray<readonly [number, number]> };

/**
 * Traces a footprint onto a path.
 *
 * Every closed piece opens its own subpath, so one `fill` merges them into a
 * single silhouette. That is what makes a ring a ring: the outer and inner
 * circles are wound the same way and the non-zero fill rule leaves the hole,
 * which is the same trick the evaluator's annulus distance performs
 * arithmetically.
 *
 * `insetPx` shrinks every stamp by that many screen pixels. It exists for the
 * one thing a *stroke* of this path cannot do: draw the outline of the union.
 * A footprint is a union of stamps and `Path2D` has no union operator, so
 * stroking it traces every stamp's own circle; filling it and then knocking out
 * an inset fill of the same path leaves a band along the union's true edge and
 * nothing inside it (spec.md 6.1).
 */
export function buildFootprintPath(
  sink: PathSink,
  camera: Camera,
  view: Viewport,
  footprint: Footprint,
  insetPx = 0,
): void {
  switch (footprint.kind) {
    case "swept":
      buildStrokePath(
        sink,
        camera,
        view,
        footprint.points,
        footprint.radiusKm,
        footprint.shape,
        footprint.space,
        insetPx,
      );
      return;

    case "disc": {
      const [lon, lat] = footprint.centre;
      addFootprint(
        sink,
        camera,
        view,
        lon,
        lat,
        footprint.radiusKm,
        "circle",
        footprint.space,
        insetPx,
      );
      return;
    }

    case "ring": {
      const [lon, lat] = footprint.centre;
      // Drawn as an annulus rather than a thick stroke: a stroked ellipse has a
      // uniform *screen* thickness, and a geodesic ring's thickness is a ground
      // distance that projects wider east-west like everything else.
      const outer = footprint.radiusKm + footprint.halfWidthKm;
      const inner = Math.max(0, footprint.radiusKm - footprint.halfWidthKm);
      addFootprint(sink, camera, view, lon, lat, outer, "circle", footprint.space);
      if (inner > 0) {
        addFootprint(sink, camera, view, lon, lat, inner, "circle", footprint.space);
      }
      return;
    }

    case "rect": {
      const [lon, lat] = footprint.centre;
      if (projectionFor(camera).general) {
        projectedStamp(sink,camera,view,lon,lat,footprint.halfWidthKm,footprint.halfHeightKm,"square",footprint.space,insetPx);
        return;
      }
      const point = project(camera, view, { lon, lat });
      const { rx } = footprintRadii(camera, lat, footprint.halfWidthKm, footprint.space);
      const { ry } = footprintRadii(camera, lat, footprint.halfHeightKm, footprint.space);
      sink.rect(point.x - rx, point.y - ry, rx * 2, ry * 2);
      return;
    }

    case "polygon": {
      if (projectionFor(camera).general) { projectedRing(sink,camera,view,footprint.points); return; }
      const [first, ...rest] = footprint.points;
      if (first === undefined) return;
      const start = project(camera, view, { lon: first[0], lat: first[1] });
      sink.moveTo(start.x, start.y);
      for (const [lon, lat] of rest) {
        const point = project(camera, view, { lon, lat });
        sink.lineTo(point.x, point.y);
      }
      sink.closePath();
      return;
    }
  }
}

/**
 * A point inside the footprint, for the one glyph drawn when the lattice covers
 * none (spec.md 6.1).
 *
 * The head of a stroke, or the centre of anything with one. A preview showing
 * no direction at all is worse than one glyph off the lattice.
 */
export function footprintHead(footprint: Footprint): readonly [number, number] | null {
  switch (footprint.kind) {
    case "swept":
      return footprint.points[footprint.points.length - 1] ?? null;
    case "disc":
    case "rect":
      return footprint.centre;
    // The centre of a ring is the hole, so a glyph there would sit outside the
    // shape. The top of the centreline is on it.
    case "ring":
      return [
        footprint.centre[0],
        footprint.centre[1] + footprint.radiusKm / KM_PER_DEGREE,
      ];
    case "polygon": {
      const n = footprint.points.length;
      if (n === 0) return null;
      // The mean of the vertices. Outside a sufficiently concave polygon, which
      // is a worse glyph position than a vertex but never a missing one.
      let lon = 0;
      let lat = 0;
      for (const point of footprint.points) {
        lon += normalizeLon(point[0] - footprint.points[0]![0]) / n;
        lat += point[1] / n;
      }
      return [normalizeLon(footprint.points[0]![0] + lon), lat];
    }
  }
}

/** Project an outline as short curved segments, breaking at horizons and seams. */
export function projectedRing(sink: PathSink, camera: Camera, view: Viewport, points: ReadonlyArray<readonly [number,number]>): void {
  let drawing = false, complete = true;
  let previous: {x:number;y:number} | null = null;
  for (let i=0;i<points.length;i++) {
    const a=points[i]!,b=points[(i+1)%points.length]!;
    const dx=normalizeLon(b[0]-a[0]),dy=b[1]-a[1];
    const steps=Math.min(512,Math.max(1,Math.ceil(Math.hypot(dx,dy)*camera.pxPerDeg/8)));
    for(let j=0;j<steps;j++) {
      const point=project(camera,view,{lon:a[0]+dx*j/steps,lat:a[1]+dy*j/steps});
      if(!Number.isFinite(point.x)||!Number.isFinite(point.y)) {drawing=false;complete=false;previous=null;continue;}
      if(previous && Math.hypot(point.x-previous.x,point.y-previous.y)>Math.hypot(view.width,view.height)/2) {drawing=false;complete=false;}
      if(drawing)sink.lineTo(point.x,point.y);else sink.moveTo(point.x,point.y);
      drawing=true;previous=point;
    }
  }
  if(complete && drawing)sink.closePath();
}

/** Stored footprint frames stay fixed when the viewing projection changes. */
function projectedStamp(sink: PathSink,camera: Camera,view: Viewport,lon:number,lat:number,rx:number,ry:number,shape:BrushShape,space:StampSpace,inset:number):void {
  if (projectionFor(camera).general && space !== "geodesic") {
    const centre = project(camera, view, {lon, lat});
    if (!Number.isFinite(centre.x) || !Number.isFinite(centre.y)) return;
    const pxRx = Math.max(0, rx / KM_PER_DEGREE * camera.pxPerDeg - inset);
    const pxRy = Math.max(0, ry / KM_PER_DEGREE * camera.pxPerDeg - inset);
    const points:Array<[number,number]> = [];
    for (let i=0;i<96;i++) {
      const angle=i/96*2*Math.PI;
      let x=Math.sin(angle), y=Math.cos(angle);
      if(shape==='square'){const divisor=Math.max(Math.abs(x),Math.abs(y));x/=divisor;y/=divisor;}
      const geo=unproject(camera,view,{x:centre.x+x*pxRx,y:centre.y-y*pxRy});
      if (validGeo(geo)) points.push([geo.lon,geo.lat]);
    }
    if (points.length > 1) projectedRing(sink,camera,view,points);
    return;
  }
  const scale=footprintRadii(camera,lat,1,"geodesic",lon),shrink=inset/Math.max(1e-8,Math.min(scale.rx,scale.ry));
  rx=Math.max(0,rx-shrink);ry=Math.max(0,ry-shrink);
  const source=stampProjection(space),points:Array<[number,number]>=[];
  const count=96;
  for(let i=0;i<count;i++) {
    const angle=i/count*2*Math.PI;
    let x=Math.sin(angle),y=Math.cos(angle);
    if(shape==='square'){const divisor=Math.max(Math.abs(x),Math.abs(y));x/=divisor;y/=divisor;}
    x*=rx;y*=ry;
    if(space==='geodesic'){
      const p=destination({lon,lat},Math.atan2(x,y)*180/Math.PI,Math.hypot(x,y)*1000);points.push([p.lon,p.lat]);
    }else points.push([lon+x/KM_PER_DEGREE,source.latOf(source.yOf(lat)+y/KM_PER_DEGREE)]);
  }
  projectedRing(sink,camera,view,points);
}

/**
 * Brush footprint geometry for the tool overlay.
 *
 * Extracted from the drawing code so the path construction can be tested: the
 * bug it exists to prevent is invisible to a type checker and only shows up as
 * a strange shape on screen.
 */

import type { BrushShape } from "../generated/BrushShape";
import type { StampSpace } from "../generated/StampSpace";
import { type Camera, type Viewport, normalizeLon, project } from "./camera";
import { EARTH_RADIUS_M } from "./geo";

/**
 * Kilometres per degree of latitude. Constant in equirectangular.
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

/** The part of `CanvasRenderingContext2D` this module needs. */
export interface PathSink {
  moveTo(x: number, y: number): void;
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
 * `radiusKm` is the north-south ground extent in both spaces, which is the one
 * axis the projection leaves alone; the spaces differ only in how far the stamp
 * reaches east-west.
 */
export function footprintRadii(
  camera: Camera,
  lat: number,
  radiusKm: number,
  space: StampSpace = "geodesic",
): { rx: number; ry: number } {
  const ry = (radiusKm / KM_PER_DEGREE) * camera.pxPerDeg;
  // A circle on the ground spans more longitude the further from the equator,
  // so it draws as an ellipse. A projected stamp is defined on the map instead
  // and stays a circle at every latitude — which is what a size in pixels is
  // asking for (spec.md 3.5).
  // Clamped so a geodesic footprint near a pole stays finite rather than
  // filling the map.
  const rx = space === "projected" ? ry : ry / cosLat(lat);
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
): number {
  const scale = space === "projected" ? 1 : cosLat(lat);
  return (pixels / camera.pxPerDeg) * KM_PER_DEGREE * scale;
}

/** The inverse of {@link kmFromPixels}, for showing the current size in px. */
export function pixelsFromKm(
  camera: Camera,
  lat: number,
  km: number,
  space: StampSpace = "geodesic",
): number {
  const scale = space === "projected" ? 1 : cosLat(lat);
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
): void {
  const point = project(camera, view, { lon, lat });
  const { rx, ry } = footprintRadii(camera, lat, radiusKm, space);
  if (shape === "square") {
    sink.rect(point.x - rx, point.y - ry, rx * 2, ry * 2);
    return;
  }
  sink.moveTo(point.x + rx, point.y);
  sink.ellipse(point.x, point.y, rx, ry, 0, 0, Math.PI * 2);
}

/** Most footprints one stroke preview will draw. */
const MAX_FOOTPRINTS = 4000;

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
 */
export function buildStrokePath(
  sink: PathSink,
  camera: Camera,
  view: Viewport,
  points: ReadonlyArray<readonly [number, number]>,
  radiusKm: number,
  shape: BrushShape = "circle",
  space: StampSpace = "geodesic",
): void {
  let drawn = 0;
  const stamp = (lon: number, lat: number) => {
    if (drawn >= MAX_FOOTPRINTS) return;
    addFootprint(sink, camera, view, lon, lat, radiusKm, shape, space);
    drawn += 1;
  };

  const first = points[0];
  if (first === undefined) return;
  stamp(first[0], first[1]);

  for (let i = 1; i < points.length; i++) {
    const from = points[i - 1]!;
    const to = points[i]!;

    // Longitude is stepped by the shorter way round, so a stroke across the
    // dateline interpolates through it rather than back across the world.
    const dLon = normalizeLon(to[0] - from[0]);
    const dLat = to[1] - from[1];

    // Half a radius between stamps keeps the union solid without drawing far
    // more than the fill needs.
    const { ry } = footprintRadii(camera, from[1], radiusKm);
    const spanPx = Math.hypot(dLon * camera.pxPerDeg, dLat * camera.pxPerDeg);
    const steps = Math.max(1, Math.ceil(spanPx / Math.max(ry * 0.5, 1)));

    for (let step = 1; step <= steps; step++) {
      const t = step / steps;
      stamp(normalizeLon(from[0] + dLon * t), from[1] + dLat * t);
    }
  }
}


/**
 * The region a gesture will paint, in geographic terms.
 *
 * The preview, the hover indicator and the drag outline all draw *this* rather
 * than each knowing what any tool is: a new tool supplies its footprint and
 * inherits the whole preview path, which is what spec 6.1 means by the gesture
 * previewing the field. The variants are geometries, not tools — the brush, the
 * eraser and the clone stamp all produce a `swept` one, and that is precisely
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

/** The part of `Path2D` a polygon needs beyond {@link PathSink}. */
export interface PolygonSink extends PathSink {
  lineTo(x: number, y: number): void;
  closePath(): void;
}

/**
 * Traces a footprint onto a path.
 *
 * Every closed piece opens its own subpath, so one `fill` merges them into a
 * single silhouette. That is what makes a ring a ring: the outer and inner
 * circles are wound the same way and the non-zero fill rule leaves the hole,
 * which is the same trick the evaluator's annulus distance performs
 * arithmetically.
 */
export function buildFootprintPath(
  sink: PolygonSink,
  camera: Camera,
  view: Viewport,
  footprint: Footprint,
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
      );
      return;

    case "disc": {
      const [lon, lat] = footprint.centre;
      addFootprint(sink, camera, view, lon, lat, footprint.radiusKm, "circle", footprint.space);
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
      const point = project(camera, view, { lon, lat });
      const { rx } = footprintRadii(camera, lat, footprint.halfWidthKm, footprint.space);
      const { ry } = footprintRadii(camera, lat, footprint.halfHeightKm, footprint.space);
      sink.rect(point.x - rx, point.y - ry, rx * 2, ry * 2);
      return;
    }

    case "polygon": {
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

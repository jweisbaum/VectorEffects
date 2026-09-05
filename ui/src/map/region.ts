/**
 * A selected region of the map (spec.md 8.2, M14).
 *
 * **This picks ground, not objects.** The object selection the hand tool makes
 * picks *things*; a region picks an area, and what is made from it — a fill, a
 * patch — takes the region's shape as its own geometry.
 *
 * A region is **in map space**. It is drawn on the map; `Cmd`-`A` means "the
 * view", which is only meaningful as a map rectangle; and a circle dragged on
 * screen should be round on screen at every latitude. So anything built from a
 * region takes `stamp_space: projected`, exactly as a px-sized stamp does
 * (D28, D55) — a region is not a ground shape that happens to be drawn.
 *
 * Session state, not document and not history: it is a way of pointing, and
 * pointing is not an edit.
 */

import type { RegionShape } from "../generated/RegionShape";
import { type Camera, normalizeLon, visibleBounds } from "./camera";

/** How a drag with the select tool draws its region. */
export type RegionMode = "rect" | "circle" | "lasso";

/**
 * A region, in degrees.
 *
 * The rectangle and the circle are centre-and-extent rather than corner pairs,
 * because that is what the shape fill's presets already are (D33) and what the
 * object built from one will hold.
 */
export type Region =
  | { kind: "rect"; centre: [number, number]; halfWidthDeg: number; halfHeightDeg: number }
  | { kind: "disc"; centre: [number, number]; radiusDeg: number }
  | { kind: "polygon"; points: Array<[number, number]> };

/** The smallest drag that counts as a region, in degrees of latitude. */
const MIN_EXTENT_DEG = 1e-4;

/**
 * The region a drag describes.
 *
 * A rectangle is drawn corner to corner, the way every marquee is; a circle is
 * dragged **from its centre**, as the shape fill's presets are, so the two
 * tools describe a disc the same way (D33). `null` for a drag too small to be
 * a region, which is what a click is.
 */
export function regionFromDrag(
  mode: Exclude<RegionMode, "lasso">,
  from: readonly [number, number],
  to: readonly [number, number],
): Region | null {
  const dLon = normalizeLon(to[0] - from[0]);
  const dLat = to[1] - from[1];
  if (mode === "circle") {
    const radiusDeg = Math.hypot(dLon, dLat);
    if (radiusDeg < MIN_EXTENT_DEG) return null;
    return { kind: "disc", centre: [from[0], from[1]], radiusDeg };
  }
  const halfWidthDeg = Math.abs(dLon) / 2;
  const halfHeightDeg = Math.abs(dLat) / 2;
  if (halfWidthDeg < MIN_EXTENT_DEG && halfHeightDeg < MIN_EXTENT_DEG) return null;
  return {
    kind: "rect",
    centre: [normalizeLon(from[0] + dLon / 2), from[1] + dLat / 2],
    halfWidthDeg,
    halfHeightDeg,
  };
}

/**
 * The region a lasso describes: its own points, closed on release.
 *
 * A polygon needs three vertices to have an inside; below that the drag was a
 * click and there is no region.
 */
export function regionFromLasso(points: ReadonlyArray<readonly [number, number]>): Region | null {
  if (points.length < 3) return null;
  return { kind: "polygon", points: points.map((p) => [p[0], p[1]]) };
}

/**
 * The visible viewport as a region — what `Cmd`-`A` selects.
 *
 * Clamped in latitude, because the camera can show past the poles and there is
 * no ground there to select. Not clamped in longitude: a viewport wider than
 * the world is possible at the lowest zoom, and [`wholeMap`] is what that
 * means.
 */
export function regionOfView(camera: Camera, width: number, height: number): Region {
  const halfWidthDeg = width / 2 / camera.pxPerDeg;
  if (halfWidthDeg >= 180) return wholeMap();
  // The vertical edges are found in the projection's own coordinate and read
  // back as latitudes: half a viewport is a different band of latitude in each
  // projection, and the region is a band of latitude (M11).
  const bounds = visibleBounds(camera, { width, height });
  const north = Math.min(90, bounds.north);
  const south = Math.max(-90, bounds.south);
  return {
    kind: "rect",
    centre: [normalizeLon(camera.centerLon), (north + south) / 2],
    halfWidthDeg,
    halfHeightDeg: (north - south) / 2,
  };
}

/**
 * The same region, centred somewhere else (spec.md 8.7, M16).
 *
 * A capture's region is placed per frame, and the map has to draw it where the
 * frame it is showing puts it. Only the position changes: the shape is fixed
 * once the capture starts and cannot be reshaped while it records.
 */
export function recentred(region: Region, lon: number, lat: number): Region {
  switch (region.kind) {
    case "rect":
    case "disc":
      return { ...region, centre: [lon, lat] };
    case "polygon": {
      // A polygon has no centre of its own, so it moves by the difference from
      // its **bounding-box** centre — which is what `RegionShape::anchor`
      // computes on the other side of the wire, and therefore the point the
      // backend is placing. The mean of the vertices would be a different
      // point, and a dense corner would drag the region off the pointer.
      const anchor = polygonAnchor(region.points);
      if (anchor === null) return region;
      const dLon = lon - anchor[0];
      const dLat = lat - anchor[1];
      return {
        kind: "polygon",
        points: region.points.map((point) => [point[0] + dLon, point[1] + dLat]),
      };
    }
  }
}

/**
 * A polygon's anchor: the centre of its bounding box.
 *
 * Longitudes are carried the short way from the first point, so a ring across
 * the seam keeps its shape instead of spanning the world. The port of
 * `RegionShape::anchor`, decision for decision.
 */
function polygonAnchor(points: ReadonlyArray<readonly [number, number]>): [number, number] | null {
  const first = points[0];
  if (first === undefined) return null;
  let running = first[0];
  let west = first[0];
  let east = first[0];
  let south = first[1];
  let north = first[1];
  for (const point of points) {
    running += normalizeLon(point[0] - running);
    west = Math.min(west, running);
    east = Math.max(east, running);
    south = Math.min(south, point[1]);
    north = Math.max(north, point[1]);
  }
  return [(west + east) / 2, (south + north) / 2];
}

/** The whole earth — what `Cmd`-`Shift`-`A` selects. */
export function wholeMap(): Region {
  return { kind: "rect", centre: [0, 0], halfWidthDeg: 180, halfHeightDeg: 90 };
}

/**
 * A region's outline as a closed ring of positions, for drawing.
 *
 * One representation for all three kinds, so the overlay has one path to
 * stroke and the marching ants have one thing to run along. The circle is
 * flattened at a fixed count: it is a map-space circle, so its outline is an
 * ellipse in no projection but this one, and 64 segments is smooth at any zoom
 * a region is drawn at.
 */
export function regionRing(region: Region): Array<[number, number]> {
  switch (region.kind) {
    case "polygon":
      return region.points.map((p) => [p[0], p[1]]);
    case "rect": {
      const [lon, lat] = region.centre;
      const { halfWidthDeg: w, halfHeightDeg: h } = region;
      return [
        [lon - w, lat - h],
        [lon + w, lat - h],
        [lon + w, lat + h],
        [lon - w, lat + h],
      ];
    }
    case "disc": {
      const [lon, lat] = region.centre;
      const steps = 64;
      return Array.from({ length: steps }, (_, k) => {
        const t = (k / steps) * Math.PI * 2;
        return [lon + region.radiusDeg * Math.cos(t), lat + region.radiusDeg * Math.sin(t)] as [
          number,
          number,
        ];
      });
    }
  }
}

/**
 * The region's bounding box in degrees, which is what a capture is taken over.
 *
 * Longitudes come back **unwrapped**, running east from the west edge, so a
 * region across the antimeridian keeps its width instead of becoming the 340°
 * that is not selected — the same rule `marquee.ts` states for a rubber band.
 */
export function regionBounds(region: Region): {
  west: number;
  south: number;
  east: number;
  north: number;
} {
  const ring = regionRing(region);
  const first = ring[0] ?? ([0, 0] as [number, number]);
  let west = first[0];
  let east = first[0];
  let south = first[1];
  let north = first[1];
  // Longitudes are carried forward the short way from the previous point, so a
  // ring that crosses the seam accumulates rather than jumping 360.
  let running = first[0];
  for (const [lon, lat] of ring) {
    running += normalizeLon(lon - running);
    west = Math.min(west, running);
    east = Math.max(east, running);
    south = Math.min(south, lat);
    north = Math.max(north, lat);
  }
  return { west, south, east, north };
}

/** Whether a position is inside the region. */
export function regionContains(region: Region, lon: number, lat: number): boolean {
  switch (region.kind) {
    case "rect": {
      const dLon = Math.abs(normalizeLon(lon - region.centre[0]));
      return dLon <= region.halfWidthDeg && Math.abs(lat - region.centre[1]) <= region.halfHeightDeg;
    }
    case "disc": {
      const dLon = normalizeLon(lon - region.centre[0]);
      return Math.hypot(dLon, lat - region.centre[1]) <= region.radiusDeg;
    }
    case "polygon": {
      // Even-odd crossing count, in map space, with longitudes unwrapped
      // relative to the test point so the seam is not a boundary.
      let inside = false;
      const pts = region.points;
      for (let i = 0, j = pts.length - 1; i < pts.length; j = i++) {
        const a = pts[i]!;
        const b = pts[j]!;
        const ax = normalizeLon(a[0] - lon);
        const bx = normalizeLon(b[0] - lon);
        const ay = a[1] - lat;
        const by = b[1] - lat;
        if (ay > 0 !== by > 0 && 0 < ((bx - ax) * (0 - ay)) / (by - ay) + ax) {
          inside = !inside;
        }
      }
      return inside;
    }
  }
}

/**
 * The tools that make an object from a selected region on one click
 * (spec.md 8.2).
 *
 * The brush and the four operators: with a region selected, clicking inside
 * it makes that kind of object from the region's boundary — a brush paints the
 * region, a mask masks it, and so on. The clone stamp and the warp are left
 * out on purpose: both are measured from their anchor to somewhere *else*,
 * and "the region" does not say where. The shape fill keeps its own presets
 * and is drawn, not applied.
 */
export const REGION_EDIT_TOOLS = ["brush", "mask", "intensity", "divergence", "turn"] as const;

/** Whether a tool makes its object from a selected region on a click inside it. */
export function editsRegion(tool: string): tool is (typeof REGION_EDIT_TOOLS)[number] {
  return (REGION_EDIT_TOOLS as readonly string[]).includes(tool);
}

/**
 * A region as the gesture the brush and the operators take (spec.md 8.2).
 *
 * A rectangle goes as a ring of its four corners and a polygon as itself; only
 * a circle goes as an extent. None of these tools has a `shape_source` to say
 * what an extent means, so on their side an extent can only mean a disc — and
 * a rectangle as four corners is exact anyway.
 */
export function regionGesture(region: Region): RegionGesture {
  switch (region.kind) {
    case "rect":
    case "polygon":
      return { kind: "ring", points: regionRing(region) };
    case "disc":
      return {
        kind: "extent",
        centre: [region.centre[0], region.centre[1]],
        rim: [region.centre[0], region.centre[1] + region.radiusDeg],
      };
  }
}

/** The two gesture shapes a region can become. */
export type RegionGesture =
  | { kind: "extent"; centre: [number, number]; rim: [number, number] }
  | { kind: "ring"; points: Array<[number, number]> };

/**
 * The region as the backend's `RegionShape`, for a capture (spec.md 8.5).
 *
 * The wire shape is the region itself — degrees, map space — because that is
 * what it is. The backend turns it into the patch's geometry and its lattice;
 * nothing here decides either.
 */
export function regionShape(region: Region): RegionShape {
  switch (region.kind) {
    case "rect":
      return {
        kind: "rect",
        centre: [region.centre[0], region.centre[1]],
        half_width_deg: region.halfWidthDeg,
        half_height_deg: region.halfHeightDeg,
      };
    case "disc":
      return {
        kind: "disc",
        centre: [region.centre[0], region.centre[1]],
        radius_deg: region.radiusDeg,
      };
    case "polygon":
      return { kind: "polygon", points: region.points.map((p) => [p[0], p[1]]) };
  }
}

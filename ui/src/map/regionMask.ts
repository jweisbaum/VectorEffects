/**
 * Where a regional project's region lies on the screen (spec 5.1, R8): the
 * rings the overlay fills the exterior around, even-odd against the window,
 * and strokes as the region's edge.
 *
 * Built per camera, because the camera is what moves it, and so built cheaply:
 * a cylindrical map's region is a rectangle in screen space and is computed as
 * one; a general projection's is walked from `regionRings`, subdivided only
 * where a segment is on or near the window. `projectedRing` subdivides every
 * segment to eight pixels, which zoomed into a globe is tens of thousands of
 * projections per frame for an outline almost all of which is off screen.
 */

import type { ProjectRegion } from "../generated/ProjectRegion";
import {
  normalizeLon,
  project,
  projectionFor,
  type Camera,
  type GeoPoint,
  type ScreenPoint,
  type Viewport,
} from "./camera";
import { regionRings } from "./extent";

/** The region on screen. */
export interface RegionMask {
  /** Screen polylines, device pixels as the camera's view is. */
  rings: ScreenPoint[][];
  /**
   * Whether every ring is closed and whole, so an even-odd fill of the window
   * and the rings is the exterior. A ring cut by a globe's horizon or a fixed
   * map's seam is not, and filling it would join its ends with a chord: such a
   * region is outlined and not dimmed.
   */
  closed: boolean;
}

/** How far past the window a cylindrical rectangle is carried, so its clipped
 * sides are never stroked on screen. */
const MARGIN = 4;
/** The screen length of a subdivided step near the window. */
const STEP_PX = 8;

/** The region of a camera, on screen, or `null` for a global project. */
export function regionMask(camera: Camera, view: Viewport): RegionMask | null {
  const region = camera.region;
  if (!region) return null;
  const general = projectionFor(camera).general;
  if (!general) return cylindricalMask(camera, view, region);
  // A movable map is centred inside the region, so a cap is a circle about
  // its pole and a band two. A fixed map draws a full circle as the box it is
  // in the plane, its pole line included; the east edge stops short of 180 so
  // that a map whose seam is there does not fold it onto the west.
  const rings = general.movable || !region.full_circle
    ? regionRings(region)
    : regionRings({ ...region, west: -180, east: 179.9999, full_circle: false });
  const walked = rings.map((ring) => walk(camera, view, ring));
  return {
    rings: walked.flatMap((w) => w.pieces),
    closed: walked.every((w) => w.whole),
  };
}

/**
 * A cylindrical map's region: longitude is linear in x and latitude a
 * function of y alone, so it is the rectangle between two meridians and two
 * parallels — the full width for a full circle.
 */
function cylindricalMask(camera: Camera, view: Viewport, region: ProjectRegion): RegionMask {
  const clampX = (x: number) => Math.min(Math.max(x, -MARGIN), view.width + MARGIN);
  const clampY = (y: number) => Math.min(Math.max(y, -MARGIN), view.height + MARGIN);
  const yOf = (lat: number) => clampY(project(camera, view, { lon: camera.centerLon, lat }).y);
  let left = -MARGIN, right = view.width + MARGIN;
  if (!region.full_circle) {
    // From the arc's middle, which the camera is never more than half the arc
    // from: the near copy of the region, on either side of the antimeridian.
    const span = region.east - region.west;
    const mid = normalizeLon((region.west + region.east) / 2 - camera.centerLon);
    const x = (lon: number) => view.width / 2 + lon * camera.pxPerDeg;
    left = clampX(x(mid - span / 2));
    right = clampX(x(mid + span / 2));
  }
  const top = yOf(region.north), bottom = yOf(region.south);
  return {
    rings: [[
      { x: left, y: bottom }, { x: right, y: bottom }, { x: right, y: top }, { x: left, y: top }, { x: left, y: bottom },
    ]],
    closed: true,
  };
}

const finite = (p: ScreenPoint) => Number.isFinite(p.x) && Number.isFinite(p.y);

/**
 * One ring projected, in pieces where it leaves the map: a point that does not
 * project (past a horizon) or a jump longer than half the window's diagonal
 * (across a seam) ends a piece, as `projectedRing` decides it.
 */
function walk(camera: Camera, view: Viewport, ring: readonly GeoPoint[]): { pieces: ScreenPoint[][]; whole: boolean } {
  const pieces: ScreenPoint[][] = [];
  const jump = Math.hypot(view.width, view.height) / 2;
  let current: ScreenPoint[] | null = null;
  let whole = true;
  const add = (p: ScreenPoint) => {
    if (!finite(p)) {
      whole = false;
      current = null;
      return;
    }
    const last = current?.[current.length - 1];
    if (last && Math.hypot(p.x - last.x, p.y - last.y) > jump) {
      whole = false;
      current = null;
    }
    if (!current) {
      current = [];
      pieces.push(current);
    }
    current.push(p);
  };
  const near = (a: ScreenPoint, b: ScreenPoint, slack: number) =>
    Math.max(a.x, b.x) >= -slack && Math.min(a.x, b.x) <= view.width + slack
    && Math.max(a.y, b.y) >= -slack && Math.min(a.y, b.y) <= view.height + slack;

  for (let i = 0; i + 1 < ring.length; i++) {
    const a = ring[i]!, b = ring[i + 1]!;
    const pa = project(camera, view, a), pb = project(camera, view, b);
    const dLon = normalizeLon(b.lon - a.lon), dLat = b.lat - a.lat;
    let steps = 1;
    if (finite(pa) && finite(pb)) {
      // A segment's bulge is a fraction of its length, so one within its own
      // length of the window may curve into it; one further out may not.
      const length = Math.hypot(pb.x - pa.x, pb.y - pa.y);
      if (length <= jump && near(pa, pb, length)) steps = Math.min(256, Math.max(1, Math.ceil(length / STEP_PX)));
    } else if (finite(pa) || finite(pb)) {
      // Find the horizon to within a sixteenth of the segment.
      steps = 16;
    }
    for (let j = 0; j < steps; j++) {
      add(j === 0 ? pa : project(camera, view, { lon: a.lon + (dLon * j) / steps, lat: a.lat + (dLat * j) / steps }));
    }
  }
  const end = ring[ring.length - 1];
  if (end) add(project(camera, view, end));
  return { pieces, whole: whole && pieces.length === 1 };
}

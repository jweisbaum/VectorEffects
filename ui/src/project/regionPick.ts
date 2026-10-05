/**
 * The maths behind choosing a region (spec.md 4.2, decision R9): the picker
 * canvas, the drawn box, the presets, and the numbers the form shows.
 *
 * Display only. Rust (`ve_core::region::Region::snapped`) is the authority on
 * what a request becomes; `snapEdges` mirrors it so the four fields and the
 * size estimate show what will be created, and the request still goes out as
 * the person left it.
 */

import type { RegionRequest } from "../generated/RegionRequest";
import { marqueeBounds } from "../map/marquee";

/** A point under the picker. `lon` is unwrapped about the canvas's centre. */
export interface PickPoint {
  lon: number;
  lat: number;
}

/** A cap from 60°N to the pole, all the way round. Editable afterwards. */
export const ARCTIC: RegionRequest = { west: -180, east: 180, south: 60, north: 90, full_circle: true };
/** A cap from 60°S to the pole, all the way round. Editable afterwards. */
export const ANTARCTIC: RegionRequest = { west: -180, east: 180, south: -90, north: -60, full_circle: true };

/** What Regional starts from: the North Atlantic, a place to start dragging from. */
export const DEFAULT_REGION: RegionRequest = { west: -80, east: 0, south: 0, north: 60, full_circle: false };

/** Micro-degrees, the unit `Region` keeps its edges in. */
const MICRO = 1_000_000;
const TURN = 360 * MICRO;

/** Longitude into [-180, 180). */
export function normalizeLon(lon: number): number {
  const n = ((((lon + 180) % 360) + 360) % 360) - 180;
  return n === 0 ? 0 : n; // never -0
}

function mod360(degrees: number): number {
  return ((degrees % 360) + 360) % 360;
}

const clampLat = (lat: number) => Math.min(90, Math.max(-90, lat));

/**
 * The geographic point under `(x, y)` on a picker `width` × `height` drawn in
 * equirectangular about `centreLon`. The longitude is *not* wrapped: the
 * direction of a drag is read from it, and a canvas that has turned past 180°
 * would otherwise make a rightward drag look leftward.
 */
export function pickerToLonLat(
  x: number,
  y: number,
  centreLon: number,
  width: number,
  height: number,
): PickPoint {
  const lon = centreLon + (x / width - 0.5) * 360;
  const lat = clampLat(90 - (y / height) * 180);
  return { lon: lon === 0 ? 0 : lon, lat: lat === 0 ? 0 : lat };
}

/** The canvas x of a longitude, on the copy of the world nearest the centre. */
export function lonToPickerX(lon: number, centreLon: number, width: number): number {
  return (normalizeLon(lon - centreLon) / 360 + 0.5) * width;
}

/** How far from either side, as a share of the width, the canvas starts to turn. */
const FOLLOW_MARGIN = 0.08;
/** The most it turns per pointer report, in degrees. */
const FOLLOW_STEP = 6;

/**
 * The centre longitude after a drag report at `x`: unchanged in the middle,
 * turned towards the pointer when it is in either margin, faster the deeper
 * it is. That is what lets a box be drawn across 180° — or any meridian —
 * without the pointer leaving the canvas.
 */
export function followCentre(centreLon: number, x: number, width: number): number {
  const margin = width * FOLLOW_MARGIN;
  if (x > width - margin) return centreLon + FOLLOW_STEP * Math.min(1, (x - (width - margin)) / margin);
  if (x < margin) return centreLon - FOLLOW_STEP * Math.min(1, (margin - x) / margin);
  return centreLon;
}

/**
 * The box a drag from `a` to `b` describes. Both longitudes are unwrapped, so
 * the drag's direction decides the wrap as `marqueeBounds` does: rightwards
 * from 170 to 190 is the 20° across the antimeridian, leftwards from 170 to
 * -170 the 340° the other way. A drag that has gone a whole turn covers
 * every longitude. Latitudes are clamped to the poles, which Rust refuses to
 * do for us.
 */
export function dragRegion(a: PickPoint, b: PickPoint): RegionRequest {
  const south = clampLat(Math.min(a.lat, b.lat));
  const north = clampLat(Math.max(a.lat, b.lat));
  if (Math.abs(b.lon - a.lon) >= 360) {
    return { west: -180, east: 180, south, north, full_circle: true };
  }
  const box = marqueeBounds(
    { lon: normalizeLon(a.lon), lat: a.lat },
    { lon: normalizeLon(b.lon), lat: b.lat },
    b.lon >= a.lon,
  );
  return { west: box.west, east: box.east, south, north, full_circle: false };
}

/** The eastward arc from west to east, in degrees: 360 for a full circle. */
export function regionSpan(req: RegionRequest): number {
  return req.full_circle ? 360 : mod360(req.east - req.west);
}

/**
 * The edges `Region::snapped` will make of `req` on a lattice of
 * `resolutionDeg`: each moved outward, and any full turn the canonical west
 * -180. Worked in micro-degrees, as Rust does, so 0.1° does not drift.
 * A box with no width is left as it is; Rust refuses it in words.
 */
export function snapEdges(req: RegionRequest, resolutionDeg: number): RegionRequest {
  const d = Math.round(resolutionDeg * MICRO);
  const u = (deg: number) => Math.round(deg * MICRO);
  const floor = (v: number) => Math.floor(v / d) * d;
  const ceil = (v: number) => Math.ceil(v / d) * d;
  const deg = (v: number) => (v === 0 ? 0 : v / MICRO);

  const south = deg(floor(u(clampLat(req.south))));
  const north = deg(ceil(u(clampLat(req.north))));
  const westU = u(normalizeLon(req.west));
  let span = req.full_circle ? TURN : ((u(normalizeLon(req.east)) - westU) % TURN + TURN) % TURN;
  if (span === 0) return { ...req, south, north };
  const westS = floor(westU);
  span = Math.min(ceil(westU + span) - westS, TURN);
  if (req.full_circle || span >= TURN) {
    return { west: -180, east: 180, south, north, full_circle: true };
  }
  return { west: deg(westS), east: normalizeLon(deg(westS + span)), south, north, full_circle: false };
}

/** A full circle from pole to pole: that is a global project, not a region. */
export function isWholeEarth(req: RegionRequest): boolean {
  return regionSpan(req) >= 360 && req.south <= -90 && req.north >= 90;
}

/**
 * The lattice the region's export holds, as `Region::lattice` counts it:
 * both edges are nodes, except round a full circle, which holds no
 * duplicated column — as the global grid does not.
 */
export function regionNodes(req: RegionRequest, resolutionDeg: number): { ni: number; nj: number } {
  const s = snapEdges(req, resolutionDeg);
  const columns = Math.round(regionSpan(s) / resolutionDeg);
  return {
    ni: s.full_circle ? columns : columns + 1,
    nj: Math.round((s.north - s.south) / resolutionDeg) + 1,
  };
}

/**
 * The box dragged by `dLon`, `dLat` degrees. Longitudes carry round the
 * earth; the latitudes stop at the poles with the box's height intact.
 */
export function moveRegion(start: RegionRequest, dLon: number, dLat: number): RegionRequest {
  const height = start.north - start.south;
  const south = Math.min(90 - height, Math.max(-90, start.south + dLat));
  const shifted = start.full_circle
    ? { west: start.west, east: start.east }
    : { west: normalizeLon(start.west + dLon), east: normalizeLon(start.east + dLon) };
  return { ...shifted, south, north: south + height, full_circle: start.full_circle };
}

/** Whether a point lies in the box, edges included. */
export function regionContains(req: RegionRequest, lon: number, lat: number): boolean {
  if (lat < req.south || lat > req.north) return false;
  return req.full_circle || mod360(lon - req.west) <= regionSpan(req);
}

/**
 * The region of an open map's view, from `MapHandle.bounds()` —
 * `[west, north, east, south]` with east unwrapped — for *Use current view*.
 */
export function regionFromView(bounds: [number, number, number, number]): RegionRequest {
  const [west, north, east, south] = bounds;
  const lat = { south: clampLat(south), north: clampLat(north) };
  if (east - west >= 360) return { west: -180, east: 180, ...lat, full_circle: true };
  return { west: normalizeLon(west), east: normalizeLon(east), ...lat, full_circle: false };
}

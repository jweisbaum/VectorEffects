/**
 * The camera held to a regional project's region (spec.md 5.1, R8).
 *
 * The region rides on the `Camera`, the way the projection does, so every
 * path through `clampCamera` sees it. "Cannot zoom out beyond the region"
 * means the minimum zoom is the one at which the region fits the window, and
 * panning keeps the window over the region on any axis where the region is
 * larger than the window; on an axis where it is smaller, it is centred.
 *
 * Three cases, because the camera's coordinates mean three different things:
 * - cylindrical: longitude is linear in x and is clamped in the region's
 *   unwrapped arc (a full circle keeps today's free wrap); latitude is clamped
 *   in the projection's own y, never in degrees of latitude;
 * - movable (the globe and the azimuthals): the centre is clamped into the
 *   region and the zoom floor fits the region projected about that centre;
 * - fixed general maps: the region's projected outline gives a box in the
 *   plane, held like the cylindrical case in plane coordinates. The box
 *   depends only on the projection and the region, so it is built once per
 *   pair and never per camera.
 */

import type { ProjectRegion } from "../generated/ProjectRegion";
import { MAX_PX_PER_DEG, type Camera, type GeoPoint, type Viewport } from "./camera";
import { containsLon } from "./marquee";
import { DEFAULT_PROJECTION, projectionOf, type Projection } from "./projection";
import { defaultCentre, mapTransform, type Box, type GeneralMap, type XY } from "./projections/general";

const norm = (lon: number) => ((((lon + 180) % 360) + 360) % 360) - 180;
const clamp = (value: number, lo: number, hi: number) => Math.min(Math.max(value, lo), hi);

/** The pole a full-circle region reaches, if any. */
function poleOf(region: ProjectRegion): 90 | -90 | null {
  if (!region.full_circle) return null;
  if (region.north >= 90) return 90;
  if (region.south <= -90) return -90;
  return null;
}

/**
 * The region's boundary, subdivided so no step exceeds `stepDeg`.
 *
 * A rectangle is one closed ring — south edge west to east, east edge, north
 * edge back, west edge — with its longitudes *unwrapped* (east may exceed
 * 180), so the ring never jumps across the antimeridian; a caller that needs
 * [-180, 180) normalises. A cap is one parallel, closed: -180 to 180 at the
 * edge away from its pole. A full-circle band, which touches neither pole,
 * has two parallels: its south one and then its north one, each closed.
 */
export function regionOutline(region: ProjectRegion, stepDeg = 2): GeoPoint[] {
  const parallel = (lat: number): GeoPoint[] => {
    const n = Math.max(1, Math.ceil(360 / stepDeg));
    return Array.from({ length: n + 1 }, (_, i) => ({ lon: -180 + (360 * i) / n, lat }));
  };
  if (region.full_circle) {
    const pole = poleOf(region);
    if (pole === 90) return parallel(region.south);
    if (pole === -90) return parallel(region.north);
    return [...parallel(region.south), ...parallel(region.north)];
  }
  const { west, east, south, north } = region;
  const edge = (a: GeoPoint, b: GeoPoint): GeoPoint[] => {
    const n = Math.max(1, Math.ceil(Math.max(Math.abs(b.lon - a.lon), Math.abs(b.lat - a.lat)) / stepDeg));
    return Array.from({ length: n }, (_, i) => ({
      lon: a.lon + ((b.lon - a.lon) * i) / n,
      lat: a.lat + ((b.lat - a.lat) * i) / n,
    }));
  };
  const sw = { lon: west, lat: south }, se = { lon: east, lat: south };
  const ne = { lon: east, lat: north }, nw = { lon: west, lat: north };
  return [...edge(sw, se), ...edge(se, ne), ...edge(ne, nw), ...edge(nw, sw), { ...sw }];
}

/** Whether a position lies in the region. Longitude is read on the arc. */
export function regionContains(region: ProjectRegion, lon: number, lat: number): boolean {
  if (lat < region.south || lat > region.north) return false;
  if (region.full_circle) return true;
  return containsLon(region, lon);
}

/** The middle of the region; a cap's is its pole. Longitude in [-180, 180). */
export function regionCentre(region: ProjectRegion): GeoPoint {
  const pole = poleOf(region);
  if (pole !== null) return { lon: 0, lat: pole };
  const lat = (region.south + region.north) / 2;
  return { lon: region.full_circle ? 0 : norm((region.west + region.east) / 2), lat };
}

/**
 * Points whose projection bounds the region's: the outline at 2°, and a
 * coarse interior grid with the pole for a cap, since under a projection
 * whose horizon cuts the region the outline alone can vanish.
 */
function regionSamples(region: ProjectRegion): GeoPoint[] {
  const points = regionOutline(region, 2);
  const west = region.full_circle ? -180 : region.west;
  const span = region.full_circle ? 360 : region.east - region.west;
  for (let j = 1; j < 8; j++) {
    for (let i = 0; i < 16; i++) {
      points.push({ lon: west + (span * i) / 16, lat: region.south + ((region.north - region.south) * j) / 8 });
    }
  }
  const pole = poleOf(region);
  if (pole !== null) points.push({ lon: 0, lat: pole });
  return points;
}

/** Where a fixed map is drawn from. Its origin does not move with the camera. */
function fixedTransform(map: GeneralMap) {
  return mapTransform(map, defaultCentre(map));
}

const planeBoxes = new Map<string, Box | null>();
/** The last box asked for: the clamp asks once per pointer report, and a
 * string key per report is the kind of cost the globe turn already paid for. */
let lastBox: { map: GeneralMap; region: ProjectRegion; box: Box | null } | null = null;

/**
 * The region's box in a fixed map's plane, `[west, south, east, north]` in
 * the plane's degree units, or `null` when nothing of it projects.
 *
 * Memoised by (projection, region): it depends on nothing else, and nothing
 * on a projection may be rebuilt per camera.
 */
export function regionPlaneBox(map: GeneralMap, region: ProjectRegion): Box | null {
  if (lastBox && lastBox.map === map && lastBox.region === region) return lastBox.box;
  const key = `${map.id}|${region.west}|${region.east}|${region.south}|${region.north}|${region.full_circle}`;
  let box = planeBoxes.get(key);
  if (box === undefined) {
    const transform = fixedTransform(map);
    const xy = regionSamples(region)
      // Avoid +180 folding to -180 in a projection whose seam is there.
      .map((p) => transform.forward({ lon: norm(p.lon) === -180 && p.lon > 0 ? 179.999999 : p.lon, lat: p.lat }))
      .filter((p): p is XY => p !== null);
    box = xy.length
      ? [Math.min(...xy.map((p) => p.x)), Math.min(...xy.map((p) => p.y)),
        Math.max(...xy.map((p) => p.x)), Math.max(...xy.map((p) => p.y))]
      : null;
    planeBoxes.set(key, box);
    if (planeBoxes.size > 32) planeBoxes.delete(planeBoxes.keys().next().value!);
  }
  lastBox = { map, region, box };
  return box;
}

/**
 * The zoom, in pixels per degree, at which the whole region fits the window
 * when the camera looks at `centre`.
 *
 * Cylindrical: the region's longitude span and its span in the projection's
 * y; a full circle cannot fit horizontally and fits its latitude alone.
 * Movable: the region projected about `centre`, which the camera puts at the
 * window's middle, so the fit is to the larger half-extent on each axis.
 * Fixed: the region's plane box. `centre` matters only to a movable one.
 */
export function regionFitPxPerDeg(
  region: ProjectRegion,
  view: Viewport,
  projection: Projection,
  centre: GeoPoint,
): number {
  const map = projection.general;
  if (!map) {
    const ySpan = Math.max(1e-9, projection.yOf(region.north) - projection.yOf(region.south));
    if (region.full_circle) return view.height / ySpan;
    return Math.min(view.width / Math.max(1e-9, region.east - region.west), view.height / ySpan);
  }
  if (map.movable) {
    const transform = mapTransform(map, centre);
    let halfX = 0, halfY = 0;
    for (const p of regionSamples(region)) {
      const xy = transform.forward(p);
      if (!xy) continue;
      halfX = Math.max(halfX, Math.abs(xy.x));
      halfY = Math.max(halfY, Math.abs(xy.y));
    }
    if (halfX === 0 && halfY === 0) return 0;
    return Math.min(view.width / 2 / Math.max(1e-9, halfX), view.height / 2 / Math.max(1e-9, halfY));
  }
  const box = regionPlaneBox(map, region);
  if (!box) return 0;
  return Math.min(view.width / Math.max(1e-9, box[2] - box[0]), view.height / Math.max(1e-9, box[3] - box[1]));
}

/**
 * Keeps a centre coordinate's window inside `[lo, hi]`, or centres the window
 * on it when the span is the smaller of the two. `half` is half the window.
 */
function hold(value: number, lo: number, hi: number, half: number): number {
  return hi - lo <= 2 * half ? (lo + hi) / 2 : clamp(value, lo + half, hi - half);
}

/**
 * Longitude held in the region's arc, `half` degrees of window either side.
 *
 * Measured from the arc's middle along the short way round, so a drag past
 * the east edge stops there rather than reappearing at the west one: a pan
 * moves the centre by far less than the half of the world opposite the arc.
 */
function holdLon(region: ProjectRegion, lon: number, half: number): number {
  const mid = (region.west + region.east) / 2;
  const limit = Math.max(0, (region.east - region.west) / 2 - half);
  return norm(mid + clamp(norm(lon - mid), -limit, limit));
}

/**
 * Holds an already-clamped camera to its region. A camera without one is
 * returned as it came: a global project's map behaves exactly as before.
 */
export function clampToRegion(camera: Camera, view: Viewport): Camera {
  const region = camera.region;
  if (!region) return camera;
  const projection = projectionOf(camera.projection ?? DEFAULT_PROJECTION);
  const map = projection.general;

  if (!map) {
    const pxPerDeg = clamp(
      Math.max(camera.pxPerDeg, regionFitPxPerDeg(region, view, projection, regionCentre(region))),
      0, MAX_PX_PER_DEG,
    );
    const centerLon = region.full_circle
      ? norm(camera.centerLon)
      : holdLon(region, camera.centerLon, view.width / 2 / pxPerDeg);
    // In the projection's own y: yOf stops at the projection's edge, so a
    // region past Mercator's 85.05° stops there too.
    const y = hold(
      projection.yOf(camera.centerLat),
      projection.yOf(region.south),
      projection.yOf(region.north),
      view.height / 2 / pxPerDeg,
    );
    return { ...camera, centerLon, centerLat: projection.latOf(y), pxPerDeg };
  }

  if (map.movable) {
    let centerLon = norm(camera.centerLon);
    let centerLat = clamp(camera.centerLat, region.south, region.north);
    const pole = poleOf(region);
    if (pole !== null) {
      // A cap is held about its pole. The centre may leave the pole only as
      // far as the window, half its smaller side in the plane's degrees
      // (which are degrees of arc at the centre), lies inside the cap; so a
      // view zoomed out to the cap looks straight down on the pole.
      const radius = pole === 90 ? 90 - region.south : region.north + 90;
      const away = Math.max(0, radius - Math.min(view.width, view.height) / 2 / camera.pxPerDeg);
      centerLat = pole === 90 ? Math.max(centerLat, 90 - away) : Math.min(centerLat, away - 90);
    } else if (!region.full_circle) {
      centerLon = holdLon(region, centerLon, 0);
    }
    const fit = regionFitPxPerDeg(region, view, projection, { lon: centerLon, lat: centerLat });
    const pxPerDeg = clamp(Math.max(camera.pxPerDeg, fit), 0, MAX_PX_PER_DEG);
    return { ...camera, centerLon, centerLat, pxPerDeg };
  }

  const box = regionPlaneBox(map, region);
  if (!box) return camera;
  const pxPerDeg = clamp(Math.max(camera.pxPerDeg, regionFitPxPerDeg(region, view, projection, regionCentre(region))), 0, MAX_PX_PER_DEG);
  const transform = fixedTransform(map);
  const at = transform.forward({ lon: camera.centerLon, lat: camera.centerLat })
    ?? { x: (box[0] + box[2]) / 2, y: (box[1] + box[3]) / 2 };
  const held = {
    x: hold(at.x, box[0], box[2], view.width / 2 / pxPerDeg),
    y: hold(at.y, box[1], box[3], view.height / 2 / pxPerDeg),
  };
  const centre = transform.inverse(held);
  if (!centre) return { ...camera, pxPerDeg };
  return { ...camera, centerLon: centre.lon, centerLat: centre.lat, pxPerDeg };
}

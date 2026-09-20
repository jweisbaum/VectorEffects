/**
 * Camera maths (spec.md 5.1).
 *
 * Equirectangular by default, because the data grid is global lat/lon: the map
 * is 1:1 with the grid, both poles are visible, and there is no zoom-dependent
 * distortion of the editing surface. The cylindrical alternatives (M11) ride
 * on the camera itself — `Camera.projection` — so every function here reads it
 * from the camera it was already given, and a camera without one is
 * equirectangular, which is the identity.
 *
 * Everything here is pure so it can be tested without a GPU. The renderer does
 * no geometry of its own beyond what these functions produce.
 */

import {
  DEFAULT_PROJECTION,
  type Projection,
  type ProjectionId,
  projectionOf,
  worldHeightDeg,
} from "./projection";

import { defaultCentre, mapExtent, mapTransform } from "./projections/general";
import { geographicMesh, type GeographicMesh } from "./projections/mesh";

/** What a camera that names no projection is drawn in. */
const PLATE_CARREE = projectionOf(DEFAULT_PROJECTION);

/**
 * The projection a camera is looking through.
 *
 * The projection rides on the camera rather than being passed alongside it,
 * because it is the same kind of thing as the centre and the scale: part of
 * how the map is currently looking at the world, and needed by everything the
 * camera is already threaded through. A camera built without one — every test
 * fixture, every dev-capture scenario — is equirectangular, which is what the
 * app has always drawn.
 */
export function projectionFor(camera: Camera): Projection {
  return projectionOf(camera.projection ?? DEFAULT_PROJECTION);
}

/** Deepest tile level served by the backend. Mirrors `ve_render::tile`. */
export const MAX_TILE_LEVEL = 12;
/** Tile edge in pixels. Mirrors `ve_render::tile`. */
export const TILE_SIZE = 256;
/** Sharpest zoom, in pixels per degree. About 1 px per 0.002 degrees. */
export const MAX_PX_PER_DEG = 512;

/** Camera state. */
export interface Camera {
  /** Longitude at the centre of the viewport. */
  centerLon: number;
  /** Latitude at the centre of the viewport. */
  centerLat: number;
  /** Scale, in screen pixels per degree. */
  pxPerDeg: number;
  /**
   * How the world is laid out on the map (M11). Absent means equirectangular.
   *
   * A view setting and nothing more: it changes where a latitude lands on the
   * screen, never what is stored or exported (invariant 3).
   */
  projection?: ProjectionId;
}

/** Viewport size in CSS pixels. */
export interface Viewport {
  width: number;
  height: number;
}

/** A screen position. */
export interface ScreenPoint {
  x: number;
  y: number;
}

/** A geographic position. */
export interface GeoPoint {
  lon: number;
  lat: number;
}

/** Normalises a longitude into [-180, 180). */
export function normalizeLon(lon: number): number {
  return ((((lon + 180) % 360) + 360) % 360) - 180;
}

/** The smallest scale that still fits the whole world in the viewport. */
export function minPxPerDeg(
  view: Viewport,
  projection: Projection = PLATE_CARREE,
): number {
  if (projection.general) {
    const [west,south,east,north] = mapExtent(projection.general);
    return Math.min(view.width / Math.max(1e-6,east-west), view.height / Math.max(1e-6,north-south)) * 0.96;
  }
  // Mercator's world is 360 degrees tall on this scale rather than 180, so
  // "fit the world" is a different number in it.
  return Math.min(view.width / 360, view.height / worldHeightDeg(projection));
}

/**
 * Constrains a camera to the viewport.
 *
 * Longitude wraps freely — panning past the dateline is seamless and must never
 * stop. Latitude is clamped so the viewport cannot scroll past a pole into
 * empty space; when the world is shorter than the viewport, it centres instead.
 */
export function clampCamera(camera: Camera, view: Viewport): Camera {
  const projection = projectionFor(camera);
  const pxPerDeg = Math.min(
    Math.max(camera.pxPerDeg, minPxPerDeg(view, projection)),
    MAX_PX_PER_DEG,
  );
  if (projection.general) {
    const lat = Math.max(-90, Math.min(90, camera.centerLat));
    const next = { ...camera, centerLon: normalizeLon(camera.centerLon), centerLat: lat, pxPerDeg };
    if (!projection.general.movable && !mapTransform(projection.general, defaultCentre(projection.general)).forward({lon: next.centerLon, lat})) {
      const centre = defaultCentre(projection.general);
      next.centerLon = centre.lon; next.centerLat = centre.lat;
    }
    return next;
  }
  const halfHeightDeg = view.height / 2 / pxPerDeg;
  // The clamp is in the projection's own vertical coordinate, not in degrees
  // of latitude: what must not scroll off is the *map*, and where a latitude
  // sits on it is the projection's business.
  const worldHalf = worldHeightDeg(projection) / 2;

  let centerLat: number;
  if (halfHeightDeg >= worldHalf) {
    centerLat = 0;
  } else {
    const limit = worldHalf - halfHeightDeg;
    const y = Math.min(Math.max(projection.yOf(camera.centerLat), -limit), limit);
    centerLat = projection.latOf(y);
  }

  return { ...camera, centerLon: normalizeLon(camera.centerLon), centerLat, pxPerDeg };
}

/**
 * Projects a geographic position to screen pixels.
 *
 * Longitude is taken to the copy of the world nearest the camera centre, so a
 * point near the dateline lands next to the centre rather than a world away.
 */
export function project(camera: Camera, view: Viewport, point: GeoPoint): ScreenPoint {
  const projection = projectionFor(camera);
  if (projection.general) {
    const transform = mapTransform(projection.general, {lon: camera.centerLon, lat: camera.centerLat});
    const centre = projection.general.movable ? {x:0,y:0} : transform.forward({lon:camera.centerLon,lat:camera.centerLat});
    const xy = transform.forward(point);
    if (!xy || !centre) return {x:NaN,y:NaN};
    return {x:view.width/2+(xy.x-centre.x)*camera.pxPerDeg, y:view.height/2-(xy.y-centre.y)*camera.pxPerDeg};
  }
  const dLon = normalizeLon(point.lon - camera.centerLon);
  return {
    x: view.width / 2 + dLon * camera.pxPerDeg,
    y:
      view.height / 2 +
      (projection.yOf(camera.centerLat) - projection.yOf(point.lat)) * camera.pxPerDeg,
  };
}

/** Converts a screen position back to a geographic one. */
export function unproject(camera: Camera, view: Viewport, point: ScreenPoint): GeoPoint {
  const projection = projectionFor(camera);
  if (projection.general) {
    const transform = mapTransform(projection.general, {lon:camera.centerLon,lat:camera.centerLat});
    const centre = projection.general.movable ? {x:0,y:0} : transform.forward({lon:camera.centerLon,lat:camera.centerLat});
    if (!centre) return {lon:NaN,lat:NaN};
    return transform.inverse({x:centre.x+(point.x-view.width/2)/camera.pxPerDeg,
      y:centre.y-(point.y-view.height/2)/camera.pxPerDeg}) ?? {lon:NaN,lat:NaN};
  }
  const lon = camera.centerLon + (point.x - view.width / 2) / camera.pxPerDeg;
  const y =
    projection.yOf(camera.centerLat) - (point.y - view.height / 2) / camera.pxPerDeg;
  const lat = projection.latOf(y);
  return { lon: normalizeLon(lon), lat: Math.min(Math.max(lat, -90), 90) };
}

/**
 * Moves the camera by a screen offset, y down.
 *
 * The vertical move is made in the projection's own coordinate, not in degrees
 * of latitude: a drag of ten pixels is ten pixels of *map*, and how many
 * degrees of latitude that is depends on where the map is looking (M11).
 * Doing it in latitude instead makes a drag under Mercator run away from the
 * pointer, further the higher the latitude.
 *
 * Longitude needs no such care, being linear in `x` in every projection here,
 * and is never clamped: a drag past the dateline keeps going and wraps.
 */
export function panBy(camera: Camera, view: Viewport, dxPx: number, dyPx: number): Camera {
  const projection = projectionFor(camera);
  if (projection.general) {
    const centre = unproject(camera, view, {x:view.width/2+dxPx,y:view.height/2+dyPx});
    return validGeo(centre) ? clampCamera({...camera,centerLon:centre.lon,centerLat:centre.lat},view) : camera;
  }
  const y = projection.yOf(camera.centerLat) - dyPx / camera.pxPerDeg;
  return clampCamera(
    {
      ...camera,
      centerLon: camera.centerLon + dxPx / camera.pxPerDeg,
      centerLat: projection.latOf(y),
    },
    view,
  );
}

/**
 * Zooms about a fixed screen point.
 *
 * The geographic position under the cursor stays under the cursor, which is
 * what makes wheel-zoom feel anchored rather than drifting.
 */
export function zoomAbout(
  camera: Camera,
  view: Viewport,
  anchor: ScreenPoint,
  factor: number,
): Camera {
  const projection = projectionFor(camera);
  const before = unproject(camera, view, anchor);
  const zoomed = clampCamera({ ...camera, pxPerDeg: camera.pxPerDeg * factor }, view);
  if (projection.general) {
    if (!validGeo(before)) return zoomed;
    return cameraWithAnchor(zoomed,view,before,anchor);
  }

  const after = unproject(zoomed, view, anchor);
  // The correction is applied in the projection's own vertical coordinate:
  // a difference in latitude is not a difference in pixels once the two are
  // not the same thing, and applying one as the other slides the anchor.
  const centerY =
    projection.yOf(zoomed.centerLat) + (projection.yOf(before.lat) - projection.yOf(after.lat));
  return clampCamera(
    {
      ...zoomed,
      centerLon: zoomed.centerLon + normalizeLon(before.lon - after.lon),
      centerLat: projection.latOf(centerY),
    },
    view,
  );
}

/** The visible extent. `west` may exceed `east` numerically before wrapping. */
export interface ViewBounds {
  /** Western edge, unwrapped: may be less than -180. */
  west: number;
  /** Eastern edge, unwrapped: may exceed 180. */
  east: number;
  north: number;
  south: number;
}

/** The geographic extent currently on screen. Rendering includes repeated worlds;
 * data queries need at most one full world. */
export function visibleBounds(camera: Camera, view: Viewport, repeatWorlds = false): ViewBounds {
  const projection = projectionFor(camera);
  if (projection.general) {
    const points:GeoPoint[]=[];
    for(let j=0;j<=16;j++)for(let i=0;i<=16;i++){
      const p=unproject(camera,view,{x:i*view.width/16,y:j*view.height/16});if(validGeo(p))points.push(p);
    }
    if(!points.length)return {west:-180,east:180,north:90,south:-90};
    const lons=points.map(p=>camera.centerLon+normalizeLon(p.lon-camera.centerLon));
    let north=Math.max(...points.map(p=>p.lat)),south=Math.min(...points.map(p=>p.lat));
    for(const lat of [-90,90]){
      const p=project(camera,view,{lon:camera.centerLon,lat});
      if(p.x>=0&&p.x<=view.width&&p.y>=0&&p.y<=view.height){if(lat>0)north=90;else south=-90;}
    }
    return {west:north===90||south===-90?-180:Math.min(...lons),east:north===90||south===-90?180:Math.max(...lons),north,south};
  }
  const halfLon = view.width / 2 / camera.pxPerDeg;
  // Half the viewport in the projection's own vertical coordinate, then back
  // to latitudes. Under Mercator the same pixel height is a much narrower band
  // of latitude at the top of the map than at the equator, and culling to the
  // wrong band would drop tiles that are on screen.
  const halfY = view.height / 2 / camera.pxPerDeg;
  const centreY = projection.yOf(camera.centerLat);
  // More than a full world across: there is no meaningful sub-range to cull to.
  const spanLon = repeatWorlds ? halfLon * 2 : Math.min(halfLon * 2, 360);
  return {
    west: camera.centerLon - spanLon / 2,
    east: camera.centerLon + spanLon / 2,
    north: Math.min(projection.latOf(centreY + halfY), 90),
    south: Math.max(projection.latOf(centreY - halfY), -90),
  };
}

/**
 * The tile level whose pixels are at least as fine as the screen's.
 *
 * Rounding up rather than to nearest: an over-sampled tile looks sharp, an
 * under-sampled one looks blurry, and the cost difference is one level.
 */
export function tileLevelFor(pxPerDeg: number): number {
  const wanted = Math.log2((360 * pxPerDeg) / TILE_SIZE) - 1;
  return Math.min(Math.max(Math.ceil(wanted), 0), MAX_TILE_LEVEL);
}

/** A tile to draw, with the world copy it belongs to. */
export interface VisibleTile {
  z: number;
  x: number;
  y: number;
  /** Longitude offset of the world copy this instance is drawn in: -360, 0, 360. */
  lonOffset: number;
}

/** Columns at a level. */
export function tileColumns(z: number): number {
  return 2 << z;
}

/** Rows at a level. */
export function tileRows(z: number): number {
  return 1 << z;
}

/**
 * Every tile touching the viewport, including wrapped copies.
 *
 * Wrapping is handled by emitting the same tile more than once at different
 * longitude offsets rather than by widening the tile grid, so tile identity
 * (and therefore the fetch cache) stays one-to-one with the backend's pyramid.
 *
 * The result always covers the whole viewport. If the ideal level would need
 * more tiles than `budget`, a coarser level is chosen instead of returning a
 * partial list — an unpainted corner is a far worse artefact than soft pixels.
 */
export function visibleTiles(
  camera: Camera,
  view: Viewport,
  budget = 192,
): VisibleTile[] {
  const general = projectionFor(camera).general;
  if (general?.movable) return azimuthalTiles(camera, view, budget);
  if (general) return projectedMesh(camera,view,budget).tiles;
  const bounds = visibleBounds(camera, view, true);
  const ideal = tileLevelFor(camera.pxPerDeg);

  // Step back a level rather than truncate.
  //
  // A retina viewport covers far more tiles than a logical one: at 2880x1684
  // device pixels a mid-zoom view wants 286 tiles, and simply cutting the list
  // at the budget leaves a visibly unpainted block in the corner. Dropping a
  // level quarters the count and costs only sharpness, which is recovered as
  // soon as the user zooms further in.
  let z = ideal;
  let columns = tileColumns(z);
  let rows = tileRows(z);
  let firstRow = 0;
  let lastRow = 0;
  let firstCol = 0;
  let lastCol = 0;

  for (;;) {
    columns = tileColumns(z);
    rows = tileRows(z);
    const spanX = 360 / columns;
    const spanY = 180 / rows;

    firstRow = Math.max(0, Math.floor((90 - bounds.north) / spanY));
    lastRow = Math.min(rows - 1, Math.floor((90 - bounds.south) / spanY));
    firstCol = Math.floor((bounds.west + 180) / spanX);
    lastCol = Math.floor((bounds.east + 180) / spanX);

    const needed = (lastRow - firstRow + 1) * (lastCol - firstCol + 1);
    if (needed <= budget || z === 0) break;
    z -= 1;
  }

  const spanX = 360 / columns;
  const out: VisibleTile[] = [];
  for (let row = firstRow; row <= lastRow; row++) {
    for (let col = firstCol; col <= lastCol; col++) {
      const wrapped = ((col % columns) + columns) % columns;
      const lonOffset = (col - wrapped) * spanX;
      out.push({ z, x: wrapped, y: row, lonOffset });
    }
  }
  return out;
}

/** The geographic extent of a tile, before any world-copy offset. */
export function tileBounds(z: number, x: number, y: number): ViewBounds {
  const spanX = 360 / tileColumns(z);
  const spanY = 180 / tileRows(z);
  const west = -180 + x * spanX;
  const north = 90 - y * spanY;
  return { west, east: west + spanX, north, south: north - spanY };
}

// --- Glyph lattice ----------------------------------------------------------

/**
 * Degree steps a glyph lattice may use, coarsest first.
 *
 * Snapping to a fixed ladder rather than computing an exact step means the
 * lattice only changes at discrete zoom thresholds, so glyphs stay put during a
 * zoom instead of continuously re-flowing.
 */
export const GLYPH_STEPS_DEG = [
  90, 60, 45, 30, 20, 15, 10, 7.5, 5, 3, 2.5, 2, 1.5, 1, 0.75, 0.5, 0.3, 0.25,
  0.15, 0.1, 0.075, 0.05, 0.03, 0.025, 0.015, 0.01,
];

/**
 * The densest lattice step whose on-screen spacing still clears `targetPx`.
 *
 * The ladder reaches 90 degrees at the coarse end so that even a very small
 * window, where the whole globe is only a couple of hundred pixels wide, still
 * has a step that clears the target. Its rungs are close together — half a
 * step at most — because the spacing a rung gives is what a glyph is sized
 * from, and a ladder that overshot the target by two and a half times drew
 * glyphs two and a half times too big (M33).
 */
export function glyphStepDegrees(pxPerDeg: number, targetPx: number): number {
  let chosen = GLYPH_STEPS_DEG[0] as number;
  for (const step of GLYPH_STEPS_DEG) {
    if (step * pxPerDeg >= targetPx) chosen = step;
  }
  return chosen;
}

/** A block of lattice points falling inside one tile. */
export interface GlyphLattice {
  /** Longitude of the first column. */
  originLon: number;
  /** Latitude of the first row. */
  originLat: number;
  /** Columns in this block. */
  cols: number;
  /** Rows in this block. */
  rows: number;
}

/**
 * The lattice points inside `bounds`, on a globally anchored grid.
 *
 * Points sit at multiples of `stepDeg` measured from (-180, 90), *not* from the
 * tile's own corner. Anchoring per tile instead leaves `tileWidth % spacing` of
 * empty space at every tile edge, which reads as glyphs clustered into blocks
 * with gaps between them.
 *
 * The longitude range is half-open so adjacent tiles never both draw the point
 * on their shared edge.
 */
export function glyphLattice(
  bounds: { west: number; east: number; north: number; south: number },
  stepDeg: number,
): GlyphLattice {
  const epsilon = 1e-9;

  const firstCol = Math.ceil((bounds.west + 180) / stepDeg - epsilon);
  const lastCol = Math.ceil((bounds.east + 180) / stepDeg - epsilon) - 1;
  const firstRow = Math.ceil((90 - bounds.north) / stepDeg - epsilon);
  const lastRow = Math.ceil((90 - bounds.south) / stepDeg - epsilon) - 1;

  return {
    originLon: -180 + firstCol * stepDeg,
    originLat: 90 - firstRow * stepDeg,
    cols: Math.max(0, lastCol - firstCol + 1),
    rows: Math.max(0, lastRow - firstRow + 1),
  };
}

/** Non-geographic pixels (outside a globe or map outline) cannot begin edits. */
export function validGeo(point: GeoPoint): boolean {
  return Number.isFinite(point.lon) && Number.isFinite(point.lat) && Math.abs(point.lat)<=90;
}
/** How many points a side a tile is tried at when asking whether it shows. */
const TILE_PROBE = 4;
/** The screen is asked what is under it on a lattice this many cells a side. */
const SCREEN_PROBE = { columns: 12, rows: 8 };
const azimuthalTileSets = new Map<string, VisibleTile[]>();

/**
 * The tiles a globe or one of its azimuthal kin is showing.
 *
 * These are drawn by the GPU from a static grid (projectionShaders.ts), so
 * there is no screen mesh to read the tiles off, and building one was the
 * cost being removed. The pyramid is walked from its two root tiles instead,
 * keeping a tile that either has a point on screen or is under a point of
 * the screen. Both questions are needed: a tile far larger than the view has
 * every probe of its own off screen, and a tile squeezed against the limb is
 * thinner than the gap between the screen's. A tile kept in error costs a
 * fetch; one dropped in error is a hole in the map, so ties go to keeping.
 */
export function azimuthalTiles(camera: Camera, view: Viewport, budget = 192): VisibleTile[] {
  const key = JSON.stringify([camera, view, budget]);
  const cached = azimuthalTileSets.get(key);
  if (cached) return cached;

  const under: GeoPoint[] = [];
  for (let j = 0; j <= SCREEN_PROBE.rows; j++) {
    for (let i = 0; i <= SCREEN_PROBE.columns; i++) {
      const geo = unproject(camera, view, {
        x: (view.width * i) / SCREEN_PROBE.columns,
        y: (view.height * j) / SCREEN_PROBE.rows,
      });
      if (validGeo(geo)) under.push({ lon: normalizeLon(geo.lon), lat: geo.lat });
    }
  }
  // The centre is always on the map, whatever the corners are.
  under.push({ lon: normalizeLon(camera.centerLon), lat: camera.centerLat });

  const shows = (b: ViewBounds): boolean => {
    if (under.some((p) => p.lon >= b.west && p.lon <= b.east && p.lat <= b.north && p.lat >= b.south)) {
      return true;
    }
    let west = Infinity, east = -Infinity, top = Infinity, bottom = -Infinity;
    for (let j = 0; j <= TILE_PROBE; j++) {
      for (let i = 0; i <= TILE_PROBE; i++) {
        const p = project(camera, view, {
          lon: b.west + ((b.east - b.west) * i) / TILE_PROBE,
          lat: b.north - ((b.north - b.south) * j) / TILE_PROBE,
        });
        if (!Number.isFinite(p.x) || !Number.isFinite(p.y)) continue;
        west = Math.min(west, p.x); east = Math.max(east, p.x);
        top = Math.min(top, p.y); bottom = Math.max(bottom, p.y);
      }
    }
    const slack = 8;
    return east >= -slack && west <= view.width + slack && bottom >= -slack && top <= view.height + slack;
  };

  let tiles: VisibleTile[] = [];
  // The level the general mesh chose, so a view keeps the sharpness it had.
  let level = Math.min(MAX_TILE_LEVEL, Math.max(0, Math.ceil(Math.log2((360 * camera.pxPerDeg) / TILE_SIZE)) - 1));
  for (; ; level--) {
    tiles = [];
    const visit = (z: number, x: number, y: number): void => {
      if (tiles.length > budget || !shows(tileBounds(z, x, y))) return;
      if (z === level) {
        tiles.push({ z, x, y, lonOffset: 0 });
        return;
      }
      for (const [dx, dy] of [[0, 0], [1, 0], [0, 1], [1, 1]] as const) visit(z + 1, x * 2 + dx, y * 2 + dy);
    };
    for (let x = 0; x < tileColumns(0); x++) visit(0, x, 0);
    // Step back a level rather than truncate, as `visibleTiles` does.
    if (tiles.length <= budget || level === 0) break;
  }

  azimuthalTileSets.set(key, tiles);
  if (azimuthalTileSets.size > 8) azimuthalTileSets.delete(azimuthalTileSets.keys().next().value!);
  return tiles;
}

const projectedMeshes = new Map<string,GeographicMesh>();
export function projectedMesh(camera:Camera,view:Viewport,budget=192):GeographicMesh {
  const key=JSON.stringify([camera,view,budget]);
  const cached=projectedMeshes.get(key);if(cached)return cached;
  const general=projectionFor(camera).general;
  if(!general)throw new Error("Only general projections need a geographic mesh");
  const transform=mapTransform(general,{lon:camera.centerLon,lat:camera.centerLat});
  const centre=(general.movable?null:transform.forward({lon:camera.centerLon,lat:camera.centerLat}))??{x:0,y:0};
  const mesh=geographicMesh(view.width,view.height,camera.pxPerDeg,
    p=>transform.inverse({x:centre.x+(p.x-view.width/2)/camera.pxPerDeg,y:centre.y-(p.y-view.height/2)/camera.pxPerDeg}),
    p=>{const xy=transform.forward(p);return xy?{x:view.width/2+(xy.x-centre.x)*camera.pxPerDeg,y:view.height/2-(xy.y-centre.y)*camera.pxPerDeg}:null;},budget);
  projectedMeshes.set(key,mesh);
  if(projectedMeshes.size>4)projectedMeshes.delete(projectedMeshes.keys().next().value!);
  return mesh;
}
/** Fit a newly chosen CRS to its region; a globe opens with the current focus. */
export function cameraForProjection(camera:Camera,view:Viewport,id:ProjectionId):Camera {
  const projection=projectionOf(id);
  if(!projection.general)return clampCamera({...camera,projection:id},view);
  const centre=projection.general.movable?{lon:camera.centerLon,lat:camera.centerLat}:defaultCentre(projection.general);
  return clampCamera({projection:id,centerLon:centre.lon,centerLat:centre.lat,pxPerDeg:minPxPerDeg(view,projection)},view);
}

/** Solve a geographic anchor's screen position for zooming and clone previews. */
export function cameraWithAnchor(camera:Camera,view:Viewport,before:GeoPoint,anchor:ScreenPoint):Camera {
  const projection=projectionFor(camera);
  if(!projection.general || !validGeo(before) || !Number.isFinite(anchor.x))return camera;
    if (!projection.general.movable) {
      const transform=mapTransform(projection.general,{lon:0,lat:0});
      const fixed=transform.forward(before);
      if (!fixed) return camera;
      const centre=transform.inverse({x:fixed.x-(anchor.x-view.width/2)/camera.pxPerDeg,
        y:fixed.y+(anchor.y-view.height/2)/camera.pxPerDeg});
      return centre ? clampCamera({...camera,centerLon:centre.lon,centerLat:centre.lat},view) : camera;
    }
    // Two angular camera coordinates, solved against the same forward map as
    // the pointer. This keeps wheel zoom anchored while turning a globe.
    let next=camera;
    for(let i=0;i<10;i++){
      const p=project(next,view,before),h=1e-4;
      if(!Number.isFinite(p.x))break;
      const dx=p.x-anchor.x,dy=p.y-anchor.y;if(Math.hypot(dx,dy)<0.001)break;
      const a=project({...next,centerLon:next.centerLon+h},view,before);
      const b=project({...next,centerLat:next.centerLat+h},view,before);
      const ax=(a.x-p.x)/h,ay=(a.y-p.y)/h,bx=(b.x-p.x)/h,by=(b.y-p.y)/h,det=ax*by-ay*bx;
      if(!Number.isFinite(det)||Math.abs(det)<1e-8)break;
      next=clampCamera({...next,centerLon:next.centerLon-Math.max(-20,Math.min(20,(dx*by-dy*bx)/det)),
        centerLat:next.centerLat-Math.max(-20,Math.min(20,(dy*ax-dx*ay)/det))},view);
    }
    return next;
}

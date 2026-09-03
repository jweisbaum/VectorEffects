/**
 * Direction glyphs as 2D geometry, for the tool overlay.
 *
 * The map draws its glyphs in `GLYPH_VERT` (`shaders.ts`); this is the same
 * construction in plain numbers, for the brush preview, which is a 2D canvas
 * over the GL one and cannot reach the shader. The two are kept side by side
 * deliberately: what a stroke will look like once committed is exactly what the
 * preview has to show, so a change to one belongs with a change to the other.
 *
 * Pure and in local screen coordinates -- x right, y **down**, origin at the
 * station -- so the encoding can be checked against a reference chart rather
 * than by squinting at the screen.
 */

import type { BrushShape } from "../generated/BrushShape";
import type { StampSpace } from "../generated/StampSpace";
import { barbElements, CALM_KNOTS } from "./barbs";
import { glyphStepDegrees, normalizeLon } from "./camera";
import { cosLat, type Footprint, KM_PER_DEGREE } from "./footprint";

/** A point in local screen coordinates, y down. */
export type Point = readonly [number, number];

/** One glyph, ready to stroke and fill. */
export interface GlyphGeometry {
  /** Open polylines: the shaft, the barbs, the calm marker. */
  lines: Point[][];
  /** Closed polygons: pennants, and the arrowhead. */
  fills: Point[][];
}

const DEG = Math.PI / 180;

function scale(v: Point, k: number): Point {
  return [v[0] * k, v[1] * k];
}

function add(a: Point, b: Point): Point {
  return [a[0] + b[0], a[1] + b[1]];
}

function sub(a: Point, b: Point): Point {
  return [a[0] - b[0], a[1] - b[1]];
}

/** Rotated a quarter turn. In a y-down frame this is clockwise on screen. */
function perp(v: Point): Point {
  return [-v[1], v[0]];
}

function normalise(v: Point): Point {
  const length = Math.hypot(v[0], v[1]) || 1;
  return [v[0] / length, v[1] / length];
}

/**
 * The unit vector the wind blows *towards*, on screen.
 *
 * Azimuth is clockwise from north and screen space is y-down, so north is -y.
 */
export function towardVector(azimuthTowardDeg: number): Point {
  const az = azimuthTowardDeg * DEG;
  return [Math.sin(az), -Math.cos(az)];
}

/**
 * A wind barb: shaft into the wind, flags at the far end.
 *
 * `lat` is not decoration -- the flags sit on the poleward side, so the
 * convention mirrors across the equator.
 */
export function barbGeometry(
  azimuthTowardDeg: number,
  knots: number,
  lengthPx: number,
  lat: number,
  strokeWidth: number,
): GlyphGeometry {
  if (!Number.isFinite(knots) || knots < CALM_KNOTS) {
    // Calm: a small open square at the station, as the shader draws it.
    const r = Math.max(2, strokeWidth * 2);
    return {
      lines: [[[-r, -r], [r, -r], [r, r], [-r, r], [-r, -r]]],
      fills: [],
    };
  }

  const toward = towardVector(azimuthTowardDeg);
  // The shaft points where the wind comes *from*, which is the convention.
  const shaftDir: Point = scale(toward, -1);
  const tip = scale(shaftDir, lengthPx);
  const handed = lat < 0 ? -1 : 1;
  const side = scale(perp(shaftDir), handed);

  const { pennants, full, half } = barbElements(knots);
  const total = pennants + full + half;

  const stepPx = lengthPx * 0.15;
  const flagLen = lengthPx * 0.42;
  const flagDir = normalise(sub(scale(side, 0.86), scale(shaftDir, 0.5)));

  const lines: Point[][] = [[[0, 0], tip]];
  const fills: Point[][] = [];

  for (let slot = 0; slot < total; slot++) {
    const anchor = sub(tip, scale(shaftDir, slot * stepPx));
    if (slot < pennants) {
      fills.push([anchor, add(anchor, scale(flagDir, flagLen)), sub(anchor, scale(shaftDir, stepPx))]);
    } else {
      const len = slot < pennants + full ? flagLen : flagLen * 0.5;
      lines.push([anchor, add(anchor, scale(flagDir, len))]);
    }
  }

  return { lines, fills };
}

/** An arrow: a shaft along the flow with a head at the leading end. */
export function arrowGeometry(azimuthTowardDeg: number, lengthPx: number): GlyphGeometry {
  const toward = towardVector(azimuthTowardDeg);
  const tail = scale(toward, -lengthPx * 0.5);
  const head = scale(toward, lengthPx * 0.5);
  const neck = sub(head, scale(toward, lengthPx * 0.38));
  const side = perp(toward);

  return {
    lines: [[tail, neck]],
    fills: [[
      head,
      add(neck, scale(side, lengthPx * 0.22)),
      sub(neck, scale(side, lengthPx * 0.22)),
    ]],
  };
}

/** Either glyph, chosen by the style the map is drawing. */
export function glyphGeometry(
  style: "arrow" | "barb",
  azimuthTowardDeg: number,
  knots: number,
  lengthPx: number,
  lat: number,
  strokeWidth: number,
): GlyphGeometry {
  return style === "barb"
    ? barbGeometry(azimuthTowardDeg, knots, lengthPx, lat, strokeWidth)
    : arrowGeometry(azimuthTowardDeg, lengthPx);
}

/** Most stamps one stroke's lattice walk will place. */
const MAX_STAMPS = 4000;

/** Most stamps one segment is divided into, however long it is. */
const MAX_SEGMENT_STEPS = 512;

/**
 * How far along a stroke its lattice walk has got.
 *
 * The lattice under a stroke in progress is extended a segment at a time
 * rather than re-walked from the first point on every pointer report; the
 * visited set and the caps are carried here so the extension is exact.
 */
export interface LatticeProgress {
  /** How many of the stroke's points have been walked. */
  done: number;
  /** Stamps placed so far, against the cap. */
  stamps: number;
  /** Lattice keys already collected, so a point is never placed twice. */
  seen: Set<number>;
  /** The points collected, in the order they were found. */
  found: Array<[number, number]>;
  /**
   * Whether a cap was hit. Once one is, the walk stops for good: extending
   * further would add points a fresh walk of the same stroke would never
   * reach, and the two must agree.
   */
  exhausted: boolean;
}

/** A lattice walk with nothing walked yet. */
export function freshLattice(): LatticeProgress {
  return { done: 0, stamps: 0, seen: new Set(), found: [], exhausted: false };
}

/**
 * The lattice points a stroke covers, for its preview glyphs.
 *
 * The map draws glyphs on a globe-anchored lattice (`glyphLattice` in
 * `renderer.ts`), and the brush preview draws them in the same places, so what
 * is previewed is what the field will show once the stroke is committed.
 *
 * Walks the stroke and collects the lattice points inside each stamp rather
 * than testing every point in the stroke's bounding box against it. The work
 * then scales with the number of glyphs actually drawn instead of with the area
 * the stroke happens to span -- a long thin stroke across the viewport spans
 * everything and covers almost none of it, and the difference is the frame
 * budget on every pointer move.
 *
 * `limit` caps the result; a stroke around the world at a fine lattice would
 * ask for more glyphs than are worth drawing on a preview.
 *
 * The whole stroke from nothing: {@link extendLatticeUnderStroke} started at
 * zero, so the two cannot disagree.
 */
export function latticeUnderStroke(
  points: ReadonlyArray<readonly [number, number]>,
  radiusKm: number,
  stepDeg: number,
  limit: number,
  shape: BrushShape = "circle",
  space: StampSpace = "geodesic",
): Array<[number, number]> {
  return extendLatticeUnderStroke(points, freshLattice(), radiusKm, stepDeg, limit, shape, space);
}

/**
 * Collects the lattice points under the segments walked since `progress`.
 *
 * Returns `progress.found`, which now holds every point under the stroke so
 * far. Exact for the same reason {@link extendStrokePath} is: a segment's
 * stamps depend only on that segment, and everything that spans segments — the
 * visited set, the stamp count, the result cap — is carried in `progress`.
 */
export function extendLatticeUnderStroke(
  points: ReadonlyArray<readonly [number, number]>,
  progress: LatticeProgress,
  radiusKm: number,
  stepDeg: number,
  limit: number,
  shape: BrushShape = "circle",
  space: StampSpace = "geodesic",
): Array<[number, number]> {
  const { found, seen } = progress;
  if (progress.exhausted || limit <= 0) return found;

  // Every ladder step divides 360, so a column index wraps exactly.
  const columns = Math.round(360 / stepDeg);
  const lastRow = Math.round(180 / stepDeg);
  const radiusDeg = radiusKm / KM_PER_DEGREE;

  /** Collects the lattice points inside one footprint. */
  const stamp = (lon: number, lat: number): boolean => {
    if (progress.stamps++ >= MAX_STAMPS) return false;
    // A geodesic footprint spans more longitude the further from the equator; a
    // projected one is the same in both axes by construction (spec.md 3.5).
    const radiusLon = space === "projected" ? radiusDeg : radiusDeg / cosLat(lat);

    const rowFrom = Math.max(0, Math.ceil((90 - (lat + radiusDeg)) / stepDeg));
    const rowTo = Math.min(lastRow, Math.floor((90 - (lat - radiusDeg)) / stepDeg));
    const colFrom = Math.ceil((lon - radiusLon + 180) / stepDeg);
    const colTo = Math.floor((lon + radiusLon + 180) / stepDeg);

    for (let row = rowFrom; row <= rowTo; row++) {
      const pointLat = 90 - row * stepDeg;
      for (let col = colFrom; col <= colTo; col++) {
        // The lattice wraps: a footprint over the dateline reaches columns on
        // both sides of it.
        const wrapped = ((col % columns) + columns) % columns;
        const key = row * columns + wrapped;
        if (seen.has(key)) continue;

        const pointLon = -180 + wrapped * stepDeg;
        const dLat = pointLat - lat;
        const dLon = normalizeLon(pointLon - lon);
        // The square stamp covers its whole box; the round one only its inside.
        const inside =
          shape === "square"
            ? Math.abs(dLat) <= radiusDeg && Math.abs(dLon) <= radiusLon
            : Math.hypot(dLat / radiusDeg, dLon / radiusLon) <= 1;
        if (!inside) continue;

        seen.add(key);
        found.push([pointLon, pointLat]);
        if (found.length >= limit) return false;
      }
    }
    return true;
  };

  /** Stops the walk, now and for every extension after this one. */
  const stop = () => {
    progress.exhausted = true;
    return found;
  };

  if (progress.done === 0) {
    const first = points[0];
    if (first === undefined) return found;
    progress.done = 1;
    if (!stamp(first[0], first[1])) return stop();
  }

  for (let i = progress.done; i < points.length; i++) {
    const from = points[i - 1]!;
    const to = points[i]!;

    // The shorter way round, so a stroke across the dateline is walked through
    // it rather than back across the world.
    const dLon = normalizeLon(to[0] - from[0]);
    const dLat = to[1] - from[1];

    // Half a radius between stamps: the gap a straighter chain would leave is a
    // few percent of the radius, which no glyph lattice can resolve.
    const spanDeg = Math.hypot(dLon * cosLat(from[1]), dLat);
    const steps = Math.min(
      MAX_SEGMENT_STEPS,
      Math.max(1, Math.ceil(spanDeg / Math.max(radiusDeg * 0.5, 1e-9))),
    );

    for (let step = 1; step <= steps; step++) {
      const t = step / steps;
      if (!stamp(normalizeLon(from[0] + dLon * t), from[1] + dLat * t)) return stop();
    }
    progress.done = i + 1;
  }

  return found;
}


/**
 * Target on-screen spacing of the glyph lattice, in CSS pixels, by style.
 *
 * A barb carries flags and needs more room than an arrow.
 */
export const GLYPH_TARGET_PX = { barb: 46, arrow: 34 } as const;

/** Glyph length as a fraction of the lattice spacing, by style. */
export const GLYPH_SIZE_SCALE = { barb: 0.68, arrow: 0.66 } as const;

/** How a lattice of glyphs is laid out at this scale. */
export interface GlyphLayout {
  /** Lattice step in degrees, snapped to the ladder in `GLYPH_STEPS_DEG`. */
  stepDeg: number;
  /** On-screen distance between lattice points, in device pixels. */
  spacing: number;
  /** Length of one glyph, in device pixels. */
  lengthPx: number;
}

/**
 * Glyph spacing and size for a camera scale.
 *
 * Shared by the map renderer and the brush preview: the preview draws glyphs
 * where the field's will land, so the two have to size them the same way.
 * `pixelRatio` converts the CSS-pixel target to device pixels -- specified in
 * device pixels the lattice comes out twice as dense on a retina display.
 */
export function glyphLayout(
  style: "arrow" | "barb",
  pxPerDeg: number,
  pixelRatio: number,
): GlyphLayout {
  const stepDeg = glyphStepDegrees(pxPerDeg, GLYPH_TARGET_PX[style] * pixelRatio);
  const spacing = stepDeg * pxPerDeg;
  return { stepDeg, spacing, lengthPx: spacing * GLYPH_SIZE_SCALE[style] };
}

/**
 * Whether a compact footprint covers a lattice point.
 *
 * Measured in degrees about the footprint's centre, the same way
 * {@link latticeUnderStroke} measures one stamp: a geodesic shape spans more
 * longitude the further from the equator, a projected one is the same in both
 * axes by construction (spec.md 3.5).
 */
function coversPoint(
  footprint: Extract<Footprint, { kind: "disc" | "ring" | "rect" }>,
  lon: number,
  lat: number,
): boolean {
  const [centreLon, centreLat] = footprint.centre;
  const dLat = lat - centreLat;
  const dLon = normalizeLon(lon - centreLon);
  const scale = footprint.space === "projected" ? 1 : 1 / cosLat(centreLat);

  if (footprint.kind === "rect") {
    const halfLat = footprint.halfHeightKm / KM_PER_DEGREE;
    const halfLon = (footprint.halfWidthKm / KM_PER_DEGREE) * scale;
    return Math.abs(dLat) <= halfLat && Math.abs(dLon) <= halfLon;
  }

  const radiusLat = footprint.radiusKm / KM_PER_DEGREE;
  const radiusLon = radiusLat * scale;
  if (radiusLat <= 0) return false;
  // Normalised into the footprint's own axes, so the ellipse test is a circle
  // test and the ring's two bounds are two radii of the same measure.
  const r = Math.hypot(dLat / radiusLat, dLon / radiusLon);
  if (footprint.kind === "disc") return r <= 1;
  const halfWidth = footprint.halfWidthKm / footprint.radiusKm;
  return Math.abs(r - 1) <= halfWidth;
}

/** Whether a polygon contains a point, by ray crossing. */
function polygonContains(
  points: ReadonlyArray<readonly [number, number]>,
  lon: number,
  lat: number,
): boolean {
  // Longitudes are taken relative to the first vertex so a polygon spanning the
  // dateline is tested in one continuous strip rather than two.
  const origin = points[0];
  if (origin === undefined) return false;
  const x = normalizeLon(lon - origin[0]);
  let inside = false;
  for (let i = 0; i < points.length; i++) {
    const a = points[i]!;
    const b = points[(i + 1) % points.length]!;
    const ax = normalizeLon(a[0] - origin[0]);
    const bx = normalizeLon(b[0] - origin[0]);
    if (a[1] > lat !== b[1] > lat) {
      const t = (lat - a[1]) / (b[1] - a[1]);
      if (x < ax + t * (bx - ax)) inside = !inside;
    }
  }
  return inside;
}

/**
 * The lattice points a footprint covers, for its preview glyphs.
 *
 * Generic over the footprint kinds, so a new tool previews its direction by
 * saying what it paints rather than by growing another branch here. The swept
 * case delegates to {@link latticeUnderStroke}, which walks the stroke rather
 * than its bounding box: a long thin stroke spans the viewport and covers
 * almost none of it. The compact kinds have no such gap between their box and
 * themselves, so a box scan is the cheaper answer for them.
 */
export function latticeUnder(
  footprint: Footprint,
  stepDeg: number,
  limit: number,
): Array<[number, number]> {
  if (footprint.kind === "swept") {
    return latticeUnderStroke(
      footprint.points,
      footprint.radiusKm,
      stepDeg,
      limit,
      footprint.shape,
      footprint.space,
    );
  }

  const found: Array<[number, number]> = [];
  if (limit <= 0) return found;
  const columns = Math.round(360 / stepDeg);
  const lastRow = Math.round(180 / stepDeg);

  // The box to scan, in degrees, and the test that decides what is really in.
  let south: number;
  let north: number;
  let west: number;
  let east: number;
  let inside: (lon: number, lat: number) => boolean;

  if (footprint.kind === "polygon") {
    const origin = footprint.points[0];
    if (origin === undefined) return found;
    let minX = 0;
    let maxX = 0;
    south = origin[1];
    north = origin[1];
    for (const point of footprint.points) {
      const x = normalizeLon(point[0] - origin[0]);
      minX = Math.min(minX, x);
      maxX = Math.max(maxX, x);
      south = Math.min(south, point[1]);
      north = Math.max(north, point[1]);
    }
    west = origin[0] + minX;
    east = origin[0] + maxX;
    inside = (lon, lat) => polygonContains(footprint.points, lon, lat);
  } else {
    const [centreLon, centreLat] = footprint.centre;
    const reachKm =
      footprint.kind === "rect"
        ? Math.max(footprint.halfWidthKm, footprint.halfHeightKm)
        : footprint.radiusKm + (footprint.kind === "ring" ? footprint.halfWidthKm : 0);
    const reachLat =
      footprint.kind === "rect"
        ? footprint.halfHeightKm / KM_PER_DEGREE
        : reachKm / KM_PER_DEGREE;
    const scale = footprint.space === "projected" ? 1 : 1 / cosLat(centreLat);
    const reachLon = (reachKm / KM_PER_DEGREE) * scale;
    south = centreLat - reachLat;
    north = centreLat + reachLat;
    west = centreLon - reachLon;
    east = centreLon + reachLon;
    inside = (lon, lat) => coversPoint(footprint, lon, lat);
  }

  const rowFrom = Math.max(0, Math.ceil((90 - north) / stepDeg));
  const rowTo = Math.min(lastRow, Math.floor((90 - south) / stepDeg));
  const colFrom = Math.ceil((west + 180) / stepDeg);
  const colTo = Math.floor((east + 180) / stepDeg);

  // A shape wider than the world would otherwise scan the same columns many
  // times over; one pass round the lattice reaches every point there is.
  const lastCol = Math.min(colTo, colFrom + columns - 1);

  for (let row = rowFrom; row <= rowTo; row++) {
    const lat = 90 - row * stepDeg;
    for (let col = colFrom; col <= lastCol; col++) {
      const wrapped = ((col % columns) + columns) % columns;
      const lon = -180 + wrapped * stepDeg;
      if (!inside(lon, lat)) continue;
      found.push([lon, lat]);
      if (found.length >= limit) return found;
    }
  }
  return found;
}

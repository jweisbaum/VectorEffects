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
import { cosLat, KM_PER_DEGREE } from "./footprint";

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
 * The lattice points a stroke covers, as [lon, lat].
 *
 * The map draws its glyphs on a lattice anchored to the globe (`glyphLattice`),
 * and the brush preview draws them in the same places, so what is previewed is
 * what the field will show once the stroke is committed.
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
 */
export function latticeUnderStroke(
  points: ReadonlyArray<readonly [number, number]>,
  radiusKm: number,
  stepDeg: number,
  limit: number,
  shape: BrushShape = "circle",
  space: StampSpace = "geodesic",
): Array<[number, number]> {
  const found: Array<[number, number]> = [];
  const first = points[0];
  if (first === undefined || limit <= 0) return found;

  // Every ladder step divides 360, so a column index wraps exactly.
  const columns = Math.round(360 / stepDeg);
  const lastRow = Math.round(180 / stepDeg);
  const radiusDeg = radiusKm / KM_PER_DEGREE;

  const seen = new Set<number>();
  let stamps = 0;

  /** Collects the lattice points inside one footprint. */
  const stamp = (lon: number, lat: number): boolean => {
    if (stamps++ >= MAX_STAMPS) return false;
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

  if (!stamp(first[0], first[1])) return found;

  for (let i = 1; i < points.length; i++) {
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
      if (!stamp(normalizeLon(from[0] + dLon * t), from[1] + dLat * t)) return found;
    }
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

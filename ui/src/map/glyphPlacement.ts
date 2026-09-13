/** Fill gaps in sparse fields without making a solid field's glyphs denser. */
import { projectionFor, type Camera, type GeoPoint } from "./camera";

/** Candidate sites are quarter steps of the usual globe-anchored lattice. */
export const GLYPH_SUBDIVISIONS = 4;

/**
 * Keep the ordinary lattice first, then fill holes from its half/quarter steps.
 * All sites must already have field coverage. Spacing is checked across tile
 * boundaries, in the current projection, so thin rings and strokes get marks
 * without either filling their holes or crowding the marks at a tile edge.
 * Longitudes may be unwrapped for a repeated world or a dateline-crossing view.
 */
export function spacedGlyphs<T extends GeoPoint>(
  candidates: readonly T[],
  stepDeg: number | ((point: T) => number),
  camera: Camera,
): T[] {
  const projection = projectionFor(camera);
  const ranked = candidates.map((point) => {
    const step = typeof stepDeg === "number" ? stepDeg : stepDeg(point);
    const fineStep = step / GLYPH_SUBDIVISIONS;
    const col = Math.round((point.lon + 180) / fineStep);
    const row = Math.round((90 - point.lat) / fineStep);
    const rank = col % 4 === 0 && row % 4 === 0 ? 0 : col % 2 === 0 && row % 2 === 0 ? 1 : 2;
    return { point, rank, gap: step * camera.pxPerDeg * 0.8, x: point.lon * camera.pxPerDeg, y: projection.yOf(point.lat) * camera.pxPerDeg };
  });
  // Independent of tile arrival/order and camera translation.
  ranked.sort((a, b) => a.rank - b.rank || b.point.lat - a.point.lat || a.point.lon - b.point.lon);
  const gap = ranked.reduce((max, site) => Math.max(max, site.gap), 1);
  const bins = new Map<number, Map<number, Array<{ x: number; y: number; gap: number }>>>();
  const result: T[] = [];
  for (const site of ranked) {
    const bx = Math.floor(site.x / gap);
    const by = Math.floor(site.y / gap);
    let crowded = false;
    for (let dy = -1; dy <= 1 && !crowded; dy++) {
      const line = bins.get(by + dy);
      if (!line) continue;
      for (let dx = -1; dx <= 1 && !crowded; dx++) {
        for (const other of line.get(bx + dx) ?? []) {
          if ((site.x - other.x) ** 2 + (site.y - other.y) ** 2 < ((site.gap + other.gap) / 2) ** 2 - 1e-6) {
            crowded = true;
            break;
          }
        }
      }
    }
    if (crowded) continue;
    let line = bins.get(by);
    if (!line) { line = new Map(); bins.set(by, line); }
    const bin = line.get(bx);
    if (bin) bin.push(site);
    else line.set(bx, [site]);
    result.push(site.point);
  }
  return result;
}

const EMPTY_COVERAGE = new Uint8Array(8192);
const FULL_COVERAGE = new Uint8Array(8192).fill(255);

/** True for a tile with field at every site, including calm values. */
export function glyphTileIsFull(mask: Uint8Array): boolean {
  return mask === FULL_COVERAGE;
}

/** No covered texels of this kind. */
export function glyphTileIsEmpty(mask: Uint8Array): boolean {
  return mask === EMPTY_COVERAGE;
}

/**
 * A bit per texel saying whether any layer wrote it, including written calm.
 * Kept beside the GPU texture (8 KiB for a 256² tile), never persisted. Layout
 * can inspect coverage without reading pixels back or retaining field values.
 */
export function glyphCoverage(bytes: Uint8Array, windOnly = false): Uint8Array {
  const mask = new Uint8Array(Math.ceil(bytes.length / 32));
  let covered = 0;
  for (let texel = 0; texel < bytes.length / 4; texel++) {
    if ((bytes[texel * 4 + 3]! & 0x7c) !== 0 && (!windOnly || (bytes[texel * 4 + 3]! & 0x80) !== 0)) {
      mask[texel >> 3]! |= 1 << (texel & 7);
      covered++;
    }
  }
  // Most imported tiles are completely covered. Sharing their mask also lets
  // playback reuse a layout when only the vectors, not their coverage, change.
  if (bytes.length === 256 * 256 * 4) {
    if (covered === 0) return EMPTY_COVERAGE;
    if (covered === 256 * 256) return FULL_COVERAGE;
  }
  return mask;
}

/** The same nearest-texel lookup the glyph shader uses. */
export function glyphCovered(mask: Uint8Array, u: number, v: number): boolean {
  const x = Math.max(0, Math.min(255, Math.floor(u * 256)));
  const y = Math.max(0, Math.min(255, Math.floor(v * 256)));
  const texel = y * 256 + x;
  return (mask[texel >> 3]! & (1 << (texel & 7))) !== 0;
}

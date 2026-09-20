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

/**
 * A screen lattice's sites in the order they are offered a place: the coarse
 * lattice first (every fourth site each way), then every second, then the
 * rest, each rank row by row. A sparse field's glyphs fill in between the
 * coarse ones without ever displacing them.
 *
 * Generated rank by rank rather than collected and sorted. Under a general
 * projection this runs every frame the camera moves, and at a retina window
 * it is twenty thousand sites: sorting them was a fifth of a frame.
 */
export function* rankedSites(
  width: number,
  height: number,
  fine: number,
  margin: number,
): Generator<{ x: number; y: number }> {
  const lastRow = Math.ceil((height + margin) / fine);
  const lastCol = Math.ceil((width + margin) / fine);
  for (let rank = 0; rank < 3; rank++) {
    for (let row = -4; row <= lastRow; row++) {
      for (let col = -4; col <= lastCol; col++) {
        const own = col % 4 === 0 && row % 4 === 0 ? 0 : col % 2 === 0 && row % 2 === 0 ? 1 : 2;
        if (own === rank) yield { x: col * fine, y: row * fine };
      }
    }
  }
}

/**
 * The glyphs placed so far, asked whether a new one would crowd any of them.
 *
 * Two glyphs crowd when they are nearer than the mean of their gaps. Asked of
 * every placed glyph in turn that is quadratic, and it was most of what
 * turning a globe with glyphs on cost: fifteen hundred placed, twenty
 * thousand asking. Binned by `reach` — no smaller than any gap — only the
 * nine bins round a site can hold a glyph near enough to matter.
 */
export class PlacedGlyphs {
  private readonly bins = new Map<number, Array<{ x: number; y: number; gap: number }>>();

  constructor(private readonly reach: number) {}

  private key(column: number, row: number): number {
    // Sites start four cells off screen, so the indices can be negative.
    return (column + 32768) * 65536 + (row + 32768);
  }

  crowds(x: number, y: number, gap: number): boolean {
    const column = Math.floor(x / this.reach);
    const row = Math.floor(y / this.reach);
    for (let j = row - 1; j <= row + 1; j++) {
      for (let i = column - 1; i <= column + 1; i++) {
        const bin = this.bins.get(this.key(i, j));
        if (bin?.some((p) => Math.hypot(p.x - x, p.y - y) < (p.gap + gap) / 2)) return true;
      }
    }
    return false;
  }

  place(x: number, y: number, gap: number): void {
    const key = this.key(Math.floor(x / this.reach), Math.floor(y / this.reach));
    const bin = this.bins.get(key);
    if (bin) bin.push({ x, y, gap });
    else this.bins.set(key, [{ x, y, gap }]);
  }
}

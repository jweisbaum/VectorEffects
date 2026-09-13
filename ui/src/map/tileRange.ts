/**
 * The range of speeds a tile holds, read from its bytes as they are uploaded.
 *
 * What the auto scale (spec.md 5.3, M27) runs the colour ramp over: the
 * slowest and fastest speed among the tiles on screen. Read once per tile on
 * the CPU, from the same bytes the GPU gets, so it costs nothing per frame
 * and never needs a readback. Per kind (M31): wind and current are drawn on
 * their own scales, and one tile can hold both.
 */

import type { FieldKindName } from "../kind";

/** One little-endian 32-bit word per texel (spec.md 7.7). */
const BYTES_PER_TEXEL = 4;
/** The speed is the low 14 bits, a fraction of full scale. */
export const SPEED_MAX = 0x3fff;
const COVERAGE_SHIFT = 26;
const COVERAGE_MASK = 0x1f;
/**
 * The coverage at which a cell counts as field rather than as an edge's fade.
 *
 * Half. A cell more than half covered is showing what was painted; one less
 * than half is most of the way to nothing, and its speed says how far down the
 * feather it is rather than how fast the field is there.
 */
const SOLID_COVERAGE = (COVERAGE_MASK + 1) / 2;

/** The smallest and largest speed of each kind in a tile, as 14-bit fractions of full scale. */
export type TileRanges = Record<FieldKindName, [number, number] | null>;

/**
 * The range of speeds of each kind in a tile, as 14-bit fractions of full
 * scale, or null for a kind the tile holds none of.
 *
 * **An unwritten cell is left out.** Its coverage is zero, which is how the
 * tile tells a cell nothing wrote from a written calm (D58, M31): the calm
 * counts, and can put the bottom of the ramp at zero; the unwritten cell
 * does not, or the ramp would start at zero whenever any of the viewport
 * is unpainted, which is nearly always.
 *
 * **A faded cell is left out too** (M81). A cell's vector is premultiplied by
 * its coverage, so the rim of every feathered object ramps from its own speed
 * down to nothing — and since a feather is the default, some cell in view is
 * always most of the way down that ramp. The bottom of the scale was
 * therefore pinned near zero whatever was on screen, and only the top ever
 * moved. A cell that is mostly covered is field; one that is mostly not is
 * the fade at an edge, and describes the edge rather than the field. If a
 * kind has no solid cell at all — a wholly feathered object, a thin rim and
 * nothing else — the faded ones are used rather than reporting nothing.
 */
export function tileSpeedRange(bytes: Uint8Array): TileRanges {
  // Scalar accumulators avoid a dynamic string-key lookup for every texel.
  // DataView reads the packed little-endian word in one operation and also
  // supports subarrays whose byteOffset is not aligned to four bytes.
  let windMin = SPEED_MAX + 1, windMax = -1;
  let currentMin = SPEED_MAX + 1, currentMax = -1;
  let faintWindMin = SPEED_MAX + 1, faintWindMax = -1;
  let faintCurrentMin = SPEED_MAX + 1, faintCurrentMax = -1;
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  for (let offset = 0; offset + 3 < bytes.length; offset += BYTES_PER_TEXEL) {
    const word = view.getUint32(offset, true);
    const coverage = (word >>> COVERAGE_SHIFT) & COVERAGE_MASK;
    if (coverage === 0) continue;
    const speed = word & SPEED_MAX;
    if (word >>> 31) {
      if (speed < faintWindMin) faintWindMin = speed;
      if (speed > faintWindMax) faintWindMax = speed;
      if (coverage >= SOLID_COVERAGE) {
        if (speed < windMin) windMin = speed;
        if (speed > windMax) windMax = speed;
      }
    } else {
      if (speed < faintCurrentMin) faintCurrentMin = speed;
      if (speed > faintCurrentMax) faintCurrentMax = speed;
      if (coverage >= SOLID_COVERAGE) {
        if (speed < currentMin) currentMin = speed;
        if (speed > currentMax) currentMax = speed;
      }
    }
  }
  return {
    wind: windMax >= 0 ? [windMin, windMax] : faintWindMax >= 0 ? [faintWindMin, faintWindMax] : null,
    current: currentMax >= 0 ? [currentMin, currentMax] : faintCurrentMax >= 0 ? [faintCurrentMin, faintCurrentMax] : null,
  };
}

/** Whether a tile holds any written cell of either kind. */
export function hasField(ranges: TileRanges): boolean {
  return ranges.wind !== null || ranges.current !== null;
}

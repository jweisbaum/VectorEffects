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
  const min = { wind: SPEED_MAX + 1, current: SPEED_MAX + 1 };
  const max = { wind: -1, current: -1 };
  // The same, over every written cell whatever its coverage, for a kind whose
  // cells are all fade.
  const faintMin = { wind: SPEED_MAX + 1, current: SPEED_MAX + 1 };
  const faintMax = { wind: -1, current: -1 };
  for (let o = 0; o + 3 < bytes.length; o += BYTES_PER_TEXEL) {
    // Assembled in two halves: a 32-bit shift of the top byte would go negative.
    const low = (bytes[o] ?? 0) | ((bytes[o + 1] ?? 0) << 8);
    const high = (bytes[o + 2] ?? 0) | ((bytes[o + 3] ?? 0) << 8);
    const coverage = (high >> (COVERAGE_SHIFT - 16)) & COVERAGE_MASK;
    if (coverage === 0) continue;
    const kind: FieldKindName = high >> 15 === 1 ? "wind" : "current";
    const speed = low & SPEED_MAX;
    if (speed < faintMin[kind]) faintMin[kind] = speed;
    if (speed > faintMax[kind]) faintMax[kind] = speed;
    if (coverage < SOLID_COVERAGE) continue;
    if (speed < min[kind]) min[kind] = speed;
    if (speed > max[kind]) max[kind] = speed;
  }
  const rangeOf = (kind: FieldKindName): [number, number] | null => {
    if (max[kind] >= 0) return [min[kind], max[kind]];
    return faintMax[kind] < 0 ? null : [faintMin[kind], faintMax[kind]];
  };
  return {
    wind: rangeOf("wind"),
    current: rangeOf("current"),
  };
}

/** Whether a tile holds any written cell of either kind. */
export function hasField(ranges: TileRanges): boolean {
  return ranges.wind !== null || ranges.current !== null;
}

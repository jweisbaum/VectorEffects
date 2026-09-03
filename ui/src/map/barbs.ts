/**
 * Wind barb decomposition.
 *
 * Barbs encode speed with flags rather than length: a pennant is 50 knots, a
 * full barb 10, a half barb 5, and calm is a bare circle. Speed is rounded to
 * the nearest 5 knots first, which is the convention -- a barb showing 13 knots
 * as "10 plus a half" would be misread as 15.
 *
 * Pure, so the encoding can be checked against a reference chart in tests
 * rather than by squinting at the screen.
 */

/** Barb elements for one observation. */
export interface BarbElements {
  /** 50-knot pennants. */
  pennants: number;
  /** 10-knot full barbs. */
  full: number;
  /** 5-knot half barbs: 0 or 1. */
  half: number;
  /** True below 2.5 knots, drawn as a bare circle. */
  calm: boolean;
}

/** Speed below which a barb becomes a calm circle, in knots. */
export const CALM_KNOTS = 2.5;

/** Decomposes a speed in knots into barb elements. */
export function barbElements(knots: number): BarbElements {
  if (!Number.isFinite(knots) || knots < CALM_KNOTS) {
    return { pennants: 0, full: 0, half: 0, calm: true };
  }
  // Round to the nearest 5 kt before decomposing, per convention.
  let remaining = Math.round(knots / 5) * 5;

  const pennants = Math.floor(remaining / 50);
  remaining -= pennants * 50;
  const full = Math.floor(remaining / 10);
  remaining -= full * 10;
  const half = remaining >= 5 ? 1 : 0;

  return { pennants, full, half, calm: false };
}

/** Total elements to draw, used to size the per-instance vertex budget. */
export function barbElementCount(elements: BarbElements): number {
  return elements.pennants + elements.full + elements.half;
}

/** The most elements any drawable barb needs. */
export const MAX_BARB_ELEMENTS = 8;

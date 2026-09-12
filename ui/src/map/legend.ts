/**
 * The colour legend's numbers (spec.md 5.3, M81).
 *
 * Its own module so the formatting can be tested without a WebGL context: the
 * map view that draws the legend cannot be imported in a plain test.
 */

/**
 * A speed for one end of the scale, with the precision the span needs.
 *
 * Whole knots said nothing about a current. They run at a knot or two, so both
 * ends rounded to the same number and the legend read "0 to 1" over a field
 * with a tenth of a knot at one end and nine tenths at the other — the numbers
 * were there but told you nothing about what you were looking at.
 *
 * The step comes from the **span**, not from the kind, so a slow wind is
 * treated like a slow current and a fast current is not given false precision.
 * Trailing zeroes go, because "0.50" claims a hundredth the scale does not
 * have.
 */
export function legendKnots(knots: number, spanKnots: number): string {
  if (!Number.isFinite(knots)) return "0";
  if (spanKnots < 2) return trimmed(knots.toFixed(2));
  if (spanKnots < 20) return trimmed(knots.toFixed(1));
  return String(Math.round(knots));
}

/** Drops trailing zeroes, and the point with them. */
function trimmed(text: string): string {
  return text.includes(".") ? text.replace(/0+$/, "").replace(/\.$/, "") : text;
}

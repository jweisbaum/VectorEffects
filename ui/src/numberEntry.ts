/**
 * What a keystroke in a number field means.
 *
 * The rule these two functions exist for: **a number field must be clearable.**
 * A controlled input written as `value={n}` with
 * `onChange={e => commit(Number(e.target.value) || fallback)}` cannot be — the
 * empty string parses as `NaN`, the fallback is committed, and the box refills
 * under the cursor before the next key arrives. A size of 500 could then become
 * 5000 by typing in the middle, and 40 not at all.
 *
 * Kept apart from the component so the rule can be tested without a DOM.
 */

/**
 * The number a field's contents mean, or `null` for contents that are not a
 * number *yet*.
 *
 * Empty is not a number, and neither is a lone `-` or `.` — both are what a
 * negative or a fractional entry looks like on its way in. Returning `null`
 * says "leave the value alone and let them keep typing", which is exactly what
 * the broken version could not express.
 */
export function typedValue(raw: string): number | null {
  const trimmed = raw.trim();
  if (trimmed === "") return null;
  const value = Number(trimmed);
  return Number.isFinite(value) ? value : null;
}

/**
 * A value brought inside its bounds.
 *
 * Applied when the field is *left*, not as it is typed: clamping a half-typed
 * number is how a minimum of 1 turns an attempt to enter 40 into 1 and then 14.
 */
export function clamped(value: number, min: number | null, max: number | null): number {
  let out = value;
  if (min !== null && out < min) out = min;
  if (max !== null && out > max) out = max;
  return out;
}

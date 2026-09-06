/**
 * A centred slider's geometry (M29).
 *
 * The thumb's travel is `-1..1` with zero in the middle, whatever the stored
 * range: the left half maps onto `[min, 0]` and the right onto `[0, max]`,
 * each linearly, so an amount from −100% to +200% still has its "do nothing"
 * in the middle of the track. A range that does not straddle zero is one
 * straight line.
 */

/** Where on the track (`-1..1`) a stored value sits. */
export function positionOf(value: number, min: number, max: number): number {
  if (!(min < 0 && max > 0)) {
    const span = max - min;
    return span === 0 ? 0 : ((value - min) / span) * 2 - 1;
  }
  // Each half is its own straight line: a negative value is a fraction of
  // the way to `min` on the left, a positive one of the way to `max`.
  const t = value < 0 ? -(value / min) : value / max;
  return Math.max(-1, Math.min(1, t));
}

/** The stored value at a track position. */
export function valueOf(position: number, min: number, max: number): number {
  const t = Math.max(-1, Math.min(1, position));
  if (!(min < 0 && max > 0)) return min + ((t + 1) / 2) * (max - min);
  return t < 0 ? -t * min : t * max;
}

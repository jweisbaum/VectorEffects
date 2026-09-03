/**
 * The lat/lon rectangle a rubber-band drag describes.
 *
 * The latitudes are just a min and a max. The longitudes are not: a band
 * dragged rightwards across the dateline runs from, say, 170° to -170°, and the
 * region it covers is the 20° that wraps, not the 340° that does not. Only the
 * *order* of the drag says which, so it has to be carried here rather than
 * recovered by comparing numbers — which is why this is a function with a test
 * instead of two `Math.min` calls at the call site.
 */
export interface MarqueeBounds {
  west: number;
  south: number;
  east: number;
  north: number;
}

/**
 * @param a Geographic position where the drag started.
 * @param b Geographic position where it ended.
 * @param leftToRight Whether the pointer moved rightwards on screen.
 */
export function marqueeBounds(
  a: { lon: number; lat: number },
  b: { lon: number; lat: number },
  leftToRight: boolean,
): MarqueeBounds {
  return {
    west: leftToRight ? a.lon : b.lon,
    east: leftToRight ? b.lon : a.lon,
    south: Math.min(a.lat, b.lat),
    north: Math.max(a.lat, b.lat),
  };
}

/** Whether `lon` lies in the arc running east from `west` to `east`. */
export function containsLon(bounds: MarqueeBounds, lon: number): boolean {
  const span = mod360(bounds.east - bounds.west);
  return mod360(lon - bounds.west) <= span;
}

function mod360(degrees: number): number {
  return ((degrees % 360) + 360) % 360;
}

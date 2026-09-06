/**
 * The keyframes of a macro's track (spec.md 8.7, M27).
 *
 * A saved macro holds each frame's displacement from the first and not the
 * keys that were placed while it was recorded. Between keys the position
 * interpolates, so the track runs straight and evenly from one key to the
 * next and bends, speeds up, slows down or stops only *at* a key. The keys
 * are therefore recoverable from the track as the frames where the step
 * changes — which is what the stamp hover draws a box at.
 */

/** A displacement from the first frame, degrees east and north. */
export type TrackPoint = readonly [number, number];

/**
 * How much the step between frames may change, as a fraction of the mean
 * step, before a frame counts as a key. Great-circle interpolation is not
 * quite even in degrees, so a little play is needed; a placed key moves the
 * region by far more than this.
 */
const BEND_FRACTION = 0.05;

/** Indices of the track's keyframes, in order. The first and last frame always are. */
export function trackKeyframes(track: readonly TrackPoint[]): number[] {
  const last = track.length - 1;
  if (last <= 0) return track.length === 0 ? [] : [0];
  if (last === 1) return [0, 1];

  let travelled = 0;
  for (let i = 1; i <= last; i += 1) {
    const [ax, ay] = track[i - 1] as TrackPoint;
    const [bx, by] = track[i] as TrackPoint;
    travelled += Math.hypot(bx - ax, by - ay);
  }
  // A track that never moves is one box: a still macro, or a static recording.
  if (travelled === 0) return [0, last];
  const tolerance = (travelled / last) * BEND_FRACTION;

  const keys = [0];
  for (let i = 1; i < last; i += 1) {
    const [px, py] = track[i - 1] as TrackPoint;
    const [x, y] = track[i] as TrackPoint;
    const [nx, ny] = track[i + 1] as TrackPoint;
    const bend = Math.hypot(nx - x - (x - px), ny - y - (y - py));
    if (bend > tolerance) keys.push(i);
  }
  keys.push(last);
  return keys;
}

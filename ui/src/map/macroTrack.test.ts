import { describe, expect, it } from "vitest";

import { type TrackPoint, trackKeyframes } from "./macroTrack";

/** A track that runs straight from `from` to `to` in `steps` even steps. */
function run(from: TrackPoint, to: TrackPoint, steps: number): TrackPoint[] {
  return Array.from({ length: steps + 1 }, (_, i) => [
    from[0] + ((to[0] - from[0]) * i) / steps,
    from[1] + ((to[1] - from[1]) * i) / steps,
  ]);
}

describe("trackKeyframes", () => {
  it("is one box for a still macro and two for a single hop", () => {
    expect(trackKeyframes([])).toEqual([]);
    expect(trackKeyframes([[0, 0]])).toEqual([0]);
    expect(trackKeyframes([[0, 0], [3, 1]])).toEqual([0, 1]);
  });

  it("is the two ends of a straight, even run", () => {
    expect(trackKeyframes(run([0, 0], [10, 5], 8))).toEqual([0, 8]);
  });

  it("finds the frame where the course changes", () => {
    const track = [...run([0, 0], [6, 0], 3), ...run([6, 0], [6, 6], 3).slice(1)];
    expect(trackKeyframes(track)).toEqual([0, 3, 6]);
  });

  it("finds a pause and the frame the motion resumes", () => {
    const track: TrackPoint[] = [...run([0, 0], [4, 0], 2), [4, 0], [4, 0], ...run([4, 0], [8, 0], 2).slice(1)];
    // Frames: 0,1,2 moving; 2,3,4 still; 4,5,6 moving again.
    expect(trackKeyframes(track)).toEqual([0, 2, 4, 6]);
  });

  it("tolerates the unevenness of a great circle in degrees", () => {
    // Steps that vary by a percent are one leg, not five keys.
    const track: TrackPoint[] = [[0, 0]];
    for (let i = 1; i <= 6; i += 1) {
      const step = 1 + 0.01 * Math.sin(i);
      const [x, y] = track[i - 1] as TrackPoint;
      track.push([x + step, y]);
    }
    expect(trackKeyframes(track)).toEqual([0, 6]);
  });

  it("is two boxes at one place for a recording that never moved", () => {
    expect(trackKeyframes([[0, 0], [0, 0], [0, 0], [0, 0]])).toEqual([0, 3]);
  });
});

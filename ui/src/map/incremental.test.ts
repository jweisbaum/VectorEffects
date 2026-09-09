/**
 * Extending a stroke's preview is exactly rebuilding it.
 *
 * A stroke in progress gains a point per pointer report. Rebuilding its swept
 * path and its glyph lattice from the first point on every report is quadratic
 * in the stroke's length, so both are extended instead — and an extension that
 * drifted from a rebuild would be a preview that changed shape as it was drawn,
 * which no user could tell from a bug in the brush.
 *
 * So the claim is equality, not closeness: the same stamps in the same order,
 * the same lattice points in the same order, through every cap, at every
 * length, across the dateline.
 */
import { describe, expect, it } from "vitest";

import type { Camera, Viewport } from "./camera";
import {
  buildStrokePath,
  extendStrokePath,
  freshSweptPath,
  type PathSink,
} from "./footprint";
import { extendLatticeUnderStroke, freshLattice, latticeUnderStroke } from "./glyph";

const camera: Camera = { centerLon: 0, centerLat: 20, pxPerDeg: 12 };
const view: Viewport = { width: 1440, height: 900 };

/** Records every call, so two paths can be compared op for op. */
class Recorder implements PathSink {
  readonly ops: string[] = [];
  moveTo(x: number, y: number) {
    this.ops.push(`M${x},${y}`);
  }
  ellipse(x: number, y: number, rx: number, ry: number) {
    this.ops.push(`E${x},${y},${rx},${ry}`);
  }
  rect(x: number, y: number, w: number, h: number) {
    this.ops.push(`R${x},${y},${w},${h}`);
  }
  lineTo(x: number, y: number) {
    this.ops.push(`L${x},${y}`);
  }
  closePath() {
    this.ops.push("Z");
  }
}

/** A deterministic wandering stroke of `n` points, optionally over the dateline. */
function stroke(n: number, seed: number, acrossDateline = false): Array<[number, number]> {
  let state = seed;
  const next = () => {
    state = (state * 1_103_515_245 + 12_345) & 0x7fff_ffff;
    return state / 0x7fff_ffff;
  };
  const points: Array<[number, number]> = [];
  let lon = acrossDateline ? 176 : -20;
  let lat = 20;
  for (let i = 0; i < n; i++) {
    // A drag: mostly small steps, the occasional fast flick.
    const flick = next() < 0.05 ? 6 : 1;
    lon += (next() - 0.3) * 0.4 * flick;
    lat += (next() - 0.5) * 0.3 * flick;
    lon = ((lon + 540) % 360) - 180;
    lat = Math.max(-85, Math.min(85, lat));
    points.push([lon, lat]);
  }
  return points;
}

const CASES = [
  { name: "a short stroke", n: 12, seed: 1 },
  { name: "a long stroke", n: 600, seed: 2 },
  { name: "a stroke across the dateline", n: 200, seed: 3, dateline: true },
  // Enough stamps to hit the path's cap and enough glyphs to hit the limit.
  { name: "a stroke past every cap", n: 3000, seed: 4 },
] as const;

describe("extending a swept path", () => {
  for (const { name, n, seed, ...rest } of CASES) {
    const dateline = "dateline" in rest && rest.dateline === true;
    for (const shape of ["circle", "square"] as const) {
      for (const space of ["geodesic", "projected"] as const) {
        it(`matches a rebuild for ${name} (${shape}, ${space})`, () => {
          const points = stroke(n, seed, dateline);

          // Extended one point at a time, as a drag delivers them.
          const extended = new Recorder();
          const progress = freshSweptPath();
          for (let k = 1; k <= points.length; k++) {
            extendStrokePath(extended, camera, view, points.slice(0, k), progress, 150, shape, space);
          }

          const rebuilt = new Recorder();
          buildStrokePath(rebuilt, camera, view, points, 150, shape, space);

          expect(extended.ops).toEqual(rebuilt.ops);
          expect(progress.done).toBe(points.length);
        });
      }
    }
  }

  it("adds nothing when extended with no new points", () => {
    const points = stroke(40, 5);
    const sink = new Recorder();
    const progress = freshSweptPath();
    extendStrokePath(sink, camera, view, points, progress, 150);
    const before = sink.ops.length;
    extendStrokePath(sink, camera, view, points, progress, 150);
    expect(sink.ops.length).toBe(before);
  });

  it("adds nothing for an empty stroke", () => {
    const sink = new Recorder();
    extendStrokePath(sink, camera, view, [], freshSweptPath(), 150);
    expect(sink.ops).toEqual([]);
  });
});

describe("extending a lattice walk", () => {
  for (const { name, n, seed, ...rest } of CASES) {
    const dateline = "dateline" in rest && rest.dateline === true;
    for (const shape of ["circle", "square"] as const) {
      for (const space of ["geodesic", "projected"] as const) {
        it(`matches a rebuild for ${name} (${shape}, ${space})`, () => {
          const points = stroke(n, seed, dateline);

          const progress = freshLattice();
          let extended: Array<[number, number]> = [];
          for (let k = 1; k <= points.length; k++) {
            extended = extendLatticeUnderStroke(
              points.slice(0, k),
              progress,
              150,
              0.5,
              600,
              shape,
              space,
            );
          }

          const rebuilt = latticeUnderStroke(points, 150, 0.5, 600, shape, space);
          expect(extended).toEqual(rebuilt);
        });
      }
    }
  }

  /**
   * Once a cap is hit, a rebuild of the same stroke stops there too. An
   * extension that carried on past it would collect points the rebuild never
   * reaches, and the two would disagree from then on.
   */
  it("stays stopped after a cap, exactly as a rebuild would", () => {
    const points = stroke(3000, 6);
    const progress = freshLattice();
    for (let k = 1; k <= points.length; k++) {
      extendLatticeUnderStroke(points.slice(0, k), progress, 150, 0.5, 40);
    }
    expect(progress.exhausted).toBe(true);
    expect(progress.found.length).toBe(40);
    expect(progress.found).toEqual(latticeUnderStroke(points, 150, 0.5, 40));
  });

  it("honours a limit of nothing", () => {
    expect(extendLatticeUnderStroke(stroke(5, 7), freshLattice(), 150, 0.5, 0)).toEqual([]);
  });
});

/**
 * What a stroke preview costs per pointer report, before and after.
 *
 * The frontend's `tile_cost`: it reports rather than asserts, so the number is
 * there to read whenever the walk changes. The one assertion is the property
 * the extension exists for — that a long stroke costs about the same per
 * report as a short one, where rebuilding from the first point cost more with
 * every point added.
 *
 * Pure JS only: the canvas rasterising the path is the larger cost in the app
 * and cannot be measured here, but it scales the same way — every stamp in the
 * path is filled again on a rebuild and only the new ones on an extension.
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
const view: Viewport = { width: 2880, height: 1684 };

/** Counts ops without keeping them; the path builder's cost, not the array's. */
class Counter implements PathSink {
  n = 0;
  moveTo() {
    this.n += 1;
  }
  ellipse() {
    this.n += 1;
  }
  rect() {
    this.n += 1;
  }
}

function wander(n: number): Array<[number, number]> {
  const points: Array<[number, number]> = [];
  for (let i = 0; i < n; i++) points.push([i * 0.05, 20 + Math.sin(i / 7) * 2]);
  return points;
}

/** The whole drag, one report per point, rebuilt from scratch each time. */
function rebuiltDrag(points: Array<[number, number]>): { ms: number; ops: number } {
  const t0 = performance.now();
  let ops = 0;
  for (let k = 1; k <= points.length; k++) {
    const prefix = points.slice(0, k);
    const sink = new Counter();
    buildStrokePath(sink, camera, view, prefix, 150);
    ops += sink.n;
    latticeUnderStroke(prefix, 150, 0.5, 600);
  }
  return { ms: performance.now() - t0, ops };
}

/** The same drag, extended a segment at a time. */
function extendedDrag(points: Array<[number, number]>): { ms: number; ops: number } {
  const t0 = performance.now();
  const sink = new Counter();
  const progress = freshSweptPath();
  const lattice = freshLattice();
  // The stroke array grows in place, as the gesture's does.
  const live: Array<[number, number]> = [];
  for (const point of points) {
    live.push(point);
    extendStrokePath(sink, camera, view, live, progress, 150);
    extendLatticeUnderStroke(live, lattice, 150, 0.5, 600);
  }
  return { ms: performance.now() - t0, ops: sink.n };
}

describe("stroke preview cost", () => {
  it("reports the cost of a drag, rebuilt and extended", () => {
    const lines: string[] = [];
    let ratioShort = 0;
    let ratioLong = 0;
    for (const n of [50, 200, 500, 1000]) {
      const points = wander(n);
      // Warm both paths once so neither pays for JIT on the clock.
      rebuiltDrag(points.slice(0, 20));
      extendedDrag(points.slice(0, 20));

      const rebuilt = rebuiltDrag(points);
      const extended = extendedDrag(points);
      lines.push(
        `n=${String(n).padStart(4)}: rebuilt ${rebuilt.ms.toFixed(1).padStart(6)} ms ` +
          `(${rebuilt.ops} stamps), extended ${extended.ms.toFixed(1).padStart(5)} ms ` +
          `(${extended.ops} stamps), ${(rebuilt.ms / Math.max(extended.ms, 0.01)).toFixed(0)}x`,
      );
      if (n === 50) ratioShort = extended.ms / n;
      if (n === 1000) ratioLong = extended.ms / n;
    }
    console.log(`stroke preview cost per drag:\n  ${lines.join("\n  ")}`);

    // The property: per-report cost does not grow with the stroke. A generous
    // bound, since a node timer is noisy; what it rules out is the quadratic.
    expect(ratioLong).toBeLessThan(ratioShort * 5 + 0.05);
  });
});

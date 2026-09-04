/**
 * The playback and readiness rules (spec.md 9.4, 9.5).
 *
 * The acceptance criterion that matters most here — playback holds rather
 * than stutters when frames are not ready — is a rule, and it is tested as one.
 */
import { describe, expect, it } from "vitest";

import type { TimelineReadiness } from "../generated/TimelineReadiness";
import {
  classify,
  draggedStep,
  forecastLabel,
  freshMemory,
  labelEvery,
  mayAdvanceInto,
  nextStep,
  stepAt,
  type StepState,
  tick,
  uniqueTiles,
  utcLabel,
} from "./playback";

function report(revision: number, ready: number[], total = 4): TimelineReadiness {
  return {
    revision,
    steps: ready.map((n, step) => ({ step, ready: n, total })),
  };
}

describe("classify", () => {
  it("names empty, partial and solid from the counts alone", () => {
    const { states, progress } = classify(report(1, [0, 2, 4]), freshMemory());
    expect(states).toEqual(["empty", "partial", "solid"]);
    expect(progress).toEqual([0, 0.5, 1]);
  });

  /**
   * Spec 9.5: a distinct stale state when an edit has invalidated a
   * previously-ready frame. The backend reports only what the cache holds; the
   * difference between "never rendered" and "rendered, then invalidated" is
   * memory the timeline keeps.
   */
  it("marks a frame stale when it was solid at an earlier revision", () => {
    const memory = freshMemory();
    classify(report(1, [4, 4, 4]), memory);

    // An edit at revision 2 invalidated step 1 only.
    const { states } = classify(report(2, [4, 0, 4]), memory);
    expect(states).toEqual(["solid", "stale", "solid"]);
  });

  it("forgets stale once the frame is solid again", () => {
    const memory = freshMemory();
    classify(report(1, [4]), memory);
    classify(report(2, [0]), memory);
    expect(classify(report(2, [4]), memory).states).toEqual(["solid"]);
    // A later invalidation is stale again, relative to the new solid.
    expect(classify(report(3, [1]), memory).states).toEqual(["stale"]);
  });

  /** Never solid means never stale: the first render is just not done. */
  it("does not call a frame stale that was never ready", () => {
    const memory = freshMemory();
    classify(report(1, [0]), memory);
    expect(classify(report(2, [2]), memory).states).toEqual(["partial"]);
  });

  /** A viewport of no tiles is not solid; it is nothing. */
  it("treats an empty viewport as empty", () => {
    expect(classify(report(1, [0], 0), freshMemory()).states).toEqual(["empty"]);
  });
});

describe("advancing", () => {
  it("moves to the next step and stops or loops at the end", () => {
    expect(nextStep(3, 9, false)).toBe(4);
    expect(nextStep(9, 9, false)).toBeNull();
    expect(nextStep(9, 9, true)).toBe(0);
  });

  /**
   * Spec 9.4: playback only advances into frames that are ready. Partial,
   * empty and stale all hold — stale in particular, since the tiles on screen
   * for that step belong to a revision that no longer exists.
   */
  it("advances only into a solid frame", () => {
    expect(mayAdvanceInto(1, ["solid", "solid"])).toBe(true);
    expect(mayAdvanceInto(1, ["solid", "partial"])).toBe(false);
    expect(mayAdvanceInto(1, ["solid", "empty"])).toBe(false);
    expect(mayAdvanceInto(1, ["solid", "stale"])).toBe(false);
    expect(mayAdvanceInto(5, ["solid"])).toBe(false);
  });

  /**
   * Rendered is not shown: the backend can hold a tile the GPU has not fetched
   * yet, and advancing on the backend's word alone draws the previous step
   * under the new one for a frame. Both have to agree.
   */
  it("advances only into a frame the map holds", () => {
    expect(mayAdvanceInto(1, ["solid", "solid"], () => false)).toBe(false);
    expect(mayAdvanceInto(1, ["solid", "solid"], (step) => step === 1)).toBe(true);
  });
});

describe("tick", () => {
  const allSolid: StepState[] = Array<StepState>(10).fill("solid");

  it("waits for the interval at the chosen rate", () => {
    // 8 steps a second is 125 ms a step.
    expect(tick(0, 9, false, 8, 100, allSolid).advanced).toBe(false);
    expect(tick(0, 9, false, 8, 125, allSolid)).toMatchObject({ step: 1, advanced: true });
  });

  /** The criterion: hold, and say so, rather than stutter. */
  it("holds and reports buffering when the next frame is not ready", () => {
    const states = [...allSolid];
    states[1] = "partial";
    const result = tick(0, 9, false, 8, 500, states);
    expect(result).toEqual({ step: 0, advanced: false, buffering: true, finished: false });
  });

  it("holds and reports buffering until the map holds the next frame", () => {
    const result = tick(0, 9, false, 8, 500, allSolid, () => false);
    expect(result).toEqual({ step: 0, advanced: false, buffering: true, finished: false });
    expect(tick(0, 9, false, 8, 500, allSolid, () => true).step).toBe(1);
  });

  /**
   * A stall of many intervals resumes at the *next* step. Advancing as many
   * steps as the clock says would turn a long render into a skip.
   */
  it("never skips steps after a stall", () => {
    expect(tick(2, 9, false, 8, 5000, allSolid).step).toBe(3);
  });

  it("finishes at the end without loop, and wraps with it", () => {
    expect(tick(9, 9, false, 8, 200, allSolid).finished).toBe(true);
    expect(tick(9, 9, true, 8, 200, allSolid)).toMatchObject({ step: 0, advanced: true });
  });

  it("holds at the wrap too, if step 0 is not ready", () => {
    const states = [...allSolid];
    states[0] = "empty";
    expect(tick(9, 9, true, 8, 200, states).buffering).toBe(true);
  });
});

describe("labels", () => {
  it("names a step by its forecast hour", () => {
    expect(forecastLabel(0, 3)).toBe("+0 h");
    expect(forecastLabel(5, 3)).toBe("+15 h");
    expect(forecastLabel(2, 24)).toBe("+48 h");
  });

  /**
   * UTC, always (spec.md 3). Checked against a known instant rather than
   * against `Date`'s own formatting, so a locale leak would fail.
   */
  it("names a step by its UTC time once a start is set", () => {
    // 2024-03-10T06:00:00Z
    const start = 1_710_050_400;
    expect(utcLabel(0, 3, start)).toBe("10 Mar 06:00Z");
    expect(utcLabel(3, 3, start)).toBe("10 Mar 15:00Z");
    expect(utcLabel(8, 3, start)).toBe("11 Mar 06:00Z");
    expect(utcLabel(0, 3, null)).toBeNull();
  });

  /** Labels never overlap, and the labelled steps are the expected ones. */
  it("labels every nth tick so labels stay apart", () => {
    expect(labelEvery(60)).toBe(1);
    expect(labelEvery(30)).toBe(2);
    expect(labelEvery(15)).toBe(4);
    expect(labelEvery(6)).toBe(10);
    expect(labelEvery(0.1)).toBe(240);
    for (const px of [0.5, 3, 7, 12, 25, 70]) {
      expect(labelEvery(px) * px).toBeGreaterThanOrEqual(Math.min(56, 240 * px));
    }
  });
});

describe("pointer to step", () => {
  /** A scrub lands on the cell under the pointer, not the one to its left. */
  it("rounds to the nearest cell centre", () => {
    expect(stepAt(0, 10, 9)).toBe(0);
    expect(stepAt(14, 10, 9)).toBe(1);
    expect(stepAt(16, 10, 9)).toBe(1);
    expect(stepAt(25, 10, 9)).toBe(2);
    expect(stepAt(-40, 10, 9)).toBe(0);
    expect(stepAt(400, 10, 9)).toBe(9);
  });

  /** A key moves in whole steps (spec.md 9.3) and stays on the timeline. */
  it("drags a key by whole steps within the timeline", () => {
    expect(draggedStep(4, 0, 10, 9)).toBe(4);
    expect(draggedStep(4, 14, 10, 9)).toBe(5);
    expect(draggedStep(4, -26, 10, 9)).toBe(1);
    expect(draggedStep(4, 900, 10, 9)).toBe(9);
    expect(draggedStep(4, -900, 10, 9)).toBe(0);
  });
});

describe("uniqueTiles", () => {
  /** The map lists a dateline tile per world copy; the cache holds it once. */
  it("collapses the world copies the map draws", () => {
    const tiles = uniqueTiles([
      { z: 1, x: 0, y: 0 },
      { z: 1, x: 0, y: 0 },
      { z: 1, x: 3, y: 0 },
    ]);
    expect(tiles).toEqual([
      { z: 1, x: 0, y: 0 },
      { z: 1, x: 3, y: 0 },
    ]);
  });
});

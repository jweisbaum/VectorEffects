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
  interpolatedSteps,
  labelEvery,
  mayAdvanceInto,
  nextStep,
  stepAt,
  steppedBy,
  type StepState,
  tick,
  uniqueTiles,
  utcLabel,
  WARM_AHEAD,
  warmTargets,
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

describe("warmTargets", () => {
  it("names every step of the lookahead, nearest first", () => {
    expect(warmTargets(0, 9, false)).toEqual([1, 2]);
    expect(warmTargets(4, 9, false)).toEqual([5, 6]);
  });

  /**
   * The whole point of the lookahead. A fetch for the second step ahead that
   * only starts once the first is resident cannot overlap with it, so every
   * step's fetch latency lands in the playback loop rather than being hidden
   * behind the step before it. Both are named on the same frame, so both are
   * in flight at once.
   */
  it("names the far step even though the near one is not resident yet", () => {
    const resident = new Set<number>();
    const targets = warmTargets(4, 9, false);
    // Nothing is resident: the list must still hold both, not stop at the
    // first miss.
    expect(targets.filter((step) => !resident.has(step))).toEqual([5, 6]);
    expect(targets).toHaveLength(WARM_AHEAD);
  });

  it("stops at the end of the timeline when not looping", () => {
    expect(warmTargets(8, 9, false)).toEqual([9]);
    expect(warmTargets(9, 9, false)).toEqual([]);
  });

  it("wraps when looping, and round a macro preview's own run", () => {
    expect(warmTargets(9, 9, true)).toEqual([0, 1]);
    expect(warmTargets(8, 9, true)).toEqual([9, 0]);
  });

  /**
   * A loop shorter than the lookahead comes back round to the step being
   * drawn. That step is resident already — playback only advanced into it
   * because it was — so naming it would spend a pass over the viewport's
   * tiles, every animation frame, warming what is on screen.
   */
  it("stops short rather than coming back round to the playhead", () => {
    expect(warmTargets(0, 0, true)).toEqual([]);
    expect(warmTargets(0, 1, true)).toEqual([1]);
    expect(warmTargets(5, 5, true, 4)).toEqual([4]);
    expect(warmTargets(3, 3, true, 3)).toEqual([]);
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

describe("interpolatedSteps", () => {
  /**
   * Spec 9.3/4.5: the value is interpolated strictly between two keys. Before
   * the first and after the last the nearest key holds, which is not a blend
   * and gets no dots.
   */
  it("marks the steps inside a blended segment and no others", () => {
    const keys = [
      { step: 2, hold: false },
      { step: 6, hold: false },
    ];
    expect(interpolatedSteps(keys)).toEqual([3, 4, 5]);
  });

  /**
   * The interpolation belongs to the key the segment leaves, so a holding key
   * blends nothing until the next one — even though the segment after that
   * next key may blend.
   */
  it("leaves a holding segment empty and blends the one after it", () => {
    const keys = [
      { step: 0, hold: true },
      { step: 4, hold: false },
      { step: 7, hold: false },
    ];
    expect(interpolatedSteps(keys)).toEqual([5, 6]);
  });

  it("has nothing to mark for one key, no keys, or adjacent keys", () => {
    expect(interpolatedSteps([])).toEqual([]);
    expect(interpolatedSteps([{ step: 3, hold: false }])).toEqual([]);
    expect(
      interpolatedSteps([
        { step: 3, hold: false },
        { step: 4, hold: false },
      ]),
    ).toEqual([]);
  });

  /** A drag reorders the keys it moves; the dots follow the drawn order. */
  it("sorts before pairing, so a dragged key still bounds its segment", () => {
    const keys = [
      { step: 8, hold: false },
      { step: 5, hold: false },
    ];
    expect(interpolatedSteps(keys)).toEqual([6, 7]);
  });
});

describe("steppedBy", () => {
  /**
   * Spec 9.4: the arrows walk the ruler one step at a time. They clamp where
   * playback loops — an arrow is for reaching a particular time, and wrapping
   * to the other end of the timeline is a jump nobody asked for.
   */
  it("moves one step and stops at each end", () => {
    expect(steppedBy(4, 1, 9)).toBe(5);
    expect(steppedBy(4, -1, 9)).toBe(3);
    expect(steppedBy(9, 1, 9)).toBe(9);
    expect(steppedBy(0, -1, 9)).toBe(0);
  });

  /** Where `nextStep` would wrap with looping on, this never does. */
  it("differs from playback's advance at the end", () => {
    expect(nextStep(9, 9, true)).toBe(0);
    expect(steppedBy(9, 1, 9)).toBe(9);
  });
});

describe("nextStep from a first step other than zero", () => {
  it("loops a macro preview round its own run (spec.md 8.7, M26)", () => {
    expect(nextStep(7, 7, true, 3)).toBe(3);
    expect(nextStep(5, 7, true, 3)).toBe(6);
    expect(nextStep(7, 7, false, 3)).toBeNull();
  });
});

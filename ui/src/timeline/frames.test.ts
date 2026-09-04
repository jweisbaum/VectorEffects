import { describe, expect, it } from "vitest";

import type { GribStepView } from "../generated/GribStepView";
import { markKind, pastedRun, runBetween } from "./frames";

function frame(over: Partial<GribStepView>): GribStepView {
  return { in_file: false, source: null, hidden: false, shown: false, ...over };
}

describe("markKind", () => {
  it("tells the file's own message from a pasted one", () => {
    expect(markKind(frame({ in_file: true, shown: true }))).toBe("file");
    expect(markKind(frame({ source: 4, shown: true }))).toBe("pasted");
  });

  it("draws a hidden message rather than leaving it blank", () => {
    // The file has one here; the user said not to show it. A blank mark would
    // be indistinguishable from a time the file says nothing about.
    expect(markKind(frame({ in_file: true, hidden: true }))).toBe("hidden");
  });

  it("draws nothing where the file says nothing", () => {
    expect(markKind(frame({}))).toBe("none");
  });

  it("lets a paste outrank the file's own message", () => {
    expect(markKind(frame({ in_file: true, source: 2, shown: true }))).toBe("pasted");
  });
});

describe("runBetween", () => {
  it("takes both ends in, whichever way the shift-click went", () => {
    expect(runBetween(2, 5)).toEqual([2, 3, 4, 5]);
    expect(runBetween(5, 2)).toEqual([2, 3, 4, 5]);
  });

  it("is one step when the anchor is the click", () => {
    expect(runBetween(3, 3)).toEqual([3]);
  });
});

describe("pastedRun", () => {
  it("keeps the spacing of a sparse run", () => {
    // 0, 2, 4 of a 6-hourly file on a 3-hourly timeline, pasted at 6.
    expect(pastedRun([0, 2, 4], 6, 11)).toEqual([6, 8, 10]);
  });

  it("drops what falls off the end rather than clamping it", () => {
    // Clamping would pile 9 and 11 onto step 7, each overwriting the one
    // before, and leave one frame where three were asked for.
    expect(pastedRun([0, 2, 4], 7, 7)).toEqual([7]);
  });

  it("is empty for an empty clipboard", () => {
    expect(pastedRun([], 3, 10)).toEqual([]);
  });
});

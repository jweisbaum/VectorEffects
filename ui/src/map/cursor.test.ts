import { describe, expect, it } from "vitest";

import { BUCKET_CURSOR, FORBIDDEN_CURSOR, type CursorContext, cursorFor, pointerPriorityFor } from "./cursor";
import { CAPTURE, ERASE, HAND, INSERT, MEASURE, SELECT } from "./tools";

// `onImage` and `grip` are deleted from `CursorContext`, and with them the
// "picture's grips (M50)" suite and the hand-over-image "move" cursor test
// that used to live here. Both promised gestures — the corner/edge/rotation
// drag and the M36 whole-picture drag — that the control-point model
// replaces; there is no successor cursor cue to rewrite them against yet, so
// they are deleted rather than adapted. Task 7's alignment interaction gets
// its own cursor cues, if it needs any, with its own tests.

const base: CursorContext = {
  tool: HAND,
  eyedropper: false,
  picking: false,
  insideRegion: false,
  panning: false,
  recording: false,
  forbidden: false,
  aligning: false,
};

describe("cursorFor", () => {
  it("gives the hand a hand, closed while panning", () => {
    expect(cursorFor(base)).toBe("grab");
    expect(cursorFor({ ...base, panning: true })).toBe("grabbing");
  });

  it("gives the select, capture and insert tools the arrow", () => {
    for (const tool of [SELECT, CAPTURE, INSERT]) {
      expect(cursorFor({ ...base, tool })).toBe("default");
    }
  });

  it("gives every drawing and measuring tool a crosshair on the click", () => {
    for (const tool of ["brush", "shape_fill", "curve", "circle", "mask", MEASURE, ERASE] as const) {
      expect(cursorFor({ ...base, tool })).toBe("crosshair");
    }
  });

  it("is a paint bucket inside a selected region, whatever the tool", () => {
    expect(cursorFor({ ...base, tool: "brush", insideRegion: true })).toBe(BUCKET_CURSOR);
    // The bucket is drawn inline, so nothing is fetched (invariant 5).
    expect(BUCKET_CURSOR.startsWith('url("data:image/svg+xml')).toBe(true);
    expect(BUCKET_CURSOR).not.toMatch(/https?:/);
  });

  it("hides the cursor while the eyedropper is armed, so the magnifier shows", () => {
    expect(cursorFor({ ...base, tool: "brush", eyedropper: true })).toBe("none");
    // Even inside a region: the sample is the click.
    expect(cursorFor({ ...base, tool: "brush", eyedropper: true, insideRegion: true })).toBe(
      "none",
    );
  });

  it("puts a pick ahead of everything", () => {
    expect(cursorFor({ ...base, tool: HAND, picking: true })).toBe("crosshair");
    expect(cursorFor({ ...base, tool: "brush", picking: true, eyedropper: true })).toBe(
      "crosshair",
    );
  });

  it("uses the selection cursor while a capture records", () => {
    expect(cursorFor({ ...base, tool: "brush", recording: true })).toBe("default");
  });
});

describe("the image alignment mode (Task 7)", () => {
  it("takes the crosshair whatever tool is in hand", () => {
    for (const tool of [HAND, SELECT, CAPTURE, INSERT, "brush", MEASURE] as const) {
      expect(cursorFor({ ...base, tool, aligning: true })).toBe("crosshair");
    }
  });

  /**
   * A pick armed in the inspector still takes the click ahead of aligning —
   * both give a crosshair, so this is checked through `recording`, which
   * does not: aligning must lose to a running capture, which owns every
   * click on the map before anything else does (spec.md 8.7).
   */
  it("does not outrank a running capture", () => {
    expect(cursorFor({ ...base, aligning: true, recording: true })).toBe("default");
  });

  /** But it does outrank a refused tool (M51): the click means something else now. */
  it("outranks a tool the layer would refuse", () => {
    expect(cursorFor({ ...base, tool: "brush", aligning: true, forbidden: true })).toBe(
      "crosshair",
    );
  });
});

describe("pointerPriorityFor", () => {
  /**
   * This is the function `MapView`'s own pointer-down dispatch calls to
   * decide which of picking, a running capture or the alignment mode gets
   * the click — not a second copy of the ordering, restated. A test that
   * only asserted the cursor string here would pass even if the dispatch
   * checked these three in a different order than `cursorFor` shows, which
   * is exactly the defect a review of this task found: the cursor promised
   * a capture would win and the dispatch let alignment steal the click
   * instead. Pinning this function's return value is what a change to
   * either side has to keep agreeing with.
   */
  it("picking, then a running capture, then alignment, then the tool", () => {
    const none = { picking: false, recording: false, aligning: false };
    expect(pointerPriorityFor(none)).toBe("tool");
    expect(pointerPriorityFor({ ...none, aligning: true })).toBe("aligning");
    // A running capture wins over alignment (spec.md 8.7): nothing else on
    // the map does anything while it records.
    expect(pointerPriorityFor({ ...none, recording: true, aligning: true })).toBe("recording");
    expect(pointerPriorityFor({ ...none, recording: true })).toBe("recording");
    // A pick wins over everything, including a running capture.
    expect(pointerPriorityFor({ picking: true, recording: true, aligning: true })).toBe(
      "picking",
    );
  });
});

describe("a tool the layer will not take (M51)", () => {
  /**
   * The refusal was always there, but it arrived on release — after the
   * stroke had been drawn and previewed. The cursor says it on hover.
   */
  it("says so before the click", () => {
    expect(cursorFor({ ...base, tool: ERASE, forbidden: true })).toBe(FORBIDDEN_CURSOR);
  });

  /** Nor an armed pick, which the user asked for by name. */
  it("does not outrank a pick", () => {
    expect(cursorFor({ ...base, forbidden: true, picking: true })).toBe("crosshair");
  });
});

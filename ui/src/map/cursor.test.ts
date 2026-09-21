import { describe, expect, it } from "vitest";

import { BUCKET_CURSOR, FORBIDDEN_CURSOR, type CursorContext, cursorFor, pointerPriorityFor } from "./cursor";
import { CAPTURE, ERASE, HAND, INSERT, MEASURE, SELECT } from "./tools";

// `grip` is deleted from `CursorContext`, and with it the "picture's grips
// (M50)" suite: the corner, edge and rotation drags it cued are gone, and
// the control-point model replaces them, so those tests are deleted rather
// than adapted. `onImage` is back — the M36 whole-picture drag it cues was
// restored, and "the hand over a picture" below is its successor suite.

const base: CursorContext = {
  tool: HAND,
  eyedropper: false,
  picking: false,
  insideRegion: false,
  panning: false,
  recording: false,
  forbidden: false,
  onImage: false,
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

describe("the hand over a picture", () => {
  /**
   * M36: inside the active layer's picture the hand moves the picture, not
   * the map, and the cursor has to say which — a grab hand here would
   * promise a pan that is not what the drag does.
   */
  it("says move rather than grab", () => {
    expect(cursorFor({ ...base, tool: HAND, onImage: true })).toBe("move");
    expect(cursorFor({ ...base, tool: HAND, onImage: false })).toBe("grab");
  });

  /** Once the drag is under way the grabbing hand wins, as it does for a pan. */
  it("shows the drag while it is happening", () => {
    expect(cursorFor({ ...base, tool: HAND, onImage: true, panning: true })).toBe("grabbing");
  });

  /**
   * The picture is the hand's business and nobody else's: no other tool
   * changes what it does over an image, so none of them may change cursor
   * for being over one.
   */
  it("does not change any other tool's cursor", () => {
    for (const tool of [SELECT, MEASURE, CAPTURE, ERASE, INSERT] as const) {
      expect(
        cursorFor({ ...base, tool, onImage: true }),
        `${tool} must not react to a picture`,
      ).toBe(cursorFor({ ...base, tool, onImage: false }));
    }
  });

  /**
   * And it never outranks the modes above it: a capture recording, or the
   * alignment mode armed, both take the click whatever is under the pointer
   * (spec.md 8.7, Task 7).
   */
  it("does not outrank a capture or the alignment mode", () => {
    expect(cursorFor({ ...base, tool: HAND, onImage: true, recording: true })).toBe("default");
    expect(cursorFor({ ...base, tool: HAND, onImage: true, aligning: true })).toBe("crosshair");
  });
});

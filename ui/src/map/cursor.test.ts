import { describe, expect, it } from "vitest";

import { BUCKET_CURSOR, type CursorContext, cursorFor } from "./cursor";
import { CAPTURE, HAND, INSERT, MEASURE, SELECT } from "./tools";

const base: CursorContext = {
  tool: HAND,
  eyedropper: false,
  picking: false,
  insideRegion: false,
  panning: false,
  recording: false,
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
    for (const tool of ["brush", "shape_fill", "curve", "circle", "mask", MEASURE] as const) {
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

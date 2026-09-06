import { describe, expect, it } from "vitest";

import { type HoverContext, showsHoverIndicator } from "./hover";

const brush: HoverContext = {
  hasIndicator: true,
  nib: true,
  picking: false,
  insideRegion: false,
};

describe("the hover indicator", () => {
  it("is drawn for a tool that has one, on empty map", () => {
    expect(showsHoverIndicator(brush)).toBe(true);
  });

  it("is never drawn for a tool without one, whatever else is true", () => {
    expect(showsHoverIndicator({ ...brush, hasIndicator: false })).toBe(false);
    expect(showsHoverIndicator({ ...brush, nib: false })).toBe(false);
  });

  it("stands down while a pick waits for the click", () => {
    expect(showsHoverIndicator({ ...brush, picking: true })).toBe(false);
  });

  it("stands down inside a selected region the tool would fill: the bucket is the indication", () => {
    expect(showsHoverIndicator({ ...brush, insideRegion: true })).toBe(false);
  });
});

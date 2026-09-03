import { describe, expect, it } from "vitest";

import { containsLon, marqueeBounds } from "./marquee";

describe("marqueeBounds", () => {
  it("keeps latitudes as a min and a max", () => {
    const bounds = marqueeBounds({ lon: 0, lat: 40 }, { lon: 10, lat: 10 }, true);
    expect(bounds.south).toBe(10);
    expect(bounds.north).toBe(40);
  });

  it("takes west and east from the direction of the drag", () => {
    const rightwards = marqueeBounds({ lon: 0, lat: 0 }, { lon: 30, lat: 10 }, true);
    expect([rightwards.west, rightwards.east]).toEqual([0, 30]);

    const leftwards = marqueeBounds({ lon: 30, lat: 0 }, { lon: 0, lat: 10 }, false);
    expect([leftwards.west, leftwards.east]).toEqual([0, 30]);
  });

  // The reason this file exists: comparing the two longitudes as numbers would
  // turn a 20°-wide band across the dateline into the 340° everywhere else.
  it("describes a band dragged across the dateline as the part that wraps", () => {
    const bounds = marqueeBounds({ lon: 170, lat: 0 }, { lon: -170, lat: 10 }, true);
    expect(bounds.west).toBe(170);
    expect(bounds.east).toBe(-170);

    expect(containsLon(bounds, 179)).toBe(true);
    expect(containsLon(bounds, -175)).toBe(true);
    expect(containsLon(bounds, 0)).toBe(false);
    expect(containsLon(bounds, 100)).toBe(false);
  });

  it("describes an ordinary band as the part between its edges", () => {
    const bounds = marqueeBounds({ lon: -20, lat: 0 }, { lon: 20, lat: 10 }, true);
    expect(containsLon(bounds, 0)).toBe(true);
    expect(containsLon(bounds, 179)).toBe(false);
  });
});

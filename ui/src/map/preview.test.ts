import { describe, expect, it } from "vitest";

import { previewHasLanded, SETTLE_TIMEOUT_MS, type HeldPreview } from "./preview";

/** A preview committed at t=1000, whose commit returned revision 7. */
const held: HeldPreview = {
  footprint: {
    kind: "swept",
    points: [[0, 0]],
    radiusKm: 100,
    shape: "circle",
    space: "geodesic",
  },
  paint: "rgba(0, 0, 0, 1)",
  knots: 20,
  azimuthAt: () => 90,
  revision: 7,
  at: 1000,
};

describe("previewHasLanded", () => {
  /** Nothing is known until the commit comes back with a revision. */
  it("holds a preview whose commit has not returned", () => {
    expect(previewHasLanded({ ...held, revision: null }, 9, 0, 1100)).toBe(false);
  });

  /** The document is still one revision behind the stroke. */
  it("holds a preview whose revision has not been applied", () => {
    expect(previewHasLanded(held, 6, 0, 1100)).toBe(false);
  });

  /**
   * The revision is applied but the tiles for it are still in flight, so the
   * map is drawing the revision before the stroke -- the very gap the preview
   * covers.
   */
  it("holds a preview while its tiles are outstanding", () => {
    expect(previewHasLanded(held, 7, 3, 1100)).toBe(false);
  });

  it("drops a preview once its revision is drawn", () => {
    expect(previewHasLanded(held, 7, 0, 1100)).toBe(true);
  });

  /** Later edits do not strand an earlier stroke's preview. */
  it("drops a preview overtaken by a later revision", () => {
    expect(previewHasLanded(held, 9, 0, 1100)).toBe(true);
  });

  /** A tile that never arrives must not leave paint on the overlay forever. */
  it("drops a preview whose field never arrives", () => {
    expect(previewHasLanded(held, 6, 4, 1000 + SETTLE_TIMEOUT_MS)).toBe(true);
    expect(previewHasLanded({ ...held, revision: null }, 6, 4, 1000 + SETTLE_TIMEOUT_MS))
      .toBe(true);
  });
});

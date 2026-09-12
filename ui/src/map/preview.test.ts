import { describe, expect, it } from "vitest";

import { overlayPlan, previewHasLanded, SETTLE_TIMEOUT_MS, type HeldPreview } from "./preview";

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
    expect(previewHasLanded({ ...held, revision: null }, 9, true, 1100)).toBe(false);
  });

  /** The document is still one revision behind the stroke. */
  it("holds a preview whose revision has not been applied", () => {
    expect(previewHasLanded(held, 6, true, 1100)).toBe(false);
  });

  /**
   * The revision is applied but the map is still showing the frame before it,
   * so dropping the preview now shows the gap it covers.
   */
  it("holds a preview until the edited frame is the one on screen", () => {
    expect(previewHasLanded(held, 7, false, 1100)).toBe(false);
  });

  it("drops a preview once its revision is drawn", () => {
    expect(previewHasLanded(held, 7, true, 1100)).toBe(true);
  });

  /** Later edits do not strand an earlier stroke's preview. */
  it("drops a preview overtaken by a later revision", () => {
    expect(previewHasLanded(held, 9, true, 1100)).toBe(true);
  });

  /** A tile that never arrives must not leave paint on the overlay forever. */
  it("drops a preview whose field never arrives", () => {
    expect(previewHasLanded(held, 6, false, 1000 + SETTLE_TIMEOUT_MS)).toBe(true);
    expect(previewHasLanded({ ...held, revision: null }, 6, false, 1000 + SETTLE_TIMEOUT_MS))
      .toBe(true);
  });

  /**
   * The regression (M80). The document holding the stroke is not the map
   * showing it: an edit re-addresses every tile, so for the length of the key
   * round trip that follows, nothing has been asked for and nothing is
   * pending — which the old condition read as "everything has arrived". The
   * preview was dropped there and the field it had removed came back for the
   * length of the round trip, then left again.
   */
  it("does not mistake a frame nobody has asked for yet for one that arrived", () => {
    expect(previewHasLanded(held, 7, false, 1100)).toBe(false);
  });
});

describe("overlayPlan", () => {
  /**
   * Spec 6.1: what the user is aiming is a wind, so a tool that paints one
   * shows it — and the hover footprint gives way to the gesture.
   */
  it("draws the field for a tool that paints one, and its nib when idle", () => {
    expect(overlayPlan("field", true)).toEqual({ sweep: "field", nib: false });
    expect(overlayPlan("field", false)).toEqual({ sweep: "none", nib: true });
  });

  /**
   * Spec 6.2/D37: an operator is previewed by the map, through a mask. The
   * overlay never draws over its sweep — a stroked footprint is a chain of
   * circles, not the silhouette of one — and the nib stays up through the drag
   * because it is the only thing either operator draws.
   */
  it("gives an operator its nib and nothing else, drag or no drag", () => {
    for (const kind of ["mask", "clone", "gain", "turn", "radial", "warp", "smear"] as const) {
      expect(overlayPlan(kind, true)).toEqual({ sweep: "none", nib: true });
      expect(overlayPlan(kind, false)).toEqual({ sweep: "none", nib: true });
    }
  });
});

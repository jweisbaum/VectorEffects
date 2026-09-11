/**
 * Which layer is made active when none is.
 */
import { describe, expect, it } from "vitest";

import { layerToActivate } from "./activeLayer";

/** Bottom-first, as the document is. */
const stack = [
  { id: 1, visible: true },
  { id: 2, visible: true },
  { id: 3, visible: false },
];

describe("keeping a layer selected", () => {
  /**
   * The report: a tool picked up with nothing selected had nowhere of its own
   * to draw. The topmost *visible* one, since a gesture aimed at a hidden
   * layer is refused (M68) and landing there by default would answer the
   * user's first stroke with a refusal about a layer they never picked.
   */
  it("picks the topmost visible layer when none is active", () => {
    expect(layerToActivate(null, stack)).toBe(2);
  });

  /** A choice already made is a choice; only absence is corrected. */
  it("leaves an active layer alone", () => {
    expect(layerToActivate(1, stack)).toBeNull();
    expect(layerToActivate(2, stack)).toBeNull();
  });

  /** Including a hidden one the user picked deliberately. */
  it("leaves a deliberately chosen hidden layer alone", () => {
    expect(layerToActivate(3, stack)).toBeNull();
  });

  /** A layer can be deleted from under the selection; a dangling id is absence. */
  it("replaces an id that no longer names a layer", () => {
    expect(layerToActivate(99, stack)).toBe(2);
  });

  /**
   * With nothing visible there is nothing worth choosing: every gesture is
   * refused whatever this returns, and a layer nobody picked would make the
   * refusal harder to understand rather than easier.
   */
  it("chooses nothing when no layer is visible", () => {
    expect(layerToActivate(null, [{ id: 1, visible: false }])).toBeNull();
    expect(layerToActivate(null, [])).toBeNull();
  });
});

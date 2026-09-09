/**
 * Where a gesture aimed at the map would land, and whether that is on it.
 */
import { describe, expect, it } from "vitest";

import { aimedOffTheMap, targetLayer } from "./allowed";

/** Bottom-first, as the document is. */
const stack = [
  { id: 1, visible: true },
  { id: 2, visible: false },
  { id: 3, visible: true },
];

describe("the layer a gesture lands on", () => {
  it("is the chosen one when there is one", () => {
    expect(targetLayer(2, stack)).toBe(2);
  });

  /** The same rule `creation_layer` follows, so the two cannot disagree. */
  it("is the top of the stack when nothing is chosen", () => {
    expect(targetLayer(null, stack)).toBe(3);
    expect(targetLayer(null, [])).toBeNull();
  });
});

describe("a gesture aimed off the map", () => {
  it("is one whose layer is hidden", () => {
    expect(aimedOffTheMap(2, stack)).toBe(true);
    expect(aimedOffTheMap(1, stack)).toBe(false);
    expect(aimedOffTheMap(3, stack)).toBe(false);
  });

  /** The reported case: nothing chosen, and the top of the stack is hidden. */
  it("is one aimed at a hidden top of the stack with nothing chosen", () => {
    const hiddenTop = [
      { id: 1, visible: true },
      { id: 9, visible: false },
    ];
    expect(aimedOffTheMap(null, hiddenTop)).toBe(true);
    expect(aimedOffTheMap(null, stack)).toBe(false);
  });

  /**
   * A layer the tree has not described yet counts as visible: the tree lands a
   * moment after the project, and refusing every gesture in that window would
   * be a worse lie than allowing one — the backend refuses it on release.
   */
  it("is not one whose layer is simply unknown", () => {
    expect(aimedOffTheMap(77, stack)).toBe(false);
    expect(aimedOffTheMap(null, [])).toBe(false);
  });
});

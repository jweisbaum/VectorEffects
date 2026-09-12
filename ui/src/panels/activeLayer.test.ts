/**
 * Which layer is made active when none is.
 */
import { describe, expect, it } from "vitest";

import { layerForSelection, layerToActivate } from "./activeLayer";

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

/** Bottom-first, with what each layer holds. */
const contents = [
  { id: 1, objects: [{ id: 10 }, { id: 11 }] },
  { id: 2, objects: [{ id: 20 }] },
  { id: 3, objects: [{ id: 30 }, { id: 31 }] },
];

describe("the layer a selection implies", () => {
  /**
   * The report: selecting an object should select its layer. Clicking one in
   * the panel already did both in one gesture; clicking one on the *map* moved
   * the selection and left the active layer where it was, so the handles
   * described an object in one layer while the next stroke would land in
   * another.
   */
  it("follows an object selected from a different layer", () => {
    expect(layerForSelection(1, [30], contents)).toBe(3);
    expect(layerForSelection(3, [20], contents)).toBe(2);
  });

  /** Already right: nothing to do. */
  it("leaves the active layer alone when it holds the selection", () => {
    expect(layerForSelection(3, [30, 31], contents)).toBeNull();
  });

  /**
   * A selection can span layers, and holding *any* of it is enough to be the
   * right layer — pulling the active one to the first would move it out from
   * under a selection it already describes.
   */
  it("stays put when it holds part of a selection that spans layers", () => {
    expect(layerForSelection(3, [10, 30], contents)).toBeNull();
    expect(layerForSelection(1, [10, 30], contents)).toBeNull();
  });

  /** Holding none of it is what makes it the wrong layer. */
  it("moves to the first holder when it holds none of the selection", () => {
    expect(layerForSelection(2, [11, 31], contents)).toBe(1);
  });

  /** Nothing selected implies nothing. */
  it("implies nothing from an empty selection", () => {
    expect(layerForSelection(null, [], contents)).toBeNull();
    expect(layerForSelection(2, [], contents)).toBeNull();
  });

  /** A selection of ids no layer holds says nothing either. */
  it("implies nothing from a selection nothing holds", () => {
    expect(layerForSelection(2, [99], contents)).toBeNull();
  });

  /** With no layer active at all, the selection still names one. */
  it("names a layer when none is active", () => {
    expect(layerForSelection(null, [20], contents)).toBe(2);
  });
});

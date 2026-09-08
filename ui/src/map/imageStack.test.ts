/**
 * Which image layers draw over the field (M49).
 *
 * An order is the kind of rule that is wrong by one and looks nearly right,
 * so the cases are the boundaries: an image directly above the top field
 * layer, one directly below it, and what a hidden layer does to both.
 */
import { describe, expect, it } from "vitest";

import { imagesOverField, type StackedLayer } from "./imageStack";

/** Bottom-first, as the document stores them. */
const stack = (...spec: string[]): StackedLayer[] =>
  spec.map((word, id) => ({
    id,
    visible: !word.startsWith("hidden"),
    isImage: word.endsWith("image"),
  }));

describe("an image over the field", () => {
  /** The case the report was about: a chart moved to the top of the stack. */
  it("is one above every field layer", () => {
    expect([...imagesOverField(stack("field", "image"))]).toEqual([1]);
  });

  /** And one below the field stays under it, which is what it is there for. */
  it("is not one below a field layer", () => {
    expect([...imagesOverField(stack("image", "field"))]).toEqual([]);
  });

  /** The boundary: only the images above the *last* field layer go over. */
  it("counts from the topmost field layer, not the first", () => {
    expect([...imagesOverField(stack("image", "field", "image", "field", "image"))]).toEqual([4]);
  });

  /**
   * A hidden field layer is not on the map, so it holds nothing down: an
   * image above the remaining field goes over it.
   */
  it("ignores a hidden field layer", () => {
    expect([...imagesOverField(stack("field", "image", "hidden field"))]).toEqual([1]);
  });

  /** And a hidden image is not drawn at all, wherever it sits. */
  it("never names a hidden image", () => {
    expect([...imagesOverField(stack("field", "hidden image"))]).toEqual([]);
  });

  /** With no field at all, every image is over one vacuously. */
  it("puts every image over an empty stack of fields", () => {
    expect([...imagesOverField(stack("image", "image"))]).toEqual([0, 1]);
  });

  it("says nothing about an empty stack", () => {
    expect([...imagesOverField([])]).toEqual([]);
  });
});

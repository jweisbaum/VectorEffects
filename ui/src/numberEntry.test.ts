/**
 * The rule that makes a number field clearable.
 *
 * The bug these assert against: clearing a field committed a fallback, which
 * refilled the box before the next keystroke arrived, so a value could be
 * edited in the middle but never replaced.
 */
import { describe, expect, it } from "vitest";

import { clamped, typedValue } from "./numberEntry";

describe("typedValue", () => {
  it("reads a number the way the field's contents mean it", () => {
    expect(typedValue("40")).toBe(40);
    expect(typedValue("-12.5")).toBe(-12.5);
    expect(typedValue(" 7 ")).toBe(7);
  });

  /** An empty field is not a zero, and not the value it used to hold. */
  it("says nothing for an empty field", () => {
    expect(typedValue("")).toBeNull();
    expect(typedValue("   ")).toBeNull();
  });

  /** A negative and a decimal both look like this on the way in. */
  it("says nothing for an entry still being typed", () => {
    expect(typedValue("-")).toBeNull();
    expect(typedValue(".")).toBeNull();
    expect(typedValue("1e")).toBeNull();
  });

  it("says nothing for something that is not a number at all", () => {
    expect(typedValue("abc")).toBeNull();
    expect(typedValue("Infinity")).toBeNull();
    expect(typedValue("NaN")).toBeNull();
  });
});

describe("clamped", () => {
  it("brings a value inside its bounds", () => {
    expect(clamped(0, 1, 100)).toBe(1);
    expect(clamped(500, 1, 100)).toBe(100);
    expect(clamped(40, 1, 100)).toBe(40);
  });

  it("leaves an unbounded side alone", () => {
    expect(clamped(-90, null, 100)).toBe(-90);
    expect(clamped(1e6, 1, null)).toBe(1e6);
  });
});

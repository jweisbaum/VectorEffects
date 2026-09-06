import { describe, expect, it } from "vitest";

import { stillPasteChord } from "./chords";

const keys = (over: Partial<Parameters<typeof stillPasteChord>[0]>) => ({
  shiftKey: false,
  ctrlKey: false,
  metaKey: false,
  altKey: false,
  ...over,
});

describe("the still-paste chord", () => {
  it("is Ctrl-Shift-V on a Mac, and Cmd-Shift-V is not it", () => {
    expect(stillPasteChord(keys({ ctrlKey: true, shiftKey: true }), true)).toBe(true);
    expect(stillPasteChord(keys({ metaKey: true, shiftKey: true }), true)).toBe(false);
    expect(stillPasteChord(keys({ ctrlKey: true, metaKey: true, shiftKey: true }), true)).toBe(
      false,
    );
  });

  it("is Ctrl-Alt-Shift-V where Ctrl is the command key", () => {
    expect(stillPasteChord(keys({ ctrlKey: true, altKey: true, shiftKey: true }), false)).toBe(
      true,
    );
    expect(stillPasteChord(keys({ ctrlKey: true, shiftKey: true }), false)).toBe(false);
  });

  it("needs Shift on either", () => {
    expect(stillPasteChord(keys({ ctrlKey: true }), true)).toBe(false);
    expect(stillPasteChord(keys({ ctrlKey: true, altKey: true }), false)).toBe(false);
  });
});

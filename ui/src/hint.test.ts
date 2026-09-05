import { describe, expect, it } from "vitest";

import { currentHint, reportError, setHint, shown } from "./hint";

describe("the hint store", () => {
  it("shows the hint, and an error over it until the hint changes", () => {
    setHint("Click the map to place it.");
    expect(shown(currentHint())).toEqual({ text: "Click the map to place it.", kind: "hint" });

    reportError("the layer is locked");
    expect(shown(currentHint())).toEqual({ text: "the layer is locked", kind: "error" });

    // The same hint again says nothing new: the error stands.
    setHint("Click the map to place it.");
    expect(shown(currentHint())?.kind).toBe("error");

    // A different hint is newer than the error.
    setHint("Draw a region first — it is what gets recorded.");
    expect(shown(currentHint())).toEqual({
      text: "Draw a region first — it is what gets recorded.",
      kind: "hint",
    });

    setHint(null);
    expect(shown(currentHint())).toBeNull();
  });
});

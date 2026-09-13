import { describe, expect, it } from "vitest";

import { currentHint, retryError, reportError, setActivity, setHint, shown } from "./hint";

describe("the hint store", () => {
  it("shows the hint, and an error over it until the hint changes", () => {
    setHint("Click the map to place it.");
    expect(shown(currentHint())).toEqual({
      text: "Click the map to place it.",
      kind: "hint",
      detail: null,
    });

    reportError("the layer is locked", "bad-option");
    expect(shown(currentHint())).toEqual({
      text: "the layer is locked",
      kind: "error",
      detail: "bad-option",
    });

    // The same hint again says nothing new: the error stands.
    setHint("Click the map to place it.");
    expect(shown(currentHint())?.kind).toBe("error");

    // A different hint is newer than the error.
    setHint("Draw a region first — it is what gets recorded.");
    expect(shown(currentHint())).toEqual({
      text: "Draw a region first — it is what gets recorded.",
      kind: "hint",
      detail: null,
    });

    setHint(null);
    expect(shown(currentHint())).toBeNull();
  });

  /**
   * The `kind` is a discriminant for code, not a word for a person (M59). It
   * used to lead the line as `[bad-option] …`, which reads as machine trouble
   * whatever the sentence after it says; it is the tooltip now.
   */
  it("keeps the error's kind out of the line and in the detail", () => {
    setHint(null);
    reportError("Could not save the project to /tmp/x.veproj: permission denied.", "doing");
    const line = shown(currentHint());
    expect(line?.text).not.toContain("doing");
    expect(line?.detail).toBe("doing");

    // Cleared with the error, not left behind for the next one.
    reportError(null);
    setHint("Click the map to place it.");
    expect(shown(currentHint())?.detail).toBeNull();
    setHint(null);
  });

  it("carries the map's activity beside the hint, and neither disturbs the other", () => {
    setHint("Click the map to place it.");
    setActivity("rendering 12…");
    expect(currentHint().activity).toBe("rendering 12…");
    expect(shown(currentHint())).toEqual({
      text: "Click the map to place it.",
      kind: "hint",
      detail: null,
    });

    reportError("the layer is locked");
    expect(currentHint().activity).toBe("rendering 12…");

    setActivity(null);
    expect(currentHint().activity).toBeNull();
    expect(shown(currentHint())?.kind).toBe("error");
    setHint(null);
  });
});

it("keeps a download retry through pointer hints and consumes it once", () => {
  let downloads = 0;
  reportError("Download interrupted", "network", () => { downloads++; });
  setHint("Move the brush");
  expect(shown(currentHint())?.text).toBe("Download interrupted");
  retryError(); retryError();
  expect(downloads).toBe(1);
  expect(currentHint().error).toBeNull();
});

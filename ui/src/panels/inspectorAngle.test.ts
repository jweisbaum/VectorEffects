import { describe, expect, it } from "vitest";

import { toShownAngle } from "./inspectorAngle";

describe("toShownAngle", () => {
  /**
   * The bug this exists to prevent: a stroke painted at 270 in a "from"
   * project stores azimuth-toward 90, and the inspector reported the stored
   * value — so the panel disagreed with the toolbar that had just set it.
   */
  it("shows a flow direction in the project's convention", () => {
    expect(toShownAngle("direction", "from", 90)).toBe(270);
    expect(toShownAngle("direction", "toward", 90)).toBe(90);
  });

  /** Editing is the same conversion, so what is typed comes back unchanged. */
  it("round-trips what the user types", () => {
    for (const convention of ["from", "toward"]) {
      for (const entered of [0, 45, 179, 270, 359]) {
        const stored = toShownAngle("direction", convention, entered);
        expect(toShownAngle("direction", convention, stored)).toBe(entered);
      }
    }
  });

  /**
   * A rotation is geometry, not flow. Converting it would turn every object by
   * half a turn, which is why the schema marks the two units apart.
   */
  it("leaves a geometric angle alone", () => {
    expect(toShownAngle("degrees", "from", 90)).toBe(90);
    expect(toShownAngle("none", "from", 90)).toBe(90);
  });
});

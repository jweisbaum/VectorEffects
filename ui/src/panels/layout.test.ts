import { describe, expect, it } from "vitest";

import { OPEN, normalise, togglePanel } from "./layout";

describe("the panel layout", () => {
  it("flips one panel and leaves the rest", () => {
    const next = togglePanel(OPEN, "bottom");
    expect(next.bottom).toBe(false);
    expect(next.left && next.right && next.properties && next.history).toBe(true);
    expect(togglePanel(next, "bottom")).toEqual(OPEN);
  });

  it("opens anything a stored layout got wrong", () => {
    expect(normalise(null)).toEqual(OPEN);
    expect(normalise({ left: false, right: "no" as unknown as boolean })).toEqual({
      ...OPEN,
      left: false,
    });
  });
});

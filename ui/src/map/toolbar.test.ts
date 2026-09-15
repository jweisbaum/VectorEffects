/** Centred, content-sized toolbar with a bound that keeps every tool reachable. */
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

const css = readFileSync(fileURLToPath(new URL("../styles.css", import.meta.url)), "utf8");

/** The declarations inside one rule, by selector. */
function block(selector: string): string {
  const at = css.indexOf(`${selector} {`);
  expect(at, `${selector} is not in the stylesheet`).toBeGreaterThanOrEqual(0);
  return css.slice(at, css.indexOf("}", at));
}

describe("the toolbar", () => {
  it("centres its contents inside the visible map and caps its width", () => {
    const rule = block(".map-toolbar");
    expect(rule).toContain("left: calc(50% + (var(--dock-left, 0px) - var(--dock-right, 0px)) / 2)");
    expect(rule).toContain("transform: translateX(-50%)");
    expect(rule).toContain("width: max-content");
    expect(rule).toContain("max-width: calc(100% - var(--dock-left, 0px) - var(--dock-right, 0px) - 24px)");
  });

  it("wraps the tool buttons on narrow windows", () => {
    expect(block(".map-toolbar .history")).toMatch(/flex-wrap:\s*wrap/);
  });

  /**
   * And the options wrap within it. The bar wrapping is not enough on its own:
   * a tool's options are one flex item of the toolbar, so without this the
   * whole group is placed as a single unbreakable block and the tool with the
   * most options — the shape fill, on a gradient — pushes past the right edge.
   */
  it("wraps a tool's own options within the bar", () => {
    const rule = block(".tool-options");
    expect(rule).toMatch(/display:\s*flex/);
    expect(rule).toMatch(/flex-wrap:\s*wrap/);
  });

  /**
   * Nothing else may occupy the map's top edge (spec.md 5.5). The legend and
   * the readout are the other two things drawn over the map, and both are
   * anchored to the bottom.
   */
  it("has the map's top edge to itself", () => {
    for (const selector of [".map-legend", ".map-readout"]) {
      const rule = block(selector);
      expect(rule, `${selector} must not be anchored to the top`).not.toMatch(/\btop:/);
      expect(rule).toMatch(/bottom:/);
    }
  });
});

/**
 * The option bar's layout rule (spec.md 5.5, 6.1).
 *
 * "A tool whose option bar does not fit the map at 1280 px wide, wrapped, is
 * not done" is M6's acceptance criterion, and what makes it hold for every tool
 * at once is that the bar **spans the map and wraps within it** rather than
 * sizing to its contents. A bar that sizes to its contents puts the last
 * options off screen where they cannot be reached, and it does so silently —
 * the layout is correct by every other measure and the controls are simply
 * gone.
 *
 * Asserted against the stylesheet because that is where the rule lives. jsdom
 * has no layout engine, so a rendered test could not measure the bar; what it
 * can do is fail when the two declarations that make wrapping work are removed,
 * which is the change that would break the criterion.
 */
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
  /**
   * Anchored to both edges rather than given a width: the bar is as wide as the
   * map at any window size, which is what leaves room for the options to wrap
   * into instead of overflowing. The edges are the *visible* map's: the map
   * runs under the docked panels (spec.md 5.5, M27), so each side is inset by
   * that side's dock variable.
   */
  it("spans the visible map", () => {
    const rule = block(".map-toolbar");
    expect(rule).toMatch(/left:\s*calc\(var\(--dock-left, 0px\) \+ 12px\)/);
    expect(rule).toMatch(/right:\s*calc\(var\(--dock-right, 0px\) \+ 12px\)/);
  });

  it("wraps rather than overflowing", () => {
    expect(block(".map-toolbar")).toMatch(/flex-wrap:\s*wrap/);
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

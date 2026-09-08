/**
 * How the docked panels share the sidebar (M43).
 *
 * The sidebar is a flex column and each panel is one item in it. A flex item
 * shrinks below its content by default, and nothing here clips — so when the
 * history grew past the room left for it, the *properties* above were squeezed
 * and their rows were drawn straight over the history's: two panels' text on
 * the same lines, both unreadable.
 *
 * Asserted against the stylesheet, as the toolbar's rule is: happy-dom has no
 * layout engine and could not measure the overlap, but it can fail when the
 * declarations that prevent it are removed, which is the change that would
 * bring it back.
 */
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

const css = readFileSync(fileURLToPath(new URL("../styles.css", import.meta.url)), "utf8");

/**
 * The declarations inside one rule, by selector.
 *
 * Anchored to the start of a line, because several of these selectors are
 * also the tail of a more specific one — `.view-controls .history` and
 * `.workspace > .sidebar` among them — and matching those would read the
 * wrong rule and pass or fail for the wrong reason.
 */
function block(selector: string): string {
  const at = css.indexOf(`\n${selector} {`);
  expect(at, `${selector} is not a rule of its own in the stylesheet`).toBeGreaterThanOrEqual(0);
  return css.slice(at, css.indexOf("}", at));
}

describe("a docked panel section", () => {
  /**
   * Sized to its content and never squeezed. This is the declaration that
   * stops one panel being drawn over another; without it the column shrinks
   * whichever item it can and the overflow is painted, not clipped.
   */
  it("does not shrink below what it holds", () => {
    expect(block(".panel-section")).toMatch(/flex:\s*0 0 auto/);
  });

  /** Which only works because the sidebar itself takes up the slack. */
  it("sits in a sidebar that scrolls instead", () => {
    const rule = block(".sidebar");
    expect(rule).toMatch(/overflow-y:\s*auto/);
    expect(rule).toMatch(/flex-direction:\s*column/);
  });
});

describe("the history list", () => {
  /**
   * The history grows for as long as the user works, so it is the panel that
   * would otherwise push every other one off the top of the sidebar. It is
   * bounded and scrolls in its own box.
   */
  it("bounds its own height and scrolls", () => {
    const rule = block(".history");
    expect(rule).toMatch(/max-height:/);
    expect(rule).toMatch(/overflow-y:\s*auto/);
  });

  /** A bound in the viewport's terms, so a short window is not all history. */
  it("is bounded relative to the window", () => {
    expect(block(".history")).toMatch(/max-height:\s*min\([^)]*vh/);
  });
});

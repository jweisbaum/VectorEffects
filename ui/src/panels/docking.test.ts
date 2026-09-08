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

describe("the map's chrome", () => {
  /**
   * The readout, the legend and the preview badge are all positioned from the
   * bottom of the map. The badge was on the same line as the other two and was
   * drawn under the readout on any window narrow enough for a centred box and
   * a left-hand one to meet — which is most of them once a dock is open
   * (spec.md 5.5, "chrome does not overlap chrome", M52).
   *
   * The rows are shared values rather than repeated numbers, so what is
   * checked is that each rule uses them: two literals that happened to agree
   * today would drift the first time one was tuned.
   */
  it("puts the readout and the legend on the same row", () => {
    for (const selector of [".map-readout", ".map-legend"]) {
      expect(block(selector), selector).toMatch(
        /bottom:\s*calc\(var\(--dock-bottom, 0px\) \+ var\(--map-chrome-gap\)\)/,
      );
    }
  });

  it("puts the macro preview badge on the row above them", () => {
    expect(block(".map-preview-badge")).toMatch(/var\(--map-chrome-row\)/);
  });

  it("defines the rows once, where the map is laid out", () => {
    const rule = block(".stage");
    expect(rule).toMatch(/--map-chrome-gap:/);
    expect(rule).toMatch(/--map-chrome-row:/);
  });
});

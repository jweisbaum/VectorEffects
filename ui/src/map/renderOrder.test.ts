/**
 * What the map draws first, and what it draws last.
 *
 * The order of the passes in `MapRenderer.render` is a correctness property,
 * not a detail: a backdrop drawn after the field *hides* the field, and a
 * chart's deep water is opaque. That went wrong once — charts were drawn
 * after the raster and covered whatever had been painted wherever a cell
 * reached — so the order is pinned here.
 *
 * Read from the source, because the alternative is a GL context: the passes
 * are method calls in one function, and their order in the text is their
 * order on screen.
 */
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

const source = readFileSync(fileURLToPath(new URL("./renderer.ts", import.meta.url)), "utf8");

/** The body of `render`, which is where the passes are. */
const render = (() => {
  const at = source.indexOf("  render(state: RenderState): SeenRanges {");
  expect(at, "render is not in the renderer").toBeGreaterThanOrEqual(0);
  // To the next method at the same indentation, which ends the function.
  const end = source.indexOf("\n  }\n", source.indexOf("return this.seen", at));
  return source.slice(at, end > at ? end : undefined);
})();

/** Where a pass first appears in `render`. */
function at(what: string): number {
  const found = render.indexOf(what);
  expect(found, `${what} is not one of render's passes`).toBeGreaterThanOrEqual(0);
  return found;
}

describe("the order the map draws in", () => {
  /**
   * A backdrop is something to paint *against* (spec.md 4.11). Nothing the
   * user made may end up behind one.
   */
  it("puts the charts and GIS layers under everything of the user's", () => {
    const backdrops = at("this.drawBackdrops(state, overlaying)");
    expect(backdrops).toBeLessThan(at("this.drawImages(state, offsets, images.filter((image) => !image.over))"));
    expect(backdrops).toBeLessThan(at("this.drawRaster(state, state.camera, tiles, stage"));
    expect(backdrops).toBeLessThan(at("this.drawGlyphs("));
  });

  /**
   * And over the basemap, which is what they are published to sit on: the
   * land under a chart is the world's, and the chart's own coastline and
   * depths go on top of it.
   */
  it("puts them over the basemap's land", () => {
    expect(at('this.drawGlobeBase(state, tiles, "land"')).toBeLessThan(
      at("this.drawBackdrops(state, overlaying)"),
    );
  });

  /**
   * The map tiles are a map of the whole world and stand *in place of* the
   * basemap, so they are drawn before it and it is not drawn at all.
   */
  it("draws the map tiles before the basemap they replace", () => {
    expect(at("this.drawBackdrops(state, replacing)")).toBeLessThan(
      at('this.drawGlobeBase(state, tiles, "land"'),
    );
    expect(render).toContain("const overBase = replacing.length > 0");
  });
});

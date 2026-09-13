import { describe, expect, it } from "vitest";
import { DEFAULT_GLYPH_APPEARANCE, glyphDisplayLayout, glyphOpacity, glyphRgb } from "./glyphAppearance";
import { glyphCoverage, glyphCovered, spacedGlyphs } from "./glyphPlacement";
import { barbGeometry } from "./glyph";

describe("glyph appearance", () => {
  it("scales calm wind markers too", () => {
    const normal = barbGeometry(0, 0, 32, 40, 2).lines[0]!;
    const large = barbGeometry(0, 0, 64, 40, 2, 2).lines[0]!;
    expect(large).toEqual(normal.map(([x, y]) => [x * 2, y * 2]));
  });
  it("changes density without changing size, and size without changing density", () => {
    for (const style of ["arrow", "barb"] as const) {
      const base = glyphDisplayLayout(style, 50, 1, DEFAULT_GLYPH_APPEARANCE);
      const dense = glyphDisplayLayout(style, 50, 1, { ...DEFAULT_GLYPH_APPEARANCE, density_percent: 300 });
      const sparse = glyphDisplayLayout(style, 50, 1, { ...DEFAULT_GLYPH_APPEARANCE, density_percent: 25 });
      const large = glyphDisplayLayout(style, 50, 1, { ...DEFAULT_GLYPH_APPEARANCE, size_percent: 200 });
      expect(dense.stepDeg).toBeLessThan(base.stepDeg);
      expect(sparse.stepDeg).toBeGreaterThan(base.stepDeg);
      expect(dense.lengthPx).toBe(base.lengthPx);
      expect(large.lengthPx).toBe(base.lengthPx * 2);
      expect(large.stepDeg).toBe(base.stepDeg);
      const retina = glyphDisplayLayout(style, 100, 2, DEFAULT_GLYPH_APPEARANCE);
      expect(retina.stepDeg).toBe(base.stepDeg);
      expect(retina.lengthPx).toBe(base.lengthPx * 2);
    }
  });

  it("honors zero opacity and the optional speed fade", () => {
    expect(glyphOpacity({ ...DEFAULT_GLYPH_APPEARANCE, opacity_percent: 0 }, 40)).toBe(0);
    expect(glyphOpacity({ ...DEFAULT_GLYPH_APPEARANCE, opacity_percent: 60, fade_with_speed: false }, 1)).toBe(0.6);
    expect(glyphOpacity({ ...DEFAULT_GLYPH_APPEARANCE, opacity_percent: 60 }, 12.5)).toBe(0.3);
    expect(glyphRgb("#ff8000")).toEqual([1, 128 / 255, 0]);
  });

  it("keeps mixed field kinds distinct even when both contain calm values", () => {
    const bytes = new Uint8Array(256 * 256 * 4);
    bytes[3] = 0xfc; // Wind, fully covered calm.
    bytes[7] = 0x7c; // Current, fully covered calm.
    const all = glyphCoverage(bytes);
    const wind = glyphCoverage(bytes, true);
    expect(glyphCovered(all, 0, 0)).toBe(true);
    expect(glyphCovered(all, 1.5 / 256, 0)).toBe(true);
    expect(glyphCovered(wind, 0, 0)).toBe(true);
    expect(glyphCovered(wind, 1.5 / 256, 0)).toBe(false);
  });

  it("respects both styles' spacing where different densities meet", () => {
    const candidates = Array.from({ length: 48 }, (_, i) => ({ lon: i * 0.25 - 6, lat: 0, step: i < 24 ? 2 : 1 }));
    const placed = spacedGlyphs(candidates, p => p.step, { centerLon: 0, centerLat: 0, pxPerDeg: 50 });
    expect(placed.filter(p => p.lon >= 0).length).toBeGreaterThan(placed.filter(p => p.lon < 0).length);
    for (const p of placed) for (const q of placed) if (p !== q) {
      expect(Math.abs(p.lon - q.lon)).toBeGreaterThanOrEqual((p.step + q.step) * 0.4 - 1e-9);
    }
  });
});

/**
 * The shader sources, checked statically.
 *
 * GLSL is compiled by the driver, so a mistake in it fails at runtime and only
 * once a project is open and the map has mounted — the point furthest from
 * where the mistake was made. These are the errors that can be caught without a
 * GL context: a uniform the renderer looks up but the shader never declares, a
 * helper called in a program that does not include it, and a template hole left
 * un-interpolated.
 *
 * They are not a substitute for compiling. They are the half of the problem
 * that does not need a GPU.
 */
import { describe, expect, it } from "vitest";

import {
  GEO_FRAG,
  GEO_VERT,
  GLYPH_FRAG,
  GLYPH_VERT,
  RASTER_FRAG,
  RASTER_VERT,
} from "./shaders";

/** The programs, as the renderer links them. */
const PROGRAMS = {
  geo: [GEO_VERT, GEO_FRAG],
  raster: [RASTER_VERT, RASTER_FRAG],
  glyph: [GLYPH_VERT, GLYPH_FRAG],
} as const;

/** Every uniform a source declares. */
function declared(source: string): Set<string> {
  const names = new Set<string>();
  for (const match of source.matchAll(/uniform\s+\w+\s+(\w+)/g)) names.add(match[1]!);
  return names;
}

/** Every function a source defines. */
function defines(source: string): Set<string> {
  const names = new Set<string>();
  for (const match of source.matchAll(/^\s*\w+\s+(\w+)\s*\([^)]*\)\s*\{/gm)) {
    names.add(match[1]!);
  }
  return names;
}

describe("every shader source", () => {
  /**
   * A `${...}` that survived into the source is a fragment that was never
   * substituted — GLSL has no such syntax, so the program would fail to
   * compile with an error pointing at a line nobody wrote.
   */
  it("has no un-interpolated template holes", () => {
    for (const [name, sources] of Object.entries(PROGRAMS)) {
      for (const source of sources) {
        expect(source, `${name} carries a template hole`).not.toMatch(/\$\{/);
      }
    }
  });

  it("declares its version and precision first", () => {
    for (const [name, sources] of Object.entries(PROGRAMS)) {
      for (const source of sources) {
        expect(source.startsWith("#version 300 es"), `${name}`).toBe(true);
        expect(source).toContain("precision highp float;");
      }
    }
  });

  /** A reserved word as a variable is the WGSL trap's GLSL cousin. */
  it("does not use a keyword as an identifier", () => {
    for (const [name, sources] of Object.entries(PROGRAMS)) {
      for (const source of sources) {
        expect(source, `${name}`).not.toMatch(/\b(?:float|vec2|vec4|int)\s+(?:in|out)\b/);
      }
    }
  });
});

describe("the live-gesture mask", () => {
  /**
   * The screen mask is what makes a mask cover and a clone clone while the pointer
   * is down (spec.md 6.1). It is applied in the two programs that draw the
   * field — the raster and the glyphs — and a program that calls `maskFactor`
   * without including the block would not compile.
   */
  it("is included by every program that applies it", () => {
    for (const [name, sources] of Object.entries(PROGRAMS)) {
      for (const source of sources) {
        if (!source.includes("maskFactor()")) continue;
        expect(defines(source), `${name} calls maskFactor without defining it`).toContain(
          "maskFactor",
        );
        for (const uniform of ["uMask", "uMaskSize", "uMaskMode"]) {
          expect(declared(source), `${name} is missing ${uniform}`).toContain(uniform);
        }
      }
    }
  });

  /**
   * Both halves of the field respect it. A glyph left standing over a masked
   * patch would point at a wind that is no longer there, which reads as the
   * erasure having half worked.
   */
  it("is applied to the speed raster and to the glyphs", () => {
    expect(RASTER_FRAG).toContain("maskFactor()");
    expect(GLYPH_FRAG).toContain("maskFactor()");
  });

  /**
   * The basemap is never masked. A mask takes the field away, not the
   * coastline underneath it — that is the whole reason the field can be removed
   * at all, since something has to be left to see.
   */
  it("is not applied to the basemap", () => {
    expect(GEO_FRAG).not.toContain("maskFactor");
    expect(GEO_VERT).not.toContain("maskFactor");
  });

  /** Mode 0 must leave the field exactly as it was, or every ordinary frame
   * would be altered by a preview that is not happening. */
  it("has an off mode that changes nothing", () => {
    expect(RASTER_FRAG).toMatch(/uMaskMode\s*==\s*0.*return 1\.0/s);
  });
});

describe("the renderer's uniform lookups", () => {
  /**
   * Every uniform the shader declares should be one the renderer knows how to
   * set. The reverse — looking up a name the shader never declares — is
   * harmless at link time and silently does nothing at draw time, which is the
   * worse of the two failures and the reason this is checked from the source
   * rather than from the lookup list.
   */
  it("cover the uniforms the field programs declare", async () => {
    // The renderer names them in one place; this is that list, restated so a
    // shader gaining a uniform has to be noticed here too.
    const known = new Set([
      "uCamera", "uViewport", "uLonOffset",
      "uMask", "uMaskSize", "uMaskMode",
      "uTileGeo", "uTile", "uSpeedScale", "uRampMax", "uDim",
      "uGlyphOrigin", "uGlyphStep", "uGrid", "uSpacing",
      "uStyle", "uSizeScale", "uColor", "uPixelRatio",
    ]);

    for (const [name, sources] of Object.entries(PROGRAMS)) {
      for (const source of sources) {
        for (const uniform of declared(source)) {
          expect(known, `${name} declares ${uniform}, which the renderer never sets`).toContain(
            uniform,
          );
        }
      }
    }
  });
});

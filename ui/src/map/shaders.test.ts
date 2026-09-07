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
import { PROJECTIONS, projectionOf } from "./projection";

/** The programs, as the renderer links them. */
const PROGRAMS = {
  geo: [GEO_VERT, GEO_FRAG],
  raster: [RASTER_VERT, RASTER_FRAG],
  glyph: [GLYPH_VERT, GLYPH_FRAG],
} as const;

/** Matches a uniform declaration, capturing its precision, type and name. */
const UNIFORM = /uniform\s+(?:(lowp|mediump|highp)\s+)?(\w+)\s+(\w+)\s*;/g;

/** Every uniform a source declares. */
function declared(source: string): Set<string> {
  const names = new Set<string>();
  for (const match of source.matchAll(UNIFORM)) names.add(match[3]!);
  return names;
}

/** Every uniform a source declares, with the precision and type it gives it. */
function declarations(source: string): Map<string, string> {
  const out = new Map<string, string>();
  for (const match of source.matchAll(UNIFORM)) {
    out.set(match[3]!, `${match[1] ?? ""} ${match[2]}`.trim());
  }
  return out;
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

describe("uniforms declared in both stages of a program", () => {
  /**
   * A shared prelude means a uniform can be declared twice, once per stage,
   * and GLSL ES requires the two declarations to agree — on precision as well
   * as on type. That is a trap rather than a formality: an `int` defaults to
   * `highp` in a vertex shader and `mediump` in a fragment one, so a bare
   * `uniform int` in a block both stages include links with "Precisions of
   * uniform 'x' differ between VERTEX and FRAGMENT shaders" and the map never
   * mounts. Floats escape it only because every source here opens with
   * `precision highp float`.
   *
   * This was live: `uProjection` was declared bare in the shared projection
   * block, and the raster program — the only one that includes it in both
   * stages — refused to link.
   */
  it("agree on precision as well as on type", () => {
    for (const [name, [vert, frag]] of Object.entries(PROGRAMS)) {
      const inVertex = declarations(vert!);
      const inFragment = declarations(frag!);
      for (const [uniform, vertexDecl] of inVertex) {
        const fragmentDecl = inFragment.get(uniform);
        if (fragmentDecl === undefined) continue;
        expect(fragmentDecl, `${name}: ${uniform} is declared differently`).toBe(vertexDecl);
        // Type agreement is not enough for an integer: the two stages disagree
        // about what an unqualified one means, so it has to say.
        if (/\b(u?int|ivec[234]|uvec[234])\b/.test(vertexDecl)) {
          expect(
            vertexDecl,
            `${name}: ${uniform} is an integer shared by both stages and must state its precision`,
          ).toMatch(/\b(lowp|mediump|highp)\b/);
        }
      }
    }
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
      "uCamera", "uViewport", "uLonOffset", "uProjection",
      "uMask", "uMaskSize", "uMaskMode",
      "uTileGeo", "uTile", "uSpeedScale", "uRampWind", "uRampCurrent", "uDim",
      "uGlyphOrigin", "uGlyphStep", "uGrid", "uSpacing",
      "uSizeScaleArrow", "uSizeScaleBarb", "uColor", "uPixelRatio",
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

/**
 * The projection lives twice: once in `projection.ts` for the pointer and the
 * overlay, once in GLSL for the map itself. Nothing but agreement between them
 * makes a stroke land where the field is drawn, and a divergence would show as
 * arrows sitting slightly off the colour they describe — which is easy to look
 * at and not see.
 *
 * So the shader's own text is run here. `glslFn` lifts a function out of the
 * shipped source and evaluates it as JavaScript: the two languages agree on
 * arithmetic, and the handful of names that differ are supplied below. It is
 * the actual string that gets compiled, not a copy of it.
 */
function glslFn(source: string, name: string): (arg: number, mode: number) => number {
  const signature = new RegExp(`float ${name}\\(float (\\w+)\\) \\{`).exec(source);
  if (!signature) throw new Error(`no ${name} in the shader`);
  const parameter = signature[1] as string;
  let depth = 0;
  let end = source.indexOf("{", signature.index);
  const open = end;
  do {
    if (source[end] === "{") depth += 1;
    if (source[end] === "}") depth -= 1;
    end += 1;
  } while (depth > 0 && end < source.length);

  // The shader's own top-level constants, so the test reads VE_DEG from the
  // source rather than restating it.
  const constants = [...source.matchAll(/const float (\w+) = ([^;]+);/g)]
    .map((match) => `const ${match[1]} = ${match[2]};`)
    .join("\n");

  const body = constants + source
    .slice(open + 1, end - 1)
    .replace(/\/\/[^\n]*/g, "")
    .replace(/\bfloat\s+/g, "const ")
    .replace(/\b(log|tan|atan|exp|clamp)\(/g, "M.$1(");
  // eslint-disable-next-line @typescript-eslint/no-implied-eval
  const compiled = new Function(parameter, "uProjection", "M", body) as (
    arg: number,
    mode: number,
    maths: unknown,
  ) => number;
  const M = {
    log: Math.log,
    tan: Math.tan,
    atan: Math.atan,
    exp: Math.exp,
    clamp: (x: number, low: number, high: number) => Math.min(Math.max(x, low), high),
  };
  return (arg, mode) => compiled(arg, mode, M);
}

describe("the shader's projection and the pointer's", () => {
  const latToY = glslFn(GEO_VERT, "latToY");
  const yToLat = glslFn(RASTER_FRAG, "yToLat");

  it("agree on where every latitude lands", () => {
    for (const projection of PROJECTIONS) {
      for (let lat = -90; lat <= 90; lat += 2.5) {
        expect(latToY(lat, projection.mode), `${projection.id} at ${lat}`).toBeCloseTo(
          projection.yOf(lat),
          9,
        );
      }
    }
  });

  it("agree on the inverse the raster reads a pixel through", () => {
    for (const projection of PROJECTIONS) {
      const limit = projection.yOf(projection.maxLat);
      for (let y = -limit; y <= limit; y += limit / 20) {
        expect(yToLat(y, projection.mode), `${projection.id} at ${y}`).toBeCloseTo(
          projection.latOf(y),
          9,
        );
      }
    }
  });

  it("numbers the projections the way the shader branches", () => {
    // The default branch is mode 0, so equirectangular has to be it: anything
    // else would draw flat wherever its own branch was not written.
    expect(PROJECTIONS.map((p) => p.mode)).toEqual([0, 1, 2]);
    expect(projectionOf("equirectangular").mode).toBe(0);
  });
});

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
  BASE_FRAG,
  BASE_VERT,
  GEO_FRAG,
  GEO_VERT,
  GLYPH_FRAG,
  GLYPH_VERT,
  IMAGE_FRAG,
  IMAGE_VERT,
  RASTER_FRAG,
  RASTER_VERT,
  SMEAR_FRAG,
  SMEAR_VERT,
} from "./shaders";
import { PROJECTIONS, projectionOf } from "./projection";

/** The programs, as the renderer links them. */
const PROGRAMS = {
  geo: [GEO_VERT, GEO_FRAG],
  base: [BASE_VERT, BASE_FRAG],
  raster: [RASTER_VERT, RASTER_FRAG],
  glyph: [GLYPH_VERT, GLYPH_FRAG],
  smear: [SMEAR_VERT, SMEAR_FRAG],
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

describe("the live-gesture operator", () => {
  /**
   * The screen mask is what makes a mask cover, a clone clone and a modifier
   * modify while the pointer is down (spec.md 6.1, M32). It is applied in the
   * programs that draw the field — the raster, the glyphs and the liquify's
   * read-back — and a program that calls into the block without including it
   * would not compile.
   */
  it("is included by every program that applies it", () => {
    for (const [name, sources] of Object.entries(PROGRAMS)) {
      for (const source of sources) {
        if (!source.includes("maskFactor") && !source.includes("opCoverage")) continue;
        expect(defines(source), `${name} applies the operator without defining it`).toContain(
          "opApply",
        );
        for (const uniform of ["uSourceCoverage", "uUseSourceCoverage", "uMask", "uMaskSize", "uOpKind", "uOpAmount", "uOpCount"]) {
          expect(declared(source), `${name} is missing ${uniform}`).toContain(uniform);
        }
      }
    }
  });

  /**
   * Both halves of the field respect it. A glyph left standing over a masked
   * patch would point at a wind that is no longer there, which reads as the
   * erasure having half worked; a glyph over an intensified patch that kept
   * its old length would say the intensity had not taken.
   */
  it("is applied to the speed raster and to the glyphs", () => {
    expect(RASTER_FRAG).toContain("maskFactor()");
    expect(RASTER_FRAG).toContain("opApply(");
    expect(GLYPH_VERT).toContain("maskFactorAt(");
    expect(GLYPH_VERT).toContain("opApply(");
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

  /** Kind 0 must leave the field exactly as it was, or every ordinary frame
   * would be altered by a preview that is not happening. */
  it("has an off kind that changes nothing", () => {
    expect(RASTER_FRAG).toMatch(/uOpKind\s*==\s*0\) return 1\.0/s);
    expect(RASTER_FRAG).toMatch(/void opApply[^}]*if \(coverage <= 0\.0\) return;/s);
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

describe("scoping a live edit to one layer (M44)", () => {
  /**
   * An edit acts on one layer, but a tile is the whole visible stack in one
   * texel. A program that applies an operator without the means to tell which
   * pixels belong to the edited layer applies it to every layer at once —
   * which is what a stroke on a lower layer used to do, changing the layers
   * above it until the button came up.
   *
   * So: whatever applies an operator must also be able to scope it. Stated as
   * a property of the sources rather than of one program, because the way
   * this comes back is a *new* program that reads `uOpKind` and stops there.
   */
  it("gives every program that applies an operator the tile to compare against", () => {
    for (const [name, sources] of Object.entries(PROGRAMS)) {
      const uniforms = sources.flatMap((source) => [...declared(source)]);
      if (!uniforms.includes("uOpKind")) continue;
      // The smear program is the exception, and a known one: it displaces a
      // field already rendered to a texture rather than reading tiles, so it
      // has nothing to compare. Recorded in plan.md as not done.
      if (name === "smear") continue;
      expect(uniforms, `${name} applies an operator`).toContain("uBelow");
      expect(uniforms, `${name} applies an operator`).toContain("uEditScoped");
    }
  });

  /**
   * And the gate has to be *used*. Declaring the uniforms and then applying
   * the operator unconditionally would pass the check above and change
   * nothing on screen.
   */
  it("guards the raster's operator on it", () => {
    expect(RASTER_FRAG).toMatch(/bool mine = editedHere\(/);
    expect(RASTER_FRAG, "the modifier").toMatch(/if \(mine && uOpKind >= 3\)/);
    expect(RASTER_FRAG, "the mask and the eraser").toMatch(/mine \? maskFactor\(\) : 1\.0/);
  });

  it("guards the glyphs on it too", () => {
    // A glyph outside the edited layer keeps its direction and its opacity:
    // no coverage for a modifier to act on, and no fade from a mask.
    expect(GLYPH_VERT).toMatch(/uEditScoped == 1 && word == texelWordFrom\(uBelow, texel\)/);
    expect(GLYPH_VERT).toMatch(/if \(mine && uOpKind >= 3\)|covered = 0\.0;/);
  });

  /** Unbound, the pass falls back to acting everywhere rather than nowhere. */
  it("acts unscoped when no comparison tile is bound", () => {
    expect(RASTER_FRAG).toMatch(/if \(uEditScoped == 0\) return true;/);
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
      "uCamera", "uViewport", "uLonOffset", "uProjection", "uOrigin", "uRim", "uExact", "uMesh", "uTexture",
      "uSourceCoverage", "uUseSourceCoverage", "uMask", "uMaskSize", "uOpKind", "uOpAmount", "uOpCount", "uOpRadius", "uOpFeather",
      "uField",
      "uTileGeo", "uTile", "uSpeedScale", "uRampWind", "uRampCurrent", "uDim",
      "uRampStopsWind", "uRampStopsCurrent", "uRampCountWind", "uRampCountCurrent",
      "uBelow", "uCoverageOnly", "uEditScoped",
      "uColor", "uPixelRatio", "uLengthArrow", "uLengthBarb", "uStrokeArrow", "uStrokeBarb",
      "uColorArrow", "uColorBarb", "uFadeArrow", "uFadeBarb", "uShadowPass",
      "uShadowArrow", "uShadowBarb", "uShadowOffsetArrow", "uShadowOffsetBarb",
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
  const names = ["equalAreaK", "cylindricalPolynomial", "cylindricalY", "cylindricalLat", name];
  const constants = [...source.matchAll(/const float (\w+) = ([^;]+);/g)]
    .map(match => `const ${match[1]} = ${match[2]};`).join("\n");
  const functions = names.map(fn => {
    const signature = new RegExp(`(?:float|vec2) ${fn}\\(([^)]*)\\) \\{`).exec(source);
    if (!signature) throw new Error(`no ${fn} in the shader`);
    const parameters = signature[1]!.replace(/\b(?:int|float)\s+/g, "");
    const open = source.indexOf("{", signature.index);
    let end = open, depth = 0;
    do {
      if (source[end] === "{") depth++;
      if (source[end] === "}") depth--;
      end++;
    } while (depth > 0 && end < source.length);
    const body = source.slice(open + 1, end - 1)
      .replace(/\/\/[^\n]*/g, "")
      .replace(/\b(?:float|int|vec2)\s+/g, "let ")
      .replace(/\b(log|tan|atan|exp|sin|asin|clamp|vec2)\(/g, "M.$1(");
    return `function ${fn}(${parameters}) { ${body} }`;
  }).join("\n");
  const compiled = new Function("arg", "uProjection", "M", `${constants}\n${functions}\nreturn ${name}(arg);`);
  const maths = {
    log: Math.log, tan: Math.tan, atan: Math.atan, exp: Math.exp,
    sin: Math.sin, asin: Math.asin,
    clamp: (x: number, low: number, high: number) => Math.min(Math.max(x, low), high),
    vec2: (x: number, y: number) => ({ x, y }),
  };
  return (arg, mode) => compiled(arg, mode, maths) as number;
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
    expect(PROJECTIONS.map((p) => p.mode)).toEqual(Array.from({ length: 14 }, (_, i) => i));
    expect(projectionOf("equirectangular").mode).toBe(0);
  });
});

describe("a warped image (spec.md 4.9, control points)", () => {
  /**
   * The warped path hands the vertex shader lon/lat directly, so projection
   * happens strictly after the warp — which is the whole reason this works in
   * every projection. The shader must therefore read `aGeo` when `uWarped` is
   * set and never recompute the place from the affine uniforms.
   */
  it("takes a warped image's position from the mesh, not from the affine", () => {
    expect(IMAGE_VERT).toContain("aGeo");
    expect(IMAGE_VERT).toMatch(/uWarped\s*\?\s*aGeo/);
  });

  /**
   * The globe's per-pixel inverse cannot invert a spline, so a warped image
   * must not take that branch. `uProjection >= 15` has to be guarded by
   * `!uWarped` or a bent chart is sampled through a 2x2 affine inverse and
   * comes out scrambled.
   */
  it("keeps a warped image off the globe's per-pixel inverse", () => {
    // The shared projection prelude also matches `uProjection >= 15` — that
    // is `geoToScreen`'s own forward branch, correct for a warped image too
    // (it is what draws one on the globe, vertex by vertex). The lookahead
    // singles out the per-pixel *inverse* block specifically, by the call
    // that only it makes.
    const branch = IMAGE_FRAG.match(
      /if\s*\([^)]*uProjection\s*>=\s*15[^)]*\)(?=\s*\{[^}]*azimuthalInverse)/,
    );
    expect(branch, "the globe's per-pixel inverse branch moved").not.toBeNull();
    expect(branch![0]).toContain("!uWarped");
  });

  /** An integer uniform in a shared prelude must state its precision. */
  it("declares uWarped in both stages without relying on a default", () => {
    for (const source of [IMAGE_VERT, IMAGE_FRAG]) {
      expect(source).toMatch(/uniform bool uWarped;/);
    }
  });

  /**
   * A warped image on the globe (mode 15 up) is drawn through `geoToScreen`
   * vertex by vertex, exactly like `GEO_VERT`/`GEO_FRAG` and
   * `BASE_VERT`/`BASE_FRAG` — and every one of those carries a `vHorizon`
   * out/discard pair, because an azimuthal projects a vertex on the far side
   * of the globe to a real point on screen, just past the rim, rather than
   * refusing it. Without the same pair here, a warped chart on the far side
   * of the globe draws as a smear across the limb instead of not drawing.
   * Regression: `IMAGE_VERT` and `IMAGE_FRAG` did not carry it when the
   * warp mesh path was first added.
   */
  it("discards the far side of the globe, like every other vertex-projected surface", () => {
    expect(IMAGE_VERT).toMatch(/out float vHorizon;/);
    expect(IMAGE_VERT).toMatch(/vHorizon\s*=/);
    expect(IMAGE_FRAG).toMatch(/in float vHorizon;/);
    expect(IMAGE_FRAG).toMatch(/if\s*\(\s*vHorizon\s*<\s*0\.0\s*\)\s*discard;/);
  });
});

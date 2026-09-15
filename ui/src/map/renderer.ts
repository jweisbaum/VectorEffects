/**
 * The WebGL2 map renderer.
 *
 * Draw order, bottom to top: land fill, coastlines, the speed raster, the
 * graticule, then direction glyphs. The raster sits above the land because the
 * field is global — it covers land as well as sea — and the graticule sits above
 * the raster so it stays legible against strong colour.
 */

import {
  type VisibleTile,
  type Camera,
  type Viewport,
  glyphLattice,
  projectionFor,
  tileBounds,
  visibleBounds,
  visibleTiles,
} from "./camera";
import type { Basemap } from "./format";
import { KINDS, type FieldKindName } from "../kind";
import { DEFAULT_GLYPHS, glyphDisplayLayout, glyphRgb } from "./glyphAppearance";
import type { GlyphSettings } from "../generated/GlyphSettings";
import type { GlyphStyle } from "../generated/GlyphStyle";
import { GLYPH_SUBDIVISIONS, glyphCovered, glyphTileIsFull, glyphTileIsEmpty, spacedGlyphs } from "./glyphPlacement";
import { SPEED_MAX } from "./tileRange";
import {
  GEO_FRAG,
  GEO_VERT,
  GLYPH_FRAG,
  SMEAR_FRAG,
  SMEAR_VERT,
  GLYPH_VERT,
  IMAGE_FRAG,
  IMAGE_VERT,
  RAMP_MAX_STOPS,
  RASTER_FRAG,
  RASTER_VERT,
} from "./shaders";
import { editScope, type TileCache, tileSource } from "./tiles";

/** Full-scale speed of the tile encoding. Mirrors `ve_render::tile`. */
export const SPEED_SCALE_MPS = 100.0;

/**
 * Texture unit the live-gesture mask is bound to.
 *
 * Unit 0 is the tile every pass is already reading, so the mask needs one of
 * its own rather than a bind between draws.
 */
const MASK_UNIT = 1;
/**
 * Texture unit the tile-without-the-edited-layer is bound to (M44).
 *
 * Its own unit for the same reason the mask has one: it is read alongside the
 * tile on unit 0 in the same pass, so the two cannot share.
 */
const BELOW_UNIT = 2;
const SOURCE_COVERAGE_UNIT = 3;
/** Vertices per glyph instance; see the glyph vertex shader. */
const GLYPH_VERTICES = 54;

/**
 * How finely an image quad is subdivided, per side.
 *
 * A placement is an affine in degrees, so the image's edges are straight in
 * lon/lat and *curved* on the map under any projection but the flat one (M11).
 * Sixteen cells a side is 512 triangles — nothing on a GPU — and puts the
 * worst error at a fraction of a pixel even for an image spanning the globe.
 */
const IMAGE_CELLS = 16;

/** One georeferenced image, as the renderer draws it (spec.md 4.9, M18). */
export interface ImageDraw {
  /** Mask against the displayed vector field, in m/s. */
  speedRange?: [number, number] | undefined;
  /** The layer, which is also the texture's key. */
  layer: number;
  /**
   * Whether this image sits above every visible field layer (M49).
   *
   * An image is a reference to trace against, so it is drawn under the field
   * — but a layer moved to the top of the stack is one the user has asked to
   * see, and the field is nearly opaque, so drawing it underneath makes it
   * vanish. The stack order decides which side of the field it lands on.
   */
  over: boolean;
  /** Its texture, or null while the picture is still being fetched. */
  texture: WebGLTexture | null;
  /** `lon = a·u + b·v + c` with `u` and `v` across the whole image, 0 to 1. */
  placeLon: [number, number, number];
  /** And the same for the latitude. */
  placeLat: [number, number, number];
  /** How strongly it shows. */
  opacity: number;
}

/**
 * A gesture in progress that operates on the field rather than adding one.
 *
 * The mask and the clone stamp are defined against what is already there
 * (spec.md 6.2), so previewing them means changing what the map draws — the 2D
 * overlay sits above the field and can add pixels, never take them away. This
 * is how the two get a live preview at all, and it is why they are the only two
 * that need one.
 */
/**
 * What a gesture in progress does to the field, drawn live (spec.md 6.2, M32).
 *
 * - `remove`: the field goes where the gesture covers — the mask, the eraser.
 * - `clone`: the field from elsewhere comes in where it covers — the clone
 *   stamp, and a warp's push, which moves the field under the start to the
 *   end. Read through `source`.
 * - `gain`, `turn`, `radial`: a modifier changes what is there by `amount` —
 *   a fraction of the speed, degrees clockwise, a fraction radiated outward
 *   from the stroke's centreline.
 * - `smear`: a liquify moves the field along the stroke, each stamp by its
 *   own `deltas`.
 */
export type OperatorKind = "remove" | "clone" | "gain" | "turn" | "radial" | "smear";

export interface OperatorPreview {
  /**
   * The gesture's coverage, in screen space and at the framebuffer's size.
   *
   * Rasterised by the same path builder the overlay draws with, so this carries
   * no notion of which tool made it or what shape it is: a new tool inherits
   * the live preview by supplying a footprint.
   */
  mask: TexImageSource;
  kind: OperatorKind;
  /**
   * For a clone, the camera the source is read through.
   *
   * The main camera shifted so that the source lands where the brush is; see
   * `cloneSourceCamera`, which builds it.
   */
  source?: Camera;
  /** A clone leaves undefined source pixels untouched; a warp moves holes too. */
  transparentSource?: boolean;
  /** A modifier's setting: gain as a fraction, turn in degrees, radial as a fraction. */
  amount?: number;
  /**
   * The stroke's centreline in framebuffer pixels, y up, for a divergence
   * and a liquify. At most `OP_POINTS`; a longer stroke is thinned.
   */
  points?: readonly (readonly [number, number])[];
  /** A liquify's movement per stamp, framebuffer pixels, parallel to `points`. */
  deltas?: readonly (readonly [number, number])[];
  /** A liquify's stamp radius in framebuffer pixels, and its feather. */
  radiusPx?: number;
  feather?: number;
}

/** Most centreline points the shaders take; `OPERATOR` declares the arrays. */
export const OP_POINTS = 64;

/** Which part of an operation a pass draws. */
type OpStage = "none" | "remove" | "keep" | "source" | "apply";

/** The kind's code in the shaders, for the `apply` stage. */
const OP_CODE: Record<OperatorKind, number> = {
  remove: 1,
  clone: 1,
  gain: 3,
  turn: 4,
  radial: 5,
  smear: 6,
};

/** What to draw. */
/** The range of speeds of each kind the last frame drew, m/s, or null for none. */
export type SeenRanges = Record<FieldKindName, { min: number; max: number } | null>;

/** One end-to-end span of the colour ramp, m/s. */
export interface RampSpan {
  /** Speed mapped to the bottom of the ramp: 0 unless the auto scale is on (spec.md 5.3, M27). */
  min: number;
  /** Speed mapped to the top of the ramp. */
  max: number;
}

export interface RenderState {
  camera: Camera;
  view: Viewport;
  /** Opaque token naming the field's tiles, e.g. `<revision>/<step>`. */
  frame: string;
  /**
   * The frame shown before this one, or null.
   *
   * A tile this frame does not have yet is drawn from it, dimmed, rather than
   * left blank: a step change or an edit re-addresses every tile, and blanking
   * the map until the new ones land is a flicker on every scrub and a flash on
   * every stroke. The dim says the tile is not this frame's; playback never
   * shows one at all, because it advances only into resident frames.
   */
  heldFrame: string | null;
  /**
   * The same frame with the layer being erased left out, or null (M40).
   *
   * An eraser takes one layer's contribution away, but the mask preview acts
   * on the composited tile — so removing where the gesture covers took the
   * whole stack with it, and a stroke on a lower layer blanked every layer
   * above it until the button came up. This is what the hole is filled back
   * in with: the field the map would show if that layer were not there,
   * which is exactly what the erase leaves behind.
   */
  belowFrame?: string | null;
  /**
   * The same scope for the frame being held over (M82).
   *
   * A commit re-addresses `belowFrame`, so for the round trip after a stroke
   * it names tiles nobody has fetched — and a scope that has not arrived
   * applies to nothing (M72), which took the held removal off the map and let
   * the field it had taken flash back. The frame on screen through that
   * window is the one before the commit, and this is its scope.
   */
  belowHeldFrame?: string | null;
  /**
   * The layer being edited, by itself, or null (M45).
   *
   * Where a clone stamp reads its source from. Its commit samples the field
   * beneath the stamp in the layer the stamp is in, so a preview drawn from
   * the whole composited stack showed the wrong field arriving under the
   * brush — the top layer's, wherever a layer above covered the source.
   */
  sourceFrame?: string | null;
  /**
   * The colour ramp of each kind (M31). One tile holds both kinds, each cell
   * saying which it is, and the shader picks the ramp per cell: wind and
   * current are an order of magnitude apart.
   */
  ramps: Record<FieldKindName, RampSpan>;
  /**
   * The gradient each kind is painted with (spec.md 5.3, M42): stops as
   * `0`–`1` RGB, calm first, evenly spaced. Uploaded per frame, which costs
   * nothing — the tiles carry speed and not colour, so a gradient is a redraw
   * and never a re-render.
   */
  gradients: Record<FieldKindName, readonly (readonly [number, number, number])[]>;
  showGlyphs: boolean;
  /** Global per-style appearance preferences; omitted only by older fixtures. */
  glyphs?: GlyphSettings;
  showGraticule: boolean;
  pixelRatio: number;
  /** A gesture that operates on the field, while one is being drawn. */
  operator?: OperatorPreview | null;
  /**
   * Georeferenced images, bottom of the stack first (spec.md 4.9, M18).
   *
   * Drawn above the land and below the field: an image is a reference to trace
   * or compare against, so the coastline underneath it stays a coastline and
   * the field the user is painting stays on top of it.
   */
  images?: readonly ImageDraw[];
}

const SEA: [number, number, number, number] = [0.043, 0.078, 0.133, 1];
const LAND: [number, number, number, number] = [0.20, 0.25, 0.23, 1];
const COAST: [number, number, number, number] = [0.86, 0.93, 1.0, 0.75];
const GRATICULE: [number, number, number, number] = [0.55, 0.68, 0.85, 0.16];

/**
 * How much a tile held over from the previous frame is dimmed (M73).
 *
 * **1.0: not at all.** A held tile is the last good pixels for that patch of
 * map, and after M70 the only ones still marked held are genuinely stale —
 * the key is known, it differs, and the replacement is on its way. Dimming
 * those said "this is not current" at the cost of making every edit over a
 * slow layer flicker through a grey stage, and on a map that is mostly dark
 * ocean the grey read as a rendering fault rather than as a status.
 *
 * The mechanism stays rather than being torn out: `tileSource` still tells a
 * stale tile from a plain one, and this constant is the single place that
 * decides what the difference looks like. Lower it again and the marking
 * comes back everywhere at once.
 *
 * What is given up is the only signal that a tile is behind the document. A
 * slow render now shows the previous field at full strength with nothing to
 * say so — the spinner and the readiness strip (§9.5) are what report it
 * instead.
 */
const HELD_DIM = 1.0;

function compile(gl: WebGL2RenderingContext, type: number, source: string): WebGLShader {
  const shader = gl.createShader(type);
  if (!shader) throw new Error("could not create shader");
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    const log = gl.getShaderInfoLog(shader) ?? "unknown error";
    gl.deleteShader(shader);
    throw new Error(`shader compile failed: ${log}`);
  }
  return shader;
}

function link(gl: WebGL2RenderingContext, vert: string, frag: string): WebGLProgram {
  const program = gl.createProgram();
  if (!program) throw new Error("could not create program");
  const v = compile(gl, gl.VERTEX_SHADER, vert);
  const f = compile(gl, gl.FRAGMENT_SHADER, frag);
  gl.attachShader(program, v);
  gl.attachShader(program, f);
  gl.linkProgram(program);
  gl.deleteShader(v);
  gl.deleteShader(f);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    const log = gl.getProgramInfoLog(program) ?? "unknown error";
    gl.deleteProgram(program);
    throw new Error(`program link failed: ${log}`);
  }
  return program;
}

/** Uniform locations, resolved once. */
type Uniforms = Record<string, WebGLUniformLocation | null>;

function uniforms(gl: WebGL2RenderingContext, program: WebGLProgram, names: string[]): Uniforms {
  const out: Uniforms = {};
  for (const name of names) out[name] = gl.getUniformLocation(program, name);
  return out;
}

interface GeoBuffers {
  vao: WebGLVertexArrayObject;
  indexCount: number;
}

export class MapRenderer {
  private readonly gl: WebGL2RenderingContext;
  private readonly tiles: TileCache;

  private readonly imageProgram: WebGLProgram;
  private readonly imageUniforms: Uniforms;
  private readonly imageMesh: { vao: WebGLVertexArrayObject; count: number };
  private readonly geoProgram: WebGLProgram;
  private readonly rasterProgram: WebGLProgram;
  private readonly glyphProgram: WebGLProgram;
  private readonly geoUniforms: Uniforms;
  private readonly rasterUniforms: Uniforms;
  private readonly glyphUniforms: Uniforms;

  private readonly landByLod = new Map<number, GeoBuffers>();
  private readonly coastByLod = new Map<number, GeoBuffers>();
  private readonly quadVao: WebGLVertexArrayObject;
  private readonly glyphVao: WebGLVertexArrayObject;
  private readonly glyphBuffer: WebGLBuffer;
  /** Layout depends on coverage, not on the changing vectors during playback. */
  private readonly glyphMaskIds = new WeakMap<Uint8Array, number>();
  private nextGlyphMaskId = 0;
  private readonly glyphLayouts = new Map<string, readonly Float32Array[]>();
  private graticule: { vao: WebGLVertexArrayObject; buffer: WebGLBuffer; count: number } | null =
    null;
  private graticuleKey = "";
  /** Screen-space coverage of the gesture in progress, uploaded per frame. */
  private maskTexture: WebGLTexture | null = null;
  /** The field alone at the framebuffer's size, for a liquify's preview. */
  private fieldTarget: {
    framebuffer: WebGLFramebuffer;
    texture: WebGLTexture;
    width: number;
    height: number;
  } | null = null;
  private sourceCoverageReady = false;
  private capturingCoverage = false;
  private readonly smearProgram: WebGLProgram;
  private readonly smearUniforms: Uniforms;

  /** Set to have the next render read its pixels back. */
  private captureRequest: ((data: ImageData | null) => void) | null = null;

  constructor(gl: WebGL2RenderingContext, basemap: Basemap, tiles: TileCache) {
    this.gl = gl;
    this.tiles = tiles;

    this.imageProgram = link(gl, IMAGE_VERT, IMAGE_FRAG);
    this.geoProgram = link(gl, GEO_VERT, GEO_FRAG);
    this.rasterProgram = link(gl, RASTER_VERT, RASTER_FRAG);
    this.glyphProgram = link(gl, GLYPH_VERT, GLYPH_FRAG);
    this.smearProgram = link(gl, SMEAR_VERT, SMEAR_FRAG);

    const shared = ["uCamera", "uViewport", "uLonOffset", "uProjection"];
    this.geoUniforms = uniforms(gl, this.geoProgram, [...shared, "uColor"]);
    const mask = [
      "uMask", "uMaskSize", "uOpKind", "uOpAmount", "uOpCount", "uOpPoints", "uOpDeltas",
      "uOpRadius", "uOpFeather", "uSourceCoverage", "uUseSourceCoverage",
    ];
    this.smearUniforms = uniforms(gl, this.smearProgram, [...mask, "uField"]);
    this.rasterUniforms = uniforms(gl, this.rasterProgram, [
      ...shared, ...mask, "uTileGeo", "uTile", "uSpeedScale", "uRampWind", "uRampCurrent", "uDim",
      // An array's location is asked for by its first element, which is what
      // `getUniformLocation` accepts; `uniform3fv` then writes the whole run.
      "uRampStopsWind[0]", "uRampStopsCurrent[0]", "uRampCountWind", "uRampCountCurrent",
      "uBelow", "uEditScoped", "uCoverageOnly",
    ]);
    this.imageUniforms = uniforms(gl, this.imageProgram, [
      ...shared, "uPlaceLon", "uPlaceLat", "uImage", "uOpacity",
      "uFiltered", "uSpeedRange", "uFilterTile", "uFilterGeo", "uSpeedScale",
    ]);
    this.glyphUniforms = uniforms(gl, this.glyphProgram, [
      ...shared, ...mask, "uTileGeo", "uTile", "uSpeedScale", "uPixelRatio",
      "uLengthArrow", "uLengthBarb", "uStrokeArrow", "uStrokeBarb", "uColorArrow", "uColorBarb",
      "uFadeArrow", "uFadeBarb", "uShadowPass", "uShadowArrow", "uShadowBarb",
      "uShadowOffsetArrow", "uShadowOffsetBarb",
      "uBelow", "uEditScoped",
    ]);

    for (const lod of basemap.lods) {
      this.landByLod.set(lod.marker, this.buildGeo(lod.triVertices, lod.triIndices));
      this.coastByLod.set(lod.marker, this.buildGeo(lod.lineVertices, lod.lineIndices));
    }

    this.quadVao = this.buildQuad();
    this.imageMesh = this.buildImageMesh();
    // Only stations are uploaded; each mark's geometry comes from gl_VertexID.
    const glyphVao = gl.createVertexArray();
    const glyphBuffer = gl.createBuffer();
    if (!glyphVao || !glyphBuffer) throw new Error("could not create glyph buffers");
    this.glyphVao = glyphVao;
    this.glyphBuffer = glyphBuffer;
    gl.bindVertexArray(glyphVao);
    gl.bindBuffer(gl.ARRAY_BUFFER, glyphBuffer);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
    gl.vertexAttribDivisor(0, 1);
    gl.bindVertexArray(null);

    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
  }

  private buildGeo(vertices: Float32Array, indices: Uint32Array): GeoBuffers {
    const gl = this.gl;
    const vao = gl.createVertexArray();
    const vbo = gl.createBuffer();
    const ibo = gl.createBuffer();
    if (!vao || !vbo || !ibo) throw new Error("could not allocate basemap buffers");

    gl.bindVertexArray(vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, vbo);
    gl.bufferData(gl.ARRAY_BUFFER, vertices, gl.STATIC_DRAW);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, ibo);
    gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, indices, gl.STATIC_DRAW);
    gl.bindVertexArray(null);

    return { vao, indexCount: indices.length };
  }

  private buildQuad(): WebGLVertexArrayObject {
    const gl = this.gl;
    const vao = gl.createVertexArray();
    const vbo = gl.createBuffer();
    if (!vao || !vbo) throw new Error("could not allocate quad buffers");
    gl.bindVertexArray(vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, vbo);
    gl.bufferData(
      gl.ARRAY_BUFFER,
      new Float32Array([0, 0, 1, 0, 0, 1, 0, 1, 1, 0, 1, 1]),
      gl.STATIC_DRAW,
    );
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
    gl.bindVertexArray(null);
    return vao;
  }

  /**
   * A subdivided unit square, for image layers.
   *
   * Built once: the mesh is in the image's own 0-to-1 coordinates, so every
   * image of every size and every placement draws from this one buffer.
   */
  private buildImageMesh(): { vao: WebGLVertexArrayObject; count: number } {
    const gl = this.gl;
    const vao = gl.createVertexArray();
    const vbo = gl.createBuffer();
    if (!vao || !vbo) throw new Error("could not allocate image mesh buffers");

    const step = 1 / IMAGE_CELLS;
    const cells: number[] = [];
    for (let row = 0; row < IMAGE_CELLS; row++) {
      for (let col = 0; col < IMAGE_CELLS; col++) {
        const u = col * step;
        const v = row * step;
        cells.push(
          u, v, u + step, v, u, v + step,
          u, v + step, u + step, v, u + step, v + step,
        );
      }
    }

    gl.bindVertexArray(vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, vbo);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array(cells), gl.STATIC_DRAW);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
    gl.bindVertexArray(null);
    return { vao, count: cells.length / 2 };
  }

  /** World copies to draw so the map wraps seamlessly at the dateline. */
  private worldOffsets(state: RenderState): number[] {
    const bounds = visibleBounds(state.camera, state.view);
    const first = Math.floor((bounds.west + 180) / 360);
    const last = Math.floor((bounds.east + 180) / 360);
    const out: number[] = [];
    for (let i = first; i <= last; i++) out.push(i * 360);
    return out.length > 0 ? out : [0];
  }

  private setShared(
    u: Uniforms,
    camera: Camera,
    view: Viewport,
    lonOffset: number,
  ): void {
    const gl = this.gl;
    const projection = projectionFor(camera);
    // The camera's second component reaches the shader as the projection's own
    // vertical coordinate, computed here rather than there: the centre is the
    // one point every draw is measured from, so it is the one point the two
    // implementations of the formula must not differ on by a bit.
    gl.uniform3f(
      u.uCamera ?? null,
      camera.centerLon,
      projection.yOf(camera.centerLat),
      camera.pxPerDeg,
    );
    gl.uniform2f(u.uViewport ?? null, view.width, view.height);
    gl.uniform1f(u.uLonOffset ?? null, lonOffset);
    gl.uniform1i(u.uProjection ?? null, projection.mode);
  }

  /** The centreline arrays, padded to what the shaders declare. */
  private opArrays = {
    points: new Float32Array(OP_POINTS * 2),
    deltas: new Float32Array(OP_POINTS * 2),
  };

  /**
   * Points a program's operator uniforms at the gesture in progress.
   *
   * `none` leaves the field alone. `remove` takes it away where the gesture
   * covers — a mask, an eraser, and a clone before its source is drawn in.
   * `keep` keeps only what the gesture covers, which is how that source
   * arrives. `apply` is a modifier's own effect, by the operator's kind.
   */
  private setOperator(
    u: Uniforms,
    view: Viewport,
    operator: OperatorPreview | null,
    stage: OpStage,
  ): void {
    const gl = this.gl;
    const kind =
      operator === null || stage === "none"
        ? 0
        : stage === "remove"
          ? 1
          : stage === "keep" || stage === "source"
            ? 2
            : OP_CODE[operator.kind];
    gl.uniform1i(u.uOpKind ?? null, kind);
    gl.uniform1i(u.uSourceCoverage ?? null, SOURCE_COVERAGE_UNIT);
    gl.uniform1i(u.uUseSourceCoverage ?? null,
      operator?.transparentSource && this.sourceCoverageReady &&
      (stage === "remove" || stage === "keep") ? 1 : 0);
    gl.uniform1i(u.uCoverageOnly ?? null, this.capturingCoverage ? 1 : 0);
    gl.uniform2f(u.uMaskSize ?? null, view.width, view.height);
    gl.uniform1i(u.uMask ?? null, MASK_UNIT);
    gl.uniform1f(u.uOpAmount ?? null, operator?.amount ?? 0);
    const points = operator?.points ?? [];
    const count = Math.min(points.length, OP_POINTS);
    const { points: flatPoints, deltas: flatDeltas } = this.opArrays;
    flatPoints.fill(0);
    flatDeltas.fill(0);
    for (let i = 0; i < count; i++) {
      const point = points[i];
      const delta = operator?.deltas?.[i];
      if (point) {
        flatPoints[i * 2] = point[0];
        flatPoints[i * 2 + 1] = point[1];
      }
      if (delta) {
        flatDeltas[i * 2] = delta[0];
        flatDeltas[i * 2 + 1] = delta[1];
      }
    }
    gl.uniform1i(u.uOpCount ?? null, count);
    gl.uniform2fv(u.uOpPoints ?? null, flatPoints);
    gl.uniform2fv(u.uOpDeltas ?? null, flatDeltas);
    gl.uniform1f(u.uOpRadius ?? null, operator?.radiusPx ?? 0);
    gl.uniform1f(u.uOpFeather ?? null, operator?.feather ?? 0);
  }

  /** The offscreen target the field alone is drawn into, at the viewport's size. */
  private ensureFieldTarget(view: Viewport): WebGLFramebuffer | null {
    const gl = this.gl;
    const width = Math.max(1, Math.floor(view.width));
    const height = Math.max(1, Math.floor(view.height));
    const current = this.fieldTarget;
    if (current && current.width === width && current.height === height) {
      return current.framebuffer;
    }
    if (current) {
      gl.deleteFramebuffer(current.framebuffer);
      gl.deleteTexture(current.texture);
      this.fieldTarget = null;
    }
    const texture = gl.createTexture();
    const framebuffer = gl.createFramebuffer();
    if (!texture || !framebuffer) return null;
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, width, height, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    this.fieldTarget = { framebuffer, texture, width, height };
    return framebuffer;
  }

  /**
   * Uploads the gesture's coverage, and returns whether there is any.
   *
   * The mask is the swept footprint rasterised by the same path builder the
   * overlay draws with, so this knows nothing about which tool is being used or
   * what shape it makes — a new tool inherits the live preview by supplying a
   * footprint, exactly as it inherits the overlay one.
   */
  private uploadMask(source: TexImageSource | null): boolean {
    const gl = this.gl;
    if (!source) return false;
    if (!this.maskTexture) {
      const texture = gl.createTexture();
      if (!texture) return false;
      this.maskTexture = texture;
      gl.bindTexture(gl.TEXTURE_2D, texture);
      // Clamped and linear: the mask is a coverage field, so a soft edge is
      // wanted and sampling past the edge must read as "not covered".
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    }
    gl.activeTexture(gl.TEXTURE0 + MASK_UNIT);
    gl.bindTexture(gl.TEXTURE_2D, this.maskTexture);
    gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, true);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, source);
    gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
    return true;
  }

  /** The speed raster, for one camera and one mask mode. */
  private drawRaster(
    state: RenderState,
    camera: Camera,
    tiles: readonly VisibleTile[],
    stage: OpStage,
    frame?: string,
    scope?: string,
  ): void {
    const gl = this.gl;
    gl.useProgram(this.rasterProgram);
    gl.bindVertexArray(this.quadVao);
    gl.uniform1i(this.rasterUniforms.uTile ?? null, 0);
    gl.uniform1f(this.rasterUniforms.uSpeedScale ?? null, SPEED_SCALE_MPS);
    gl.uniform2f(this.rasterUniforms.uRampWind ?? null, state.ramps.wind.min, state.ramps.wind.max);
    gl.uniform2f(
      this.rasterUniforms.uRampCurrent ?? null,
      state.ramps.current.min,
      state.ramps.current.max,
    );
    this.setGradients(state);
    this.setOperator(this.rasterUniforms, state.view, state.operator ?? null, stage);
    gl.uniform1i(this.rasterUniforms.uBelow ?? null, BELOW_UNIT);

    for (const tile of tiles) {
      const shown = this.textureFor(state, tile, frame);
      if (!shown.texture) continue;
      const { texture, held } = shown;
      this.noteRange(shown.frame, tile);
      this.bindScope(this.rasterUniforms, scope, tile, texture, state.belowHeldFrame);
      gl.activeTexture(gl.TEXTURE0);
      const b = tileBounds(tile.z, tile.x, tile.y);
      this.setShared(this.rasterUniforms, camera, state.view, tile.lonOffset);
      gl.uniform1f(this.rasterUniforms.uDim ?? null, held ? HELD_DIM : 1.0);
      gl.uniform4f(
        this.rasterUniforms.uTileGeo ?? null,
        b.west, b.north, b.east - b.west, b.north - b.south,
      );
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.drawArrays(gl.TRIANGLES, 0, 6);
    }
  }

  /** Draws a run of georeferenced images, in the order given. */
  private drawImages(
    state: RenderState,
    offsets: readonly number[],
    images: readonly ImageDraw[],
  ): void {
    if (images.length === 0) return;
    const gl = this.gl;
    gl.useProgram(this.imageProgram);
    gl.bindVertexArray(this.imageMesh.vao);
    gl.uniform1i(this.imageUniforms.uImage ?? null, 0);
    gl.activeTexture(gl.TEXTURE0);
    for (const image of images) {
      if (!image.texture || image.opacity <= 0) continue;
      gl.uniform3f(this.imageUniforms.uPlaceLon ?? null, ...image.placeLon);
      gl.uniform3f(this.imageUniforms.uPlaceLat ?? null, ...image.placeLat);
      gl.uniform1f(this.imageUniforms.uOpacity ?? null, image.opacity);
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, image.texture);
      gl.uniform1i(this.imageUniforms.uFiltered ?? null, image.speedRange ? 1 : 0);
      if (image.speedRange) {
        gl.uniform2f(this.imageUniforms.uSpeedRange ?? null, ...image.speedRange);
        gl.uniform1i(this.imageUniforms.uFilterTile ?? null, 1);
        gl.uniform1f(this.imageUniforms.uSpeedScale ?? null, SPEED_SCALE_MPS);
        const visited = new Set<string>();
        for (const tile of visibleTiles(state.camera, state.view)) {
          const key = `${tile.z}/${tile.x}/${tile.y}`;
          if (visited.has(key)) continue;
          visited.add(key);
          const shown = this.textureFor(state, tile);
          if (!shown.texture) continue;
          const b = tileBounds(tile.z, tile.x, tile.y);
          gl.uniform4f(this.imageUniforms.uFilterGeo ?? null, b.west, b.north, b.east - b.west, b.north - b.south);
          gl.activeTexture(gl.TEXTURE1);
          gl.bindTexture(gl.TEXTURE_2D, shown.texture);
          for (const offset of offsets) {
            this.setShared(this.imageUniforms, state.camera, state.view, offset);
            gl.drawArrays(gl.TRIANGLES, 0, this.imageMesh.count);
          }
        }
        continue;
      }
      for (const offset of offsets) {
        this.setShared(this.imageUniforms, state.camera, state.view, offset);
        gl.drawArrays(gl.TRIANGLES, 0, this.imageMesh.count);
      }
    }
  }

  /**
   * Binds the tile-without-the-edited-layer for this tile, and says whether
   * it is there (M44).
   *
   * A live edit acts on one layer, but a tile is the whole visible stack in
   * one texel, so the shader needs something to compare against to know which
   * pixels the edited layer is what you are looking at. That is this frame:
   * the same tile with that layer left out.
   *
   * With **no scope asked for** the pass is unscoped and the gesture applies
   * everywhere, which is right: the tool is not aimed at one layer.
   *
   * With a scope asked for but **not yet resident**, this frame's own tile
   * stands in for it (M72). Comparing a tile against itself finds no pixel
   * the layer contributed, so the gesture applies to nothing and the tile is
   * drawn plainly until the real scope lands.
   *
   * That is the opposite of what it did, which was to fall back to *unscoped*
   * on the grounds that showing the gesture over every layer for a frame or
   * two was a smaller wrong than showing it over none. It is the larger wrong:
   * an intensity aimed at a current layer visibly intensified the wind
   * beneath it for the whole of the stroke, which is the thing scoping exists
   * to prevent. Showing nothing for a moment is a delay; showing an edit to a
   * layer the user did not aim at is a lie about what the tool does.
   *
   * And it *fetches* rather than peeking, so the scope arrives on its own
   * instead of only if something else happened to warm it.
   */
  private bindScope(
    u: Uniforms,
    scope: string | undefined,
    tile: VisibleTile,
    fallback: WebGLTexture,
    heldScope?: string | null,
  ): void {
    const gl = this.gl;
    // Fetched, not peeked: the scope then arrives on its own rather than only
    // if something else happened to warm it.
    //
    // Falling back to the *held* frame's scope rather than straight to
    // "nowhere" (M82): while the map is still showing the frame before a
    // commit, that frame's scope is the one that describes what is on screen,
    // and it is resident. Only with neither does the edit apply to nothing.
    const below =
      (scope ? this.tiles.get(scope, tile.z, tile.x, tile.y) : null) ??
      (heldScope ? this.tiles.peek(heldScope, tile.z, tile.x, tile.y) : null);
    const reach = editScope(scope !== undefined, below !== null);
    gl.uniform1i(u.uEditScoped ?? null, reach === "everywhere" ? 0 : 1);
    if (reach === "everywhere") return;
    // `nowhere` binds this frame's own tile as the thing to compare against:
    // a tile differs from itself nowhere, so the gesture touches nothing until
    // the real scope lands.
    gl.activeTexture(gl.TEXTURE0 + BELOW_UNIT);
    gl.bindTexture(gl.TEXTURE_2D, reach === "layer" && below ? below : fallback);
  }

  /**
   * Points the raster program at the gradients this frame is drawn with.
   *
   * Longer runs than the shader reserves are cut rather than overflowed, and
   * the catalogue is held to that length by a test, so a gradient that would
   * be silently truncated here fails the suite instead.
   */
  private setGradients(state: RenderState): void {
    const gl = this.gl;
    for (const [kind, stops] of [
      ["wind", state.gradients.wind],
      ["current", state.gradients.current],
    ] as const) {
      const used = stops.slice(0, RAMP_MAX_STOPS);
      const flat = this.gradientArrays[kind];
      flat.fill(0);
      used.forEach((stop, at) => {
        flat[at * 3] = stop[0];
        flat[at * 3 + 1] = stop[1];
        flat[at * 3 + 2] = stop[2];
      });
      const suffix = kind === "wind" ? "Wind" : "Current";
      gl.uniform3fv(this.rasterUniforms[`uRampStops${suffix}[0]`] ?? null, flat);
      gl.uniform1i(this.rasterUniforms[`uRampCount${suffix}`] ?? null, used.length);
    }
  }

  /** The stop arrays, padded to what the shader declares. */
  private gradientArrays = {
    wind: new Float32Array(RAMP_MAX_STOPS * 3),
    current: new Float32Array(RAMP_MAX_STOPS * 3),
  };

  /**
   * This frame's texture for a tile, fetching it if absent — or, until it
   * lands, the held frame's, which is marked so the raster can dim it.
   */
  private textureFor(
    state: RenderState,
    tile: VisibleTile,
    from?: string,
  ): { texture: WebGLTexture | null; held: boolean; frame: string } {
    // A frame other than this one's — the field beneath an erase (M40) — is
    // drawn from what it has and from nothing else. Falling back to the held
    // frame there would fill the hole with the very stack the erase is
    // taking a layer out of, which is the bug this exists to fix.
    if (from !== undefined && from !== state.frame) {
      return { texture: this.tiles.get(from, tile.z, tile.x, tile.y), held: false, frame: from };
    }
    const texture = this.tiles.get(state.frame, tile.z, tile.x, tile.y);
    // Stale and merely unresolved are different things (M70): before a frame's
    // keys come back nothing says this tile changed, and dimming on that
    // flashed the whole map after every edit.
    const source = tileSource({
      hasTexture: texture !== null,
      hasHeldFrame: state.heldFrame !== null,
      unresolved: this.tiles.unresolved(state.frame, tile.z, tile.x, tile.y),
    });
    if (source === "frame") return { texture, held: false, frame: state.frame };
    const heldFrame = state.heldFrame as string;
    return {
      texture: this.tiles.peek(heldFrame, tile.z, tile.x, tile.y),
      held: source === "held-stale",
      frame: heldFrame,
    };
  }

  /**
   * The range of speeds of each kind across the tiles the last frame drew,
   * in m/s, or null for a kind none of them held (spec.md 5.3, M27).
   * Accumulated by the raster pass from each tile's own ranges, which the
   * cache read at upload.
   */
  private seen: SeenRanges = { wind: null, current: null };

  private noteRange(frame: string, tile: VisibleTile): void {
    const ranges = this.tiles.rangeOf(frame, tile.z, tile.x, tile.y);
    if (ranges === null) return;
    for (const kind of KINDS) {
      const range = ranges[kind];
      if (range === null) continue;
      const min = (range[0] / SPEED_MAX) * SPEED_SCALE_MPS;
      const max = (range[1] / SPEED_MAX) * SPEED_SCALE_MPS;
      const seen = this.seen[kind];
      if (seen === null) this.seen[kind] = { min, max };
      else {
        if (min < seen.min) seen.min = min;
        if (max > seen.max) seen.max = max;
      }
    }
  }

  /** Graticule interval that keeps lines at least ~70 px apart. */
  private graticuleInterval(pxPerDeg: number): number {
    for (const step of [30, 15, 10, 5, 2, 1, 0.5, 0.25]) {
      if (step * pxPerDeg >= 70) return step;
    }
    return 0.25;
  }

  private ensureGraticule(state: RenderState): void {
    const gl = this.gl;
    const step = this.graticuleInterval(state.camera.pxPerDeg);
    const key = `${step}`;
    if (this.graticuleKey === key && this.graticule) return;

    const points: number[] = [];
    for (let lon = -180; lon <= 180; lon += step) {
      points.push(lon, -90, lon, 90);
    }
    for (let lat = -90; lat <= 90; lat += step) {
      points.push(-180, lat, 180, lat);
    }

    if (!this.graticule) {
      const vao = gl.createVertexArray();
      const buffer = gl.createBuffer();
      if (!vao || !buffer) throw new Error("could not allocate graticule buffers");
      gl.bindVertexArray(vao);
      gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
      gl.enableVertexAttribArray(0);
      gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
      gl.bindVertexArray(null);
      this.graticule = { vao, buffer, count: 0 };
    }

    gl.bindBuffer(gl.ARRAY_BUFFER, this.graticule.buffer);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array(points), gl.DYNAMIC_DRAW);
    this.graticule.count = points.length / 2;
    this.graticuleKey = key;
  }

  /** Coarse coastlines when zoomed out; the detailed set is wasted there. */
  private lodFor(pxPerDeg: number): number {
    return pxPerDeg < 6 ? 110 : 50;
  }

  /** The direction glyphs, for one camera and one mask mode. */
  private drawGlyphs(
    state: RenderState,
    camera: Camera,
    tiles: readonly VisibleTile[],
    stage: OpStage,
    frame?: string,
    scope?: string,
  ): void {
    const gl = this.gl;
    // Spacing is resolved to a whole-degree lattice step so the grid is
    // globally anchored. `glyphDisplayLayout` is shared with the gesture
    // preview. Each style can have its own density; coverage and kind ensure
    // a barb and an arrow never claim the same geographic point.
    const glyphs = state.glyphs ?? DEFAULT_GLYPHS;
    const layouts = {
      arrow: glyphDisplayLayout("arrow", camera.pxPerDeg, state.pixelRatio, glyphs.arrow),
      barb: glyphDisplayLayout("barb", camera.pxPerDeg, state.pixelRatio, glyphs.barb),
    };
    const spacing = Math.max(layouts.arrow.spacing, layouts.barb.spacing);
    const margin = Math.max(spacing * 2, layouts.arrow.lengthPx * 1.5, layouts.barb.lengthPx * 1.5) + 12 * state.pixelRatio;
    gl.useProgram(this.glyphProgram);
    gl.bindVertexArray(this.glyphVao);
    gl.uniform1i(this.glyphUniforms.uTile ?? null, 0);
    gl.uniform1f(this.glyphUniforms.uSpeedScale ?? null, SPEED_SCALE_MPS);
    gl.uniform1f(this.glyphUniforms.uPixelRatio ?? null, state.pixelRatio);
    for (const style of ["arrow", "barb"] as const) {
      const appearance = glyphs[style];
      const suffix = style === "arrow" ? "Arrow" : "Barb";
      gl.uniform1f(this.glyphUniforms[`uLength${suffix}`] ?? null, layouts[style].lengthPx);
      gl.uniform1f(this.glyphUniforms[`uStroke${suffix}`] ?? null, appearance.stroke_width_px * state.pixelRatio);
      gl.uniform4f(this.glyphUniforms[`uColor${suffix}`] ?? null, ...glyphRgb(appearance.color), appearance.opacity_percent / 100);
      gl.uniform1i(this.glyphUniforms[`uFade${suffix}`] ?? null, appearance.fade_with_speed ? 1 : 0);
      gl.uniform4f(this.glyphUniforms[`uShadow${suffix}`] ?? null, ...glyphRgb(appearance.shadow.color),
        appearance.shadow.enabled ? appearance.shadow.opacity_percent / 100 : 0);
      gl.uniform2f(this.glyphUniforms[`uShadowOffset${suffix}`] ?? null,
        appearance.shadow.offset_x_px * state.pixelRatio, appearance.shadow.offset_y_px * state.pixelRatio);
    }
    this.setOperator(this.glyphUniforms, state.view, state.operator ?? null, stage);
    gl.uniform1i(this.glyphUniforms.uBelow ?? null, BELOW_UNIT);

    const projection = projectionFor(camera);
    const centreY = projection.yOf(camera.centerLat);
    const batches: Array<{ tile: VisibleTile; texture: WebGLTexture; coverage: Uint8Array; wind: Uint8Array }> = [];
    const layoutParts = [layouts.arrow.stepDeg, layouts.barb.stepDeg, glyphs.arrow.opacity_percent > 0, glyphs.barb.opacity_percent > 0, margin, camera.pxPerDeg, camera.centerLon, camera.centerLat,
      camera.projection, state.view.width, state.view.height].join("/");
    const coverageKeys: string[] = [];

    for (const tile of tiles) {
      const { texture, frame: textureFrame } = this.textureFor(state, tile, frame);
      if (!texture) continue;
      const coverage = this.tiles.coverageOf(textureFrame, tile.z, tile.x, tile.y);
      const wind = this.tiles.windCoverageOf(textureFrame, tile.z, tile.x, tile.y);
      if (!coverage || !wind) continue;
      const b = tileBounds(tile.z, tile.x, tile.y);
      const originX =
        (b.west + tile.lonOffset - camera.centerLon) * camera.pxPerDeg + state.view.width / 2;
      // The tile's top and bottom edges on the map, not in latitude: a tile of
      // a fixed latitude span is a different height at different latitudes in
      // every projection but the flat one, and culling on the flat height
      // would skip tiles that are on screen (M11).
      const originY =
        (centreY - projection.yOf(b.north)) * camera.pxPerDeg + state.view.height / 2;
      const widthPx = (b.east - b.west) * camera.pxPerDeg;
      const heightPx = (projection.yOf(b.north) - projection.yOf(b.south)) * camera.pxPerDeg;

      // Keep a margin for spacing against neighboring sites just outside view.
      if (
        originX + widthPx < -margin || originX > state.view.width + margin ||
        originY + heightPx < -margin || originY > state.view.height + margin
      ) {
        continue;
      }
      const ids = [coverage, wind].map(mask => {
        let id = this.glyphMaskIds.get(mask);
        if (id === undefined) { id = this.nextGlyphMaskId++; this.glyphMaskIds.set(mask, id); }
        return id;
      });
      coverageKeys.push(`${tile.z}/${tile.x}/${tile.y}/${tile.lonOffset}/${ids.join("/")}`);
      batches.push({ tile, texture, coverage, wind });
    }
    const layoutKey = `${layoutParts}|${coverageKeys.join("|")}`;
    let pointsByTile = this.glyphLayouts.get(layoutKey);
    if (pointsByTile) {
      this.glyphLayouts.delete(layoutKey);
    } else {
      const candidates: Array<{ lon: number; lat: number; tileIndex: number; style: GlyphStyle }> = [];
      // Fully covered forecasts need no infill, including on the first frame
      // or while panning, when there is no cached layout yet.
      for (const style of ["arrow", "barb"] as const) {
        if (glyphs[style].opacity_percent === 0) continue;
        const stepDeg = layouts[style].stepDeg;
        const solid = batches.every(({ coverage, wind }) => style === "barb" ? glyphTileIsFull(wind) : glyphTileIsFull(coverage) && glyphTileIsEmpty(wind));
        for (const [tileIndex, { tile, coverage, wind }] of batches.entries()) {
          if (style === "barb" ? glyphTileIsEmpty(wind) : glyphTileIsFull(wind)) continue;
          const b = tileBounds(tile.z, tile.x, tile.y);
          const fineStep = solid ? stepDeg : stepDeg / GLYPH_SUBDIVISIONS;
          const lattice = glyphLattice(b, fineStep);
          if (lattice.cols === 0 || lattice.rows === 0) continue;
          for (let row = 0; row < lattice.rows; row++) {
            const lat = lattice.originLat - row * fineStep;
            const y = (centreY - projection.yOf(lat)) * camera.pxPerDeg + state.view.height / 2;
            if (y < -margin || y > state.view.height + margin) continue;
            for (let col = 0; col < lattice.cols; col++) {
              const lon = lattice.originLon + col * fineStep;
              const x = (lon + tile.lonOffset - camera.centerLon) * camera.pxPerDeg + state.view.width / 2;
              if (x < -margin || x > state.view.width + margin) continue;
              const u = (lon - b.west) / (b.east - b.west);
              const v = (b.north - lat) / (b.north - b.south);
              if (glyphCovered(coverage, u, v) && glyphCovered(wind, u, v) === (style === "barb")) {
                candidates.push({ lon: lon + tile.lonOffset, lat, tileIndex, style });
              }
            }
          }
        }
      }
      const points = batches.map((): number[] => []);
      // Select across the whole view: per-tile selection clusters marks at seams.
      for (const point of spacedGlyphs(candidates, p => layouts[p.style].stepDeg, camera)) {
        points[point.tileIndex]!.push(point.lon - batches[point.tileIndex]!.tile.lonOffset, point.lat);
      }
      pointsByTile = points.map(p => new Float32Array(p));
    }
    this.glyphLayouts.set(layoutKey, pointsByTile);
    if (this.glyphLayouts.size > 64) this.glyphLayouts.delete(this.glyphLayouts.keys().next().value!);
    // Draw every shadow first so none can cover a neighboring glyph's ink.
    const shadows = glyphs.arrow.shadow.enabled || glyphs.barb.shadow.enabled;
    for (const shadow of shadows ? [true, false] : [false]) {
      gl.uniform1i(this.glyphUniforms.uShadowPass ?? null, shadow ? 1 : 0);
      for (const [index, { tile, texture }] of batches.entries()) {
        const points = pointsByTile[index]!;
        if (points.length === 0) continue;
        const b = tileBounds(tile.z, tile.x, tile.y);
        this.bindScope(this.glyphUniforms, scope, tile, texture, state.belowHeldFrame);
        gl.activeTexture(gl.TEXTURE0);
        this.setShared(this.glyphUniforms, camera, state.view, tile.lonOffset);
        gl.uniform4f(
          this.glyphUniforms.uTileGeo ?? null,
          b.west, b.north, b.east - b.west, b.north - b.south,
        );
        gl.bindBuffer(gl.ARRAY_BUFFER, this.glyphBuffer);
        gl.bufferData(gl.ARRAY_BUFFER, points, gl.STREAM_DRAW);
        gl.bindTexture(gl.TEXTURE_2D, texture);
        gl.drawArraysInstanced(gl.TRIANGLES, 0, GLYPH_VERTICES, points.length / 2);
      }
    }
  }

  /**
   * Draws a frame and returns the range of speeds of each kind across the
   * tiles it drew, in m/s — null for a kind none held. What the auto scale
   * (spec.md 5.3) sets the next frame's ramps from, one per kind: wind and
   * current share no scale.
   */
  render(state: RenderState): SeenRanges {
    const gl = this.gl;
    const offsets = this.worldOffsets(state);
    this.seen = { wind: null, current: null };
    const lod = this.lodFor(state.camera.pxPerDeg);

    gl.viewport(0, 0, state.view.width, state.view.height);
    gl.clearColor(...SEA);
    gl.clear(gl.COLOR_BUFFER_BIT);

    // The gesture in progress, if it operates on the field rather than adding
    // one of its own (spec.md 6.1). Uploading it is what makes a mask cover
    // and a clone clone *while the pointer is down*, rather than at the commit
    // a round trip later.
    const operating = this.uploadMask(state.operator?.mask ?? null);
    const operator = operating ? (state.operator ?? null) : null;
    // What the passes over the field do inside the gesture (M32): take it
    // away for a mask, an eraser, a clone or a liquify, whose replacement
    // is drawn afterwards; change it in place for a modifier.
    let stage: OpStage =
      operator === null
        ? "none"
        : operator.kind === "gain" || operator.kind === "turn" || operator.kind === "radial"
          ? "apply"
          : "remove";
    // A clone draws the field a second time, read through a camera shifted so
    // the source lands where the brush is, and kept only where the gesture
    // covers (spec.md 5.1). A warp's push is the same shift, from the start
    // of the drag to its end.
    const source = operator?.kind === "clone" ? (operator.source ?? null) : null;
    // A liquify reads the field from where it moved it from, which varies
    // from pixel to pixel: the field is drawn alone into a texture first and
    // read back displaced.
    const smearing = operator?.kind === "smear";
    // The stack without the layer being erased. Only meaningful while a
    // remove is live, and only when it is a frame of its own: with no layer
    // named it is the same tiles, and drawing them into the hole would put
    // back exactly what was taken out.
    const below =
      operator !== null && state.belowFrame && state.belowFrame !== state.frame
        ? state.belowFrame
        : null;

    // Which tiles each camera sees, once per frame rather than once per pass:
    // the raster and the glyphs walk the same set.
    const tiles = visibleTiles(state.camera, state.view);
    const sourceTiles = source ? visibleTiles(source, state.view) : [];
    this.sourceCoverageReady = false;
    // Avoid sampling the render target while drawing into it. The mask is a
    // complete fallback texture even when this sampler is disabled.
    gl.activeTexture(gl.TEXTURE0 + SOURCE_COVERAGE_UNIT);
    gl.bindTexture(gl.TEXTURE_2D, this.maskTexture);

    // --- Land and coastlines ---
    gl.useProgram(this.geoProgram);
    const land = this.landByLod.get(lod);
    const coast = this.coastByLod.get(lod);
    if (land) {
      gl.bindVertexArray(land.vao);
      gl.uniform4f(this.geoUniforms.uColor ?? null, ...LAND);
      for (const offset of offsets) {
        this.setShared(this.geoUniforms, state.camera, state.view, offset);
        gl.drawElements(gl.TRIANGLES, land.indexCount, gl.UNSIGNED_INT, 0);
      }
    }

    // --- Image layers (spec.md 4.9, M18) ---
    // Above the land and below the field: an image is a reference to trace or
    // compare against, so the coastline under it stays visible and the field
    // being painted stays on top. Never masked, for the same reason the
    // basemap is not — a mask takes the field away, not what is beneath it.
    const images = state.images ?? [];
    this.drawImages(state, offsets, images.filter((image) => !image.over));

    // A clone removes destination pixels only where its source contains data.
    // This target holds source coverage alone: no basemap, colors, or glyphs.
    if (source && operator?.transparentSource) {
      const target = this.ensureFieldTarget(state.view);
      if (target && this.fieldTarget) {
        gl.bindFramebuffer(gl.FRAMEBUFFER, target);
        gl.clearColor(0, 0, 0, 0);
        gl.clear(gl.COLOR_BUFFER_BIT);
        gl.disable(gl.BLEND);
        this.capturingCoverage = true;
        if (state.sourceFrame) this.drawRaster(state, source, sourceTiles, "none", state.sourceFrame);
        this.capturingCoverage = false;
        gl.enable(gl.BLEND);
        gl.bindFramebuffer(gl.FRAMEBUFFER, null);
        gl.activeTexture(gl.TEXTURE0 + SOURCE_COVERAGE_UNIT);
        gl.bindTexture(gl.TEXTURE_2D, this.fieldTarget.texture);
        this.sourceCoverageReady = true;
      }
      // If the temporary target cannot be allocated, retain the field until
      // the committed preview arrives instead of cutting an unfilled hole.
      if (!this.sourceCoverageReady) stage = "none";
    }

    // --- The field alone, for a liquify to read back ---
    if (smearing) {
      const target = this.ensureFieldTarget(state.view);
      if (target) {
        gl.bindFramebuffer(gl.FRAMEBUFFER, target);
        gl.clearColor(0, 0, 0, 0);
        gl.clear(gl.COLOR_BUFFER_BIT);
        this.drawRaster(state, state.camera, tiles, "none");
        gl.bindFramebuffer(gl.FRAMEBUFFER, null);
        gl.viewport(0, 0, state.view.width, state.view.height);
      }
    }

    // --- Speed raster ---
    // The basemap is never masked: a mask takes away the field, not the
    // coastline underneath it.
    // The main pass carries the scope, so every operator — a remove, a
    // modifier, a clone's hole — applies only where the pixel is showing the
    // layer being edited (M44).
    this.drawRaster(state, state.camera, tiles, stage, undefined, below ?? undefined);
    // What the erase leaves behind, drawn into the hole the remove opened
    // (M40). An eraser takes one layer away and the mask acts on the whole
    // composite, so without this a stroke on a lower layer blanked every
    // layer above it until the button came up. Unscoped, because where the
    // edited layer is not on top the two frames hold the same texel and
    // drawing one over the other changes nothing.
    if (stage === "remove" && below) this.drawRaster(state, state.camera, tiles, "keep", below);
    // The source is the edited layer alone (M45), not the whole stack: that
    // is what the commit samples. Unscoped, because the source pass draws
    // only where the gesture covers and there is nothing there to compare.
    if (source && (!operator?.transparentSource || (state.sourceFrame && this.sourceCoverageReady))) {
      this.drawRaster(state, source, sourceTiles, "source", state.sourceFrame ?? undefined);
    }
    if (smearing && this.fieldTarget) {
      gl.useProgram(this.smearProgram);
      gl.bindVertexArray(this.quadVao);
      this.setOperator(this.smearUniforms, state.view, operator, "apply");
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, this.fieldTarget.texture);
      gl.uniform1i(this.smearUniforms.uField ?? null, 0);
      gl.drawArrays(gl.TRIANGLES, 0, 6);
    }

    // --- Images above the field (M49) ---
    // An image is a reference to trace against and belongs under the field,
    // but a layer moved above every field layer is one the user has asked to
    // see: the field is nearly opaque, so leaving it underneath would make it
    // vanish. The coastlines and the glyphs still draw over it, as they draw
    // over everything.
    this.drawImages(state, offsets, images.filter((image) => image.over));

    // --- Coastlines, above the raster ---
    // The field covers land as well as sea, so a coastline drawn underneath it
    // is almost invisible. Drawn here it stays legible at any wind speed.
    if (coast) {
      gl.useProgram(this.geoProgram);
      gl.bindVertexArray(coast.vao);
      gl.uniform4f(this.geoUniforms.uColor ?? null, ...COAST);
      for (const offset of offsets) {
        this.setShared(this.geoUniforms, state.camera, state.view, offset);
        gl.drawElements(gl.LINES, coast.indexCount, gl.UNSIGNED_INT, 0);
      }
    }

    // --- Graticule ---
    if (state.showGraticule) {
      this.ensureGraticule(state);
      if (this.graticule) {
        gl.useProgram(this.geoProgram);
        gl.bindVertexArray(this.graticule.vao);
        gl.uniform4f(this.geoUniforms.uColor ?? null, ...GRATICULE);
        for (const offset of offsets) {
          this.setShared(this.geoUniforms, state.camera, state.view, offset);
          gl.drawArrays(gl.LINES, 0, this.graticule.count);
        }
      }
    }

    // --- Glyphs ---
    // A liquify's glyphs read the field from where it was moved from, so
    // they are the apply stage rather than the remove one.
    if (state.showGlyphs) {
      this.drawGlyphs(
        state,
        state.camera,
        tiles,
        smearing ? "apply" : stage,
        undefined,
        below ?? undefined,
      );
      if (stage === "remove" && below) {
        this.drawGlyphs(state, state.camera, tiles, "keep", below);
      }
      if (source && (!operator?.transparentSource || (state.sourceFrame && this.sourceCoverageReady))) {
        this.drawGlyphs(state, source, sourceTiles, "source", state.sourceFrame ?? undefined);
      }
    }

    gl.bindVertexArray(null);

    // Read back within the same frame; this avoids preserveDrawingBuffer,
    // which would slow every frame for the sake of an occasional capture.
    if (this.captureRequest) {
      const resolve = this.captureRequest;
      this.captureRequest = null;
      resolve(this.readPixels(state.view));
    }
    return this.seen;
  }

  private readPixels(view: Viewport): ImageData | null {
    const gl = this.gl;
    const width = Math.floor(view.width);
    const height = Math.floor(view.height);
    if (width <= 0 || height <= 0) return null;

    const pixels = new Uint8Array(width * height * 4);
    gl.readPixels(0, 0, width, height, gl.RGBA, gl.UNSIGNED_BYTE, pixels);

    // GL origin is bottom-left; ImageData is top-left.
    const flipped = new Uint8ClampedArray(pixels.length);
    const stride = width * 4;
    for (let row = 0; row < height; row++) {
      const from = (height - 1 - row) * stride;
      flipped.set(pixels.subarray(from, from + stride), row * stride);
    }
    return new ImageData(flipped, width, height);
  }

  /** Captures the next rendered frame. Used by the dev capture path. */
  captureNextFrame(): Promise<ImageData | null> {
    return new Promise((resolve) => {
      this.captureRequest = resolve;
    });
  }

  dispose(): void {
    const gl = this.gl;
    gl.deleteProgram(this.smearProgram);
    if (this.fieldTarget) {
      gl.deleteFramebuffer(this.fieldTarget.framebuffer);
      gl.deleteTexture(this.fieldTarget.texture);
      this.fieldTarget = null;
    }
    for (const buffers of [...this.landByLod.values(), ...this.coastByLod.values()]) {
      gl.deleteVertexArray(buffers.vao);
    }
    gl.deleteVertexArray(this.imageMesh.vao);
    if (this.graticule) {
      gl.deleteVertexArray(this.graticule.vao);
      gl.deleteBuffer(this.graticule.buffer);
    }
    gl.deleteVertexArray(this.quadVao);
    gl.deleteVertexArray(this.glyphVao);
    gl.deleteBuffer(this.glyphBuffer);
    gl.deleteProgram(this.geoProgram);
    gl.deleteProgram(this.rasterProgram);
    gl.deleteProgram(this.glyphProgram);
  }
}

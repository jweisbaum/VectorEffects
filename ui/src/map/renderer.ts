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
  type PlaneMesh,
  type Viewport,
  glyphLattice,
  meshPlacement,
  normalizeLon,
  projectedMesh,
  projectionFor,
  project,
  unproject,
  validGeo,
  tileColumns,
  tileRows,
  tileBounds,
  visibleBounds,
  visibleTiles,
} from "./camera";
import { destination } from "./geo";
import { ProjectedSurface } from "./projections/surface";
import type { BackdropDraw } from "./backdrops";
import { projectedOnGpu, shaderMode } from "./projection";
import type { Basemap } from "./format";
import { KINDS, type FieldKindName } from "../kind";
import { DEFAULT_GLYPHS, glyphDisplayLayout, glyphRgb } from "./glyphAppearance";
import type { GlyphSettings } from "../generated/GlyphSettings";
import type { GlyphStyle } from "../generated/GlyphStyle";
import { GLYPH_SUBDIVISIONS, PlacedGlyphs, glyphCovered, glyphTileIsFull, glyphTileIsEmpty, rankedSites, spacedGlyphs } from "./glyphPlacement";
import { SPEED_MAX } from "./tileRange";
import {
  BACKDROP_FRAG,
  BASE_FRAG,
  BASE_VERT,
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

/**
 * Cells a side of the grid a globe's tiles are drawn through.
 *
 * On a globe the vertex shader projects each tile's own lon/lat grid, and
 * what lies between the vertices is interpolated straight across the screen.
 * A tile is chosen to be a few hundred pixels wide, so at thirty-two cells a
 * side a cell is ten or so, and the bow of the sphere across one is a few
 * hundredths of a pixel. Six thousand vertices a tile is nothing to a GPU,
 * and unlike the mesh this replaced none of it is made per frame.
 */
const GLOBE_CELLS = 32;

/** Degrees between the vertices of a globe's graticule lines. */
const GRATICULE_PIECE_DEG = 0.5;

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
  /**
   * The warp evaluated at every mesh vertex, lon/lat interleaved
   * (`ImageLayerView.warp_mesh`), or null for a plain affine image. Compared
   * by identity, not content, to decide whether to re-upload (spec.md 4.9):
   * the mesh is rebuilt in Rust only when the control points change, and
   * uploading it again every frame is exactly the cost that design avoids.
   */
  warpMesh: Float32Array | null;
  /** The cell count `warpMesh` was evaluated at. Unused when `warpMesh` is null. */
  warpCells: number;
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
  /**
   * What the map draws *under* everything (spec.md 4.11): a chart, the map
   * tiles, a GIS layer. Pictures, in the order given; none of it reaches an
   * evaluation, a cache key or an export.
   */
  backdrops?: readonly BackdropDraw[];
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

/**
 * The GL geometry built for one warped image layer (spec.md 4.9).
 *
 * One vertex per mesh point, not per triangle corner: `aGeo` is uploaded
 * from the backend's own array (`geoBuffer`), unmodified and unreordered —
 * that is what "the mesh is passed through untouched" (spec.md 4.9) means at
 * the GPU boundary, and `cellScreenBuffer`'s `aCell`/`aScreen` are built to
 * match its vertex order rather than the other way round. `indexBuffer`
 * supplies the two triangles a cell is drawn as, so nothing is duplicated to
 * make a triangle list.
 *
 * `source` and `screenFor` are what a rebuild is keyed on: the backend's
 * mesh array by identity (rebuilt only when the control points change, never
 * per frame) and, for a fixed general projection only, the plane mesh
 * `aScreen` was forward-projected against (rebuilt only when panning or
 * zooming moves to a different zoom band, via `projectedMesh`'s own
 * memoisation — never per frame either). `screenFor` stays null off a fixed
 * general projection, so nothing here is rebuilt for a pan or a globe turn.
 */
interface WarpMesh {
  vao: WebGLVertexArrayObject;
  cellScreenBuffer: WebGLBuffer;
  geoBuffer: WebGLBuffer;
  indexBuffer: WebGLBuffer;
  /** Indices to draw, for `drawElements` — not a vertex count. */
  indexCount: number;
  source: Float32Array;
  screenFor: PlaneMesh | null;
}

export class MapRenderer {
  private readonly gl: WebGL2RenderingContext;
  private readonly projectedSurface: ProjectedSurface;
  private readonly tiles: TileCache;

  private readonly imageProgram: WebGLProgram;
  private readonly imageUniforms: Uniforms;
  private readonly imageMesh: { vao: WebGLVertexArrayObject; count: number };
  /** The unit grid a globe's tiles, base map and images are drawn through. */
  private readonly globeGrid: { vao: WebGLVertexArrayObject; count: number };
  /** A warped image's own mesh, keyed by layer (spec.md 4.9). */
  private readonly warpMeshes = new Map<number, WarpMesh>();
  private readonly baseProgram: WebGLProgram;
  private readonly baseUniforms: Uniforms;
  private readonly backdropProgram: WebGLProgram;
  private readonly backdropUniforms: Uniforms;
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
    this.baseProgram = link(gl, BASE_VERT, BASE_FRAG);

    const shared = ["uCamera", "uViewport", "uLonOffset", "uProjection", "uOrigin", "uRim", "uExact", "uMesh"];
    this.baseUniforms = uniforms(gl, this.baseProgram, [...shared, "uTileGeo", "uTexture"]);
    this.backdropProgram = link(gl, RASTER_VERT, BACKDROP_FRAG);
    this.backdropUniforms = uniforms(gl, this.backdropProgram, [
      ...shared,
      "uTileGeo",
      "uTile",
      "uOpacity",
    ]);
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
      "uFiltered", "uSpeedRange", "uFilterTile", "uFilterGeo", "uSpeedScale", "uWarped",
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

    this.projectedSurface = new ProjectedSurface(gl);
    this.quadVao = this.buildQuad();
    this.imageMesh = this.buildImageMesh(IMAGE_CELLS);
    this.globeGrid = this.buildImageMesh(GLOBE_CELLS);
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
  private buildImageMesh(cellsASide: number): { vao: WebGLVertexArrayObject; count: number } {
    const gl = this.gl;
    const vao = gl.createVertexArray();
    const vbo = gl.createBuffer();
    if (!vao || !vbo) throw new Error("could not allocate image mesh buffers");

    const step = 1 / cellsASide;
    const cells: number[] = [];
    for (let row = 0; row < cellsASide; row++) {
      for (let col = 0; col < cellsASide; col++) {
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

  /**
   * The mesh a warped image is drawn through (spec.md 4.9): `aCell` and
   * `aScreen` built here, one vertex per mesh point, and `aGeo` uploaded
   * straight from `source` — the backend's own array, byte for byte, never
   * reordered or recomputed. That is what lets the mesh work in every
   * projection: the vertex shader projects `aGeo` after the fact, and
   * nothing here has touched it first.
   *
   * `aScreen` is the one exception `aGeo` alone cannot answer. A fixed
   * general projection (`uProjection == 14`, Robinson and the other
   * 270-odd presets) has no closed-form forward transform in GLSL — that is
   * why an *unwarped* image under one is drawn through
   * `ProjectedSurface.bindImage`, cut from a mesh built by forward-projecting
   * on the CPU. A warp cannot use that cut (it clips by inverting the
   * placement, and a spline has no inverse), so instead every warp-mesh
   * vertex is forward-projected here directly, through the same plane mesh
   * (`projectedMesh`) the rest of the general-projection drawing already
   * uses. Off a fixed general projection `aScreen` is never read by the
   * shader, so it is left zero there and no projection is done for it.
   *
   * Rebuilt only when `source`'s identity changes (the warp was refitted, in
   * Rust, not here) or when the plane mesh changes under a fixed general
   * projection — never per frame, and never for a pan, a zoom or a turn of
   * the globe.
   */
  private warpMeshFor(
    layer: number,
    source: Float32Array,
    cells: number,
    camera: Camera,
    view: Viewport,
  ): WarpMesh {
    const gl = this.gl;
    const general = projectionFor(camera).general;
    const fixedGeneral = general && !general.movable;
    const screenFor = fixedGeneral ? projectedMesh(camera, view) : null;
    const existing = this.warpMeshes.get(layer);
    if (existing && existing.source === source && existing.screenFor === screenFor) {
      return existing;
    }
    if (existing) this.dropWarpMesh(layer);

    const perSide = Math.max(1, cells);
    const perRow = perSide + 1;
    const screenOf = (lon: number, lat: number): [number, number] => {
      if (!screenFor) return [0, 0];
      const xy = screenFor.toVirtual({ lon, lat });
      return xy ? [xy.x, xy.y] : [0, 0];
    };
    // One vertex per mesh point, in the same row-major order `source` is in,
    // so its index into `cellScreen` is `aGeo`'s index into `source`.
    const cellScreen: number[] = [];
    for (let row = 0; row < perRow; row++) {
      for (let col = 0; col < perRow; col++) {
        const i = (row * perRow + col) * 2;
        const [sx, sy] = screenOf(source[i] ?? 0, source[i + 1] ?? 0);
        cellScreen.push(col / perSide, row / perSide, sx, sy);
      }
    }
    // Two triangles a cell, indexing the shared corners rather than
    // repeating them — the same winding as buildImageMesh above.
    const indices: number[] = [];
    for (let row = 0; row < perSide; row++) {
      for (let col = 0; col < perSide; col++) {
        const tl = row * perRow + col;
        const tr = tl + 1;
        const bl = tl + perRow;
        const br = bl + 1;
        indices.push(tl, tr, bl, bl, tr, br);
      }
    }

    const vao = gl.createVertexArray();
    const cellScreenBuffer = gl.createBuffer();
    const geoBuffer = gl.createBuffer();
    const indexBuffer = gl.createBuffer();
    if (!vao || !cellScreenBuffer || !geoBuffer || !indexBuffer) {
      throw new Error("could not allocate a warped image's mesh buffers");
    }
    gl.bindVertexArray(vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, cellScreenBuffer);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array(cellScreen), gl.DYNAMIC_DRAW);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 16, 0);
    gl.enableVertexAttribArray(1);
    gl.vertexAttribPointer(1, 2, gl.FLOAT, false, 16, 8);
    gl.bindBuffer(gl.ARRAY_BUFFER, geoBuffer);
    // `source` itself, untouched: see the doc comment above.
    gl.bufferData(gl.ARRAY_BUFFER, source, gl.DYNAMIC_DRAW);
    gl.enableVertexAttribArray(2);
    gl.vertexAttribPointer(2, 2, gl.FLOAT, false, 0, 0);
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, indexBuffer);
    gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, new Uint32Array(indices), gl.DYNAMIC_DRAW);
    gl.bindVertexArray(null);

    const entry: WarpMesh = {
      vao, cellScreenBuffer, geoBuffer, indexBuffer,
      indexCount: indices.length, source, screenFor,
    };
    this.warpMeshes.set(layer, entry);
    return entry;
  }

  /** Drops a warped image's GL geometry: its layer was deleted or unwarped. */
  private dropWarpMesh(layer: number): void {
    const existing = this.warpMeshes.get(layer);
    if (!existing) return;
    const gl = this.gl;
    gl.deleteBuffer(existing.cellScreenBuffer);
    gl.deleteBuffer(existing.geoBuffer);
    gl.deleteBuffer(existing.indexBuffer);
    gl.deleteVertexArray(existing.vao);
    this.warpMeshes.delete(layer);
  }

  /** World copies to draw so the map wraps seamlessly at the dateline. */
  private worldOffsets(state: RenderState): number[] {
    if (projectionFor(state.camera).general) return [0];
    const bounds = visibleBounds(state.camera, state.view, true);
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
    rimDeg = 0,
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
    gl.uniform1i(u.uProjection ?? null, shaderMode(projection));
    // A globe's centre, for the shader that projects it (projectionShaders.ts).
    // The sine and cosine are taken here, in double precision, once a draw.
    const lat0 = (camera.centerLat * Math.PI) / 180;
    gl.uniform3f(u.uOrigin ?? null, camera.centerLat, Math.sin(lat0), Math.cos(lat0));
    gl.uniform1f(u.uRim ?? null, (rimDeg * Math.PI) / 180);
    // Where a fixed general projection's plane mesh lies on the screen. The
    // glyphs' positions are screen pixels already and are not run through it.
    if (projection.general && !projection.general.movable) {
      const place = meshPlacement(camera, view);
      gl.uniform3f(u.uMesh ?? null, place.x, place.y, place.scale);
    }
  }

  /**
   * Whether a tile is drawn as the viewport, its place found per pixel,
   * rather than through the grid (see `uExact` in shaders.ts).
   *
   * Only the two maps that show the antipode need it, and only for the tiles
   * within half their own width of it: beside the antipode a cell is a wedge
   * of the rim, which straight edges between its corners do not follow. That
   * is two tiles at a world zoom and a handful at any other, so the cost is
   * a few viewports of a cheap inverse, and only while the rim is in view.
   */
  private drawnExactly(camera: Camera, tile: VisibleTile): boolean {
    const movable = projectionFor(camera).general?.movable;
    if (movable !== "aeqd" && movable !== "laea") return false;
    const b = tileBounds(tile.z, tile.x, tile.y);
    const antipode = { lon: normalizeLon(camera.centerLon + 180), lat: -camera.centerLat };
    // The tile's nearest place to it: the antipode itself, held to the tile.
    const lat = Math.min(Math.max(antipode.lat, b.south), b.north);
    const middle = (b.west + b.east) / 2;
    const half = (b.east - b.west) / 2;
    const lon = middle + Math.min(Math.max(normalizeLon(antipode.lon - middle), -half), half);
    const rad = Math.PI / 180;
    const cos =
      Math.sin(lat * rad) * Math.sin(antipode.lat * rad) +
      Math.cos(lat * rad) * Math.cos(antipode.lat * rad) * Math.cos((lon - antipode.lon) * rad);
    const away = Math.acos(Math.min(1, Math.max(-1, cos))) / rad;
    return away < (b.east - b.west) / 2;
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
    const onGpu = projectedOnGpu(projectionFor(camera));

    for (const tile of tiles) {
      const shown = this.textureFor(state, tile, frame);
      if (!shown.texture) continue;
      const { texture, held } = shown;
      this.noteRange(shown.frame, tile);
      this.bindScope(this.rasterUniforms, scope, tile, texture, state.belowHeldFrame);
      gl.activeTexture(gl.TEXTURE0);
      const b = tileBounds(tile.z, tile.x, tile.y);
      this.setShared(this.rasterUniforms, camera, state.view, tile.lonOffset);
      const exact = onGpu && this.drawnExactly(camera, tile);
      gl.uniform1i(this.rasterUniforms.uExact ?? null, exact ? 1 : 0);
      gl.uniform1f(this.rasterUniforms.uDim ?? null, held ? HELD_DIM : 1.0);
      gl.uniform4f(
        this.rasterUniforms.uTileGeo ?? null,
        b.west, b.north, b.east - b.west, b.north - b.south,
      );
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.drawArrays(gl.TRIANGLES, 0, this.bindTileGeometry(camera, state.view, tile, exact, onGpu));
    }
  }

  /**
   * Binds the geometry one tile is drawn through, and says how many
   * vertices that is.
   *
   * The three projection families, in one place: a flat map draws a tile as
   * two triangles, a globe through the static grid its vertex shader
   * projects, and a fixed general projection through its plane mesh. Every
   * pass over a tile — the field, a backdrop — goes through here, or one of
   * them would draw the world in a projection the others are not in.
   */
  private bindTileGeometry(
    camera: Camera,
    view: Viewport,
    tile: VisibleTile,
    exact: boolean,
    onGpu: boolean,
  ): number {
    const gl = this.gl;
    if (exact) {
      gl.bindVertexArray(this.quadVao);
      return 6;
    }
    if (onGpu) {
      gl.bindVertexArray(this.globeGrid.vao);
      return this.globeGrid.count;
    }
    if (projectionFor(camera).general) {
      return this.projectedSurface.bindTile(camera, view, tile);
    }
    gl.bindVertexArray(this.quadVao);
    return 6;
  }

  /**
   * Draws the backdrops, under everything (spec.md 4.11).
   *
   * Straight textures on the field's own tile grid, so they follow the
   * projection without knowing what one is. Nothing here reads or writes
   * the field: a backdrop is a picture under the map.
   */
  private drawBackdrops(state: RenderState, backdrops: readonly BackdropDraw[]): void {
    if (backdrops.length === 0) return;
    const gl = this.gl;
    const camera = state.camera;
    const onGpu = projectedOnGpu(projectionFor(camera));
    gl.useProgram(this.backdropProgram);
    gl.uniform1i(this.backdropUniforms.uTile ?? null, 0);
    gl.activeTexture(gl.TEXTURE0);
    for (const backdrop of backdrops) {
      gl.uniform1f(this.backdropUniforms.uOpacity ?? null, backdrop.opacity);
      for (const { tile, texture } of backdrop.textures) {
        if (!texture) continue;
        const b = tileBounds(tile.z, tile.x, tile.y);
        this.setShared(this.backdropUniforms, camera, state.view, tile.lonOffset);
        const exact = onGpu && this.drawnExactly(camera, tile);
        gl.uniform1i(this.backdropUniforms.uExact ?? null, exact ? 1 : 0);
        gl.uniform4f(
          this.backdropUniforms.uTileGeo ?? null,
          b.west, b.north, b.east - b.west, b.north - b.south,
        );
        gl.bindTexture(gl.TEXTURE_2D, texture);
        gl.drawArrays(
          gl.TRIANGLES,
          0,
          this.bindTileGeometry(camera, state.view, tile, exact, onGpu),
        );
      }
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
    const onGpu = projectedOnGpu(projectionFor(state.camera));
    for (const image of images) {
      let count = this.imageMesh.count;
      // A warped mesh is indexed (WarpMesh above), so it draws through
      // drawElements; every other image draws through drawArrays.
      let indexed = false;
      // A warped image is drawn through its own mesh in every projection —
      // chosen by "has control points", not by size (spec.md 4.9) — because
      // neither the globe's per-pixel inverse nor the general projection's
      // affine-clipped cut can invert a spline. It therefore takes neither of
      // the two branches below, on the globe or on a fixed general projection.
      const warped = image.warpMesh !== null && image.warpMesh.length > 0;
      if (warped) {
        const mesh = this.warpMeshFor(image.layer, image.warpMesh!, image.warpCells, state.camera, state.view);
        gl.bindVertexArray(mesh.vao);
        count = mesh.indexCount;
        indexed = true;
      } else if (onGpu) {
        // The viewport, as two triangles: IMAGE_FRAG finds the image itself.
        gl.bindVertexArray(this.quadVao);
        count = 6;
      } else if (projectionFor(state.camera).general) {
        count = this.projectedSurface.bindImage(state.camera, state.view, image);
      } else {
        gl.bindVertexArray(this.imageMesh.vao);
      }
      gl.uniform1i(this.imageUniforms.uWarped ?? null, warped ? 1 : 0);
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
            if (indexed) gl.drawElements(gl.TRIANGLES, count, gl.UNSIGNED_INT, 0);
            else gl.drawArrays(gl.TRIANGLES, 0, count);
          }
        }
        continue;
      }
      for (const offset of offsets) {
        this.setShared(this.imageUniforms, state.camera, state.view, offset);
        if (indexed) gl.drawElements(gl.TRIANGLES, count, gl.UNSIGNED_INT, 0);
        else gl.drawArrays(gl.TRIANGLES, 0, count);
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
    let chosen = 30;
    for (const step of [30, 15, 10, 5, 2, 1, 0.5, 0.25, 0.1]) {
      if (step * pxPerDeg >= 70) chosen = step;
    }
    return chosen;
  }

  private ensureGraticule(state: RenderState, pieceDeg = 0): void {
    const gl = this.gl;
    const step = this.graticuleInterval(state.camera.pxPerDeg);
    const key = `${step}/${pieceDeg}`;
    if (this.graticuleKey === key && this.graticule) return;

    // A line is one piece on a flat map, where it is straight. On a globe it
    // is cut into `pieceDeg` lengths, each projected at both ends.
    const points: number[] = [];
    const line = (lon0: number, lat0: number, lon1: number, lat1: number) => {
      const pieces = pieceDeg > 0 ? Math.ceil(Math.max(lon1 - lon0, lat1 - lat0) / pieceDeg) : 1;
      for (let i = 0; i < pieces; i++) {
        const a = i / pieces;
        const b = (i + 1) / pieces;
        points.push(
          lon0 + (lon1 - lon0) * a, lat0 + (lat1 - lat0) * a,
          lon0 + (lon1 - lon0) * b, lat0 + (lat1 - lat0) * b,
        );
      }
    };
    for (let lon = -180; lon <= 180; lon += step) line(lon, -90, lon, 90);
    for (let lat = -90; lat <= 90; lat += step) line(-180, lat, 180, lat);

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

  /**
   * Land or coast on a globe: the cached source tiles, through the static
   * grid and the vertex shader's own projection. Nothing here is made per
   * frame but the uniforms.
   */
  private drawGlobeBase(
    state: RenderState,
    tiles: readonly VisibleTile[],
    kind: "land" | "coast",
    lod: number,
    draw: (camera: Camera, view: Viewport, kind: "land" | "coast") => void,
  ): void {
    const gl = this.gl;
    for (const tile of tiles) {
      // May render the tile, which leaves another program and target bound.
      const texture = this.projectedSurface.baseTexture(state.view, tile, kind, lod, draw);
      const exact = this.drawnExactly(state.camera, tile);
      gl.useProgram(this.baseProgram);
      gl.bindVertexArray(exact ? this.quadVao : this.globeGrid.vao);
      this.setShared(this.baseUniforms, state.camera, state.view, 0);
      gl.uniform1i(this.baseUniforms.uExact ?? null, exact ? 1 : 0);
      const b = tileBounds(tile.z, tile.x, tile.y);
      gl.uniform4f(this.baseUniforms.uTileGeo ?? null, b.west, b.north, b.east - b.west, b.north - b.south);
      gl.uniform1i(this.baseUniforms.uTexture ?? null, 0);
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.drawArrays(gl.TRIANGLES, 0, exact ? 6 : this.globeGrid.count);
    }
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

    if (projectionFor(camera).general) {
      this.drawGeneralGlyphs(state, camera, tiles, stage, frame, scope);
      return;
    }
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

  /** A screen lattice gives globe and regional maps evenly spaced glyphs.
   * Each station still samples a geographic tile and uses a true east/north
   * differential, so grid convergence never changes the stored direction. */
  private drawGeneralGlyphs(
    state: RenderState, camera: Camera, tiles: readonly VisibleTile[], stage: OpStage,
    frame?: string, scope?: string,
  ): void {
    if (!tiles.length) return;
    const gl=this.gl, glyphs=state.glyphs??DEFAULT_GLYPHS;
    const batches=tiles.map(tile=>({tile,...this.textureFor(state,tile,frame),data:[] as number[]}));
    const lookup=new Map(batches.map((batch,index)=>[`${batch.tile.x}/${batch.tile.y}`,index]));
    const z=tiles[0]!.z, columns=tileColumns(z), rows=tileRows(z);
    // No gap is wider than the widest style's, which is what the bins need.
    const reach=Math.max(...(["arrow","barb"] as const).map(style=>glyphDisplayLayout(style,camera.pxPerDeg,state.pixelRatio,glyphs[style]).spacing*0.8));
    const occupied=new PlacedGlyphs(reach);
    for (const style of ["arrow","barb"] as const) {
      if (glyphs[style].opacity_percent<=0) continue;
      const layout=glyphDisplayLayout(style,camera.pxPerDeg,state.pixelRatio,glyphs[style]);
      const masks=batches.map(batch=>({coverage:this.tiles.coverageOf(batch.frame,z,batch.tile.x,batch.tile.y),wind:this.tiles.windCoverageOf(batch.frame,z,batch.tile.x,batch.tile.y)}));
      if(masks.every(({coverage,wind})=>!coverage||!wind||(style==="barb"?glyphTileIsEmpty(wind):glyphTileIsFull(wind))))continue;
      const solid=masks.every(({coverage,wind})=>coverage&&wind&&(style==="barb"?glyphTileIsFull(wind):glyphTileIsFull(coverage)&&glyphTileIsEmpty(wind)));
      const fine=layout.spacing/(solid?1:4),margin=layout.lengthPx;
      for (const site of rankedSites(state.view.width,state.view.height,fine,margin)) {
        const gap=layout.spacing*0.8;
        if(occupied.crowds(site.x,site.y,gap))continue;
        const geo=unproject(camera,state.view,site);if(!validGeo(geo))continue;
        let sample=geo;
        const operator=state.operator;
        if(stage==="apply"&&operator?.kind==="smear") {
          let dx=0,dy=0;const radius=operator.radiusPx??0,band=(operator.feather??0)*radius;
          for(let i=0;i<(operator.points?.length??0);i++){
            const p=operator.points![i]!,delta=operator.deltas?.[i];if(!delta)continue;
            const inside=radius-Math.hypot(site.x-p[0],state.view.height-site.y-p[1]);if(inside<0)continue;
            const t=band>0?Math.min(1,inside/band):1,w=t*t*(3-2*t);dx+=delta[0]*w;dy+=delta[1]*w;
          }
          sample=unproject(camera,state.view,{x:site.x-dx,y:site.y+dy});if(!validGeo(sample))continue;
        }
        const col=Math.min(columns-1,Math.floor((sample.lon+180)/360*columns));
        const row=Math.min(rows-1,Math.floor((90-sample.lat)/180*rows));
        const index=lookup.get(`${col}/${row}`);if(index===undefined)continue;
        const batch=batches[index]!;if(!batch.texture)continue;
        const bounds=tileBounds(z,col,row),u=(sample.lon-bounds.west)/(bounds.east-bounds.west),v=(bounds.north-sample.lat)/(bounds.north-bounds.south);
        const coverage=this.tiles.coverageOf(batch.frame,z,col,row),wind=this.tiles.windCoverageOf(batch.frame,z,col,row);
        if(!coverage||!wind||!glyphCovered(coverage,u,v)||glyphCovered(wind,u,v)!==(style==="barb"))continue;
        const east=project(camera,state.view,destination(geo,90,100));
        const north=project(camera,state.view,destination(geo,0,100));
        if(!Number.isFinite(east.x)||!Number.isFinite(north.x))continue;
        occupied.place(site.x,site.y,gap);
        batch.data.push(sample.lon,sample.lat,site.x,site.y,east.x-site.x,east.y-site.y,north.x-site.x,north.y-site.y);
      }
    }
    gl.bindVertexArray(this.glyphVao);gl.bindBuffer(gl.ARRAY_BUFFER,this.glyphBuffer);
    gl.vertexAttribPointer(0,2,gl.FLOAT,false,32,0);
    gl.enableVertexAttribArray(1);gl.vertexAttribPointer(1,2,gl.FLOAT,false,32,8);gl.vertexAttribDivisor(1,1);
    gl.enableVertexAttribArray(2);gl.vertexAttribPointer(2,4,gl.FLOAT,false,32,16);gl.vertexAttribDivisor(2,1);
    const shadows=glyphs.arrow.shadow.enabled||glyphs.barb.shadow.enabled;
    for(const shadow of shadows?[true,false]:[false]){
      gl.uniform1i(this.glyphUniforms.uShadowPass??null,shadow?1:0);
      for(const batch of batches){
        if(!batch.texture||!batch.data.length)continue;
        this.bindScope(this.glyphUniforms,scope,batch.tile,batch.texture,state.belowHeldFrame);
        this.setShared(this.glyphUniforms,camera,state.view,0);
        const b=tileBounds(z,batch.tile.x,batch.tile.y);
        gl.uniform4f(this.glyphUniforms.uTileGeo??null,b.west,b.north,b.east-b.west,b.north-b.south);
        gl.activeTexture(gl.TEXTURE0);gl.bindTexture(gl.TEXTURE_2D,batch.texture);
        gl.bufferData(gl.ARRAY_BUFFER,new Float32Array(batch.data),gl.STREAM_DRAW);
        gl.drawArraysInstanced(gl.TRIANGLES,0,GLYPH_VERTICES,batch.data.length/8);
      }
    }
    gl.vertexAttribPointer(0,2,gl.FLOAT,false,0,0);gl.disableVertexAttribArray(1);gl.disableVertexAttribArray(2);
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
    if (projectionFor(state.camera).general) gl.clearColor(0.02, 0.03, 0.05, 1);
    else gl.clearColor(...SEA);
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

    const general = Boolean(projectionFor(state.camera).general);
    const drawGeographicTile = (camera: Camera, view: Viewport, kind: "land" | "coast") => {
      const geo = (kind === "land" ? this.landByLod : this.coastByLod).get(lod);
      if (!geo) return;
      gl.useProgram(this.geoProgram); gl.bindVertexArray(geo.vao);
      gl.uniform4f(this.geoUniforms.uColor ?? null, ...(kind === "land" ? LAND : COAST));
      // Bleed also samples the adjoining copy at the dateline.
      for (const offset of [-360, 0, 360]) {
        this.setShared(this.geoUniforms, camera, view, offset);
        gl.drawElements(kind === "land" ? gl.TRIANGLES : gl.LINES, geo.indexCount, gl.UNSIGNED_INT, 0);
      }
    };
    // The map tiles stand in for the basemap, so they are drawn first and
    // the basemap is not drawn at all. A chart and a GIS layer sit *over*
    // it: they cover the stretch of coast they were published for, and
    // hiding the world's land for one would leave black around it.
    const backdrops = state.backdrops ?? [];
    const replacing = backdrops.filter((backdrop) => backdrop.replacesBase);
    const overlaying = backdrops.filter((backdrop) => !backdrop.replacesBase);
    this.drawBackdrops(state, replacing);
    const onGpu = projectedOnGpu(projectionFor(state.camera));
    const overBase = replacing.length > 0;
    if (onGpu && !overBase) this.drawGlobeBase(state, tiles, "land", lod, drawGeographicTile);
    else if (general && !overBase) this.projectedSurface.drawBase(state.camera, state.view, tiles, "land", lod, drawGeographicTile);

    // --- Land and coastlines ---
    gl.useProgram(this.geoProgram);
    const land = this.landByLod.get(lod);
    const coast = this.coastByLod.get(lod);
    if (land && !general && !overBase) {
      gl.bindVertexArray(land.vao);
      gl.uniform4f(this.geoUniforms.uColor ?? null, ...LAND);
      for (const offset of offsets) {
        this.setShared(this.geoUniforms, state.camera, state.view, offset);
        gl.drawElements(gl.TRIANGLES, land.indexCount, gl.UNSIGNED_INT, 0);
      }
    }

    // --- Charts and GIS data (spec.md 4.11) ---
    // Over the basemap's land, which they are published to sit on, and under
    // everything of the user's: the field, the images, the glyphs. A backdrop
    // is something to paint *against*, so nothing the user made may end up
    // behind one — a chart's deep water is opaque, and drawn later it hid the
    // field entirely wherever a cell reached.
    this.drawBackdrops(state, overlaying);

    // --- Image layers (spec.md 4.9, M18) ---
    // Above the land and below the field: an image is a reference to trace or
    // compare against, so the coastline under it stays visible and the field
    // being painted stays on top. Never masked, for the same reason the
    // basemap is not — a mask takes the field away, not what is beneath it.
    const images = state.images ?? [];
    // A layer that is gone, or is no longer warped, takes its mesh with it.
    const stillWarped = new Set(
      images.filter((image) => image.warpMesh !== null).map((image) => image.layer),
    );
    for (const layer of [...this.warpMeshes.keys()]) {
      if (!stillWarped.has(layer)) this.dropWarpMesh(layer);
    }
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
    if (onGpu && !overBase) this.drawGlobeBase(state, tiles, "coast", lod, drawGeographicTile);
    else if (general && !overBase) this.projectedSurface.drawBase(state.camera, state.view, tiles, "coast", lod, drawGeographicTile);
    if (coast && !general && !overBase) {
      gl.useProgram(this.geoProgram);
      gl.bindVertexArray(coast.vao);
      gl.uniform4f(this.geoUniforms.uColor ?? null, ...COAST);
      for (const offset of offsets) {
        this.setShared(this.geoUniforms, state.camera, state.view, offset);
        gl.drawElements(gl.LINES, coast.indexCount, gl.UNSIGNED_INT, 0);
      }
    }

    // --- Graticule ---
    if (state.showGraticule && onGpu) {
      // The flat map's own lines, in pieces short enough to follow a sphere,
      // through the same vertex shader as everything else on the globe. A
      // piece that holds the antipode is a chord across the map, so the lines
      // stop a piece and a half short of it: a hair at the rim of the two
      // maps that show it, and nothing at all on the globe.
      this.ensureGraticule(state, GRATICULE_PIECE_DEG);
      if (this.graticule) {
        gl.useProgram(this.geoProgram);
        gl.bindVertexArray(this.graticule.vao);
        gl.uniform4f(this.geoUniforms.uColor ?? null, ...GRATICULE);
        this.setShared(this.geoUniforms, state.camera, state.view, 0, 1.5 * GRATICULE_PIECE_DEG);
        gl.drawArrays(gl.LINES, 0, this.graticule.count);
      }
    } else if (state.showGraticule && general) {
      gl.useProgram(this.geoProgram);
      const count = this.projectedSurface.bindGraticule(state.camera, state.view, this.graticuleInterval(state.camera.pxPerDeg));
      this.setShared(this.geoUniforms, state.camera, state.view, 0);
      gl.uniform4f(this.geoUniforms.uColor ?? null, ...GRATICULE);
      gl.drawArrays(gl.LINES, 0, count);
    } else if (state.showGraticule) {
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
    this.projectedSurface.dispose();
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
    for (const layer of [...this.warpMeshes.keys()]) this.dropWarpMesh(layer);
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

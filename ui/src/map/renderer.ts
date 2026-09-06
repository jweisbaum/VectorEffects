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
import { GLYPH_SIZE_SCALE, glyphLayout } from "./glyph";
import {
  GEO_FRAG,
  GEO_VERT,
  GLYPH_FRAG,
  GLYPH_VERT,
  IMAGE_FRAG,
  IMAGE_VERT,
  RASTER_FRAG,
  RASTER_VERT,
} from "./shaders";
import type { TileCache } from "./tiles";

/** Full-scale speed of the tile encoding. Mirrors `ve_render::tile`. */
export const SPEED_SCALE_MPS = 100.0;

/**
 * Texture unit the live-gesture mask is bound to.
 *
 * Unit 0 is the tile every pass is already reading, so the mask needs one of
 * its own rather than a bind between draws.
 */
const MASK_UNIT = 1;
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
  /** The layer, which is also the texture's key. */
  layer: number;
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
export interface OperatorPreview {
  /**
   * The gesture's coverage, in screen space and at the framebuffer's size.
   *
   * Rasterised by the same path builder the overlay draws with, so this carries
   * no notion of which tool made it or what shape it is: a new tool inherits
   * the live preview by supplying a footprint.
   */
  mask: TexImageSource;
  /** Whether the covered field is taken away, or replaced from elsewhere. */
  kind: "mask" | "clone";
  /**
   * For a clone, the camera the source is read through.
   *
   * The main camera shifted so that the source lands where the brush is; see
   * `cloneSourceCamera`, which builds it.
   */
  source?: Camera;
}

/** What to draw. */
export interface RenderState {
  camera: Camera;
  view: Viewport;
  /** Opaque token naming the field, e.g. `step-3`. */
  frame: string;
  glyphStyle: "arrow" | "barb";
  showGlyphs: boolean;
  showGraticule: boolean;
  /**
   * Speed mapped to the bottom of the colour ramp, m/s: 0 unless the auto
   * scale is on (spec.md 5.3, M27).
   */
  rampMin: number;
  /** Speed mapped to the top of the colour ramp, m/s. */
  rampMax: number;
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
   * Device pixels per CSS pixel.
   *
   * Glyph spacing and size are specified in CSS pixels and scaled by this, or
   * they come out half-size on a retina display.
   */
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
const GLYPH: [number, number, number, number] = [0.94, 0.97, 1.0, 0.9];

/** How much a tile held over from the previous frame is dimmed. */
const HELD_DIM = 0.55;

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
  private graticule: { vao: WebGLVertexArrayObject; buffer: WebGLBuffer; count: number } | null =
    null;
  private graticuleKey = "";
  /** Screen-space coverage of the gesture in progress, uploaded per frame. */
  private maskTexture: WebGLTexture | null = null;

  /** Set to have the next render read its pixels back. */
  private captureRequest: ((data: ImageData | null) => void) | null = null;

  constructor(gl: WebGL2RenderingContext, basemap: Basemap, tiles: TileCache) {
    this.gl = gl;
    this.tiles = tiles;

    this.imageProgram = link(gl, IMAGE_VERT, IMAGE_FRAG);
    this.geoProgram = link(gl, GEO_VERT, GEO_FRAG);
    this.rasterProgram = link(gl, RASTER_VERT, RASTER_FRAG);
    this.glyphProgram = link(gl, GLYPH_VERT, GLYPH_FRAG);

    const shared = ["uCamera", "uViewport", "uLonOffset", "uProjection"];
    this.geoUniforms = uniforms(gl, this.geoProgram, [...shared, "uColor"]);
    const mask = ["uMask", "uMaskSize", "uMaskMode"];
    this.rasterUniforms = uniforms(gl, this.rasterProgram, [
      ...shared, ...mask, "uTileGeo", "uTile", "uSpeedScale", "uRampMin", "uRampMax", "uDim",
    ]);
    this.imageUniforms = uniforms(gl, this.imageProgram, [
      ...shared, "uPlaceLon", "uPlaceLat", "uImage", "uOpacity",
    ]);
    this.glyphUniforms = uniforms(gl, this.glyphProgram, [
      ...shared, ...mask, "uTileGeo", "uGlyphOrigin", "uGlyphStep", "uGrid", "uSpacing",
      "uTile", "uSpeedScale", "uStyle", "uSizeScale", "uColor", "uPixelRatio",
    ]);

    for (const lod of basemap.lods) {
      this.landByLod.set(lod.marker, this.buildGeo(lod.triVertices, lod.triIndices));
      this.coastByLod.set(lod.marker, this.buildGeo(lod.lineVertices, lod.lineIndices));
    }

    this.quadVao = this.buildQuad();
    this.imageMesh = this.buildImageMesh();
    // Glyphs need no vertex data at all: geometry comes from gl_VertexID.
    const glyphVao = gl.createVertexArray();
    if (!glyphVao) throw new Error("could not create glyph vao");
    this.glyphVao = glyphVao;

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

  /**
   * Points a program's mask uniforms at the gesture in progress.
   *
   * Mode 0 leaves the field alone. Mode 1 takes it away where the gesture
   * covers — a mask, and a clone before its source is drawn in. Mode 2 keeps
   * only what the gesture covers, which is how that source arrives.
   */
  private setMask(u: Uniforms, view: Viewport, mode: 0 | 1 | 2): void {
    const gl = this.gl;
    gl.uniform1i(u.uMaskMode ?? null, mode);
    gl.uniform2f(u.uMaskSize ?? null, view.width, view.height);
    gl.uniform1i(u.uMask ?? null, MASK_UNIT);
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
    mode: 0 | 1 | 2,
  ): void {
    const gl = this.gl;
    gl.useProgram(this.rasterProgram);
    gl.bindVertexArray(this.quadVao);
    gl.uniform1i(this.rasterUniforms.uTile ?? null, 0);
    gl.uniform1f(this.rasterUniforms.uSpeedScale ?? null, SPEED_SCALE_MPS);
    gl.uniform1f(this.rasterUniforms.uRampMin ?? null, state.rampMin);
    gl.uniform1f(this.rasterUniforms.uRampMax ?? null, state.rampMax);
    this.setMask(this.rasterUniforms, state.view, mode);
    gl.activeTexture(gl.TEXTURE0);

    for (const tile of tiles) {
      const { texture, held, frame } = this.textureFor(state, tile);
      if (!texture) continue;
      this.noteRange(frame, tile);
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

  /**
   * This frame's texture for a tile, fetching it if absent — or, until it
   * lands, the held frame's, which is marked so the raster can dim it.
   */
  private textureFor(
    state: RenderState,
    tile: VisibleTile,
  ): { texture: WebGLTexture | null; held: boolean; frame: string } {
    const texture = this.tiles.get(state.frame, tile.z, tile.x, tile.y);
    if (texture || !state.heldFrame) return { texture, held: false, frame: state.frame };
    return {
      texture: this.tiles.peek(state.heldFrame, tile.z, tile.x, tile.y),
      held: true,
      frame: state.heldFrame,
    };
  }

  /**
   * The range of speeds across the tiles the last frame drew, in m/s, or null
   * when none of them held a field (spec.md 5.3, M27). Accumulated by the
   * raster pass from each tile's own range, which the cache read at upload.
   */
  private seen: { min: number; max: number } | null = null;

  private noteRange(frame: string, tile: VisibleTile): void {
    const range = this.tiles.rangeOf(frame, tile.z, tile.x, tile.y);
    if (range === null) return;
    const min = (range[0] / 0xffff) * SPEED_SCALE_MPS;
    const max = (range[1] / 0xffff) * SPEED_SCALE_MPS;
    if (this.seen === null) this.seen = { min, max };
    else {
      if (min < this.seen.min) this.seen.min = min;
      if (max > this.seen.max) this.seen.max = max;
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
    mode: 0 | 1 | 2,
  ): void {
    const gl = this.gl;
    // Spacing is resolved to a whole-degree lattice step so the grid is
    // globally anchored. `glyphLayout` is shared with the gesture preview,
    // which draws the same glyphs on the same lattice.
    const { stepDeg, spacing } = glyphLayout(
      state.glyphStyle,
      camera.pxPerDeg,
      state.pixelRatio,
    );
    gl.useProgram(this.glyphProgram);
    gl.bindVertexArray(this.glyphVao);
    gl.uniform1i(this.glyphUniforms.uTile ?? null, 0);
    gl.uniform1f(this.glyphUniforms.uSpeedScale ?? null, SPEED_SCALE_MPS);
    gl.uniform1f(this.glyphUniforms.uSpacing ?? null, spacing);
    gl.uniform1f(this.glyphUniforms.uGlyphStep ?? null, stepDeg);
    gl.uniform1i(this.glyphUniforms.uStyle ?? null, state.glyphStyle === "barb" ? 1 : 0);
    gl.uniform1f(this.glyphUniforms.uSizeScale ?? null, GLYPH_SIZE_SCALE[state.glyphStyle]);
    gl.uniform1f(this.glyphUniforms.uPixelRatio ?? null, state.pixelRatio);
    gl.uniform4f(this.glyphUniforms.uColor ?? null, ...GLYPH);
    this.setMask(this.glyphUniforms, state.view, mode);
    gl.activeTexture(gl.TEXTURE0);

    const projection = projectionFor(camera);
    const centreY = projection.yOf(camera.centerLat);

    for (const tile of tiles) {
      const { texture } = this.textureFor(state, tile);
      if (!texture) continue;
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

      // Entirely off screen: skip before spending instances on it.
      if (
        originX + widthPx < 0 || originX > state.view.width ||
        originY + heightPx < 0 || originY > state.view.height
      ) {
        continue;
      }

      const lattice = glyphLattice(b, stepDeg);
      if (lattice.cols === 0 || lattice.rows === 0) continue;
      if (lattice.cols * lattice.rows > 4096) continue;

      this.setShared(this.glyphUniforms, camera, state.view, tile.lonOffset);
      gl.uniform4f(
        this.glyphUniforms.uTileGeo ?? null,
        b.west, b.north, b.east - b.west, b.north - b.south,
      );
      gl.uniform2f(
        this.glyphUniforms.uGlyphOrigin ?? null,
        lattice.originLon, lattice.originLat,
      );
      gl.uniform2f(this.glyphUniforms.uGrid ?? null, lattice.cols, lattice.rows);
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.drawArraysInstanced(gl.TRIANGLES, 0, GLYPH_VERTICES, lattice.cols * lattice.rows);
    }
  }

  /**
   * Draws a frame and returns the range of speeds across the field tiles it
   * drew, in m/s — null when none held a field. What the auto scale
   * (spec.md 5.3) sets the next frame's ramp from.
   */
  render(state: RenderState): { min: number; max: number } | null {
    const gl = this.gl;
    const offsets = this.worldOffsets(state);
    this.seen = null;
    const lod = this.lodFor(state.camera.pxPerDeg);

    gl.viewport(0, 0, state.view.width, state.view.height);
    gl.clearColor(...SEA);
    gl.clear(gl.COLOR_BUFFER_BIT);

    // The gesture in progress, if it operates on the field rather than adding
    // one of its own (spec.md 6.1). Uploading it is what makes a mask cover
    // and a clone clone *while the pointer is down*, rather than at the commit
    // a round trip later.
    const operating = this.uploadMask(state.operator?.mask ?? null);
    const mode: 0 | 1 | 2 = operating ? 1 : 0;
    // A clone draws the field a second time, read through a camera shifted so
    // the source lands where the brush is, and kept only where the gesture
    // covers (spec.md 5.1).
    const source =
      operating && state.operator?.kind === "clone" ? (state.operator.source ?? null) : null;

    // Which tiles each camera sees, once per frame rather than once per pass:
    // the raster and the glyphs walk the same set.
    const tiles = visibleTiles(state.camera, state.view);
    const sourceTiles = source ? visibleTiles(source, state.view) : [];

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
    if (images.length > 0) {
      gl.useProgram(this.imageProgram);
      gl.bindVertexArray(this.imageMesh.vao);
      gl.uniform1i(this.imageUniforms.uImage ?? null, 0);
      gl.activeTexture(gl.TEXTURE0);
      for (const image of images) {
        if (!image.texture || image.opacity <= 0) continue;
        gl.uniform3f(this.imageUniforms.uPlaceLon ?? null, ...image.placeLon);
        gl.uniform3f(this.imageUniforms.uPlaceLat ?? null, ...image.placeLat);
        gl.uniform1f(this.imageUniforms.uOpacity ?? null, image.opacity);
        gl.bindTexture(gl.TEXTURE_2D, image.texture);
        for (const offset of offsets) {
          this.setShared(this.imageUniforms, state.camera, state.view, offset);
          gl.drawArrays(gl.TRIANGLES, 0, this.imageMesh.count);
        }
      }
    }

    // --- Speed raster ---
    // The basemap is never masked: a mask takes away the field, not the
    // coastline underneath it.
    this.drawRaster(state, state.camera, tiles, mode);
    if (source) this.drawRaster(state, source, sourceTiles, 2);

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
    if (state.showGlyphs) {
      this.drawGlyphs(state, state.camera, tiles, mode);
      if (source) this.drawGlyphs(state, source, sourceTiles, 2);
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
    gl.deleteProgram(this.geoProgram);
    gl.deleteProgram(this.rasterProgram);
    gl.deleteProgram(this.glyphProgram);
  }
}

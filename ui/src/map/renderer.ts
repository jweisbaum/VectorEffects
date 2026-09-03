/**
 * The WebGL2 map renderer.
 *
 * Draw order, bottom to top: land fill, coastlines, the speed raster, the
 * graticule, then direction glyphs. The raster sits above the land because the
 * field is global — it covers land as well as sea — and the graticule sits above
 * the raster so it stays legible against strong colour.
 */

import {
  type Camera,
  type Viewport,
  glyphLattice,
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
  RASTER_FRAG,
  RASTER_VERT,
} from "./shaders";
import type { TileCache } from "./tiles";

/** Full-scale speed of the tile encoding. Mirrors `ve_render::tile`. */
export const SPEED_SCALE_MPS = 100.0;
/** Vertices per glyph instance; see the glyph vertex shader. */
const GLYPH_VERTICES = 54;

/** What to draw. */
export interface RenderState {
  camera: Camera;
  view: Viewport;
  /** Opaque token naming the field, e.g. `step-3`. */
  frame: string;
  glyphStyle: "arrow" | "barb";
  showGlyphs: boolean;
  showGraticule: boolean;
  /** Speed mapped to the top of the colour ramp, m/s. */
  rampMax: number;
  /** Dim the raster while a frame is still resolving. */
  stale: boolean;
  /**
   * Device pixels per CSS pixel.
   *
   * Glyph spacing and size are specified in CSS pixels and scaled by this, or
   * they come out half-size on a retina display.
   */
  pixelRatio: number;
}

const SEA: [number, number, number, number] = [0.043, 0.078, 0.133, 1];
const LAND: [number, number, number, number] = [0.20, 0.25, 0.23, 1];
const COAST: [number, number, number, number] = [0.86, 0.93, 1.0, 0.75];
const GRATICULE: [number, number, number, number] = [0.55, 0.68, 0.85, 0.16];
const GLYPH: [number, number, number, number] = [0.94, 0.97, 1.0, 0.9];

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

  /** Set to have the next render read its pixels back. */
  private captureRequest: ((data: ImageData | null) => void) | null = null;

  constructor(gl: WebGL2RenderingContext, basemap: Basemap, tiles: TileCache) {
    this.gl = gl;
    this.tiles = tiles;

    this.geoProgram = link(gl, GEO_VERT, GEO_FRAG);
    this.rasterProgram = link(gl, RASTER_VERT, RASTER_FRAG);
    this.glyphProgram = link(gl, GLYPH_VERT, GLYPH_FRAG);

    const shared = ["uCamera", "uViewport", "uLonOffset"];
    this.geoUniforms = uniforms(gl, this.geoProgram, [...shared, "uColor"]);
    this.rasterUniforms = uniforms(gl, this.rasterProgram, [
      ...shared, "uTileGeo", "uTile", "uSpeedScale", "uRampMax", "uDim",
    ]);
    this.glyphUniforms = uniforms(gl, this.glyphProgram, [
      ...shared, "uTileGeo", "uGlyphOrigin", "uGlyphStep", "uGrid", "uSpacing",
      "uTile", "uSpeedScale", "uStyle", "uSizeScale", "uColor", "uPixelRatio",
    ]);

    for (const lod of basemap.lods) {
      this.landByLod.set(lod.marker, this.buildGeo(lod.triVertices, lod.triIndices));
      this.coastByLod.set(lod.marker, this.buildGeo(lod.lineVertices, lod.lineIndices));
    }

    this.quadVao = this.buildQuad();
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

  /** World copies to draw so the map wraps seamlessly at the dateline. */
  private worldOffsets(state: RenderState): number[] {
    const bounds = visibleBounds(state.camera, state.view);
    const first = Math.floor((bounds.west + 180) / 360);
    const last = Math.floor((bounds.east + 180) / 360);
    const out: number[] = [];
    for (let i = first; i <= last; i++) out.push(i * 360);
    return out.length > 0 ? out : [0];
  }

  private setShared(u: Uniforms, state: RenderState, lonOffset: number): void {
    const gl = this.gl;
    gl.uniform3f(
      u.uCamera ?? null,
      state.camera.centerLon,
      state.camera.centerLat,
      state.camera.pxPerDeg,
    );
    gl.uniform2f(u.uViewport ?? null, state.view.width, state.view.height);
    gl.uniform1f(u.uLonOffset ?? null, lonOffset);
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

  render(state: RenderState): void {
    const gl = this.gl;
    const offsets = this.worldOffsets(state);
    const lod = this.lodFor(state.camera.pxPerDeg);

    gl.viewport(0, 0, state.view.width, state.view.height);
    gl.clearColor(...SEA);
    gl.clear(gl.COLOR_BUFFER_BIT);

    // --- Land and coastlines ---
    gl.useProgram(this.geoProgram);
    const land = this.landByLod.get(lod);
    const coast = this.coastByLod.get(lod);
    if (land) {
      gl.bindVertexArray(land.vao);
      gl.uniform4f(this.geoUniforms.uColor ?? null, ...LAND);
      for (const offset of offsets) {
        this.setShared(this.geoUniforms, state, offset);
        gl.drawElements(gl.TRIANGLES, land.indexCount, gl.UNSIGNED_INT, 0);
      }
    }
    // --- Speed raster ---
    const tiles = visibleTiles(state.camera, state.view);
    gl.useProgram(this.rasterProgram);
    gl.bindVertexArray(this.quadVao);
    gl.uniform1i(this.rasterUniforms.uTile ?? null, 0);
    gl.uniform1f(this.rasterUniforms.uSpeedScale ?? null, SPEED_SCALE_MPS);
    gl.uniform1f(this.rasterUniforms.uRampMax ?? null, state.rampMax);
    gl.uniform1f(this.rasterUniforms.uDim ?? null, state.stale ? 0.55 : 1.0);
    gl.activeTexture(gl.TEXTURE0);

    for (const tile of tiles) {
      const texture = this.tiles.get(state.frame, tile.z, tile.x, tile.y);
      if (!texture) continue;
      const b = tileBounds(tile.z, tile.x, tile.y);
      this.setShared(this.rasterUniforms, state, tile.lonOffset);
      gl.uniform4f(
        this.rasterUniforms.uTileGeo ?? null,
        b.west, b.north, b.east - b.west, b.north - b.south,
      );
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.drawArrays(gl.TRIANGLES, 0, 6);
    }

    // --- Coastlines, above the raster ---
    // The field covers land as well as sea, so a coastline drawn underneath it
    // is almost invisible. Drawn here it stays legible at any wind speed.
    if (coast) {
      gl.useProgram(this.geoProgram);
      gl.bindVertexArray(coast.vao);
      gl.uniform4f(this.geoUniforms.uColor ?? null, ...COAST);
      for (const offset of offsets) {
        this.setShared(this.geoUniforms, state, offset);
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
          this.setShared(this.geoUniforms, state, offset);
          gl.drawArrays(gl.LINES, 0, this.graticule.count);
        }
      }
    }

    // --- Glyphs ---
    if (state.showGlyphs) {
      // Spacing is resolved to a whole-degree lattice step so the grid is
      // globally anchored. `glyphLayout` is shared with the brush preview,
      // which draws the same glyphs on the same lattice.
      const { stepDeg, spacing } = glyphLayout(
        state.glyphStyle,
        state.camera.pxPerDeg,
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
      gl.activeTexture(gl.TEXTURE0);

      for (const tile of tiles) {
        const texture = this.tiles.get(state.frame, tile.z, tile.x, tile.y);
        if (!texture) continue;
        const b = tileBounds(tile.z, tile.x, tile.y);
        const originX =
          (b.west + tile.lonOffset - state.camera.centerLon) * state.camera.pxPerDeg +
          state.view.width / 2;
        const originY =
          (state.camera.centerLat - b.north) * state.camera.pxPerDeg + state.view.height / 2;
        const widthPx = (b.east - b.west) * state.camera.pxPerDeg;
        const heightPx = (b.north - b.south) * state.camera.pxPerDeg;

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

        this.setShared(this.glyphUniforms, state, tile.lonOffset);
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
        gl.drawArraysInstanced(
          gl.TRIANGLES, 0, GLYPH_VERTICES, lattice.cols * lattice.rows,
        );
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

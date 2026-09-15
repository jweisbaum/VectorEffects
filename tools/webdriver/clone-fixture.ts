/** Tests holes and true calm in clone preview using actual WebGL pixels. */
import { MapRenderer, type RenderState } from "../../ui/src/map/renderer";
import { tileBounds } from "../../ui/src/map/camera";
import type { TileCache } from "../../ui/src/map/tiles";

export function cloneFixture(mode: "wind" | "calm" | "empty") {
  const canvas = document.createElement("canvas"); canvas.width = 800; canvas.height = 600;
  const gl = canvas.getContext("webgl2", { preserveDrawingBuffer: true })!;
  if (!gl) throw new Error("WebGL unavailable");
  const textures = new Map<string, WebGLTexture>();
  const entry = (frame: string, z: number, x: number, y: number) => {
    const key = `${frame}/${z}/${x}/${y}`;
    if (textures.has(key)) return textures.get(key)!;
    const b = tileBounds(z, x, y), bytes = new Uint8Array(256 * 256 * 4), words = new DataView(bytes.buffer);
    for (let row = 0; row < 256; row++) for (let col = 0; col < 256; col++) {
      const lon = b.west + (col + .5) / 256 * (b.east - b.west);
      const lat = b.north - (row + .5) / 256 * (b.north - b.south);
      const source = frame === "source";
      if (source && (mode === "empty" || Math.hypot(lon + 4, lat - 2) > 1.5)) continue;
      const speed = source ? mode === "calm" ? 0 : 20 : 8;
      words.setUint32((row * 256 + col) * 4, (Math.round(speed / 100 * 16383) | (1024 << 14) | (31 << 26) | (1 << 31)) >>> 0, true);
    }
    const texture = gl.createTexture()!;
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, 256, 256, 0, gl.RGBA, gl.UNSIGNED_BYTE, bytes);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    textures.set(key, texture); return texture;
  };
  const tiles = { get: entry, peek: entry, unresolved: () => false,
    rangeOf: () => ({ wind: [0, 3277], current: null }) } as unknown as TileCache;
  const renderer = new MapRenderer(gl, { version: 1, lods: [] }, tiles);
  const state: RenderState = { camera: { centerLon: 0, centerLat: 0, pxPerDeg: 40 },
    view: { width: 800, height: 600 }, frame: "destination", heldFrame: null,
    ramps: { wind: { min: 0, max: 20 }, current: { min: 0, max: 1 } },
    gradients: { wind: [[0, 0, 1], [1, 0, 0]], current: [[0,0,0], [1,1,1]] },
    showGlyphs: false, showGraticule: false, pixelRatio: 1 };
  const pixels = () => { const bytes = new Uint8Array(800 * 600 * 4); gl.readPixels(0,0,800,600,gl.RGBA,gl.UNSIGNED_BYTE,bytes); return bytes; };
  renderer.render(state); const before = pixels();
  const mask = document.createElement("canvas"); mask.width = 800; mask.height = 600;
  const ctx = mask.getContext("2d")!; ctx.fillStyle = "white"; ctx.fillRect(400, 20, 360, 560);
  state.sourceFrame = "source";
  state.operator = { mask, kind: "clone", transparentSource: true,
    source: { ...state.camera, centerLon: -8 } };
  const times: number[] = [];
  for (let i = 0; i < 40; i++) {
    const start = performance.now(); renderer.render(state); gl.finish();
    if (i >= 10) times.push(performance.now() - start);
  }
  times.sort((a,b) => a-b);
  const after = pixels();
  const at = (data: Uint8Array, x: number, y: number) => [...data.slice(((599-y)*800+x)*4, ((599-y)*800+x)*4+4)];
  const sites = [[560,220], [560,440], [730,100], [200,220]];
  const samples = sites.map(([x,y]) => ({ x,y,before: at(before,x!,y!),after: at(after,x!,y!) }));
  const result = { mode, samples, medianMs: times[15], p95Ms: times[28], glError: gl.getError(), image: canvas.toDataURL() };
  renderer.dispose(); for (const t of textures.values()) gl.deleteTexture(t);
  gl.getExtension("WEBGL_lose_context")?.loseContext(); return result;
}

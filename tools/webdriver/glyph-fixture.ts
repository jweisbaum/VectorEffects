/** Synthetic field for glyph coverage screenshots and draw-cost checks. */
import { MapRenderer, type RenderState } from "../../ui/src/map/renderer";
import { tileBounds } from "../../ui/src/map/camera";
import { glyphCoverage } from "../../ui/src/map/glyphPlacement";
import type { TileCache } from "../../ui/src/map/tiles";
import { DEFAULT_GLYPHS } from "../../ui/src/map/glyphAppearance";
import { createElement } from "react";
import { createRoot } from "react-dom/client";
import GlyphSettingsPanel from "../../ui/src/settings/GlyphSettings";
import type { AppSettings } from "../../ui/src/generated/AppSettings";

/** Check the actual settings layout in WebKit, including expanded shadows. */
export async function glyphSettingsLayout(css: string) {
  const style = document.createElement("style");
  style.textContent = css;
  document.head.append(style);
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const glyphs = structuredClone(DEFAULT_GLYPHS);
  glyphs.arrow.shadow.enabled = true;
  glyphs.barb.shadow.enabled = true;
  root.render(createElement("div", { className: "modal-backdrop" },
    createElement("div", { className: "modal settings" },
      createElement(GlyphSettingsPanel, { settings: { glyphs } as AppSettings, onSettings: () => {}, onError: () => {} }))));
  await new Promise(resolve => setTimeout(resolve, 80));
  const fields = [...host.querySelectorAll<HTMLElement>(".glyph-field")];
  const result = {
    controls: fields.length,
    previews: host.querySelectorAll("svg").length,
    overflowing: fields.filter(field => field.scrollWidth > field.clientWidth + 1).length,
    stacked: fields.filter(field => getComputedStyle(field).flexDirection !== "row").length,
    checkboxWidths: [...host.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')].map(input => input.getBoundingClientRect().width),
  };
  root.unmount();
  host.remove();
  style.remove();
  return result;
}

export function glyphFixture(solid = false, styled = false) {
  const canvas = document.createElement("canvas");
  canvas.width = 1200;
  canvas.height = 900;
  const gl = canvas.getContext("webgl2", { preserveDrawingBuffer: true });
  if (!gl) throw new Error("WebGL2 unavailable");
  const entries = new Map<string, { texture: WebGLTexture; coverage: Uint8Array; wind: Uint8Array }>();
  const entry = (z: number, x: number, y: number) => {
    const key = `${z}/${x}/${y}`;
    let found = entries.get(key);
    if (found) return found;
    const b = tileBounds(z, x, y);
    const bytes = new Uint8Array(256 * 256 * 4);
    const words = new DataView(bytes.buffer);
    for (let row = 0; row < 256; row++) for (let col = 0; col < 256; col++) {
      const lon = b.west + (col + 0.5) / 256 * (b.east - b.west);
      const lat = b.north - (row + 0.5) / 256 * (b.north - b.south);
      const radius = Math.hypot(lon - 3.3, lat + 0.3);
      const disc = Math.hypot(lon + 7, lat - 4);
      if (!solid && Math.abs(radius - 4.8) > 0.38 && disc > 3.1) continue;
      const azimuth = ((Math.atan2(lat + 0.3, lon - 3.3) * 180 / Math.PI) % 360 + 360) % 360;
      const word = Math.round(15 / 100 * 16383) | (Math.round(azimuth / 360 * 4096) % 4096 << 14) | (31 << 26) | ((!styled || lon >= 0) ? (1 << 31) : 0);
      words.setUint32((row * 256 + col) * 4, word >>> 0, true);
    }
    const texture = gl.createTexture();
    if (!texture) throw new Error("Texture allocation failed");
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, 256, 256, 0, gl.RGBA, gl.UNSIGNED_BYTE, bytes);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    found = { texture, coverage: glyphCoverage(bytes), wind: glyphCoverage(bytes, true) };
    entries.set(key, found);
    return found;
  };
  const tiles = {
    get: (_frame: string, z: number, x: number, y: number) => entry(z, x, y).texture,
    peek: (_frame: string, z: number, x: number, y: number) => entry(z, x, y).texture,
    coverageOf: (_frame: string, z: number, x: number, y: number) => entry(z, x, y).coverage,
    windCoverageOf: (_frame: string, z: number, x: number, y: number) => entry(z, x, y).wind,
    unresolved: () => false,
    rangeOf: () => ({ wind: [2457, 2457], current: null }),
  } as unknown as TileCache;
  const renderer = new MapRenderer(gl, { version: 1, lods: [] }, tiles);
  const state: RenderState = {
    camera: { centerLon: 0, centerLat: 0, pxPerDeg: 50 },
    view: { width: canvas.width, height: canvas.height },
    frame: "fixture/0", heldFrame: null,
    ramps: { wind: { min: 0, max: 15 }, current: { min: 0, max: 1 } },
    gradients: { wind: [[0.02, 0.06, 0.12], [0.75, 0.08, 0.3]], current: [[0, 0, 0], [1, 1, 1]] },
    showGlyphs: true, showGraticule: false, pixelRatio: 1,
  };
  if (styled) {
    state.glyphs = structuredClone(DEFAULT_GLYPHS);
    Object.assign(state.glyphs.arrow, { size_percent: 175, color: "#00ff80", opacity_percent: 80, density_percent: 50, fade_with_speed: false, stroke_width_px: 3 });
    Object.assign(state.glyphs.barb, { size_percent: 140, color: "#ffcc00", opacity_percent: 95, density_percent: 200, fade_with_speed: false, stroke_width_px: 2.5 });
    for (const glyph of Object.values(state.glyphs)) Object.assign(glyph.shadow, { enabled: true, offset_x_px: 3, offset_y_px: 3, opacity_percent: 90 });
  }
  renderer.render(state);
  gl.finish();
  const samples: number[] = [];
  for (let i = 0; i < 80; i++) {
    const start = performance.now();
    renderer.render(state);
    gl.finish();
    if (i >= 20) samples.push(performance.now() - start);
  }
  samples.sort((a, b) => a - b);
  const image = canvas.toDataURL("image/png");
  const panSamples: number[] = [];
  for (let i = 0; i < 40; i++) {
    state.camera.centerLon = (i + 1) * 0.01;
    const start = performance.now();
    renderer.render(state);
    gl.finish();
    panSamples.push(performance.now() - start);
  }
  panSamples.sort((a, b) => a - b);
  const result = {
    image,
    medianMs: samples[Math.floor(samples.length / 2)],
    p95Ms: samples[Math.floor(samples.length * 0.95)],
    panMedianMs: panSamples[Math.floor(panSamples.length / 2)],
    panP95Ms: panSamples[Math.floor(panSamples.length * 0.95)],
    glError: gl.getError(),
  };
  renderer.dispose();
  for (const { texture } of entries.values()) gl.deleteTexture(texture);
  gl.getExtension("WEBGL_lose_context")?.loseContext();
  return result;
}

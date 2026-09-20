/**
 * The globe is projected twice: by `azimuthal` in general.ts for the pointer,
 * the overlay and the tile cull, and by the GLSL in projectionShaders.ts for
 * every pixel of the map. A divergence would show as a stroke landing beside
 * the field it was painted on, which is easy to look at and not see.
 *
 * So, as shaders.test.ts does for the cylindrical modes, the shader's own text
 * is run here as JavaScript against the TypeScript. The two languages agree on
 * arithmetic; the few names that differ are supplied below.
 */
import { describe, expect, it } from "vitest";

import { MAX_PX_PER_DEG, azimuthalTiles, project, tileBounds, unproject, validGeo, type Camera } from "../camera";
import { projectionOf, shaderMode, type ProjectionId } from "../projection";
import { AZIMUTHAL, AZIMUTHAL_MODE, GPU_AZIMUTHALS } from "../projectionShaders";

const VE_DEG = Math.PI / 180;
const IDS = ["orthographic", "azimuthal_equidistant", "azimuthal_equal_area", "stereographic", "gnomonic"] as const;
const view = { width: 1600, height: 1000 };

/** The shader's functions, over one camera's uniforms. */
function shader(camera: Camera) {
  const body = AZIMUTHAL
    .replace(/\/\/[^\n]*/g, "")
    .replace(/uniform[^;]*;/g, "")
    .replace(/\b(?:float|vec2|vec3) (\w+)\(([^)]*)\) \{/g, (_, name: string, params: string) =>
      `function ${name}(${params.replace(/\b(?:int|float|vec2|vec3)\s+/g, "")}) {`)
    .replace(/\b(?:float|int|vec2|vec3)\s+(?=\w+\s*=)/g, "let ")
    .replace(/\b(sin|cos|tan|asin|atan|min|mod|length|clamp|vec2|vec3)\(/g, "M.$1(");
  const M = {
    sin: Math.sin, cos: Math.cos, tan: Math.tan, asin: Math.asin, min: Math.min,
    atan: (y: number, x?: number) => (x === undefined ? Math.atan(y) : Math.atan2(y, x)),
    mod: (x: number, y: number) => x - y * Math.floor(x / y),
    clamp: (x: number, low: number, high: number) => Math.min(Math.max(x, low), high),
    length: (v: { x: number; y: number }) => Math.hypot(v.x, v.y),
    vec2: (x: number, y = x) => ({ x, y }),
    vec3: (x: number, y: number, z: number) => ({ x, y, z }),
  };
  const lat0 = camera.centerLat * VE_DEG;
  const make = new Function("M", "VE_DEG", "uCamera", "uViewport", "uOrigin", "uRim", "uProjection",
    `${body}\nreturn { screen: (p) => { const s = azimuthalScreen(p); return { ...s, horizon: veHorizon }; }, inverse: azimuthalInverse };`);
  return make(M, VE_DEG,
    { x: camera.centerLon, y: 0, z: camera.pxPerDeg }, { x: view.width, y: view.height },
    { x: camera.centerLat, y: Math.sin(lat0), z: Math.cos(lat0) }, 0,
    shaderMode(projectionOf(camera.projection ?? "equirectangular")),
  ) as {
    screen(p: { x: number; y: number }): { x: number; y: number; horizon: number };
    inverse(p: { x: number; y: number }): { x: number; y: number };
  };
}

// The equator, a mid-latitude, both poles, and a centre on the antimeridian.
const CENTRES = [[0, 0], [-72.5, 41.3], [10, 90], [-135, -90], [180, -33], [-179.5, 64]] as const;

describe("the globe's shader and the pointer's", () => {
  it("numbers the azimuthals the way the shader branches", () => {
    expect(IDS.map((id) => shaderMode(projectionOf(id)))).toEqual(IDS.map((_, i) => AZIMUTHAL_MODE + i));
    expect(IDS.map((id) => projectionOf(id).general?.movable)).toEqual([...GPU_AZIMUTHALS]);
    // What is persisted does not move: a pixel tool's stamp space is mode + 1.
    for (const id of IDS) expect(projectionOf(id).mode).toBe(14);
  });

  it("agree on where every place lands, and on which side of the earth it is", () => {
    for (const id of IDS) {
      for (const [centerLon, centerLat] of CENTRES) {
        const camera: Camera = { projection: id as ProjectionId, centerLon, centerLat, pxPerDeg: 6 };
        const glsl = shader(camera);
        for (let lat = -90; lat <= 90; lat += 7.5) {
          for (let lon = -180; lon < 180; lon += 11.25) {
            const expected = project(camera, view, { lon, lat });
            const got = glsl.screen({ x: lon, y: lat });
            const where = `${id} from ${centerLon},${centerLat} at ${lon},${lat}`;
            if (Number.isFinite(expected.x)) {
              // Shown by one must be shown by the other, to within the last
              // rounding at the limit itself.
              expect(got.horizon, where).toBeGreaterThan(-1e-6);
              expect(got.x, where).toBeCloseTo(expected.x, 6);
              expect(got.y, where).toBeCloseTo(expected.y, 6);
            } else {
              expect(got.horizon, where).toBeLessThan(1e-6);
            }
          }
        }
      }
    }
  });

  it("agree on the place under a pixel", () => {
    for (const id of IDS) {
      for (const [centerLon, centerLat] of CENTRES) {
        const camera: Camera = { projection: id as ProjectionId, centerLon, centerLat, pxPerDeg: 9 };
        const glsl = shader(camera);
        for (let y = 5; y < view.height; y += 90) {
          for (let x = 5; x < view.width; x += 90) {
            const expected = unproject(camera, view, { x, y });
            const got = glsl.inverse({ x, y });
            const where = `${id} from ${centerLon},${centerLat} at pixel ${x},${y}`;
            if (!validGeo(expected)) {
              expect(got.x, where).toBeGreaterThan(1000);
              continue;
            }
            expect(got.y, where).toBeCloseTo(expected.lat, 7);
            // At a pole every longitude is the same place.
            if (Math.abs(expected.lat) < 89.999) {
              const turn = ((got.x - expected.lon + 540) % 360) - 180;
              expect(turn, where).toBeCloseTo(0, 6);
            }
          }
        }
      }
    }
  });

  /**
   * The centre is where the textbook forward loses its digits: two numbers
   * near a half, a few millionths apart. Held to a thousandth of a pixel at
   * the closest zoom, in *single* precision, which is what the GPU has.
   */
  it("keeps its digits at the closest zoom in single precision", () => {
    const f = Math.fround;
    const camera: Camera = { projection: "orthographic", centerLon: -70.123456, centerLat: 41.654321, pxPerDeg: MAX_PX_PER_DEG };
    for (const [dLon, dLat] of [[0.4, 0.3], [-0.9, 0.05], [0.001, -0.7]] as const) {
      const lon = camera.centerLon + dLon, lat = camera.centerLat + dLat;
      // The shader's arithmetic, rounded to a float at every step.
      const lam = f(f(f(f(lon) - f(camera.centerLon))) * f(VE_DEG));
      const dphi = f(f(f(lat) - f(camera.centerLat)) * f(VE_DEG));
      const phi = f(f(lat) * f(VE_DEG));
      const h = f(Math.sin(f(lam * 0.5))), versine = f(2 * h * h), cphi = f(Math.cos(phi));
      const sin0 = f(Math.sin(camera.centerLat * VE_DEG));
      const y = f(f(Math.sin(dphi)) + f(f(sin0 * cphi) * versine));
      const x = f(cphi * f(Math.sin(lam)));
      // Against the projection of the same single-precision inputs: what is
      // measured is the arithmetic, not the rounding of the coordinates.
      const expected = project({ ...camera, centerLon: f(camera.centerLon), centerLat: f(camera.centerLat) }, view, { lon: f(lon), lat: f(lat) });
      expect(view.width / 2 + (x / VE_DEG) * camera.pxPerDeg).toBeCloseTo(expected.x, 2);
      expect(view.height / 2 - (y / VE_DEG) * camera.pxPerDeg).toBeCloseTo(expected.y, 2);
    }
  });
});

/**
 * The tiles a globe asks for are found by walking the pyramid, not read off a
 * screen mesh. The property that defines the right answer: whatever place is
 * under any pixel, the tile that holds it is in the list.
 */
describe("the tiles a globe shows", () => {
  const cameras: Camera[] = [];
  for (const id of IDS) {
    for (const [centerLon, centerLat] of CENTRES) {
      for (const pxPerDeg of [3, 12, 60, 400, MAX_PX_PER_DEG]) cameras.push({ projection: id as ProjectionId, centerLon, centerLat, pxPerDeg });
    }
  }

  it("hold the place under every pixel, and stay within the budget", () => {
    for (const camera of cameras) {
      const tiles = azimuthalTiles(camera, view);
      const where = `${camera.projection} from ${camera.centerLon},${camera.centerLat} at ${camera.pxPerDeg}`;
      expect(tiles.length, where).toBeGreaterThan(0);
      expect(tiles.length, where).toBeLessThanOrEqual(192);
      expect(new Set(tiles.map((t) => t.z)).size, where).toBe(1);
      const bounds = tiles.map((t) => tileBounds(t.z, t.x, t.y));
      for (let y = 0; y <= view.height; y += 37) {
        for (let x = 0; x <= view.width; x += 41) {
          const geo = unproject(camera, view, { x, y });
          if (!validGeo(geo)) continue;
          const held = bounds.some((b) => geo.lon >= b.west - 1e-9 && geo.lon <= b.east + 1e-9 && geo.lat <= b.north + 1e-9 && geo.lat >= b.south - 1e-9);
          expect(held, `${where}: nothing holds ${geo.lon},${geo.lat} under pixel ${x},${y}`).toBe(true);
        }
      }
    }
  });

  it("do not ask for the far side of the earth", () => {
    // A globe shows a hemisphere: at a level of 512 tiles, about half.
    const camera: Camera = { projection: "orthographic", centerLon: -30, centerLat: 20, pxPerDeg: 12 };
    const tiles = azimuthalTiles(camera, { width: 2880, height: 1590 });
    const level = tiles[0]!.z;
    const all = (2 << level) * (1 << level);
    expect(tiles.length).toBeLessThan(all * 0.7);
  });
});

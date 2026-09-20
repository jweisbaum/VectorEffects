/**
 * The screen mesh of a general projection, held to what it is for: wherever
 * the map has a place under a pixel, some triangle covers that pixel and
 * interpolates to that place, to within what the eye can see. Checked through
 * the tile-clipped vertices as well, which are what is drawn.
 */
import { describe, expect, it } from "vitest";

import { GENERAL_MAPS, mapTransform, defaultCentre } from "./general";
import { geographicMesh, type MeshTriangle } from "./mesh";

const view = { width: 900, height: 560 };

function build(id: string, pxPerDeg: number) {
  const map = GENERAL_MAPS.find((m) => m.id === id)!;
  const centreGeo = defaultCentre(map);
  const transform = mapTransform(map, centreGeo);
  const centre = transform.forward(centreGeo)!;
  const inverse = (p: { x: number; y: number }) =>
    transform.inverse({ x: centre.x + (p.x - view.width / 2) / pxPerDeg, y: centre.y - (p.y - view.height / 2) / pxPerDeg });
  const forward = (p: { lon: number; lat: number }) => {
    const xy = transform.forward(p);
    return xy ? { x: view.width / 2 + (xy.x - centre.x) * pxPerDeg, y: view.height / 2 - (xy.y - centre.y) * pxPerDeg } : null;
  };
  return { mesh: geographicMesh(view.width, view.height, pxPerDeg, inverse, forward), inverse, forward };
}

/** Barycentric weights of `p` in a triangle, or null outside it. */
function weights(t: MeshTriangle, x: number, y: number): [number, number, number] | null {
  const [a, b, c] = t;
  const det = (b.y - c.y) * (a.x - c.x) + (c.x - b.x) * (a.y - c.y);
  if (Math.abs(det) < 1e-12) return null;
  const u = ((b.y - c.y) * (x - c.x) + (c.x - b.x) * (y - c.y)) / det;
  const v = ((c.y - a.y) * (x - c.x) + (a.x - c.x) * (y - c.y)) / det;
  const w = 1 - u - v;
  return u >= -1e-9 && v >= -1e-9 && w >= -1e-9 ? [u, v, w] : null;
}

describe("a general projection's screen mesh", () => {
  for (const [id, pxPerDeg] of [["robinson", 2.2], ["mollweide", 2.3], ["epsg_3413", 5], ["epsg_27700", 60]] as const) {
    it(`${id}: covers the map and interpolates to the place under each pixel`, () => {
      const { mesh, inverse, forward } = build(id, pxPerDeg);
      expect(mesh.triangles.length).toBeGreaterThan(0);
      // Binned by screen cell, so each probe looks at a handful of triangles.
      const bin = 32, columns = Math.ceil(view.width / bin) + 1;
      const bins = new Map<number, MeshTriangle[]>();
      for (const t of mesh.triangles) {
        const x0 = Math.floor(Math.min(t[0].x, t[1].x, t[2].x) / bin), x1 = Math.floor(Math.max(t[0].x, t[1].x, t[2].x) / bin);
        const y0 = Math.floor(Math.min(t[0].y, t[1].y, t[2].y) / bin), y1 = Math.floor(Math.max(t[0].y, t[1].y, t[2].y) / bin);
        for (let j = y0; j <= y1; j++) for (let i = x0; i <= x1; i++) {
          const key = j * columns + i;
          if (!bins.has(key)) bins.set(key, []);
          bins.get(key)!.push(t);
        }
      }
      let probed = 0, covered = 0, worst = 0;
      for (let y = 3.5; y < view.height; y += 7) {
        for (let x = 3.5; x < view.width; x += 7) {
          const truth = inverse({ x, y });
          if (!truth) continue;
          // Well inside the map: every neighbour three pixels off is on it too.
          if (![[-3, 0], [3, 0], [0, -3], [0, 3]].every(([dx, dy]) => inverse({ x: x + dx!, y: y + dy! }))) continue;
          probed++;
          for (const t of bins.get(Math.floor(y / bin) * columns + Math.floor(x / bin)) ?? []) {
            const w = weights(t, x, y);
            if (!w) continue;
            covered++;
            // The triangle's own longitudes are unwrapped about its first vertex.
            const lon = w[0] * t[0].lon + w[1] * t[1].lon + w[2] * t[2].lon;
            const lat = w[0] * t[0].lat + w[1] * t[1].lat + w[2] * t[2].lat;
            const back = forward({ lon, lat });
            if (back) worst = Math.max(worst, Math.hypot(back.x - x, back.y - y));
            break;
          }
        }
      }
      expect(probed, `${id}: probes on the map`).toBeGreaterThan(500);
      expect(covered / probed, `${id}: share of the map's interior under a triangle`).toBeGreaterThan(0.999);
      expect(worst, `${id}: worst misplacement in pixels`).toBeLessThan(1);
    });

    it(`${id}: what is drawn per tile is the same surface, in tile coordinates`, () => {
      const { mesh, forward } = build(id, pxPerDeg);
      expect(mesh.tiles.length).toBeGreaterThan(0);
      let worst = 0, checked = 0;
      for (const tile of mesh.tiles) {
        const span = 360 / (2 << tile.z), west = -180 + tile.x * span + tile.lonOffset, north = 90 - tile.y * span;
        for (let i = 0; i < tile.vertices.length; i += 4) {
          const [u, v, x, y] = [tile.vertices[i]!, tile.vertices[i + 1]!, tile.vertices[i + 2]!, tile.vertices[i + 3]!];
          expect(u).toBeGreaterThanOrEqual(-1e-5); expect(u).toBeLessThanOrEqual(1 + 1e-5);
          expect(v).toBeGreaterThanOrEqual(-1e-5); expect(v).toBeLessThanOrEqual(1 + 1e-5);
          if (i % 40 !== 0) continue;
          const back = forward({ lon: west + u * span, lat: Math.max(-90, Math.min(90, north - v * span)) });
          if (!back) continue;
          checked++;
          worst = Math.max(worst, Math.hypot(back.x - x, back.y - y));
        }
      }
      expect(checked).toBeGreaterThan(100);
      expect(worst, `${id}: a tile vertex's place, in pixels from where it is drawn`).toBeLessThan(1);
    });
  }
});

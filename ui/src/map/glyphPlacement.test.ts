import { describe, expect, it } from "vitest";
import { glyphLattice, projectionFor, tileBounds, type Camera, type GeoPoint } from "./camera";
import { GLYPH_SUBDIVISIONS, glyphCoverage, glyphCovered, glyphTileIsFull, spacedGlyphs } from "./glyphPlacement";

const camera: Camera = { centerLon: 0, centerLat: 0, pxPerDeg: 50 };
const bounds = { west: -6, east: 6, north: 6, south: -6 };

function lattice(box = bounds, step = 1 / GLYPH_SUBDIVISIONS): GeoPoint[] {
  const block = glyphLattice(box, step);
  return Array.from({ length: block.cols * block.rows }, (_, i) => ({
    lon: block.originLon + (i % block.cols) * step,
    lat: block.originLat - Math.floor(i / block.cols) * step,
  }));
}

function arcGap(points: GeoPoint[], cx: number, cy: number, radius: number): number {
  const angles = points.map(({ lon, lat }) => Math.atan2(lat - cy, lon - cx)).sort((a, b) => a - b);
  return Math.max(...angles.map((a, i) => (angles[i + 1] ?? angles[0]! + Math.PI * 2) - a)) * radius;
}

describe("glyphs in sparse fields", () => {
  it("fills the empty stretches around a thin ring at different grid alignments", () => {
    const radius = 2.5;
    for (const cx of [0, 0.2, 0.5, 0.8]) for (const cy of [0, 0.3, 0.6]) {
      const covered = lattice().filter(({ lon, lat }) => Math.abs(Math.hypot(lon - cx, lat - cy) - radius) < 0.2);
      const placed = spacedGlyphs(covered, 1, camera);
      // No long unmarked arc: the station gap stays within two target spacings.
      expect(arcGap(placed, cx, cy, radius), `ring at ${cx}, ${cy}`).toBeLessThan(2);
      expect(placed.length).toBeGreaterThan(10);
      for (const p of placed) expect(Math.abs(Math.hypot(p.lon - cx, p.lat - cy) - radius)).toBeLessThan(0.2);
    }
  });

  it("keeps a solid field on its ordinary lattice", () => {
    const interior = (p: GeoPoint) => p.lon >= -4 && p.lon < 4 && p.lat <= 4 && p.lat > -4;
    expect(spacedGlyphs(lattice(), 1, camera).filter(interior)).toEqual(lattice(bounds, 1).filter(interior));
  });

  it("marks a narrow stroke that falls entirely between the ordinary rows", () => {
    const strip = lattice().filter(({ lat }) => lat > 0.1 && lat < 0.4);
    const placed = spacedGlyphs(strip, 1, camera);
    expect(placed.length).toBeGreaterThan(10);
    expect(placed.every(({ lat }) => lat > 0.1 && lat < 0.4)).toBe(true);
  });

  it("keeps erased holes empty and never packs stations closer than 80% of spacing", () => {
    const field = lattice().filter(({ lon, lat }) => Math.hypot(lon - 0.2, lat - 0.3) > 2);
    const placed = spacedGlyphs(field, 1, camera);
    for (const p of placed) {
      expect(Math.hypot(p.lon - 0.2, p.lat - 0.3)).toBeGreaterThan(2);
      for (const q of placed) if (q !== p) expect(Math.hypot(p.lon - q.lon, p.lat - q.lat)).toBeGreaterThanOrEqual(0.8 - 1e-9);
    }
  });

  it("is independent of tile order and panning, including a shared tile edge", () => {
    const left = tileBounds(5, 31, 15);
    const right = tileBounds(5, 32, 15);
    const a = lattice(left);
    const b = lattice(right);
    const candidates = [...a, ...b].filter(({ lon, lat }) => Math.abs(lat - 0.25) < 0.1 || Math.abs(lon) < 0.3);
    const expected = spacedGlyphs(candidates, 1, camera);
    expect(spacedGlyphs([...candidates].reverse(), 1, { ...camera, centerLon: 0.7, centerLat: 1 })).toEqual(expected);
    expect(new Set(expected.map(p => `${p.lon}/${p.lat}`)).size).toBe(expected.length);
    for (const p of expected) for (const q of expected) if (p !== q) {
      expect(Math.hypot(p.lon - q.lon, p.lat - q.lat)).toBeGreaterThanOrEqual(0.8 - 1e-9);
    }
  });

  it("spaces sites across the antimeridian and in polar projections", () => {
    for (const projection of ["equirectangular", "mercator", "miller"] as const) {
      const polar: Camera = { ...camera, centerLon: 180, centerLat: 78, projection };
      const candidates = lattice({ west: 176, east: 184, south: 75, north: 82 });
      const placed = spacedGlyphs(candidates, 1, polar);
      const map = projectionFor(polar);
      expect(placed.some(p => p.lon < 180)).toBe(true);
      expect(placed.some(p => p.lon > 180)).toBe(true);
      for (const p of placed) for (const q of placed) if (p !== q) {
        expect(Math.hypot(p.lon - q.lon, map.yOf(p.lat) - map.yOf(q.lat))).toBeGreaterThanOrEqual(0.8 - 1e-9);
      }
    }
  });
});

describe("resident glyph coverage", () => {
  it("reuses a fully covered layout across changing forecast vectors and replaces it after an erasure", () => {
    const bytes = new Uint8Array(256 * 256 * 4).fill(255);
    const gale = glyphCoverage(bytes);
    for (let i = 0; i < bytes.length; i += 4) {
      bytes[i] = 0;
      bytes[i + 1] = 0;
      bytes[i + 2] = 0;
      bytes[i + 3] = 0xfc;
    }
    const calm = glyphCoverage(bytes);
    expect(calm).toBe(gale);
    expect(glyphTileIsFull(calm)).toBe(true);
    bytes[3] = 0;
    const erased = glyphCoverage(bytes);
    expect(erased).not.toBe(calm);
    expect(glyphTileIsFull(erased)).toBe(false);
    expect(glyphCovered(erased, 0, 0)).toBe(false);
    expect(glyphCovered(calm, 0, 0)).toBe(true);
  });

  it("keeps written calm and feathered cells, without mistaking direction or kind bits for coverage", () => {
    const bytes = new Uint8Array(256 * 256 * 4);
    bytes[3] = 0x80; // Wind, zero coverage, zero speed.
    bytes[7] = 0x83; // Direction bits do not imply coverage either.
    bytes[11] = 0xfc; // Fully covered calm wind.
    bytes[15] = 0x04; // Faint current edge.
    bytes[bytes.length - 1] = 0x7c; // Last texel, calm current.
    const mask = glyphCoverage(bytes);
    expect(mask.byteLength).toBe(8192);
    expect(glyphCovered(mask, 0.5 / 256, 0.5 / 256)).toBe(false);
    expect(glyphCovered(mask, 1.5 / 256, 0.5 / 256)).toBe(false);
    expect(glyphCovered(mask, 2.5 / 256, 0.5 / 256)).toBe(true);
    expect(glyphCovered(mask, 3.5 / 256, 0.5 / 256)).toBe(true);
    expect(glyphCovered(mask, 1, 1)).toBe(true);
    expect(glyphCovered(mask, 0.5, 0.5)).toBe(false);
  });
});

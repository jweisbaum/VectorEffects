import { describe, expect, it } from "vitest";

import { BASEMAP_FORMAT_VERSION, parseBasemap } from "./format";

/** Builds a minimal well-formed asset: one LOD, one triangle, one 3-point ring. */
function buildAsset(): ArrayBuffer {
  const triVertices = [0, 0, 10, 0, 0, 10];
  const triIndices = [0, 1, 2];
  const lineVertices = [0, 0, 10, 0, 0, 10];
  const strips: Array<[number, number]> = [[0, 3]];

  const size =
    12 + 20 + triVertices.length * 4 + triIndices.length * 4 +
    lineVertices.length * 4 + strips.length * 8;
  const buffer = new ArrayBuffer(size);
  const view = new DataView(buffer);

  "VEBM".split("").forEach((c, i) => view.setUint8(i, c.charCodeAt(0)));
  view.setUint32(4, BASEMAP_FORMAT_VERSION, true);
  view.setUint32(8, 1, true);

  let pos = 12;
  view.setUint32(pos, 110, true);
  view.setUint32(pos + 4, triVertices.length / 2, true);
  view.setUint32(pos + 8, triIndices.length, true);
  view.setUint32(pos + 12, lineVertices.length / 2, true);
  view.setUint32(pos + 16, strips.length, true);
  pos += 20;

  for (const v of triVertices) { view.setFloat32(pos, v, true); pos += 4; }
  for (const v of triIndices) { view.setUint32(pos, v, true); pos += 4; }
  for (const v of lineVertices) { view.setFloat32(pos, v, true); pos += 4; }
  for (const [o, n] of strips) {
    view.setUint32(pos, o, true); view.setUint32(pos + 4, n, true); pos += 8;
  }
  return buffer;
}

describe("parseBasemap", () => {
  it("reads a well-formed asset", () => {
    const map = parseBasemap(buildAsset());
    expect(map.version).toBe(BASEMAP_FORMAT_VERSION);
    expect(map.lods).toHaveLength(1);

    const lod = map.lods[0]!;
    expect(lod.marker).toBe(110);
    expect(Array.from(lod.triIndices)).toEqual([0, 1, 2]);
    expect(lod.triVertices[2]).toBeCloseTo(10, 5);
  });

  /** A ring of N points becomes N segments, the last one closing it. */
  it("expands rings into closed segment pairs", () => {
    const lod = parseBasemap(buildAsset()).lods[0]!;
    expect(Array.from(lod.lineIndices)).toEqual([0, 1, 1, 2, 2, 0]);
  });

  it("rejects bad magic", () => {
    const buffer = buildAsset();
    new DataView(buffer).setUint8(0, 0);
    expect(() => parseBasemap(buffer)).toThrow(/magic/);
  });

  it("rejects an unknown format version", () => {
    const buffer = buildAsset();
    new DataView(buffer).setUint32(4, 99, true);
    expect(() => parseBasemap(buffer)).toThrow(/version/);
  });

  it("rejects trailing bytes", () => {
    const source = buildAsset();
    const padded = new ArrayBuffer(source.byteLength + 4);
    new Uint8Array(padded).set(new Uint8Array(source));
    expect(() => parseBasemap(padded)).toThrow(/trailing/);
  });
});

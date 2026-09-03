/**
 * Parses the bundled basemap binary.
 *
 * Produced by `tools/basemap-builder` from Natural Earth and committed to
 * `assets/basemap.bin`. Layout, little-endian throughout:
 *
 *   "VEBM", u32 version, u32 lodCount
 *   per LOD: u32 marker, u32 triVertexCount, u32 triIndexCount,
 *            u32 lineVertexCount, u32 lineStripCount
 *            f32[triVertexCount * 2]   lon, lat
 *            u32[triIndexCount]
 *            f32[lineVertexCount * 2]  lon, lat
 *            (u32 offset, u32 length)[lineStripCount]
 */

/** One level of detail, ready for upload as GL buffers. */
export interface BasemapLod {
  /** Natural Earth scale marker: 110 or 50. */
  marker: number;
  /** Triangle vertices as interleaved lon, lat. */
  triVertices: Float32Array;
  /** Triangle indices. */
  triIndices: Uint32Array;
  /** Coastline vertices as interleaved lon, lat. */
  lineVertices: Float32Array;
  /**
   * Coastline indices as line segments.
   *
   * Rings are expanded to explicit segment pairs at parse time, including the
   * closing segment, so the whole coastline draws in one call instead of one
   * call per ring — there are 1,422 rings at the 50m level.
   */
  lineIndices: Uint32Array;
}

/** The parsed asset. */
export interface Basemap {
  version: number;
  /** Levels, coarsest first. */
  lods: BasemapLod[];
}

/** Format version this build understands. */
export const BASEMAP_FORMAT_VERSION = 1;

export function parseBasemap(buffer: ArrayBuffer): Basemap {
  const view = new DataView(buffer);
  const magic = String.fromCharCode(
    view.getUint8(0),
    view.getUint8(1),
    view.getUint8(2),
    view.getUint8(3),
  );
  if (magic !== "VEBM") throw new Error(`basemap: bad magic ${JSON.stringify(magic)}`);

  const version = view.getUint32(4, true);
  if (version !== BASEMAP_FORMAT_VERSION) {
    throw new Error(`basemap: format version ${version}, expected ${BASEMAP_FORMAT_VERSION}`);
  }

  const lodCount = view.getUint32(8, true);
  let pos = 12;
  const lods: BasemapLod[] = [];

  for (let i = 0; i < lodCount; i++) {
    const marker = view.getUint32(pos, true);
    const triVertexCount = view.getUint32(pos + 4, true);
    const triIndexCount = view.getUint32(pos + 8, true);
    const lineVertexCount = view.getUint32(pos + 12, true);
    const lineStripCount = view.getUint32(pos + 16, true);
    pos += 20;

    // slice() rather than a view: the arrays outlive the source buffer and
    // must be aligned for GL upload.
    const triVertices = new Float32Array(buffer.slice(pos, pos + triVertexCount * 8));
    pos += triVertexCount * 8;
    const triIndices = new Uint32Array(buffer.slice(pos, pos + triIndexCount * 4));
    pos += triIndexCount * 4;
    const lineVertices = new Float32Array(buffer.slice(pos, pos + lineVertexCount * 8));
    pos += lineVertexCount * 8;

    const lineIndices = new Uint32Array(lineVertexCount * 2);
    let out = 0;
    for (let s = 0; s < lineStripCount; s++) {
      const offset = view.getUint32(pos, true);
      const length = view.getUint32(pos + 4, true);
      pos += 8;
      for (let k = 0; k < length; k++) {
        lineIndices[out++] = offset + k;
        // Rings are closed, so the last vertex joins back to the first.
        lineIndices[out++] = offset + ((k + 1) % length);
      }
    }

    lods.push({
      marker,
      triVertices,
      triIndices,
      lineVertices,
      lineIndices: lineIndices.subarray(0, out),
    });
  }

  if (pos !== buffer.byteLength) {
    throw new Error(`basemap: ${buffer.byteLength - pos} trailing bytes`);
  }
  return { version, lods };
}

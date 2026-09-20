/**
 * A fixed general projection's mesh, built in the projection's own plane, as
 * plain data: what is asked for and what comes back both cross a worker
 * boundary, so neither holds a function. `camera.ts` decides *when* a mesh is
 * wanted and wraps the answer (PlaneMesh); this only builds it, and is what
 * the worker (`meshWorker.ts`) and the main thread's fallback both run.
 */
import { generalMap, mapTransform } from "./general";
import { geographicMesh, type MeshTile, type MeshTriangle } from "./mesh";

/** A plane rectangle, y up: west, south, east, north. */
export type PlaneBox = readonly [number, number, number, number];

export interface PlaneMeshRequest {
  /** The projection's id, custom definitions included: the worker rebuilds the transform from it. */
  projection: string;
  region: PlaneBox;
  /** Virtual pixels per plane degree. */
  scale: number;
  budget: number;
  spacing: number;
}

export interface PlaneMeshData {
  tiles: MeshTile[];
  /** Each tile's box in virtual pixels, beside `tiles`: left, top, right, bottom. */
  boxes: Array<[number, number, number, number]>;
  /** The geographic box of everything in it, longitudes unwrapped. */
  bounds: { west: number; east: number; south: number; north: number };
  /** Every triangle, twelve numbers each: x, y, lon, lat for three vertices. */
  triangles: Float32Array;
}

/** A place's position on a mesh's virtual canvas, given its plane position. */
export function virtualOf(region: PlaneBox, scale: number, plane: { x: number; y: number }): { x: number; y: number } {
  return { x: (plane.x - region[0]) * scale, y: (region[3] - plane.y) * scale };
}

export function buildPlaneMeshData(request: PlaneMeshRequest): PlaneMeshData {
  const general = generalMap(request.projection);
  if (!general) throw new Error(`no such projection: ${request.projection}`);
  const { region, scale } = request;
  const transform = mapTransform(general, { lon: 0, lat: 0 });
  const width = (region[2] - region[0]) * scale, height = (region[3] - region[1]) * scale;
  // The scale is only what `geographicMesh` picks its tile level from, and it
  // is exactly the top of a band: a hair under, so rounding cannot tip it over.
  const mesh = geographicMesh(width, height, scale * (1 - 1e-9),
    (p) => transform.inverse({ x: region[0] + p.x / scale, y: region[3] - p.y / scale }),
    (p) => { const xy = transform.forward(p); return xy ? virtualOf(region, scale, xy) : null; },
    request.budget, request.spacing);

  const boxes = mesh.tiles.map((tile) => {
    let left = Infinity, top = Infinity, right = -Infinity, bottom = -Infinity;
    for (let i = 0; i < tile.vertices.length; i += 4) {
      const x = tile.vertices[i + 2]!, y = tile.vertices[i + 3]!;
      if (x < left) left = x; if (x > right) right = x;
      if (y < top) top = y; if (y > bottom) bottom = y;
    }
    return [left, top, right, bottom] as [number, number, number, number];
  });
  const bounds = { west: Infinity, east: -Infinity, south: Infinity, north: -Infinity };
  const triangles = new Float32Array(mesh.triangles.length * 12);
  mesh.triangles.forEach((triangle, t) => {
    triangle.forEach((p, v) => {
      triangles.set([p.x, p.y, p.lon, p.lat], t * 12 + v * 4);
      if (p.lon < bounds.west) bounds.west = p.lon; if (p.lon > bounds.east) bounds.east = p.lon;
      if (p.lat < bounds.south) bounds.south = p.lat; if (p.lat > bounds.north) bounds.north = p.lat;
    });
  });
  if (!mesh.triangles.length) Object.assign(bounds, { west: -180, east: 180, south: -90, north: 90 });
  return { tiles: mesh.tiles, boxes, bounds, triangles };
}

/** The triangles back as vertices, for cutting an image out of the mesh. */
export function trianglesOf(data: Float32Array): MeshTriangle[] {
  const out: MeshTriangle[] = [];
  for (let i = 0; i < data.length; i += 12) {
    const vertex = (o: number) => ({ x: data[i + o]!, y: data[i + o + 1]!, lon: data[i + o + 2]!, lat: data[i + o + 3]! });
    out.push([vertex(0), vertex(4), vertex(8)]);
  }
  return out;
}

/** What to hand over rather than copy when the answer is posted from a worker. */
export function transferablesOf(data: PlaneMeshData): ArrayBuffer[] {
  return [data.triangles.buffer as ArrayBuffer, ...data.tiles.map((tile) => tile.vertices.buffer as ArrayBuffer)];
}

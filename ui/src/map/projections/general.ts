/** Parameterized, two-dimensional map projections. All coordinates exposed to
 * the camera are metres / METRES_PER_DEGREE; stored fields stay geographic.
 * Definitions are bundled: changing a view never performs a network request. */
import proj4 from "proj4";
import catalogue from "./crs.json";

export const METRES_PER_DEGREE = 6371229 * Math.PI / 180;
const DEG = Math.PI / 180;
const norm = (lon: number) => ((lon + 180) % 360 + 360) % 360 - 180;
export type XY = { x: number; y: number };
export type LL = { lon: number; lat: number };
export type Box = readonly [number, number, number, number];
export interface GeneralMap {
  id: string;
  label: string;
  group: string;
  definition: string;
  bbox: Box;
  code?: number;
  keywords?: string;
  /** Globe/azimuthal views turn with the camera; named CRSs retain their axes. */
  movable?: "orthographic" | "aeqd" | "laea" | "stere" | "gnom";
}
export interface MapTransform {
  forward(point: LL): XY | null;
  inverse(point: XY): LL | null;
}
const sphere = "+a=6371229 +b=6371229 +x_0=0 +y_0=0 +units=m +no_defs";
const world = (id: string, label: string, method: string): GeneralMap => ({
  id, label, group: "World maps", definition: `+proj=${method} +lon_0=0 ${sphere}`, bbox: [-180, -90, 180, 90],
});
export const GENERAL_MAPS: readonly GeneralMap[] = [
  { ...world("orthographic", "Globe (orthographic)", "ortho"), group: "Globe and azimuthal", movable: "orthographic" },
  world("robinson", "Robinson", "robin"),
  world("mollweide", "Mollweide", "moll"),
  world("winkel_tripel", "Winkel Tripel", "wintri"),
  world("equal_earth", "Equal Earth", "eqearth"),
  world("sinusoidal", "Sinusoidal", "sinu"),
  ...([ ["azimuthal_equidistant", "Azimuthal equidistant", "aeqd"],
    ["azimuthal_equal_area", "Lambert azimuthal equal-area", "laea"],
    ["stereographic", "Stereographic", "stere"], ["gnomonic", "Gnomonic", "gnom"] ] as const).map(([id,label,movable]) => ({
      id, label, movable, group: "Globe and azimuthal", definition: `+proj=${movable} +lat_0=0 +lon_0=0 ${sphere}`,
      bbox: [-180,-90,180,90] as Box,
    })),
  ...catalogue.map(row => ({ ...row, bbox: row.bbox as unknown as Box })),
];
const byId = new Map(GENERAL_MAPS.map(p => [p.id,p]));

/** Custom definitions are stored in the same view preference as a preset ID. */
export function customMap(definition: string): GeneralMap {
  const text = definition.trim();
  if (!text || text.length > 8192) throw new Error("Enter a PROJ or WKT definition (up to 8192 characters).");
  const epsg = /^(?:EPSG:)?(\d+)$/i.exec(text);
  if (epsg) {
    const preset = byId.get(`epsg_${Number(epsg[1])}`);
    if (!preset) throw new Error(`EPSG:${epsg[1]} is not bundled. Paste its PROJ or WKT definition.`);
    return preset;
  }
  const parsed = new proj4.Proj(text) as unknown as {projName?: string; names?: string[]; long0?: number; lat0?: number; zone?: number};
  if (["longlat", "identity", "geocent"].includes(parsed.projName ?? "")) {
    throw new Error("Choose a projected map definition. Use Equirectangular for a longitude/latitude view.");
  }
  // A custom regional definition has no EPSG area of use. Start around its
  // declared origin, with a useful regional extent instead of its antipode.
  const regional = ["tmerc","etmerc","utm","lcc","omerc","somerc","sterea","nzmg","krovak"].some(name => name === parsed.projName || parsed.names?.includes(name));
  const lon = parsed.long0 !== undefined ? parsed.long0 / DEG : parsed.zone ? parsed.zone * 6 - 183 : 0;
  const lat = (parsed.lat0 ?? 0) / DEG;
  const bbox: Box = regional ? [lon-10,Math.max(-85,lat-10),lon+10,Math.min(85,lat+10)] : [-180,-80,180,80];
  const map: GeneralMap = {id: `custom:${encodeURIComponent(text)}`, label: "Custom projection", group: "Custom", definition:text, bbox};
  const transform=mapTransform(map,defaultCentre(map));
  const centre=defaultCentre(map),xy=transform.forward(centre);
  if(!xy || !transform.inverse(xy))throw new Error("This definition cannot be projected and inverted. Check its parameters and datum grids.");
  return map;
}
export function generalMap(id: string): GeneralMap | undefined {
  if (id.startsWith("custom:")) {
    try { return customMap(decodeURIComponent(id.slice(7))); } catch { return undefined; }
  }
  return byId.get(id);
}

function winkel(lon: number, lat: number): XY {
  const a = Math.acos(Math.max(-1, Math.min(1, Math.cos(lat) * Math.cos(lon / 2))));
  const sinc = a < 1e-10 ? 1 : Math.sin(a) / a;
  return { x: (lon * 2 / Math.PI + 2 * Math.cos(lat) * Math.sin(lon / 2) / sinc) / 2,
    y: (lat + Math.sin(lat) / sinc) / 2 };
}
const winkelTransform: MapTransform = {
  forward: p => {
    const xy = winkel(norm(p.lon) * DEG, p.lat * DEG);
    return { x: xy.x / DEG, y: xy.y / DEG };
  },
  inverse: p => {
    const x = p.x * DEG, y = p.y * DEG;
    let lon = x, lat = y;
    for (let i = 0; i < 20; i++) {
      const f = winkel(lon, lat), h = 1e-6;
      const a = winkel(lon + h, lat), b = winkel(lon, lat + h);
      const dx = f.x - x, dy = f.y - y;
      if (Math.hypot(dx, dy) < 1e-11) break;
      const ax = (a.x-f.x)/h, ay = (a.y-f.y)/h, bx = (b.x-f.x)/h, by = (b.y-f.y)/h;
      const determinant = ax*by-ay*bx;
      if (Math.abs(determinant) < 1e-12) return null;
      lon -= (dx*by-dy*bx)/determinant; lat -= (dy*ax-dx*ay)/determinant;
      if (Math.abs(lon)>Math.PI+0.1 || Math.abs(lat)>Math.PI/2+0.1) return null;
    }
    if (Math.abs(lon)>Math.PI+1e-8 || Math.abs(lat)>Math.PI/2+1e-8) return null;
    return {lon: lon/DEG, lat: lat/DEG};
  },
};

/** Spherical azimuthals share the same aspect/orientation and inverse. */
function azimuthal(method: NonNullable<GeneralMap["movable"]>, origin: LL): MapTransform {
  const phi0=origin.lat*DEG, sin0=Math.sin(phi0), cos0=Math.cos(phi0);
  const maxAngle = method === "orthographic" ? Math.PI/2 : method === "gnom" ? 80*DEG
    : method === "stere" ? 150*DEG : Math.PI-1e-6;
  const radius = (c: number) => method === "orthographic" ? Math.sin(c) : method === "aeqd" ? c
    : method === "laea" ? 2*Math.sin(c/2) : method === "stere" ? 2*Math.tan(c/2) : Math.tan(c);
  return {
    forward(p) {
      const phi=p.lat*DEG, lambda=norm(p.lon-origin.lon)*DEG;
      const cosc=Math.max(-1,Math.min(1,sin0*Math.sin(phi)+cos0*Math.cos(phi)*Math.cos(lambda)));
      const c=Math.acos(cosc);
      if (c>maxAngle+1e-9) return null;
      const k=c<1e-8 ? 1 : radius(c)/Math.sin(c);
      return {x:k*Math.cos(phi)*Math.sin(lambda)/DEG,
        y:k*(cos0*Math.sin(phi)-sin0*Math.cos(phi)*Math.cos(lambda))/DEG};
    },
    inverse(p) {
      const x=p.x*DEG,y=p.y*DEG,r=Math.hypot(x,y);
      if (r>radius(maxAngle)+1e-8) return null;
      if (r<1e-12) return {...origin};
      const c=method === "orthographic" ? Math.asin(Math.min(1,r)) : method === "aeqd" ? r
        : method === "laea" ? 2*Math.asin(Math.min(1,r/2)) : method === "stere" ? 2*Math.atan(r/2) : Math.atan(r);
      return {lon:norm(origin.lon+Math.atan2(x*Math.sin(c),r*cos0*Math.cos(c)-y*sin0*Math.sin(c))/DEG),
        lat:Math.asin(Math.max(-1,Math.min(1,Math.cos(c)*sin0+y*Math.sin(c)*cos0/r)))/DEG};
    },
  };
}
const transforms = new Map<string,MapTransform>();
export function mapTransform(map: GeneralMap, origin: LL): MapTransform {
  const key=map.movable ? `${map.id}/${origin.lon}/${origin.lat}` : map.id;
  const cached=transforms.get(key); if (cached) return cached;
  let transform: MapTransform;
  if (map.movable) transform=azimuthal(map.movable,origin);
  else if (map.id === "winkel_tripel") transform=winkelTransform;
  else {
    const converter=proj4(map.definition);
    const finite=(p: number[]) => Number.isFinite(p[0]) && Number.isFinite(p[1]);
    transform={
      forward(p) {
        if (!Number.isFinite(p.lon) || !Number.isFinite(p.lat) || Math.abs(p.lat)>90) return null;
        try { const xy=converter.forward([norm(p.lon),p.lat]);
          return finite(xy) ? {x:xy[0]!/METRES_PER_DEGREE,y:xy[1]!/METRES_PER_DEGREE} : null;
        } catch {return null;}
      },
      inverse(p) {
        try {
          const ll=converter.inverse([p.x*METRES_PER_DEGREE,p.y*METRES_PER_DEGREE]);
          if (!finite(ll) || Math.abs(ll[1]!)>90+1e-8 || Math.abs(ll[0]!)>180+1e-6) return null;
          const back=converter.forward(ll);
          if (!finite(back) || Math.hypot(back[0]!/METRES_PER_DEGREE-p.x,back[1]!/METRES_PER_DEGREE-p.y)>1e-5) return null;
          return {lon:norm(ll[0]!),lat:Math.max(-90,Math.min(90,ll[1]!))};
        } catch {return null;}
      },
    };
  }
  transforms.set(key,transform);
  if (transforms.size>64) transforms.delete(transforms.keys().next().value!);
  return transform;
}
export function defaultCentre(map: GeneralMap): LL {
  if (map.movable) return {lon:0,lat:20};
  if (map.group === "Polar and sea ice") {
    const lat=Number(/\+lat_0=([^ ]+)/.exec(map.definition)?.[1] ?? (map.bbox[1] < 0 ? -90 : 90));
    const lon=Number(/\+lon_0=([^ ]+)/.exec(map.definition)?.[1] ?? 0);
    return {lon:norm(lon),lat};
  }
  let [west,south,east,north]=map.bbox;
  if (east<west) east+=360;
  return {lon:norm((west+east)/2),lat:(south+north)/2};
}
const extents=new Map<string,Box>();
export function mapExtent(map: GeneralMap): Box {
  const cached=extents.get(map.id);if(cached)return cached;
  if(map.movable){
    const r=map.movable==='orthographic'?1:map.movable==='aeqd'?Math.PI:map.movable==='laea'?2:map.movable==='stere'?2:Math.tan(60*DEG);
    return [-r/DEG,-r/DEG,r/DEG,r/DEG];
  }
  const transform=mapTransform(map,defaultCentre(map));
  let [west,south,east,north]=map.bbox;if(east<west)east+=360;
  const points:XY[]=[];
  for(let j=0;j<=12;j++)for(let i=0;i<=24;i++){
    // Avoid normalising +180 to -180 when calculating an entire world width.
    const lon=west+(east-west)*i/24;
    const p=transform.forward({lon:lon===180?179.999999:lon,lat:south+(north-south)*j/12});if(p)points.push(p);
  }
  const box:Box=points.length ? [Math.min(...points.map(p=>p.x)),Math.min(...points.map(p=>p.y)),Math.max(...points.map(p=>p.x)),Math.max(...points.map(p=>p.y))] : [-180,-90,180,90];
  extents.set(map.id,box);return box;
}

/**
 * How the map draws the earth (spec.md 5.1, M11).
 *
 * Switching the view leaves stored geometry and exports unchanged. Ground
 * tools use geodesic frames; pixel tools freeze their creation projection.
 * The evaluator works in lat/lon and true bearings, and the GRIB writer has
 * its own grid (invariant 3).
 *
 * Cylindrical projections keep longitude linear, so tiles, dateline wrapping,
 * and map editing share one coordinate system. Each is uniformly rescaled to
 * x = longitude in degrees; standard parallels still retain the projection's
 * correct aspect ratio. Vertical scale need not equal one at the equator.
 */

import { GENERAL_MAPS, generalMap, type GeneralMap } from "./projections/general";
import { AZIMUTHAL_MODE, GPU_AZIMUTHALS } from "./projectionShaders";

/** The original cylindrical spaces are also persisted by pixel tools. */
export type CylindricalProjectionId =
  | "equirectangular" | "mercator" | "miller"
  | "lambert" | "behrmann" | "gall_peters" | "hobo_dyer"
  | "gall_stereographic" | "braun" | "central_cylindrical"
  | "patterson" | "compact_miller" | "equidistant_30" | "equidistant_45";

export type ProjectionId = CylindricalProjectionId | `epsg_${number}` | `custom:${string}`
  | "orthographic" | "robinson" | "mollweide" | "winkel_tripel" | "equal_earth" | "sinusoidal"
  | "azimuthal_equidistant" | "azimuthal_equal_area" | "stereographic" | "gnomonic"
  | "hawaii_harn_utm4" | "hawaii_albers";

/** One projection: latitude to and from the vertical map coordinate. */
export interface Projection {
  id: ProjectionId;
  /** A two-dimensional projection, rendered through an adaptive geographic mesh. */
  general?: GeneralMap;
  /** What the menu calls it. */
  label: string;
  /**
   * The number the shader branches on, sent as the `uProjection` uniform.
   *
   * Here rather than in the renderer, so the formula and the number that
   * selects it are one object: a projection cannot be given a branch the GLSL
   * does not have without this line being written.
   */
  mode: number;
  /**
   * The vertical map coordinate of a latitude, in units of longitudinal degrees.
   * Increases northward.
   */
  yOf: (lat: number) => number;
  /** And back again. */
  latOf: (y: number) => number;
  /**
   * `dy/dlat` at a latitude: how many degrees of `y` a degree of latitude is
   * worth there.
   *
   * The overlay needs it. A footprint is drawn as an ellipse about a centre,
   * and its vertical radius is a latitude span — which is not a constant number
   * of pixels once `y` is not the latitude. Using the derivative at the centre
   * keeps the ellipse symmetric, at the cost of being exact only to second
   * order; a stamp large enough for that to show is larger than any brush.
   */
  scaleAt: (lat: number) => number;
  /**
   * The furthest north this projection can draw.
   *
   * Mercator sends the poles to infinity, so it stops short of them — and the
   * grid's top and bottom rows then sit off the map, which the UI says rather
   * than leaving the user to notice.
   */
  maxLat: number;
}

const DEG = Math.PI / 180;
const clamp = (value: number, limit: number) => Math.max(-limit, Math.min(limit, value));

/** Equirectangular: `y` *is* the latitude. The projection the app grew up in. */
const EQUIRECTANGULAR: Projection & { id: CylindricalProjectionId } = {
  id: "equirectangular",
  label: "Equirectangular",
  mode: 0,
  yOf: (lat) => lat,
  latOf: (y) => y,
  scaleAt: () => 1,
  maxLat: 90,
};

/**
 * Mercator, in degree units: `y = ln(tan(45° + φ/2))` scaled so that the
 * equator's scale matches equirectangular's.
 *
 * Conformal, which is why every marine chart uses it — a bearing drawn on it
 * is the bearing sailed. It cannot reach a pole: `tan` goes to infinity, so
 * the latitude clamps.
 */
const MERCATOR: Projection & { id: CylindricalProjectionId } = {
  id: "mercator",
  label: "Mercator",
  mode: 1,
  yOf: (lat) => {
    const clamped = Math.min(Math.max(lat, -MERCATOR_MAX), MERCATOR_MAX);
    return Math.log(Math.tan(Math.PI / 4 + (clamped * DEG) / 2)) / DEG;
  },
  latOf: (y) => (2 * Math.atan(Math.exp(y * DEG)) - Math.PI / 2) / DEG,
  // d/dφ ln(tan(45° + φ/2)) = sec φ. The conformal scale factor itself, which
  // is why a geodesic circle stays a circle on a Mercator map.
  scaleAt: (lat) => 1 / Math.cos(Math.min(Math.max(lat, -MERCATOR_MAX), MERCATOR_MAX) * DEG),
  maxLat: 85.051_129,
};

/** Where Mercator stops. The Web Mercator limit, which makes the world square. */
const MERCATOR_MAX = 85.051_129;

/**
 * Miller: Mercator's latitude stretched by 4/5 and unstretched by 5/4.
 *
 * Not conformal, and deliberately: the compromise buys the poles back, so a
 * global wind field is drawn whole with far less of Mercator's polar
 * exaggeration.
 */
const MILLER: Projection & { id: CylindricalProjectionId } = {
  id: "miller",
  label: "Miller",
  mode: 2,
  yOf: (lat) => {
    const clamped = Math.min(Math.max(lat, -90), 90);
    return (1.25 * Math.log(Math.tan(Math.PI / 4 + 0.4 * clamped * DEG))) / DEG;
  },
  latOf: (y) => (2.5 * Math.atan(Math.exp(0.8 * y * DEG)) - 0.625 * Math.PI) / DEG,
  // The same derivative with the 4/5 stretch carried through: sec(0.8 φ).
  scaleAt: (lat) => 1 / Math.cos(0.8 * Math.min(Math.max(lat, -90), 90) * DEG),
  maxLat: 90,
};

/** Equal-area cylinders, uniformly rescaled from x = cos(standard) * longitude. */
function equalArea(id: CylindricalProjectionId, label: string, mode: number, standard: number): Projection & { id: CylindricalProjectionId } {
  const k = Math.cos(standard * DEG) ** 2;
  return { id, label, mode, maxLat: 90,
    yOf: lat => Math.sin(clamp(lat, 90) * DEG) / (DEG * k),
    latOf: y => Math.asin(clamp(y * DEG * k, 1)) / DEG,
    scaleAt: lat => Math.cos(clamp(lat, 90) * DEG) / k,
  };
}

/** Perspective cylinders: Gall uses standard parallels at 45°, Braun at 0°. */
function stereographic(id: CylindricalProjectionId, label: string, mode: number, k: number): Projection & { id: CylindricalProjectionId } {
  return { id, label, mode, maxLat: 90,
    yOf: lat => k * Math.tan(clamp(lat, 90) * DEG / 2) / DEG,
    latOf: y => clamp(2 * Math.atan(y * DEG / k) / DEG, 90),
    scaleAt: lat => k / (2 * Math.cos(clamp(lat, 90) * DEG / 2) ** 2),
  };
}

function equidistant(id: CylindricalProjectionId, label: string, mode: number, standard: number): Projection & { id: CylindricalProjectionId } {
  const k = Math.cos(standard * DEG);
  return { id, label, mode, maxLat: 90,
    yOf: lat => clamp(lat, 90) / k,
    latOf: y => clamp(y * k, 90),
    scaleAt: () => 1 / k,
  };
}

/** The published Patterson and Compact Miller polynomials, in radians.
 * Coefficients verified against OSGeo PROJ's patterson.cpp and comill.cpp.
 * Newton's method is bounded and clamped, including outside the world. */
function polynomial(id: CylindricalProjectionId, label: string, mode: number, coefficients: readonly number[]): Projection & { id: CylindricalProjectionId } {
  const forward = (phi: number) => coefficients.reduce((sum, c, i) => sum + c * phi ** (2 * i + 1), 0);
  const slope = (phi: number) => coefficients.reduce((sum, c, i) => sum + (2 * i + 1) * c * phi ** (2 * i), 0);
  const maxY = forward(Math.PI / 2);
  return { id, label, mode, maxLat: 90,
    yOf: lat => forward(clamp(lat, 90) * DEG) / DEG,
    latOf: y => {
      const target = clamp(y * DEG, maxY);
      let phi = clamp(target, Math.PI / 2);
      for (let i = 0; i < 12; i++) phi = clamp(phi - (forward(phi) - target) / slope(phi), Math.PI / 2);
      return phi / DEG;
    },
    scaleAt: lat => slope(clamp(lat, 90) * DEG),
  };
}

/** Stable order: mode + 1 is the persisted pixel-tool space. Append only. */
export const PROJECTIONS: readonly (Projection & { id: CylindricalProjectionId })[] = [
  EQUIRECTANGULAR, MERCATOR, MILLER,
  equalArea("lambert", "Lambert cylindrical equal-area", 3, 0),
  equalArea("behrmann", "Behrmann", 4, 30),
  equalArea("gall_peters", "Gall–Peters", 5, 45),
  equalArea("hobo_dyer", "Hobo–Dyer", 6, 37.5),
  stereographic("gall_stereographic", "Gall stereographic", 7, 1 + Math.SQRT2),
  stereographic("braun", "Braun stereographic", 8, 2),
  { id: "central_cylindrical", label: "Central cylindrical (±80°)", mode: 9, maxLat: 80,
    yOf: lat => Math.tan(clamp(lat, 80) * DEG) / DEG,
    latOf: y => Math.atan(y * DEG) / DEG,
    scaleAt: lat => 1 / Math.cos(clamp(lat, 80) * DEG) ** 2,
  },
  polynomial("patterson", "Patterson", 10, [1.0148, 0, 0.23185, -0.14499, 0.02406]),
  polynomial("compact_miller", "Compact Miller", 11, [0.9902, 0.1604, -0.03054]),
  equidistant("equidistant_30", "Equidistant cylindrical (30°)", 12, 30),
  equidistant("equidistant_45", "Equidistant cylindrical (45°)", 13, 45),
];

/** Index zero is geodesic; the original projected/mercator/miller indices stay fixed. */
export const STAMP_SPACES = ["geodesic", "projected", ...PROJECTIONS.slice(1).map(p => p.id)] as const;

/** What a camera with no projection of its own is drawn in. */
export const DEFAULT_PROJECTION: ProjectionId = "equirectangular";

/** The projection with an id, falling back to the one the app grew up in. */
const generalProjections = new Map<string, Projection>();
function fromGeneral(map: GeneralMap): Projection {
  const existing = generalProjections.get(map.id);
  if (existing) return existing;
  const projection: Projection = {
    id: map.id as ProjectionId, label: map.label, mode: 14, general: map, maxLat: 90,
    // Geographic latitude helpers remain available for ground-tool sizing.
    // All camera and rendering coordinates use mapTransform for these maps.
    yOf: lat => lat, latOf: y => Math.max(-90, Math.min(90, y)), scaleAt: () => 1,
  };
  generalProjections.set(map.id, projection);
  return projection;
}
export const MAP_PROJECTIONS: readonly Projection[] = [...PROJECTIONS, ...GENERAL_MAPS.map(fromGeneral)];
export function projectionOf(id: ProjectionId): Projection {
  const cylindrical = PROJECTIONS.find(p => p.id === id);
  if (cylindrical) return cylindrical;
  const cached = generalProjections.get(id);
  if (cached) return cached;
  const map = generalMap(id);
  return map ? fromGeneral(map) : EQUIRECTANGULAR;
}

/**
 * How much taller than 180° the whole world is in this projection.
 *
 * The zoom floor and the vertical clamp both need it: Mercator's world is
 * 360° tall on this scale, not 180, so "fit the world" is a different number.
 */
export function worldHeightDeg(projection: Projection): number {
  return projection.yOf(projection.maxLat) - projection.yOf(-projection.maxLat);
}

/**
 * The mode the shaders branch on, which is not always `Projection.mode`.
 *
 * `mode` is persisted — a pixel tool's stamp space is `mode + 1` — and every
 * general projection shares 14 there, the globe included. The shaders need
 * to tell the globe and its azimuthal kin apart from the rest, because those
 * five are projected on the GPU (projectionShaders.ts) rather than from a
 * mesh built here. So the distinction is made at the uniform and nowhere a
 * file can see it.
 */
export function shaderMode(projection: Projection): number {
  const movable = projection.general?.movable;
  return movable ? AZIMUTHAL_MODE + GPU_AZIMUTHALS.indexOf(movable) : projection.mode;
}

/** Whether the GPU projects this one itself: the globe and the azimuthals. */
export function projectedOnGpu(projection: Projection): boolean {
  return Boolean(projection.general?.movable);
}

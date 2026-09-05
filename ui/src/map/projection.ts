/**
 * How the map draws the earth (spec.md 5.1, M11).
 *
 * **Nothing below the view is coupled to this.** Objects are stored in
 * geodesic AEQD frames, the evaluator works in lat/lon and true bearings, and
 * the GRIB writer has its own grid — so a projection cannot affect a stored
 * project or an exported file (invariant 3). The whole cost is here.
 *
 * # Why this family and no other, yet
 *
 * Every projection in this module is **cylindrical**: longitude maps linearly
 * to `x`, and latitude maps to `y` through a function of latitude alone. That
 * one property is what keeps everything above it unchanged — a lat/lon
 * rectangle is still an axis-aligned screen rectangle, so a tile is still two
 * triangles, the world still repeats horizontally, and the inverse is still
 * closed-form and exact.
 *
 * The pseudo-cylindrical projections (Mollweide, Robinson, Winkel Tripel) and
 * the globe break all four at once: curved quads need subdivision, two of them
 * have no closed-form inverse, and a globe needs back-face culling, spherical
 * tile selection and a different interaction model. They are not here.
 *
 * # `y` is in *degrees*, not in pixels
 *
 * Every projection reports `y` on the same scale equirectangular uses —
 * degrees of latitude at the equator — so the camera's `pxPerDeg`, the zoom
 * limits, the tile ladder and the glyph spacing all keep meaning exactly what
 * they meant. A projection changes where a latitude lands, not what a pixel is
 * worth.
 */

/** The projections the map offers. */
export type ProjectionId = "equirectangular" | "mercator" | "miller";

/** One projection: latitude to and from the vertical map coordinate. */
export interface Projection {
  id: ProjectionId;
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
   * The vertical map coordinate of a latitude, in degrees at the equator.
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

/** Equirectangular: `y` *is* the latitude. The projection the app grew up in. */
const EQUIRECTANGULAR: Projection = {
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
const MERCATOR: Projection = {
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
const MILLER: Projection = {
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

/** Every projection, in the order the menu offers them. */
export const PROJECTIONS: readonly Projection[] = [EQUIRECTANGULAR, MERCATOR, MILLER];

/** What a camera with no projection of its own is drawn in. */
export const DEFAULT_PROJECTION: ProjectionId = "equirectangular";

/** The projection with an id, falling back to the one the app grew up in. */
export function projectionOf(id: ProjectionId): Projection {
  return PROJECTIONS.find((p) => p.id === id) ?? EQUIRECTANGULAR;
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

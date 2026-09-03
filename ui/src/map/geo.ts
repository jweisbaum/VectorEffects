/**
 * Spherical geodesy for interaction.
 *
 * Mirrors `ve_core::geo`, which is the authority: every value that reaches the
 * document goes through Rust. This exists because dragging a rotate handle has
 * to convert a cursor position into a *true bearing*, and doing that in screen
 * space is wrong away from the equator — at 60°N a degree of longitude is half
 * the ground distance of a degree of latitude, so a handle dragged "up and to
 * the right" is not at 45°.
 *
 * Same earth radius as the rest of the application, and as the exported GRIB.
 */

/** Earth radius in metres. Mirrors `ve_core::geo::EARTH_RADIUS_M`. */
export const EARTH_RADIUS_M = 6371229.0;

const DEG = Math.PI / 180;

export interface GeoPoint {
  lon: number;
  lat: number;
}

/** Normalises a longitude into [-180, 180). */
export function normalizeLon(lon: number): number {
  return ((((lon + 180) % 360) + 360) % 360) - 180;
}

/** Great-circle distance in metres. */
export function distanceM(from: GeoPoint, to: GeoPoint): number {
  const phi1 = from.lat * DEG;
  const phi2 = to.lat * DEG;
  const dPhi = phi2 - phi1;
  // Normalising the longitude delta is what makes the dateline ordinary.
  const dLambda = normalizeLon(to.lon - from.lon) * DEG;

  const a =
    Math.sin(dPhi / 2) ** 2 +
    Math.cos(phi1) * Math.cos(phi2) * Math.sin(dLambda / 2) ** 2;
  return 2 * EARTH_RADIUS_M * Math.asin(Math.min(1, Math.max(-1, Math.sqrt(a))));
}

/** Initial great-circle bearing, degrees clockwise from north. */
export function initialBearing(from: GeoPoint, to: GeoPoint): number {
  const phi1 = from.lat * DEG;
  const phi2 = to.lat * DEG;
  const dLambda = normalizeLon(to.lon - from.lon) * DEG;

  const y = Math.sin(dLambda) * Math.cos(phi2);
  const x = Math.cos(phi1) * Math.sin(phi2) - Math.sin(phi1) * Math.cos(phi2) * Math.cos(dLambda);
  return (((Math.atan2(y, x) / DEG) % 360) + 360) % 360;
}

/** The point reached by travelling `distance` metres along `bearing`. */
export function destination(from: GeoPoint, bearingDeg: number, distance: number): GeoPoint {
  const delta = distance / EARTH_RADIUS_M;
  const theta = bearingDeg * DEG;
  const phi1 = from.lat * DEG;
  const lambda1 = from.lon * DEG;

  const sinPhi2 =
    Math.sin(phi1) * Math.cos(delta) + Math.cos(phi1) * Math.sin(delta) * Math.cos(theta);
  const phi2 = Math.asin(Math.min(1, Math.max(-1, sinPhi2)));
  const lambda2 =
    lambda1 +
    Math.atan2(
      Math.sin(theta) * Math.sin(delta) * Math.cos(phi1),
      Math.cos(delta) - Math.sin(phi1) * sinPhi2,
    );

  return { lon: normalizeLon(lambda2 / DEG), lat: phi2 / DEG };
}

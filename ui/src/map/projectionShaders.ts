/** Cylindrical formulas matching projection.ts; x is normalised to longitude. */
export const EXTRA_CYLINDRICAL = `
float equalAreaK(int mode) {
  if (mode == 3) return 1.00000000000000000;
  if (mode == 4) return 0.75000000000000011;
  if (mode == 5) return 0.50000000000000011;
  if (mode == 6) return 0.62940952255126037;
  return 1.0;
}
vec2 cylindricalPolynomial(int mode, float phi) {
  float q = phi * phi;
  if (mode == 10) return vec2(
    phi * (1.0148 + q * q * (0.23185 + q * (-0.14499 + 0.02406 * q))),
    1.0148 + q * q * (1.15925 + q * (-1.01493 + 0.21654 * q)));
  return vec2(phi * (0.9902 + q * (0.1604 - 0.03054 * q)),
    0.9902 + q * (0.4812 - 0.1527 * q));
}
float cylindricalY(int mode, float lat) {
  float phi = clamp(lat, -90.0, 90.0) * VE_DEG;
  if (mode >= 3 && mode <= 6) return sin(phi) / (VE_DEG * equalAreaK(mode));
  if (mode == 7) return 2.414213562373095 * tan(phi * 0.5) / VE_DEG;
  if (mode == 8) return 2.0 * tan(phi * 0.5) / VE_DEG;
  if (mode == 9) return tan(clamp(lat, -80.0, 80.0) * VE_DEG) / VE_DEG;
  if (mode == 10 || mode == 11) return cylindricalPolynomial(mode, phi).x / VE_DEG;
  if (mode == 12) return clamp(lat, -90.0, 90.0) / 0.8660254037844386;
  if (mode == 13) return clamp(lat, -90.0, 90.0) / 0.7071067811865476;
  return lat;
}
float cylindricalLat(int mode, float degreesY) {
  float y = degreesY * VE_DEG;
  if (mode >= 3 && mode <= 6) return asin(clamp(y * equalAreaK(mode), -1.0, 1.0)) / VE_DEG;
  if (mode == 7) return clamp(2.0 * atan(y / 2.414213562373095) / VE_DEG, -90.0, 90.0);
  if (mode == 8) return clamp(2.0 * atan(y / 2.0) / VE_DEG, -90.0, 90.0);
  if (mode == 9) return atan(y) / VE_DEG;
  if (mode == 10 || mode == 11) {
    float limit = 1.5707963267948966;
    float maxY = cylindricalPolynomial(mode, limit).x;
    float target = clamp(y, -maxY, maxY);
    float phi = clamp(target, -limit, limit);
    for (int i = 0; i < 12; i++) {
      vec2 value = cylindricalPolynomial(mode, phi);
      phi = clamp(phi - (value.x - target) / value.y, -limit, limit);
    }
    return phi / VE_DEG;
  }
  if (mode == 12) return clamp(degreesY * 0.8660254037844386, -90.0, 90.0);
  if (mode == 13) return clamp(degreesY * 0.7071067811865476, -90.0, 90.0);
  return degreesY;
}
`;

/**
 * The first shader mode of the azimuthal family, in GPU_AZIMUTHALS order.
 *
 * Mode 14 is a general projection whose screen positions come from a mesh
 * built on the CPU. A globe cannot afford one: turning it changes the mapping
 * itself, so that mesh was rebuilt on every pointer move — 600 ms a frame
 * with the whole earth in view. These five are closed forms, so the vertex
 * shader projects a static grid instead and turning costs the CPU nothing.
 */
export const AZIMUTHAL_MODE = 15;

/** The movable projections of general.ts, in shader-mode order. Append only. */
export const GPU_AZIMUTHALS = ["orthographic", "aeqd", "laea", "stere", "gnom"] as const;

/**
 * The spherical azimuthals, forward and back: the port of `azimuthal` in
 * projections/general.ts, decision for decision, and held to it by
 * shaders.test.ts. Expects uCamera, uViewport, uProjection and VE_DEG.
 *
 * The forward is written around the differences from the centre rather than
 * the textbook sums. cos0 sin(phi) - sin0 cos(phi) cos(lam) is two numbers
 * near a half whose difference, at a close zoom, is a few millionths: in
 * single precision that is the map jittering as it turns. The same quantity
 * is sin(phi - phi0) + sin0 cos(phi) (1 - cos(lam)), which is small terms
 * added, and 1 - cos(lam) is taken as 2 sin^2(lam / 2) for the same reason.
 * (No backticks in here: this is a template literal, and one would end it.)
 */
export const AZIMUTHAL = `
uniform vec3 uOrigin;   // centre latitude in degrees, its sine, its cosine
uniform float uRim;     // radians kept clear of the projection's own limit

// Set by azimuthalScreen: positive where the point is on the visible side.
// A vertex shader hands it to the fragment stage, which discards below zero.
float veHorizon = 1.0;

float azimuthalLimit(int mode) {
  if (mode == 15) return 1.5707963267948966;   // orthographic: the hemisphere
  if (mode == 18) return 2.6179938779914944;   // stereographic: 150 degrees
  if (mode == 19) return 1.3962634015954636;   // gnomonic: 80 degrees
  return 3.141591653589793;                    // pi - 1e-6: all but the antipode
}

float azimuthalRadius(int mode, float c) {
  if (mode == 15) return sin(c);
  if (mode == 16) return c;
  if (mode == 17) return 2.0 * sin(c * 0.5);
  if (mode == 18) return 2.0 * tan(c * 0.5);
  return tan(c);
}

float azimuthalAngle(int mode, float r) {
  if (mode == 15) return asin(min(1.0, r));
  if (mode == 16) return r;
  if (mode == 17) return 2.0 * asin(min(1.0, r * 0.5));
  if (mode == 18) return 2.0 * atan(r * 0.5);
  return atan(r);
}

// Plane coordinates in radians of arc at the centre, x east and y north,
// with the angular distance from the centre in z.
vec3 azimuthalPlane(vec2 lonLat) {
  float lam = (mod(lonLat.x - uCamera.x + 180.0, 360.0) - 180.0) * VE_DEG;
  float phi = lonLat.y * VE_DEG;
  float dphi = (lonLat.y - uOrigin.x) * VE_DEG;
  float h = sin(lam * 0.5);
  float versine = 2.0 * h * h;
  float cphi = cos(phi);
  float x = cphi * sin(lam);
  float y = sin(dphi) + uOrigin.y * cphi * versine;
  float cosc = cos(dphi) - uOrigin.z * cphi * versine;
  float sinc = length(vec2(x, y));
  float c = atan(sinc, cosc);
  // Past the limit a point has no place on the map, but its triangle still
  // has to go somewhere: held just beyond the rim, where every fragment of
  // it is discarded, rather than at the tangent's far side of infinity.
  float held = min(c, azimuthalLimit(uProjection) + 0.06);
  float k = sinc < 1e-7 ? 1.0 : azimuthalRadius(uProjection, held) / sinc;
  return vec3(k * x, k * y, c);
}

vec2 azimuthalScreen(vec2 lonLat) {
  vec3 plane = azimuthalPlane(lonLat);
  veHorizon = azimuthalLimit(uProjection) - uRim - plane.z;
  return vec2(
    uViewport.x * 0.5 + plane.x / VE_DEG * uCamera.z,
    uViewport.y * 0.5 - plane.y / VE_DEG * uCamera.z
  );
}

// The place under a pixel, or a longitude past 1000 where the map has none.
vec2 azimuthalInverse(vec2 screen) {
  float x = (screen.x - uViewport.x * 0.5) / uCamera.z * VE_DEG;
  float y = (uViewport.y * 0.5 - screen.y) / uCamera.z * VE_DEG;
  float r = length(vec2(x, y));
  if (r > azimuthalRadius(uProjection, azimuthalLimit(uProjection) - uRim)) return vec2(9999.0);
  if (r < 1e-12) return vec2(uCamera.x, uOrigin.x);
  float c = azimuthalAngle(uProjection, r);
  float sinc = sin(c);
  float cosc = cos(c);
  float lon = uCamera.x + atan(x * sinc, r * uOrigin.z * cosc - y * uOrigin.y * sinc) / VE_DEG;
  float lat = asin(clamp(cosc * uOrigin.y + y * sinc * uOrigin.z / r, -1.0, 1.0)) / VE_DEG;
  return vec2(mod(lon + 180.0, 360.0) - 180.0, lat);
}
`;

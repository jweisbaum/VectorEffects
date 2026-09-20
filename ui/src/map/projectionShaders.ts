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

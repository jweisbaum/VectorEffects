/**
 * GLSL sources for the map renderer.
 *
 * All programs share one projection helper, with the camera as
 * (centreLon, centreY, pxPerDeg). Longitude is *not* normalised in the vertex
 * shaders — the renderer draws the world three times at offsets of -360, 0 and
 * +360 instead. That is what makes panning across the dateline seamless
 * without any wrapping special case in the geometry.
 *
 * **The camera's second component is the projection's `y`, not a latitude.**
 * Every projection the map offers is cylindrical (M11), so longitude is linear
 * in `x` and latitude reaches `y` through a function of latitude alone; the
 * shader evaluates that function and everything else is unchanged. The mode
 * is a uniform rather than a program per projection, because the branch is
 * taken identically by every vertex in a draw, and a shader that differs from
 * another only in one `if` is a shader worth not having twice.
 *
 * `ui/src/map/projection.ts` holds the same three formulas in TypeScript, for
 * the overlay and the pointer. Change one and change the other, or the arrows
 * stop landing where the field is.
 */

/** Shared projection helper, prefixed to every vertex shader. */
const PROJECTION = `
uniform vec3 uCamera;      // centreLon, centreY, pxPerDeg
uniform vec2 uViewport;    // width, height in device pixels
uniform float uLonOffset;  // world copy: -360, 0 or 360
// Explicitly highp, and not because it needs the range. An int's *default*
// precision is highp in a vertex shader and mediump in a fragment one, so a
// bare "uniform int" declared in a block both stages include fails to link
// with "precisions differ between VERTEX and FRAGMENT shaders". Floats are
// safe only because every source here opens with precision highp float.
uniform highp int uProjection;  // 0 equirectangular, 1 Mercator, 2 Miller

const float VE_DEG = 0.017453292519943295;

// The vertical map coordinate of a latitude, in degrees at the equator. The
// port of Projection.yOf in projection.ts, decision for decision. (No
// backticks in here: this is a template literal, and one would end it.)
float latToY(float lat) {
  if (uProjection == 1) {
    float clamped = clamp(lat, -85.051129, 85.051129);
    return log(tan(0.7853981633974483 + clamped * VE_DEG * 0.5)) / VE_DEG;
  }
  if (uProjection == 2) {
    float clamped = clamp(lat, -90.0, 90.0);
    return 1.25 * log(tan(0.7853981633974483 + 0.4 * clamped * VE_DEG)) / VE_DEG;
  }
  return lat;
}

// And back. Only the raster needs it, but it belongs beside its forward.
float yToLat(float y) {
  if (uProjection == 1) {
    return (2.0 * atan(exp(y * VE_DEG)) - 1.5707963267948966) / VE_DEG;
  }
  if (uProjection == 2) {
    return (2.5 * atan(exp(0.8 * y * VE_DEG)) - 1.9634954084936207) / VE_DEG;
  }
  return y;
}

vec2 geoToScreen(vec2 lonLat) {
  return vec2(
    (lonLat.x + uLonOffset - uCamera.x) * uCamera.z + uViewport.x * 0.5,
    (uCamera.y - latToY(lonLat.y)) * uCamera.z + uViewport.y * 0.5
  );
}

vec4 screenToClip(vec2 screen) {
  return vec4(
    screen.x / uViewport.x * 2.0 - 1.0,
    1.0 - screen.y / uViewport.y * 2.0,
    0.0, 1.0
  );
}
`;

/** Draws land polygons, coastlines and the graticule from lon/lat vertices. */
export const GEO_VERT = `#version 300 es
precision highp float;
in vec2 aLonLat;
${PROJECTION}
void main() {
  gl_Position = screenToClip(geoToScreen(aLonLat));
}
`;

export const GEO_FRAG = `#version 300 es
precision highp float;
uniform vec4 uColor;
out vec4 fragColor;
void main() { fragColor = uColor; }
`;

/**
 * Speed raster.
 *
 * Speed arrives as a 16-bit value split across the red and green channels, so
 * the texture must be sampled with NEAREST — hardware bilinear filtering would
 * blend the high and low bytes independently and produce nonsense. Smoothing is
 * therefore done here, by decoding four texels and interpolating the decoded
 * speeds, which is correct and costs three extra taps.
 */
/**
 * The live-operator mask.
 *
 * A gesture with the mask or the clone stamp is an operation on the field
 * that is already drawn, and the 2D overlay cannot express one: it sits above
 * the field and can add pixels, never take them away (spec.md 6.1). So the mask
 * is applied here, where the field itself is drawn.
 *
 * It is a screen-space coverage texture — the swept footprint, rasterised by
 * the same path builder the overlay uses — so it needs no knowledge of what
 * shape the gesture is, and a new tool inherits it by supplying a footprint.
 */
const MASK = `
uniform sampler2D uMask;    // screen-space coverage of the gesture, in alpha
uniform vec2 uMaskSize;     // the framebuffer's size, to read gl_FragCoord
uniform int uMaskMode;      // 0 none, 1 cut the covered part, 2 keep only it

// How much of this fragment the gesture covers, 0 to 1.
float maskCoverage() {
  if (uMaskMode == 0) return 0.0;
  return texture(uMask, gl_FragCoord.xy / uMaskSize).a;
}

// The factor the fragment's alpha is multiplied by.
float maskFactor() {
  if (uMaskMode == 0) return 1.0;
  float covered = maskCoverage();
  // Mode 1 takes the field away where the gesture covers, which is what an
  // mask does and what a clone does before it puts the source in its place.
  // Mode 2 keeps only what the gesture covers, which is how the source is
  // drawn into it.
  return uMaskMode == 1 ? 1.0 - covered : covered;
}
`;

export const RASTER_VERT = `#version 300 es
precision highp float;
in vec2 aCorner;            // 0..1 across the tile
uniform vec4 uTileGeo;      // west, north, spanX, spanY
${PROJECTION}
out vec2 vUV;
void main() {
  vUV = aCorner;
  vec2 lonLat = vec2(
    uTileGeo.x + aCorner.x * uTileGeo.z,
    uTileGeo.y - aCorner.y * uTileGeo.w
  );
  gl_Position = screenToClip(geoToScreen(lonLat));
}
`;

export const RASTER_FRAG = `#version 300 es
precision highp float;
in vec2 vUV;
${PROJECTION}
uniform vec4 uTileGeo;      // west, north, spanX, spanY
uniform sampler2D uTile;
uniform float uSpeedScale;  // full-scale speed, m/s
uniform float uRampMax;     // speed mapped to the top of the ramp
uniform float uDim;         // 1.0 normally, lower while a frame is stale
${MASK}
out vec4 fragColor;

float decodeSpeed(ivec2 texel) {
  vec4 t = texelFetch(uTile, texel, 0);
  return (t.r * 255.0 + t.g * 255.0 * 256.0) / 65535.0 * uSpeedScale;
}

// Manual bilinear over decoded speeds.
float sampleSpeed(vec2 uv) {
  vec2 size = vec2(textureSize(uTile, 0));
  vec2 texel = uv * size - 0.5;
  vec2 base = floor(texel);
  vec2 f = texel - base;
  ivec2 b = ivec2(base);
  ivec2 maxT = ivec2(size) - 1;

  float s00 = decodeSpeed(clamp(b + ivec2(0, 0), ivec2(0), maxT));
  float s10 = decodeSpeed(clamp(b + ivec2(1, 0), ivec2(0), maxT));
  float s01 = decodeSpeed(clamp(b + ivec2(0, 1), ivec2(0), maxT));
  float s11 = decodeSpeed(clamp(b + ivec2(1, 1), ivec2(0), maxT));
  return mix(mix(s00, s10, f.x), mix(s01, s11, f.x), f.y);
}

// Perceptually ordered ramp: dark and desaturated at calm, hot at the top, so
// speed reads as intensity rather than as an arbitrary hue cycle.
vec3 ramp(float t) {
  t = clamp(t, 0.0, 1.0);
  vec3 c0 = vec3(0.05, 0.09, 0.16);
  vec3 c1 = vec3(0.12, 0.35, 0.62);
  vec3 c2 = vec3(0.16, 0.68, 0.66);
  vec3 c3 = vec3(0.55, 0.80, 0.35);
  vec3 c4 = vec3(0.96, 0.78, 0.24);
  vec3 c5 = vec3(0.92, 0.42, 0.20);
  vec3 c6 = vec3(0.78, 0.16, 0.36);
  if (t < 0.1667) return mix(c0, c1, t / 0.1667);
  if (t < 0.3333) return mix(c1, c2, (t - 0.1667) / 0.1667);
  if (t < 0.5000) return mix(c2, c3, (t - 0.3333) / 0.1667);
  if (t < 0.6667) return mix(c3, c4, (t - 0.5000) / 0.1667);
  if (t < 0.8333) return mix(c4, c5, (t - 0.6667) / 0.1667);
  return mix(c5, c6, (t - 0.8333) / 0.1667);
}

/**
 * Where in the tile this pixel is.
 *
 * Longitude is linear in x in every projection here, so the interpolated u is
 * exact. Latitude is not: a tile is an axis-aligned rectangle on the map, but
 * the latitude across it is the projection's inverse, and interpolating the
 * corners instead would bow the field inside a tall tile. So v is recovered
 * from the pixel, which is exact everywhere.
 *
 * Equirectangular takes the interpolated value unchanged, so the projection
 * the app grew up in draws precisely the pixels it always did.
 */
vec2 tileUV() {
  if (uProjection == 0) return vUV;
  // gl_FragCoord is y-up from the bottom; the camera's y is y-down from the top.
  float screenY = uViewport.y - gl_FragCoord.y;
  float y = uCamera.y - (screenY - uViewport.y * 0.5) / uCamera.z;
  return vec2(vUV.x, (uTileGeo.y - yToLat(y)) / uTileGeo.w);
}

void main() {
  float speed = sampleSpeed(tileUV());
  vec3 colour = ramp(speed / max(uRampMax, 0.001));
  // Calm water stays transparent so the basemap shows through.
  // Capped below 1 so the basemap stays visible through the field; calm areas
  // fade out entirely so the map is readable where there is nothing to show.
  float alpha = clamp(speed / (uRampMax * 0.06), 0.0, 1.0) * 0.72;
  fragColor = vec4(colour * uDim, alpha * maskFactor());
}
`;

/**
 * Direction glyphs, instanced.
 *
 * One draw call per visible tile. Instances form a lattice in *screen* space
 * across that tile's rectangle, so spacing stays constant as you zoom instead
 * of clumping or thinning out. Each instance reads its own vector straight from
 * the tile texture, so glyph layout costs no CPU round-trip.
 *
 * Geometry is built from `gl_VertexID` against a fixed 54-vertex budget:
 * 6 for the shaft, then eight 6-vertex slots for barb flags. Unused slots
 * collapse to a point and are discarded by the rasteriser. That is how one
 * program draws both a plain arrow and a barb carrying up to eight flags.
 */
export const GLYPH_VERT = `#version 300 es
precision highp float;
${PROJECTION}
uniform vec4 uTileGeo;      // west, north, spanX, spanY
uniform vec2 uGlyphOrigin;  // longitude, latitude of the first lattice point
uniform float uGlyphStep;   // lattice spacing, degrees
uniform vec2 uGrid;         // columns, rows
uniform float uSpacing;     // lattice spacing, screen px (step * pxPerDeg)
uniform sampler2D uTile;
uniform float uSpeedScale;
uniform int uStyle;         // 0 = arrow, 1 = barb
uniform float uSizeScale;   // glyph length as a fraction of spacing
uniform float uPixelRatio;  // device pixels per CSS pixel; keeps strokes even
out float vShade;

const float DEG = 0.017453292519943295;

vec2 rotate(vec2 v, float radians) {
  float c = cos(radians), s = sin(radians);
  return vec2(v.x * c - v.y * s, v.x * s + v.y * c);
}

void main() {
  int cols = int(uGrid.x);
  int col = gl_InstanceID % cols;
  int row = gl_InstanceID / cols;

  // The lattice is anchored to the globe, not to this tile, so points are
  // continuous across tile edges. Anchoring per tile leaves a gap of
  // tileWidth modulo spacing at every boundary, which reads as clustered glyphs.
  float lon = uGlyphOrigin.x + float(col) * uGlyphStep;
  float lat = uGlyphOrigin.y - float(row) * uGlyphStep;

  vec2 uv = vec2((lon - uTileGeo.x) / uTileGeo.z, (uTileGeo.y - lat) / uTileGeo.w);
  if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
    gl_Position = vec4(2.0, 2.0, 2.0, 1.0);
    return;
  }

  vec2 station = geoToScreen(vec2(lon, lat));

  ivec2 size = textureSize(uTile, 0);
  ivec2 texel = clamp(ivec2(uv * vec2(size)), ivec2(0), size - 1);
  vec4 t = texelFetch(uTile, texel, 0);
  float speed = (t.r * 255.0 + t.g * 255.0 * 256.0) / 65535.0 * uSpeedScale;
  float azimuth = (t.b * 255.0 + t.a * 255.0 * 256.0) / 65535.0 * 360.0;

  vShade = clamp(speed / 25.0, 0.25, 1.0);

  // Screen space is y-down, so north is -y. Azimuth is clockwise from north.
  float az = azimuth * DEG;
  vec2 toward = vec2(sin(az), -cos(az));
  float length_px = uSpacing * uSizeScale;

  vec2 offset = vec2(0.0);
  int id = gl_VertexID;

  if (uStyle == 0) {
    // ---- Arrow: shaft quad plus a head triangle ----
    float halfW = 0.9 * uPixelRatio;
    vec2 side = vec2(-toward.y, toward.x);
    vec2 tail = -toward * length_px * 0.5;
    vec2 head = toward * length_px * 0.5;
    vec2 neck = head - toward * (length_px * 0.38);

    if (speed < 0.05) {
      gl_Position = vec4(2.0, 2.0, 2.0, 1.0);
      return;
    }
    if (id < 6) {
      vec2 quad[6] = vec2[6](
        tail + side * halfW, tail - side * halfW, neck + side * halfW,
        neck + side * halfW, tail - side * halfW, neck - side * halfW
      );
      offset = quad[id];
    } else if (id < 9) {
      vec2 tri[3] = vec2[3](
        head,
        neck + side * (length_px * 0.22),
        neck - side * (length_px * 0.22)
      );
      offset = tri[id - 6];
    } else {
      gl_Position = vec4(2.0, 2.0, 2.0, 1.0);
      return;
    }
  } else {
    // ---- Barb: shaft into the wind, flags at the far end ----
    float knots = speed * 1.9438444924406046;

    if (knots < 2.5) {
      // Calm: a small open square at the station.
      if (id >= 6) { gl_Position = vec4(2.0, 2.0, 2.0, 1.0); return; }
      float r = 2.0 * uPixelRatio;
      vec2 quad[6] = vec2[6](
        vec2(-r, -r), vec2(r, -r), vec2(-r, r),
        vec2(-r, r), vec2(r, -r), vec2(r, r)
      );
      offset = quad[id];
    } else {
      float rounded = floor(knots / 5.0 + 0.5) * 5.0;
      int pennants = int(floor(rounded / 50.0));
      float rest = rounded - float(pennants) * 50.0;
      int fulls = int(floor(rest / 10.0));
      rest -= float(fulls) * 10.0;
      int halves = rest >= 5.0 ? 1 : 0;
      int total = pennants + fulls + halves;

      // The shaft points where the wind comes *from*, which is the convention.
      vec2 shaftDir = -toward;
      vec2 tip = shaftDir * length_px;
      // Flags sit on the poleward side; the convention mirrors below the equator.
      float handed = lat < 0.0 ? -1.0 : 1.0;
      vec2 side = vec2(-shaftDir.y, shaftDir.x) * handed;

      if (id < 6) {
        float halfW = 0.9 * uPixelRatio;
        vec2 perp = vec2(-shaftDir.y, shaftDir.x) * halfW;
        vec2 quad[6] = vec2[6](
          perp, -perp, tip + perp,
          tip + perp, -perp, tip - perp
        );
        offset = quad[id];
      } else {
        int slot = (id - 6) / 6;
        int sub = (id - 6) % 6;
        if (slot >= total) { gl_Position = vec4(2.0, 2.0, 2.0, 1.0); return; }

        float step_px = length_px * 0.15;
        vec2 anchor = tip - shaftDir * (float(slot) * step_px);
        float flagLen = length_px * 0.42;
        vec2 flagDir = normalize(side * 0.86 - shaftDir * 0.5);

        if (slot < pennants) {
          // Filled triangle; the second triangle of the slot is degenerate.
          vec2 apex = anchor + flagDir * flagLen;
          vec2 back = anchor - shaftDir * step_px;
          vec2 tri[6] = vec2[6](anchor, apex, back, back, back, back);
          offset = tri[sub];
        } else {
          float len = slot < pennants + fulls ? flagLen : flagLen * 0.5;
          vec2 end = anchor + flagDir * len;
          vec2 perp = normalize(vec2(-flagDir.y, flagDir.x)) * 0.9 * uPixelRatio;
          vec2 quad[6] = vec2[6](
            anchor + perp, anchor - perp, end + perp,
            end + perp, anchor - perp, end - perp
          );
          offset = quad[sub];
        }
      }
    }
  }

  gl_Position = screenToClip(station + offset);
}
`;

export const GLYPH_FRAG = `#version 300 es
precision highp float;
in float vShade;
uniform vec4 uColor;
${MASK}
out vec4 fragColor;
// The glyphs follow the field they describe: a glyph left standing over an
// masked patch would be pointing at a wind that is no longer there.
void main() { fragColor = vec4(uColor.rgb, uColor.a * vShade * maskFactor()); }
`;

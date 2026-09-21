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
 * `ui/src/map/projection.ts` holds the same formulas in TypeScript, for
 * the overlay and the pointer. Change one and change the other, or the arrows
 * stop landing where the field is.
 */

import { AZIMUTHAL, EXTRA_CYLINDRICAL } from "./projectionShaders";
import { GLYPH_SIZE_SCALE, GLYPH_TARGET_PX } from "./glyph";

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
uniform highp int uProjection;  // Stable mode from the projection catalogue.

const float VE_DEG = 0.017453292519943295;
${EXTRA_CYLINDRICAL}
${AZIMUTHAL}

// The vertical map coordinate of a latitude, in units of longitudinal degrees. The
// port of Projection.yOf in projection.ts, decision for decision. (No
// backticks in here: this is a template literal, and one would end it.)
float latToY(float lat) {
  if (uProjection >= 3) return cylindricalY(uProjection, lat);
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
  if (uProjection >= 3) return cylindricalLat(uProjection, y);
  if (uProjection == 1) {
    return (2.0 * atan(exp(y * VE_DEG)) - 1.5707963267948966) / VE_DEG;
  }
  if (uProjection == 2) {
    return (2.5 * atan(exp(0.8 * y * VE_DEG)) - 1.9634954084936207) / VE_DEG;
  }
  return y;
}

// Set for a tile drawn as the whole viewport, its place found per pixel
// (mode 15 up only). Two azimuthal maps show everything but the antipode,
// smeared round their whole rim; a grid cell that holds it, or lies beside
// it, has its corners on that rim in different directions, and drawn through
// its vertices it is a chord across the map. The few tiles near the antipode
// are drawn this way instead, which is exact wherever they fall.
uniform highp int uExact;

vec2 geoToScreen(vec2 lonLat) {
  // The globe and its azimuthal kin (mode 15 up) are projected here, vertex
  // by vertex, and say through veHorizon which side of the earth each is on.
  if (uProjection >= 15) return azimuthalScreen(lonLat);
  return vec2(
    (lonLat.x + uLonOffset - uCamera.x) * uCamera.z + uViewport.x * 0.5,
    (uCamera.y - latToY(lonLat.y)) * uCamera.z + uViewport.y * 0.5
  );
}

// A general projection's mesh (mode 14) is made on a virtual canvas in the
// projection's own plane, and this is where that canvas lies on the screen:
// an offset and a scale, which is all a pan or a zoom changes. See
// PlaneMesh in camera.ts.
uniform vec3 uMesh;        // offset x, offset y, scale
vec2 meshToScreen(vec2 virtual) {
  return virtual * uMesh.z + uMesh.xy;
}

vec4 screenToClip(vec2 screen) {
  return vec4(
    screen.x / uViewport.x * 2.0 - 1.0,
    1.0 - screen.y / uViewport.y * 2.0,
    0.0, 1.0
  );
}
`;

/**
 * Fragment stage only, after the projection helper: it reads the pixel's own
 * position, which a vertex shader has none of.
 */
const EXACT_TILE = `
// Where in a tile the pixel being shaded is, by the exact inverse. Fragment
// stage only. The second component is past 10 where the tile does not hold it.
vec2 exactTileUV(vec4 tileGeo) {
  vec2 geo = azimuthalInverse(vec2(gl_FragCoord.x, uViewport.y - gl_FragCoord.y));
  if (geo.x > 1000.0) return vec2(0.0, 99.0);
  vec2 uv = vec2((geo.x - tileGeo.x) / tileGeo.z, (tileGeo.y - geo.y) / tileGeo.w);
  // Half open, so a pixel on the seam between two tiles belongs to one.
  if (uv.x < 0.0 || uv.x >= 1.0 || uv.y < 0.0 || uv.y >= 1.0) return vec2(0.0, 99.0);
  return uv;
}
`;

/** Draws land polygons, coastlines and the graticule from lon/lat vertices. */
export const GEO_VERT = `#version 300 es
precision highp float;
layout(location=0) in vec2 aLonLat;
${PROJECTION}
out float vHorizon;
void main() {
  gl_Position = screenToClip(uProjection == 14 ? meshToScreen(aLonLat) : geoToScreen(aLonLat));
  vHorizon = veHorizon;
}
`;

export const GEO_FRAG = `#version 300 es
precision highp float;
uniform vec4 uColor;
in float vHorizon;
out vec4 fragColor;
void main() {
  // The far side of a globe (mode 15 up); always positive on a flat map.
  if (vHorizon < 0.0) discard;
  fragColor = uColor;
}
`;

/**
 * Land and coast on a globe: a cached source tile (388 texels, two of bleed
 * a side, rendered y-up) drawn through the same static grid as the field's
 * tiles. Only the GPU-projected modes use it; a general projection's mesh
 * carries the same texture coordinates in its vertices.
 */
export const BASE_VERT = `#version 300 es
precision highp float;
layout(location=0) in vec2 aCorner;            // 0..1 across the tile
uniform vec4 uTileGeo;      // west, north, spanX, spanY
${PROJECTION}
out vec2 vUV;
out float vHorizon;
void main() {
  vUV = vec2((2.0 + aCorner.x * 384.0) / 388.0, (386.0 - aCorner.y * 384.0) / 388.0);
  vec2 lonLat = vec2(
    uTileGeo.x + aCorner.x * uTileGeo.z,
    uTileGeo.y - aCorner.y * uTileGeo.w
  );
  gl_Position = uExact == 1 ? vec4(aCorner * 2.0 - 1.0, 0.0, 1.0) : screenToClip(geoToScreen(lonLat));
  vHorizon = uExact == 1 ? 1.0 : veHorizon;
}
`;

export const BASE_FRAG = `#version 300 es
precision highp float;
uniform sampler2D uTexture;
uniform vec4 uTileGeo;
${PROJECTION}
${EXACT_TILE}
in vec2 vUV;
in float vHorizon;
out vec4 fragColor;
void main() {
  if (vHorizon < 0.0) discard;
  vec2 uv = vUV;
  if (uExact == 1) {
    vec2 exact = exactTileUV(uTileGeo);
    if (exact.y > 10.0) discard;
    uv = vec2((2.0 + exact.x * 384.0) / 388.0, (386.0 - exact.y * 384.0) / 388.0);
  }
  fragColor = texture(uTexture, uv);
  if (fragColor.a > 0.0) fragColor.rgb /= fragColor.a;
}
`;

/**
 * A georeferenced image layer (spec.md 4.9, M18).
 *
 * Drawn as a subdivided quad rather than two triangles. The placement is an
 * affine in *degrees*, so a straight line across the image is straight in
 * lon/lat — and a straight line in lon/lat is not straight on the map once the
 * projection is not the flat one (M11). Subdividing follows the curve; the
 * count is fixed rather than derived from the zoom, because it is a few hundred
 * vertices either way and a mesh that changed with the camera would rebuild
 * itself on every wheel event.
 *
 * The texture coordinate is the vertex's own place in the image, so nothing
 * about the interior is interpolated through the projection.
 *
 * A warped image (control points, spec.md 4.9) carries its own lon/lat per
 * vertex in `aGeo` instead of the affine uniforms: Rust evaluates the warp at
 * every mesh vertex and hands it over, so projection happens strictly *after*
 * the warp and every projection (cylindrical, the globe, a general preset)
 * draws it through the path it already draws an unwarped image through. The
 * uniforms still carry the plain-placement affine, which the fragment stage
 * needs for the globe's per-pixel path on an *unwarped* image.
 *
 * On the globe and its azimuthal kin (mode 15 up), a warped image is drawn
 * through `geoToScreen` vertex by vertex, exactly as `GEO_VERT` and
 * `BASE_VERT` are — so it needs the same `vHorizon` they carry: every vertex
 * is projected *somewhere*, near side or far, and only the horizon test
 * tells them apart. Without it a chart on the far side of the globe drew as
 * a smear across the limb instead of not drawing at all. An *unwarped*
 * image on the globe takes the per-pixel path below instead and needs no
 * horizon test of its own — `vHorizon` is pinned to 1.0 for it.
 */
export const IMAGE_VERT = `#version 300 es
precision highp float;
layout(location=0) in vec2 aCell;              // 0..1 across the image
layout(location=1) in vec2 aScreen;
layout(location=2) in vec2 aGeo;               // a warped vertex's own lon/lat
${PROJECTION}
uniform vec3 uPlaceLon;     // lon = x*u + y*v + z, with u and v in 0..1
uniform vec3 uPlaceLat;     // and the same for the latitude
uniform bool uWarped;
out vec2 vUV;
out vec2 vGeo;
out float vHorizon;
void main() {
  vUV = aCell;
  vec2 lonLat = uWarped
    ? aGeo
    : vec2(
        uPlaceLon.x * aCell.x + uPlaceLon.y * aCell.y + uPlaceLon.z,
        uPlaceLat.x * aCell.x + uPlaceLat.y * aCell.y + uPlaceLat.z
      );
  vGeo = lonLat;
  // On a globe (mode 15 up) the cells are the whole viewport and the
  // fragment stage finds the image under each pixel: see IMAGE_FRAG. A
  // warped image cannot take that path (no closed-form inverse of a spline),
  // so it is projected vertex by vertex like every other projection instead
  // — through geoToScreen, exactly as GEO_VERT and BASE_VERT are, and so it
  // needs the same vHorizon this vertex sets and IMAGE_FRAG discards on: the
  // azimuthals draw every vertex somewhere, on either side of the globe, and
  // only the horizon test tells the far side from the near one.
  gl_Position = (uProjection >= 15 && !uWarped)
    ? vec4(aCell * 2.0 - 1.0, 0.0, 1.0)
    : screenToClip(uProjection == 14 ? meshToScreen(aScreen) : geoToScreen(lonLat));
  vHorizon = (uProjection >= 15 && !uWarped) ? 1.0 : veHorizon;
}
`;

export const IMAGE_FRAG = `#version 300 es
precision highp float;
precision highp int;
in vec2 vUV;
in vec2 vGeo;
in float vHorizon;
${PROJECTION}
uniform vec3 uPlaceLon;
uniform vec3 uPlaceLat;
uniform sampler2D uImage;
uniform float uOpacity;
uniform bool uFiltered;
uniform vec2 uSpeedRange;
uniform highp sampler2D uFilterTile;
uniform vec4 uFilterGeo;
uniform float uSpeedScale;
uniform bool uWarped;
out vec4 fragColor;
void main() {
  // The far side of a globe (mode 15 up); always positive on a flat map or
  // through the globe's own per-pixel path (see IMAGE_VERT). Without this a
  // warped image on the globe drew through every vertex it was given, near
  // side and far side alike, which showed as a smear across the limb.
  if (vHorizon < 0.0) discard;
  vec2 geo = vGeo;
  vec2 cell = vUV;
  if (uProjection >= 15 && !uWarped) {
    // On a globe the place is taken from the pixel, not from a mesh. An
    // image can span the earth, and a mesh of it fine enough to follow the
    // sphere at every zoom is a mesh made per frame, which is what the globe
    // stopped doing. So the draw is the whole viewport, each pixel is asked
    // where on the earth it is, and the placement is inverted for the texel:
    // exact at the limb, at the antipode and at any zoom, for a few
    // operations a pixel.
    //
    // A warped image cannot take this branch: a thin-plate spline has no
    // closed-form inverse, so there is no placement to invert per pixel.
    // It arrives here with vUV already correct from the mesh instead.
    geo = azimuthalInverse(vec2(gl_FragCoord.x, uViewport.y - gl_FragCoord.y));
    if (geo.x > 1000.0) discard;
    float centre = uPlaceLon.z + 0.5 * (uPlaceLon.x + uPlaceLon.y);
    geo.x += 360.0 * floor((centre - geo.x) / 360.0 + 0.5);
    float det = uPlaceLon.x * uPlaceLat.y - uPlaceLon.y * uPlaceLat.x;
    vec2 d = vec2(geo.x - uPlaceLon.z, geo.y - uPlaceLat.z);
    cell = vec2(d.x * uPlaceLat.y - d.y * uPlaceLon.y, d.y * uPlaceLon.x - d.x * uPlaceLat.x) / det;
    if (any(lessThan(cell, vec2(0.0))) || any(greaterThan(cell, vec2(1.0)))) discard;
  }
  if (uFiltered) {
    float lon = mod(geo.x + 180.0, 360.0) - 180.0;
    vec2 uv = vec2((lon - uFilterGeo.x) / uFilterGeo.z, (uFilterGeo.y - geo.y) / uFilterGeo.w);
    if (any(lessThan(uv, vec2(0.0))) || any(greaterThanEqual(uv, vec2(1.0)))) discard;
    uvec4 b = uvec4(texture(uFilterTile, uv) * 255.0 + 0.5);
    uint w = b.r | (b.g << 8u) | (b.b << 16u) | (b.a << 24u);
    float speed = float(w & 16383u) / 16383.0 * uSpeedScale;
    if (((w >> 26u) & 31u) == 0u || speed < uSpeedRange.x || speed > uSpeedRange.y) discard;
  }
  vec4 texel = texture(uImage, cell);
  // The image's own alpha is kept and scaled: a chart scan with a transparent
  // margin must not gain an opaque one on the way to the screen.
  fragColor = vec4(texel.rgb, texel.a * uOpacity);
}
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
/**
 * The gesture in progress, applied to the field live (spec.md 6.2, M32).
 *
 * A mask of the gesture's coverage, and what the tool does inside it. The
 * mask and the clone take the field away or keep only it; a modifier changes
 * the speed and the direction of what is there, per fragment, from the
 * gesture's own settings; a liquify moves it. Every one of these is a proxy
 * in screen space — the true field lands with the commit — but it is the
 * tool's effect, seen while the pointer is still down.
 *
 * Positions are framebuffer pixels with y up, which is what gl_FragCoord
 * gives; the glyph program converts its station to match.
 */
const OP_POINTS = 64;
const OPERATOR = `
uniform sampler2D uMask;    // screen-space coverage of the gesture, in alpha
uniform sampler2D uSourceCoverage;
uniform highp int uUseSourceCoverage;
uniform vec2 uMaskSize;     // the framebuffer's size, to read gl_FragCoord
uniform int uOpKind;        // 0 none, 1 remove, 2 keep only, 3 gain, 4 turn,
                            // 5 radial, 6 smear
uniform float uOpAmount;    // gain: fraction; turn: degrees; radial: fraction
uniform int uOpCount;       // points of the stroke's centreline in use
uniform vec2 uOpPoints[${OP_POINTS}];  // the centreline, framebuffer px, y up
uniform vec2 uOpDeltas[${OP_POINTS}];  // a liquify's movement per stamp, px
uniform float uOpRadius;    // a liquify's stamp radius, px
uniform float uOpFeather;   // and its feather, 0 to 1

// How much of a point the gesture covers, 0 to 1.
float opCoverageAt(vec2 p) {
  if (uOpKind == 0) return 0.0;
  float coverage = texture(uMask, p / uMaskSize).a;
  if (uUseSourceCoverage == 1) coverage *= texture(uSourceCoverage, p / uMaskSize).a;
  return coverage;
}

// The factor a fragment's alpha is multiplied by: the mask and the eraser
// take the field away where the gesture covers, and a clone or a warp keeps
// only what it covers when its source is drawn in. A modifier leaves it.
float maskFactorAt(vec2 p) {
  if (uOpKind == 0) return 1.0;
  float covered = opCoverageAt(p);
  if (uOpKind == 1) return 1.0 - covered;
  if (uOpKind == 2) return covered;
  return 1.0;
}

// The direction away from the nearest point of the stroke's centreline: what
// a divergence radiates along, a stroke diverging from itself (spec.md 6.3).
vec2 opOutward(vec2 p) {
  float best = 1.0e30;
  vec2 nearest = p;
  for (int i = 0; i < ${OP_POINTS}; i++) {
    if (i >= uOpCount) break;
    vec2 a = uOpPoints[i];
    vec2 b = (i + 1 < uOpCount) ? uOpPoints[i + 1] : a;
    vec2 ab = b - a;
    float len2 = dot(ab, ab);
    float t = len2 > 0.0 ? clamp(dot(p - a, ab) / len2, 0.0, 1.0) : 0.0;
    vec2 q = a + ab * t;
    float d = dot(p - q, p - q);
    if (d < best) { best = d; nearest = q; }
  }
  vec2 away = p - nearest;
  float len = length(away);
  // On the line itself there is no outward, and just beside it the bearing
  // is ill-conditioned: fade in over a couple of pixels.
  return len > 0.001 ? away / len * min(len / 2.0, 1.0) : vec2(0.0);
}

// Where a liquify moved the field from: the feathered sum of the deltas of
// the stamps that cover the point, the port of smear_source_position.
vec2 opDisplacement(vec2 p) {
  vec2 moved = vec2(0.0);
  float band = uOpFeather * uOpRadius;
  for (int i = 0; i < ${OP_POINTS}; i++) {
    if (i >= uOpCount) break;
    float signed = distance(p, uOpPoints[i]) - uOpRadius;
    if (signed > 0.0) continue;
    float w = band > 0.0 ? smoothstep(0.0, band, -signed) : 1.0;
    moved += uOpDeltas[i] * w;
  }
  return moved;
}

// A modifier's effect on the field at a point covered by \`coverage\`: the
// same fade from what was there to what the tool makes of it that the
// kernels apply. Speed in m/s, azimuth-toward in degrees.
void opApply(vec2 p, float coverage, inout float speed, inout float azimuth) {
  if (coverage <= 0.0) return;
  if (uOpKind == 3) {
    speed *= 1.0 + uOpAmount * coverage;
  } else if (uOpKind == 4) {
    azimuth += uOpAmount * coverage;
  } else if (uOpKind == 5) {
    float az = azimuth * 0.017453292519943295;
    // East is +x and north is +y here, as the point is.
    vec2 v = speed * vec2(sin(az), cos(az));
    v += speed * uOpAmount * coverage * opOutward(p);
    // The direction turns and the speed does not (M54): a divergence bends
    // the flow rather than driving it. The kernels do the same, and the
    // preview has to agree with them (invariant 3).
    float turned = length(v);
    if (turned > 1.0e-6) {
      azimuth = atan(v.x, v.y) / 0.017453292519943295;
    } else {
      speed = 0.0;
    }
  }
}
`;

/** The operator at this fragment. Fragment programs only: gl_FragCoord is theirs. */
const OPERATOR_FRAG = `
float opCoverage() { return opCoverageAt(gl_FragCoord.xy); }
float maskFactor() { return maskFactorAt(gl_FragCoord.xy); }
`;

/**
 * Whether the pixel shows the layer the gesture is editing (M44).
 *
 * An edit acts on one layer, but a tile is the whole visible stack
 * composited into one texel, so a preview applied to the tile applied to
 * every layer at once: a stroke on a lower layer visibly changed the layers
 * above it until the button came up, and then the commit put them back.
 *
 * The test is the tile against the same tile with the edited layer left out
 * (spec.md 6.2). Where the two words differ, that layer is what the composite
 * is showing here and the gesture belongs; where they are equal, the layer
 * contributes nothing visible and the gesture must not either. Which also
 * means the composite's value *is* that layer's value wherever the gesture
 * applies, so a modifier can be applied to the tile as it stands.
 *
 * The nearest texel rather than the blend: a word is not a quantity to
 * interpolate, and the answer is only ever wrong within half a texel of an
 * edge, which is far below a pixel at the zooms a tile is drawn at.
 *
 * Expects the program to declare `uTile`, `uBelow` and `uEditScoped`, and to
 * include the texel block first.
 */
const EDIT_SCOPE = `
bool editedHere(vec2 uv) {
  if (uEditScoped == 0) return true;
  vec2 size = vec2(textureSize(uTile, 0));
  ivec2 at = ivec2(clamp(floor(uv * size), vec2(0.0), size - 1.0));
  return texelWordFrom(uTile, at) != texelWordFrom(uBelow, at);
}
`;

export const RASTER_VERT = `#version 300 es
precision highp float;
layout(location=0) in vec2 aCorner;            // 0..1 across the tile
layout(location=1) in vec2 aScreen;
uniform vec4 uTileGeo;      // west, north, spanX, spanY
${PROJECTION}
out vec2 vUV;
out float vHorizon;
void main() {
  vUV = aCorner;
  vec2 lonLat = vec2(
    uTileGeo.x + aCorner.x * uTileGeo.z,
    uTileGeo.y - aCorner.y * uTileGeo.w
  );
  gl_Position = uExact == 1
    ? vec4(aCorner * 2.0 - 1.0, 0.0, 1.0)
    : screenToClip(uProjection == 14 ? meshToScreen(aScreen) : geoToScreen(lonLat));
  vHorizon = uExact == 1 ? 1.0 : veHorizon;
}
`;

/**
 * How a tile's texel is read (spec.md 7.7, M31).
 *
 * One little-endian 32-bit word per texel: the speed in the low 14 bits as a
 * fraction of full scale, the azimuth-toward in the next 12 as a fraction of
 * a turn, the coverage in the next 5, and the kind in the top bit, 1 for
 * wind. Shared by the raster and the glyph programs so the two cannot read
 * the same bytes differently. Expects the program to declare "uTile" and
 * "uSpeedScale" first.
 */
const TEXEL = `
uint texelWordFrom(highp sampler2D tex, ivec2 texel) {
  uvec4 b = uvec4(texelFetch(tex, texel, 0) * 255.0 + 0.5);
  return b.r | (b.g << 8u) | (b.b << 16u) | (b.a << 24u);
}
uint texelWord(ivec2 texel) { return texelWordFrom(uTile, texel); }
float wordSpeed(uint w) { return float(w & 16383u) / 16383.0 * uSpeedScale; }
float wordAzimuth(uint w) { return float((w >> 14u) & 4095u) / 4096.0 * 360.0; }
float wordCoverage(uint w) { return float((w >> 26u) & 31u) / 31.0; }
bool wordIsWind(uint w) { return (w >> 31u) == 1u; }
`;

/**
 * How opaque a written cell is. Below one so the land's shape still reads
 * through a field painted over it; a written calm is exactly as opaque as a
 * gale (M31) — only an unwritten cell shows the map beneath.
 */
export const FIELD_OPACITY = 0.9;

/**
 * Stops the raster shader reserves for a gradient.
 *
 * A uniform array has a fixed length in GLSL, so this is the ceiling on how
 * many stops a gradient in the catalogue may have. Twelve is comfortably
 * above the eight the longest of them uses; `gradients.test.ts` holds the
 * catalogue to it, so a gradient that would be silently truncated on the map
 * fails the suite instead.
 */
export const RAMP_MAX_STOPS = 12;

export const RASTER_FRAG = `#version 300 es
precision highp float;
in vec2 vUV;
in float vHorizon;
${PROJECTION}
${EXACT_TILE}
uniform vec4 uTileGeo;      // west, north, spanX, spanY
uniform sampler2D uTile;
uniform float uSpeedScale;  // full-scale speed, m/s
uniform vec2 uRampWind;     // speeds mapped to the ends of the wind ramp, m/s
uniform vec2 uRampCurrent;  // and of the current ramp (M31)
// The gradient each kind is painted with (spec.md 5.3, M42). Uniforms rather
// than constants because a gradient is a setting: the tiles carry speed and
// not colour, so changing one is a redraw and never a re-render.
uniform vec3 uRampStopsWind[${RAMP_MAX_STOPS}];
uniform vec3 uRampStopsCurrent[${RAMP_MAX_STOPS}];
uniform highp int uRampCountWind;
uniform highp int uRampCountCurrent;
uniform float uDim;         // 1.0 normally, lower while a frame is stale
// The same tile with the layer being edited left out, and whether it is
// bound (M44). What scopes a live edit to one layer.
uniform sampler2D uBelow;
uniform highp int uEditScoped;
uniform highp int uCoverageOnly;
${OPERATOR}
${OPERATOR_FRAG}
${TEXEL}
${EDIT_SCOPE}
out vec4 fragColor;

// The decoded speed, coverage and kind at the four texels about a point,
// blended: the speed and the coverage bilinearly, the kind from the nearest,
// and the azimuth from the nearest too, for an operator that turns it.
struct Field { float speed; float coverage; bool wind; float azimuth; };

Field sampleField(vec2 uv) {
  vec2 size = vec2(textureSize(uTile, 0));
  vec2 texel = uv * size - 0.5;
  vec2 base = floor(texel);
  vec2 f = texel - base;
  ivec2 b = ivec2(base);
  ivec2 maxT = ivec2(size) - 1;

  uint w00 = texelWord(clamp(b + ivec2(0, 0), ivec2(0), maxT));
  uint w10 = texelWord(clamp(b + ivec2(1, 0), ivec2(0), maxT));
  uint w01 = texelWord(clamp(b + ivec2(0, 1), ivec2(0), maxT));
  uint w11 = texelWord(clamp(b + ivec2(1, 1), ivec2(0), maxT));
  Field out_;
  out_.speed = mix(mix(wordSpeed(w00), wordSpeed(w10), f.x),
                   mix(wordSpeed(w01), wordSpeed(w11), f.x), f.y);
  out_.coverage = mix(mix(wordCoverage(w00), wordCoverage(w10), f.x),
                      mix(wordCoverage(w01), wordCoverage(w11), f.x), f.y);
  uint nearest = f.x < 0.5 ? (f.y < 0.5 ? w00 : w01) : (f.y < 0.5 ? w10 : w11);
  out_.wind = wordIsWind(nearest);
  out_.azimuth = wordAzimuth(nearest);
  return out_;
}

// The gradient's stops, evenly spaced from calm to the top and interpolated
// linearly between — the same walk rampColour in ramp.ts makes, so the legend
// and a pixel agree.
vec3 ramp(float t, bool wind) {
  t = clamp(t, 0.0, 1.0);
  int count = wind ? uRampCountWind : uRampCountCurrent;
  // A gradient with fewer than two stops is not one. It cannot happen from
  // the catalogue, but an uninitialised uniform can, and black is a better
  // answer than reading past the end of the array.
  if (count < 2) return vec3(0.0);
  float scaled = t * float(count - 1);
  int lower = int(min(floor(scaled), float(count - 2)));
  float f = scaled - float(lower);
  vec3 a = wind ? uRampStopsWind[lower] : uRampStopsCurrent[lower];
  vec3 b = wind ? uRampStopsWind[lower + 1] : uRampStopsCurrent[lower + 1];
  return mix(a, b, f);
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
  if (uExact == 1) {
    vec2 exact = exactTileUV(uTileGeo);
    if (exact.y > 10.0) discard;
    return exact;
  }
  if (uProjection == 0 || uProjection >= 14) return vUV;
  // gl_FragCoord is y-up from the bottom; the camera's y is y-down from the top.
  float screenY = uViewport.y - gl_FragCoord.y;
  float y = uCamera.y - (screenY - uViewport.y * 0.5) / uCamera.z;
  return vec2(vUV.x, (uTileGeo.y - yToLat(y)) / uTileGeo.w);
}

void main() {
  // The far side of a globe (mode 15 up); always positive on a flat map.
  if (vHorizon < 0.0) discard;
  vec2 uv = tileUV();
  Field field = sampleField(uv);
  if (uCoverageOnly == 1) {
    fragColor = vec4(0.0, 0.0, 0.0, field.coverage);
    return;
  }
  // Only where the pixel is showing the layer being edited (M44). Elsewhere
  // the gesture is over a layer it does not touch, and the map must not move.
  bool mine = editedHere(uv);
  // A modifier in progress changes the speed here, live (M32).
  if (mine && uOpKind >= 3) opApply(gl_FragCoord.xy, opCoverage(), field.speed, field.azimuth);
  // Each kind on its own scale: wind and current are an order of magnitude
  // apart, and the cell says which it is (M31).
  vec2 rampEnds = field.wind ? uRampWind : uRampCurrent;
  vec3 colour = ramp((field.speed - rampEnds.x) / max(rampEnds.y - rampEnds.x, 0.001), field.wind);
  // Zero and undefined are different things: a written calm is as opaque as
  // any other written cell, and only where nothing wrote does the map show
  // through (M31, D58). A feathered edge fades with its coverage.
  float alpha = field.coverage * ${FIELD_OPACITY.toFixed(2)};
  fragColor = vec4(colour * uDim, alpha * (mine ? maskFactor() : 1.0));
}
`;

/**
 * A backdrop tile: a chart, a map tile, or a GIS layer (spec.md 4.11).
 *
 * The field's own vertex shader, because a backdrop is on exactly the same
 * tile grid as the field and has to follow the projection the same way; only
 * what is done with the texel differs. The texture is straight RGBA, drawn
 * under everything, and nothing about it reaches an evaluation or an export.
 */
export const BACKDROP_FRAG = `#version 300 es
precision highp float;
in vec2 vUV;
in float vHorizon;
${PROJECTION}
${EXACT_TILE}
uniform vec4 uTileGeo;
uniform sampler2D uTile;
uniform float uOpacity;
out vec4 fragColor;
void main() {
  // The far side of a globe (mode 15 up); always positive on a flat map.
  if (vHorizon < 0.0) discard;
  vec2 uv = vUV;
  if (uExact == 1) {
    vec2 exact = exactTileUV(uTileGeo);
    if (exact.y > 10.0) discard;
    uv = exact;
  }
  vec4 texel = texture(uTile, uv);
  if (texel.a <= 0.0) discard;
  fragColor = vec4(texel.rgb, texel.a * uOpacity);
}
`;

/**
 * Direction glyphs, instanced.
 *
 * One draw call per visible tile. Stations use a globe-anchored lattice, with
 * finer sites filling gaps in sparse fields. Each instance reads its vector
 * straight from the tile texture; the CPU retains coverage and kind bits only.
 *
 * Geometry is built from `gl_VertexID` against a fixed 54-vertex budget:
 * 6 for the shaft, then eight 6-vertex slots for barb flags. Unused slots
 * collapse to a point and are discarded by the rasteriser. That is how one
 * program draws both a plain arrow and a barb carrying up to eight flags.
 */
export const GLYPH_VERT = `#version 300 es
precision highp float;
${PROJECTION}
layout(location = 0) in vec2 aStation; // longitude, latitude
layout(location = 1) in vec2 aScreen;
layout(location = 2) in vec4 aBasis; // east.xy, north.xy in screen coordinates
uniform vec4 uTileGeo;      // west, north, spanX, spanY
uniform sampler2D uTile;
// The same tile without the layer being edited, and whether it is bound
// (M44): a glyph belonging to another layer must not turn under a modifier
// aimed at this one.
uniform sampler2D uBelow;
uniform highp int uEditScoped;
uniform float uSpeedScale;
uniform float uLengthArrow;
uniform float uLengthBarb;
uniform float uStrokeArrow;
uniform float uStrokeBarb;
uniform vec4 uColorArrow;
uniform vec4 uColorBarb;
uniform highp int uFadeArrow;
uniform highp int uFadeBarb;
uniform highp int uShadowPass;
uniform vec4 uShadowArrow;
uniform vec4 uShadowBarb;
uniform vec2 uShadowOffsetArrow;
uniform vec2 uShadowOffsetBarb;
uniform float uPixelRatio;  // device pixels per CSS pixel; keeps strokes even
${TEXEL}
${OPERATOR}
out float vShade;
out float vFade;
flat out vec4 vColor;

const float DEG = 0.017453292519943295;

vec2 rotate(vec2 v, float radians) {
  float c = cos(radians), s = sin(radians);
  return vec2(v.x * c - v.y * s, v.x * s + v.y * c);
}

void main() {
  float lon = aStation.x;
  float lat = aStation.y;

  vec2 uv = vec2((lon - uTileGeo.x) / uTileGeo.z, (uTileGeo.y - lat) / uTileGeo.w);
  if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
    gl_Position = vec4(2.0, 2.0, 2.0, 1.0);
    return;
  }

  vec2 station = uProjection >= 14 ? aScreen : geoToScreen(vec2(lon, lat));
  // The station with y up, as the operator measures things.
  vec2 stationUp = vec2(station.x, uViewport.y - station.y);
  float covered = opCoverageAt(stationUp);
  // What the operator leaves of this glyph: none of it where a mask or an
  // eraser covers, all of it where a clone's source is drawn in.
  vFade = maskFactorAt(stationUp);

  // A liquify reads the field from where it moved it from (M32). Outside
  // this tile the texel is another tile's, which this instance cannot read:
  // the glyph goes, and the tile the source is in draws its own.
  if (uProjection < 14 && uOpKind == 6 && covered > 0.0) {
    vec2 sourceUp = stationUp - opDisplacement(stationUp);
    vec2 source = vec2(sourceUp.x, uViewport.y - sourceUp.y);
    float sourceLon = (source.x - uViewport.x * 0.5) / uCamera.z + uCamera.x - uLonOffset;
    float sourceLat = yToLat(uCamera.y - (source.y - uViewport.y * 0.5) / uCamera.z);
    uv = vec2((sourceLon - uTileGeo.x) / uTileGeo.z, (uTileGeo.y - sourceLat) / uTileGeo.w);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
      gl_Position = vec4(2.0, 2.0, 2.0, 1.0);
      return;
    }
  }

  ivec2 size = textureSize(uTile, 0);
  ivec2 texel = clamp(ivec2(uv * vec2(size)), ivec2(0), size - 1);
  uint word = texelWord(texel);
  // Whether this glyph belongs to the layer being edited (M44). Read at the
  // station's own texel, which is the one the glyph is drawn from. A glyph
  // that is not the edited layer's is left exactly as it was: neither turned
  // by a modifier nor faded by a mask.
  if (uEditScoped == 1 && word == texelWordFrom(uBelow, texel)) {
    covered = 0.0;
    vFade = 1.0;
  }
  // Nothing wrote here: no glyph, whatever the calm beneath would draw as.
  if (wordCoverage(word) <= 0.0) {
    gl_Position = vec4(2.0, 2.0, 2.0, 1.0);
    return;
  }
  float speed = wordSpeed(word);
  float azimuth = wordAzimuth(word);
  // A modifier in progress changes the vector here, live (M32).
  if (uOpKind >= 3) opApply(stationUp, covered, speed, azimuth);
  // The cell's kind chooses the glyph (M31): wind is a barb, a current an
  // arrow, always.
  bool barb = wordIsWind(word);

  bool fade = (barb ? uFadeBarb : uFadeArrow) == 1;
  vShade = fade ? clamp(speed / 25.0, 0.25, 1.0) : 1.0;
  vec4 ink = barb ? uColorBarb : uColorArrow;
  vColor = ink;
  if (uShadowPass == 1) {
    vColor = barb ? uShadowBarb : uShadowArrow;
    vColor.a *= ink.a;
    if (vColor.a <= 0.0) { gl_Position = vec4(2.0, 2.0, 2.0, 1.0); return; }
    station += barb ? uShadowOffsetBarb : uShadowOffsetArrow;
  }

  // Screen space is y-down, so north is -y. Azimuth is clockwise from north.
  float az = azimuth * DEG;
  vec2 toward = uProjection >= 14 ? normalize(aBasis.xy * sin(az) + aBasis.zw * cos(az)) : vec2(sin(az), -cos(az));
  float length_px = barb ? uLengthBarb : uLengthArrow;
  float stroke_px = barb ? uStrokeBarb : uStrokeArrow;

  vec2 offset = vec2(0.0);
  int id = gl_VertexID;

  if (!barb) {
    // ---- Arrow: shaft quad plus a head triangle ----
    float halfW = stroke_px * 0.5;
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
      float sizeScale = uLengthBarb / (${(GLYPH_TARGET_PX.barb * GLYPH_SIZE_SCALE.barb).toFixed(2)} * uPixelRatio);
      float r = max(2.0 * uPixelRatio, stroke_px) * sizeScale;
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
        float halfW = stroke_px * 0.5;
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
          vec2 perp = normalize(vec2(-flagDir.y, flagDir.x)) * stroke_px * 0.5;
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
in float vFade;
flat in vec4 vColor;
out vec4 fragColor;
// The glyphs follow the field they describe: a glyph left standing over an
// masked patch would be pointing at a wind that is no longer there.
void main() { fragColor = vec4(vColor.rgb, vColor.a * vShade * vFade); }
`;

/**
 * A liquify in progress, composited over the map (M32).
 *
 * The field is first drawn alone into a texture the size of the viewport.
 * This pass then reads it back at each covered fragment from where the
 * stroke moved the field from, so the field slides under the pointer in
 * screen space with no seam at a tile's edge — the one displacement that
 * varies from pixel to pixel, which a camera shift cannot show.
 */
export const SMEAR_VERT = `#version 300 es
precision highp float;
in vec2 aCorner;            // 0..1 across the viewport
void main() {
  gl_Position = vec4(aCorner * 2.0 - 1.0, 0.0, 1.0);
}
`;

export const SMEAR_FRAG = `#version 300 es
precision highp float;
uniform sampler2D uField;   // the field alone, at the framebuffer's size
${OPERATOR}
${OPERATOR_FRAG}
out vec4 fragColor;
void main() {
  float covered = opCoverage();
  if (covered <= 0.0) discard;
  vec2 source = gl_FragCoord.xy - opDisplacement(gl_FragCoord.xy);
  vec4 moved = texture(uField, source / uMaskSize);
  fragColor = vec4(moved.rgb, moved.a * covered);
}
`;

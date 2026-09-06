/**
 * The range of speeds a tile holds, read from its bytes as they are uploaded.
 *
 * What the auto scale (spec.md 5.3, M27) runs the colour ramp over: the
 * slowest and fastest speed among the tiles on screen. Read once per tile on
 * the CPU, from the same bytes the GPU gets, so it costs nothing per frame
 * and never needs a readback.
 */

/** Speed is the low two bytes of each RGBA texel, a 16-bit little-endian fraction of full scale. */
const BYTES_PER_TEXEL = 4;

/**
 * The smallest and largest speed in a tile, as 16-bit fractions of full
 * scale, or null when the tile holds no field at all.
 *
 * **A zero is left out of the minimum.** A cell no object wrote is undefined,
 * and the tile encodes undefined as calm (0): counting it would put the
 * bottom of the ramp at zero whenever any of the viewport is unpainted,
 * which is nearly always. A painted calm is indistinguishable from an
 * unpainted cell in the tile, and the ramp fades both to nothing anyway.
 */
export function tileSpeedRange(bytes: Uint8Array): [number, number] | null {
  let min = 0xffff + 1;
  let max = -1;
  for (let o = 0; o + 1 < bytes.length; o += BYTES_PER_TEXEL) {
    const speed = (bytes[o] ?? 0) | ((bytes[o + 1] ?? 0) << 8);
    if (speed === 0) continue;
    if (speed < min) min = speed;
    if (speed > max) max = speed;
  }
  return max < 0 ? null : [min, max];
}

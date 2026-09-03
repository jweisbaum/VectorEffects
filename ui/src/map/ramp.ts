/**
 * The speed colour ramp, in TypeScript.
 *
 * The fragment shader in `shaders.ts` carries the same stops; they are
 * duplicated because CSS cannot read GLSL and neither can a 2D canvas. Change
 * both together, or the legend and the brush preview stop describing the map.
 *
 * Perceptually ordered: dark and desaturated at calm, hot at the top, so speed
 * reads as intensity rather than as an arbitrary hue cycle.
 */

/** One ramp stop as 0-1 RGB, low speed to high. */
export type Rgb = readonly [number, number, number];

/** Ramp stops, evenly spaced across the range, low speed to high. */
export const RAMP_COLOURS: readonly Rgb[] = [
  [0.05, 0.09, 0.16],
  [0.12, 0.35, 0.62],
  [0.16, 0.68, 0.66],
  [0.55, 0.8, 0.35],
  [0.96, 0.78, 0.24],
  [0.92, 0.42, 0.2],
  [0.78, 0.16, 0.36],
];

function channel(value: number): number {
  return Math.round(Math.min(1, Math.max(0, value)) * 255);
}

/** A stop as a CSS colour, for the legend gradient. */
function hex(colour: Rgb): string {
  return `#${colour.map((c) => channel(c).toString(16).padStart(2, "0")).join("")}`;
}

/** The ramp as CSS colour stops, for a `linear-gradient`. */
export const RAMP_STOPS: readonly string[] = RAMP_COLOURS.map(hex);

/**
 * Samples the ramp at `t`, clamped to [0, 1].
 *
 * Linear between stops, exactly as the shader interpolates, so a colour picked
 * here and a pixel drawn there agree.
 */
export function rampColour(t: number): Rgb {
  const clamped = Math.min(1, Math.max(0, t));
  const last = RAMP_COLOURS.length - 1;
  const scaled = clamped * last;
  const lower = Math.min(last, Math.floor(scaled));
  const upper = Math.min(last, lower + 1);
  const f = scaled - lower;
  const a = RAMP_COLOURS[lower] as Rgb;
  const b = RAMP_COLOURS[upper] as Rgb;
  return [
    a[0] + (b[0] - a[0]) * f,
    a[1] + (b[1] - a[1]) * f,
    a[2] + (b[2] - a[2]) * f,
  ];
}

/**
 * The colour the field renderer would paint for `speed`, as a CSS colour.
 *
 * Alpha follows the shader: calm fades out entirely so the basemap stays
 * readable where there is nothing to show, and the ramp is capped below opaque
 * so the coastlines show through everywhere else. `minAlpha` is for callers who
 * need the shape visible whatever the speed -- a preview of a calm stroke still
 * has to be something the user can aim.
 */
export function rampCss(speed: number, rampMax: number, minAlpha = 0): string {
  const [r, g, b] = rampColour(speed / Math.max(rampMax, 0.001));
  const alpha = Math.max(
    minAlpha,
    Math.min(1, Math.max(0, speed / (rampMax * 0.06))) * 0.72,
  );
  return `rgba(${channel(r)}, ${channel(g)}, ${channel(b)}, ${alpha.toFixed(3)})`;
}

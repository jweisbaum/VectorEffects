/**
 * The sea-surface temperature layer's colours and numbers (spec.md 4.10, M93).
 *
 * The tiles carry temperatures, not colours (`ve_app::sst::pack`): sixteen
 * bits per pixel, which the map's shader colours on the ramp in force. That
 * ramp is the fixed −2 to 32 °C one, or — with the auto scale on — the
 * temperatures in view, as the field's ramp follows the speeds in view
 * (spec.md 5.3). The legend and the shader read the same stops from here.
 * Celsius throughout — only what is shown changes with the unit setting,
 * never what is stored.
 */
import type { TemperatureUnit } from "../generated/TemperatureUnit";

/** The coldest temperature the ramp distinguishes, °C. Colder takes its colour. */
export const SST_MIN_C = -2;
/** The warmest, °C. Warmer takes its colour. */
export const SST_MAX_C = 32;

/** The coldest temperature a tile can carry, °C: `ve_app::sst::CODE_MIN_C`. */
export const SST_CODE_MIN_C = -5;
/** The warmest, °C: `ve_app::sst::CODE_MAX_C`. */
export const SST_CODE_MAX_C = 45;

/** A tile pixel's two bytes, low then high, as degrees Celsius. */
export function unpackTemperature(low: number, high: number): number {
  return SST_CODE_MIN_C + ((low | (high << 8)) / 65535) * (SST_CODE_MAX_C - SST_CODE_MIN_C);
}

/**
 * A tile's coldest and warmest water, °C, or null for a tile with none.
 * Read once, when the tile arrives, as a field tile's speed range is.
 */
export function sstTileRange(bytes: Uint8Array): [number, number] | null {
  let low = 65536;
  let high = -1;
  for (let offset = 0; offset + 3 < bytes.length; offset += 4) {
    if (bytes[offset + 3] === 0) continue;
    const code = bytes[offset]! | (bytes[offset + 1]! << 8);
    if (code < low) low = code;
    if (code > high) high = code;
  }
  if (high < 0) return null;
  const scale = (SST_CODE_MAX_C - SST_CODE_MIN_C) / 65535;
  return [SST_CODE_MIN_C + low * scale, SST_CODE_MIN_C + high * scale];
}

/**
 * The narrowest span the auto scale gives the temperature ramp, °C: one
 * temperature everywhere would otherwise put the view in one colour.
 */
const AUTO_SCALE_MIN_SPAN_C = 1;

/** The temperature ramp in force, °C. */
export interface SstRamp {
  min: number;
  max: number;
  /** Whether it follows the view rather than being the fixed one. */
  auto: boolean;
}

/**
 * The ramp for what the last frame drew: the fixed one when nothing was
 * seen or the auto scale is off (the caller passes null), else the range
 * seen, never narrower than a degree.
 */
export function sstRampOf(seen: { min: number; max: number } | null): SstRamp {
  if (seen === null) return { min: SST_MIN_C, max: SST_MAX_C, auto: false };
  return { min: seen.min, max: Math.max(seen.max, seen.min + AUTO_SCALE_MIN_SPAN_C), auto: true };
}

/** Seven evenly spaced sRGB stops, cold to warm, from `SST_MIN_C` to `SST_MAX_C`. */
export const SST_STOPS: ReadonlyArray<readonly [number, number, number]> = [
  [40, 26, 120],
  [33, 102, 172],
  [67, 170, 196],
  [153, 213, 148],
  [254, 224, 139],
  [244, 109, 67],
  [165, 0, 38],
];

/** The ramp as CSS colour stops, for a legend's `linear-gradient`. */
export function sstGradientStops(): string[] {
  const last = SST_STOPS.length - 1;
  return SST_STOPS.map(
    ([r, g, b], index) => `rgb(${r}, ${g}, ${b}) ${((index / last) * 100).toFixed(2)}%`,
  );
}

/**
 * The colour a temperature is drawn in: linear between the two stops it
 * falls between, the end colour beyond either end. Unrounded.
 */
export function sstColour(celsius: number): [number, number, number] {
  const last = SST_STOPS.length - 1;
  const t = Math.min(Math.max((celsius - SST_MIN_C) / (SST_MAX_C - SST_MIN_C), 0), 1) * last;
  const low = Math.min(Math.floor(t), last - 1);
  const f = t - low;
  const a = SST_STOPS[low]!;
  const b = SST_STOPS[low + 1]!;
  return [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f];
}

/** A Celsius temperature in the chosen unit. */
export function temperatureIn(celsius: number, unit: TemperatureUnit): number {
  return unit === "fahrenheit" ? (celsius * 9) / 5 + 32 : celsius;
}

/** The unit's symbol. A unit symbol, never translated. */
export function temperatureSymbol(unit: TemperatureUnit): string {
  return unit === "fahrenheit" ? "°F" : "°C";
}

/**
 * A temperature for the readout: one decimal, in the chosen unit, with the
 * legend's minus sign — and none on a value that rounds to zero.
 */
export function formatTemperature(celsius: number, unit: TemperatureUnit): string {
  const value = temperatureIn(celsius, unit);
  const digits = Math.abs(value).toFixed(1);
  const negative = value < 0 && digits !== "0.0";
  return `${negative ? "−" : ""}${digits} ${temperatureSymbol(unit)}`;
}

/**
 * A legend end: whole degrees in the chosen unit, or tenths when the ramp
 * spans under five degrees of that unit — an auto-scaled ramp a degree wide
 * would otherwise read as two equal numbers. A minus sign rather than a
 * hyphen, the way a printed scale writes it, and none on a zero.
 */
export function legendTemperature(celsius: number, unit: TemperatureUnit, spanC = Infinity): string {
  const span = unit === "fahrenheit" ? (spanC * 9) / 5 : spanC;
  const digits = span < 5 ? 1 : 0;
  const value = temperatureIn(celsius, unit);
  const text = Math.abs(value).toFixed(digits);
  const negative = value < 0 && Number(text) !== 0;
  return `${negative ? "−" : ""}${text} ${temperatureSymbol(unit)}`;
}

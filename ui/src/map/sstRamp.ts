/**
 * The sea-surface temperature layer's colours and numbers (spec.md 4.10, M93).
 *
 * The tiles are coloured in Rust; this is the same ramp repeated so the
 * legend can show it. **A copy, held to the backend's by a test, not a
 * second authority**: a legend whose colours differed from the pixels beside
 * it would be worse than none. Celsius throughout — only what is shown
 * changes with the unit setting, never what is stored.
 */
import type { TemperatureUnit } from "../generated/TemperatureUnit";

/** The coldest temperature the ramp distinguishes, °C. Colder takes its colour. */
export const SST_MIN_C = -2;
/** The warmest, °C. Warmer takes its colour. */
export const SST_MAX_C = 32;

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
 * A legend end: whole degrees in the chosen unit. A minus sign rather than a
 * hyphen, the way a printed scale writes it.
 */
export function legendTemperature(celsius: number, unit: TemperatureUnit): string {
  const value = Math.round(temperatureIn(celsius, unit));
  return `${value < 0 ? "−" : ""}${Math.abs(value)} ${temperatureSymbol(unit)}`;
}

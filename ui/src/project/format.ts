/** Project file extension. Mirrors `ve_core::io::EXTENSION`. */
export const EXTENSION = "veproj";

/** Grid resolutions offered when creating a project. */
export const RESOLUTIONS = [
  { value: "1.0", label: "1°" },
  { value: "0.5", label: "0.5°" },
  { value: "0.25", label: "0.25°" },
  { value: "0.1", label: "0.1°" },
] as const;

/** Time-step sizes offered when creating a project. */
export const STEP_HOURS = [1, 3, 6, 24] as const;

/** Largest number of steps a project may have. Mirrors `ve_core::project`. */
export const MAX_STEPS = 240;

/**
 * Grid dimensions for a resolution.
 *
 * Mirrors `Resolution::ni`/`nj` so the create dialog can show the grid size
 * before the project exists. The authority is Rust; this is display only.
 */
export function gridSize(resolutionDeg: number): { ni: number; nj: number } {
  return {
    ni: Math.round(360 / resolutionDeg),
    nj: Math.round(180 / resolutionDeg) + 1,
  };
}

/** Rough size of the GRIB this project would export, in bytes. */
export function estimatedGribBytes(resolutionDeg: number, stepCount: number): number {
  const { ni, nj } = gridSize(resolutionDeg);
  // Two components, 16-bit packing, plus a little for section headers.
  return ni * nj * 2 * 2 * stepCount;
}

/** Formats a byte count for display. */
export function formatBytes(bytes: number): string {
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${Math.round(bytes / (1024 * 1024))} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

/**
 * Knots per metre per second. Mirrors `ve_core::units::KNOTS_PER_MPS`.
 */
export const KNOTS_PER_MPS = 1.943844492440605;

/**
 * Converts a stored speed to the displayed one.
 *
 * Speed is stored in m/s — what GRIB2 encodes — and shown in knots, always.
 * Not configurable: a sailing forecast and a wind barb are both in knots,
 * and a barb is defined in 5-knot increments.
 */
export function knotsFromMps(mps: number): number {
  return mps * KNOTS_PER_MPS;
}

/** Converts an entered speed to the stored one. */
export function mpsFromKnots(knots: number): number {
  return knots / KNOTS_PER_MPS;
}

/**
 * Converts a stored azimuth-toward into the project's display convention.
 *
 * The only place this conversion happens in the frontend (spec.md 3.3).
 */
export function displayDirection(convention: string, azimuthToward: number): number {
  return convention === "from" ? (azimuthToward + 180) % 360 : azimuthToward;
}

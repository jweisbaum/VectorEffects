import type { ProjectRegion } from "../generated/ProjectRegion";
import type { RegionRequest } from "../generated/RegionRequest";
import { normalizeLon, regionNodes } from "./regionPick";

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

/**
 * Rough size of the GRIB this project would export, in bytes: of the whole
 * earth, or of `region`'s lattice when it has one.
 */
export function estimatedGribBytes(
  resolutionDeg: number,
  stepCount: number,
  region: RegionRequest | null = null,
): number {
  const { ni, nj } = region === null ? gridSize(resolutionDeg) : regionNodes(region, resolutionDeg);
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

/** Converts canonical m/s to knots for physical barbs and legacy scale fields. */
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

const number = (degrees: number) => String(Math.round(Math.abs(degrees) * 100) / 100);

function formatLon(lon: number): string {
  const n = normalizeLon(lon);
  if (n === 0 || n === -180) return `${number(n)}°`;
  return `${number(n)}°${n > 0 ? "E" : "W"}`;
}

function formatLat(lat: number): string {
  if (lat === 0) return "0°";
  return `${number(lat)}°${lat > 0 ? "N" : "S"}`;
}

/**
 * A project's region as the status bar shows it: west – east, then south –
 * north, with hemispheres rather than signs. `lon` is null for a full
 * circle, which the caller says in words.
 */
export function formatRegion(region: ProjectRegion): { lon: string | null; lat: string } {
  return {
    lon: region.full_circle ? null : `${formatLon(region.west)} – ${formatLon(region.east)}`,
    lat: `${formatLat(region.south)} – ${formatLat(region.north)}`,
  };
}

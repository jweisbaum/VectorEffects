import { createContext, useContext, useMemo, type ReactNode } from "react";
import type { AppSettings } from "../generated/AppSettings";
import { knotsFromMps, mpsFromKnots } from "../project/format";

export type UnitPreferences = Pick<AppSettings, "distance_unit" | "speed_unit">;
export const DEFAULT_UNITS: UnitPreferences = { distance_unit: "km", speed_unit: "kt" };

/** Conversions at the view boundary. Document values stay in m/s and km. */
export function displayUnits(preferences: UnitPreferences = DEFAULT_UNITS) {
  const speedUnit = preferences.speed_unit === "kmh" ? "km/h" : preferences.speed_unit;
  const distanceUnit = preferences.distance_unit;
  const speedFactor = preferences.speed_unit === "mph" ? 3600 / 1609.344
    : preferences.speed_unit === "kmh" ? 3.6 : knotsFromMps(1);
  const distanceFactor = distanceUnit === "nm" ? 1 / 1.852 : 1;
  const speedFromMps = (value: number) => value * speedFactor;
  const speedToMps = (value: number) => value / speedFactor;
  const distanceFromKm = (value: number) => value * distanceFactor;
  const distanceToKm = (value: number) => value / distanceFactor;
  return {
    speedUnit, distanceUnit, speedFromMps, speedToMps, distanceFromKm, distanceToKm,
    speedFromKnots: (value: number) => speedFromMps(mpsFromKnots(value)),
    speedToKnots: (value: number) => knotsFromMps(speedToMps(value)),
    toDisplay: (unit: string, value: number) => unit === "speed" ? speedFromMps(value)
      : unit === "kilometres" ? distanceFromKm(value) : value,
    toStored: (unit: string, value: number) => unit === "speed" ? speedToMps(value)
      : unit === "kilometres" ? distanceToKm(value) : value,
    suffix: (unit: string) => unit === "speed" ? speedUnit : unit === "kilometres" ? distanceUnit
      : unit === "degrees" || unit === "signed_degrees" || unit === "direction" ? "°" : unit === "percent" ? "%" : "",
  };
}

export type DisplayUnits = ReturnType<typeof displayUnits>;
const UnitsContext = createContext(displayUnits());

export function UnitsProvider({ settings, children }: {
  settings: UnitPreferences | null;
  children: ReactNode;
}) {
  const distance = settings?.distance_unit ?? "km";
  const speed = settings?.speed_unit ?? "kt";
  const units = useMemo(() => displayUnits({ distance_unit: distance, speed_unit: speed }), [distance, speed]);
  return <UnitsContext.Provider value={units}>{children}</UnitsContext.Provider>;
}

export const useUnits = () => useContext(UnitsContext);

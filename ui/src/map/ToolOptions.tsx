/**
 * The option bar, rendered from what the backend says a tool has.
 *
 * There is one of these for the whole catalogue, deliberately. Every rule it
 * applies — which options a mode makes inert, that a size in px selects a
 * projected stamp, that a flow direction is shown in the project's convention
 * and a geometric bearing is not — is a rule spec 6.1 states for *every* tool,
 * and a bar per tool is six chances to get one of them wrong.
 *
 * Adding an option to a tool is therefore one line in `ve_core::schema` and
 * nothing here. If it ever needs more, the fix belongs in the property system.
 */

import type { PropertyValue } from "../generated/PropertyValue";
import type { ToolOptionSpec } from "../generated/ToolOptionSpec";
import type { ToolSchema } from "../generated/ToolSchema";
import { knotsFromMps, mpsFromKnots } from "../project/format";
import type { Camera } from "./camera";
import {
  convertSizes,
  liveOptions,
  type SizeUnit,
  shownAngle,
  type ToolState,
} from "./tools";

/** A schema variant name as a label: `toward_point` becomes `Toward point`. */
export function variantLabel(name: string): string {
  const words = name.replace(/_/g, " ");
  return words.charAt(0).toUpperCase() + words.slice(1);
}

/**
 * A position option the bar is waiting to place with a map click.
 *
 * Every `LonLat` option is placeable by pointing, on the tool as well as on the
 * object (spec.md 6.1). The bar arms it; the map takes the click.
 */
export interface ToolPick {
  /** The property being placed. */
  property: string;
  /** Its label, for the marker on the map. */
  label: string;
}

export default function ToolOptions({
  schema,
  state,
  onChange,
  convention,
  camera,
  picking,
  onPick,
}: {
  schema: ToolSchema;
  state: ToolState;
  onChange: (next: ToolState) => void;
  /** The project's direction convention, for flow directions. */
  convention: string;
  /** The camera, for converting a size between px and km. */
  camera: Camera;
  /** The position option waiting for a click, if any. */
  picking: ToolPick | null;
  onPick: (pick: ToolPick | null) => void;
}) {
  const set = (property: string, value: PropertyValue) =>
    onChange({ ...state, values: { ...state.values, [property]: value } });

  const options = liveOptions(schema, state.values);
  // The unit selector belongs to the first size the tool has, and governs all
  // of them: `stamp_space` is one property per object, so a diameter in px and
  // a ring width in km would describe a shape that does not exist (spec.md 3.5).
  const firstSize = options.find((spec) => spec.unit === "kilometres")?.property;

  return (
    <div className="tool-options">
      {options.map((spec) => (
        <Option
          key={spec.property}
          spec={spec}
          state={state}
          convention={convention}
          showUnit={spec.property === firstSize}
          picking={picking}
          onPick={onPick}
          onValue={(value) => set(spec.property, value)}
          onUnit={(unit) =>
            onChange(convertSizes(state, schema, unit, camera, camera.centerLat))
          }
        />
      ))}
    </div>
  );
}

function Option({
  spec,
  state,
  convention,
  showUnit,
  picking,
  onPick,
  onValue,
  onUnit,
}: {
  spec: ToolOptionSpec;
  state: ToolState;
  convention: string;
  showUnit: boolean;
  picking: ToolPick | null;
  onPick: (pick: ToolPick | null) => void;
  onValue: (value: PropertyValue) => void;
  onUnit: (unit: SizeUnit) => void;
}) {
  const value = state.values[spec.property] ?? spec.default;

  switch (value.kind) {
    case "choice":
      return (
        <label>
          {spec.label}
          <select
            value={value.index}
            onChange={(e) => onValue({ kind: "choice", index: Number(e.target.value) })}
          >
            {spec.variants.map((name, index) => (
              <option key={name} value={index}>
                {variantLabel(name)}
              </option>
            ))}
          </select>
        </label>
      );

    case "bool":
      return (
        <label>
          <input
            type="checkbox"
            checked={value.value}
            onChange={(e) => onValue({ kind: "bool", value: e.target.checked })}
          />
          {spec.label}
        </label>
      );

    case "angle": {
      // A flow direction is shown in the project's convention; a geometric
      // bearing is not (spec.md 3.3). The conversion is its own inverse, so the
      // same call reads the input back.
      const shown = shownAngle(spec.unit, convention, value.degrees);
      return (
        <label>
          {spec.label}
          <input
            type="number"
            min={0}
            max={360}
            step={5}
            value={Math.round(shown)}
            onChange={(e) =>
              onValue({
                kind: "angle",
                degrees: shownAngle(spec.unit, convention, Number(e.target.value) || 0),
              })
            }
          />
          {spec.unit === "direction" ? `° (${convention})` : "°"}
        </label>
      );
    }

    case "position": {
      const armed = picking?.property === spec.property;
      return (
        <div className="tool-position">
          <label>
            {spec.label}
            <input
              type="number"
              step="any"
              title="Longitude"
              value={value.lon}
              onChange={(e) =>
                onValue({ kind: "position", lon: Number(e.target.value) || 0, lat: value.lat })
              }
            />
          </label>
          <input
            type="number"
            step="any"
            title="Latitude"
            value={value.lat}
            onChange={(e) =>
              onValue({
                kind: "position",
                lon: value.lon,
                lat: Math.min(90, Math.max(-90, Number(e.target.value) || 0)),
              })
            }
          />
          <button
            className={armed ? "active" : ""}
            onClick={() =>
              onPick(armed ? null : { property: spec.property, label: spec.label })
            }
            title={`Click the map to place ${spec.label.toLowerCase()}. Once placed, drag the marker to move it.`}
          >
            {armed ? "Click the map…" : "Pick on map"}
          </button>
        </div>
      );
    }

    case "number": {
      // A fraction with its own natural range reads better as a slider than as
      // a number nobody can guess the scale of.
      const isFraction = spec.min === 0 && spec.max === 1;
      if (isFraction) {
        return (
          <label>
            {spec.label}
            <input
              type="range"
              min={0}
              max={1}
              step={0.05}
              value={value.value}
              onChange={(e) => onValue({ kind: "number", value: Number(e.target.value) })}
            />
          </label>
        );
      }

      // Speed is stored in m/s and always shown in knots (`ve_core::units`).
      if (spec.unit === "speed") {
        return (
          <label>
            {spec.label}
            <input
              type="number"
              min={0}
              max={200}
              step={1}
              value={Math.round(knotsFromMps(value.value))}
              onChange={(e) =>
                onValue({ kind: "number", value: mpsFromKnots(Number(e.target.value) || 0) })
              }
            />
            kt
          </label>
        );
      }

      const size = spec.unit === "kilometres";
      return (
        <label>
          {spec.label}
          <input
            type="number"
            min={1}
            max={size && state.unit === "px" ? 2000 : (spec.max ?? undefined)}
            step={size && state.unit === "px" ? 5 : 50}
            value={Math.round(value.value)}
            onChange={(e) =>
              onValue({ kind: "number", value: Math.max(1, Number(e.target.value) || 1) })
            }
          />
          {size && showUnit && (
            <select
              value={state.unit}
              onChange={(e) => onUnit(e.target.value === "px" ? "px" : "km")}
              title="A size in pixels paints a shape on the map — the same size on screen at any latitude. It resolves to kilometres when the object is created and never changes afterwards. One unit for the whole tool, because the space it selects is one property of the object."
            >
              <option value="km">km</option>
              <option value="px">px</option>
            </select>
          )}
          {size && !showUnit && state.unit}
          {!size && spec.unit === "percent" && "%"}
        </label>
      );
    }
  }
}

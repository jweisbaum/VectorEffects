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

import { memo } from "react";

import CentredSlider, { readoutFor } from "../CentredSlider";
import NumberField from "../NumberField";

import type { PropertyValue } from "../generated/PropertyValue";
import type { ToolOptionSpec } from "../generated/ToolOptionSpec";
import type { ToolSchema } from "../generated/ToolSchema";
import { useUnits } from "../settings/units";
import type { Camera } from "./camera";
import { releaseFocus } from "./focus";
import { EYEDROPPER_ICON, IconSvg } from "./ToolIcon";
import {
  convertSizes,
  liveOptions,
  offersEyedropper,
  offersUnit,
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

function ToolOptions({
  schema,
  state,
  onChange,
  convention,
  camera,
  picking,
  onPick,
  sampling,
  onSample,
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
  /** Whether the eyedropper is armed and waiting for a click. */
  sampling: boolean;
  onSample: (on: boolean) => void;
}) {
  const units = useUnits();
  const set = (property: string, value: PropertyValue) =>
    onChange({ ...state, values: { ...state.values, [property]: value } });

  const options = liveOptions(schema, state.values);
  const setUnit = (unit: SizeUnit) =>
    onChange(convertSizes(state, schema, unit, camera, camera.centerLat));

  // The unit selector belongs to the first size the tool has, and governs all
  // of them: `stamp_space` is one property per object, so a diameter in px and
  // a ring width in km would describe a shape that does not exist (spec.md 3.5).
  const firstSize = options.find((spec) => spec.unit === "kilometres")?.property;
  const unitIsLive = offersUnit(schema, state.values);
  const eyedropper = offersEyedropper(schema, state.values);

  return (
    <div className="tool-options">
      {options.map((spec) => (
        <Option
          key={spec.property}
          spec={spec}
          state={state}
          convention={convention}
          showUnit={unitIsLive && spec.property === firstSize}
          picking={picking}
          onPick={onPick}
          onValue={(value) => set(spec.property, value)}
          onUnit={setUnit}
        />
      ))}

      {/*
        A tool that measures but types no number: the shape fill, whose presets
        are dragged out on the map and whose polygon is clicked out vertex by
        vertex. The unit is still the question of whether what was drawn is a
        shape on the ground or one on the map (spec.md 3.5), so it is asked
        here rather than nowhere.
      */}
      {/*
        The eyedropper: take the speed and direction from the field itself.
        Aiming a wind by typing two numbers is guesswork next to pointing at
        one that is already there — a stroke that continues a front, or a fill
        that matches the flow it borders (spec.md 6.1). Offered only where the
        tool paints a single vector; a gradient has two of each and no answer.
      */}
      {eyedropper && (
        <button
          className={sampling ? "active" : ""}
          onClick={() => onSample(!sampling)}
          title={
            sampling
              ? "Click the map to take the speed and direction from the field there."
              : "Eyedropper: take the speed and direction from a point on the map. Only what is visible is sampled: a hidden layer contributes nothing."
          }
          aria-label={sampling ? "Sampling: click the map" : "Sample the field"}
        >
          <IconSvg icon={EYEDROPPER_ICON} />
        </button>
      )}

      {unitIsLive && firstSize === undefined && (
        <label>
          Shape in
          <select
            value={state.unit}
            onChange={(e) => {
              setUnit(unitOf(e.target.value));
              releaseFocus(e);
            }}
            title={DRAWN_UNIT_TITLE}
          >
            <option value="km">{units.distanceUnit} (on the ground)</option>
            <option value="px">px (on the map)</option>
          </select>
        </label>
      )}
    </div>
  );
}

/** Reads a unit off a select, defaulting to kilometres. */
function unitOf(value: string): SizeUnit {
  return value === "px" ? "px" : "km";
}

/**
 * What the unit means, for the control's tooltip.
 *
 * Shared by both places the unit is offered, so the two cannot come to explain
 * it differently.
 */
const UNIT_TITLE =
  "A size in pixels paints a shape on the map — the same size on screen at any latitude. " +
  "Its ground size is fixed when the object is created. " +
  "One unit for the whole tool, because the space it selects is one property of the object.";

/**
 * The same question, for a tool whose shape is drawn rather than typed.
 *
 * The shape fill has no number to attach a unit to: its presets are dragged out
 * and its polygon is clicked out vertex by vertex. What the control still
 * chooses is the plane the shape lives in — on the chart or on the sea (M57).
 */
const DRAWN_UNIT_TITLE =
  "px draws the shape on the map: straight edges stay straight on the chart, at any latitude. " +
  "A ground distance draws it on the ground, so it keeps its real proportions and bends with the projection. " +
  "Fixed when the object is created and never changes afterwards.";

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
  const units = useUnits();
  const value = state.values[spec.property] ?? spec.default;

  switch (value.kind) {
    case "choice":
      return (
        <label>
          {spec.label}
          <select
            value={value.index}
            onChange={(e) => {
              onValue({ kind: "choice", index: Number(e.target.value) });
              releaseFocus(e);
            }}
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
            onChange={(e) => {
              onValue({ kind: "bool", value: e.target.checked });
              releaseFocus(e);
            }}
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
          <NumberField
            min={0}
            max={360}
            step={5}
            value={shown}
            format={(v) => String(Math.round(v * 100) / 100)}
            onCommit={(degrees) =>
              onValue({ kind: "angle", degrees: shownAngle(spec.unit, convention, degrees) })
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
            <NumberField
              step="any"
              title="Longitude"
              value={value.lon}
              onCommit={(lon) => onValue({ kind: "position", lon, lat: value.lat })}
            />
          </label>
          <NumberField
            step="any"
            title="Latitude"
            min={-90}
            max={90}
            value={value.lat}
            onCommit={(lat) => onValue({ kind: "position", lon: value.lon, lat })}
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
      // A signed amount whose zero does nothing is a centred slider (M29):
      // the schema says so, and both ends are named.
      if (spec.slider) {
        return (
          <label>
            {spec.label}
            <CentredSlider
              value={value.value}
              min={spec.min ?? -100}
              max={spec.max ?? 100}
              lowLabel={spec.slider.low_label}
              highLabel={spec.slider.high_label}
              reversed={spec.slider.reversed}
              format={readoutFor(spec.slider)}
              onInput={(next) => onValue({ kind: "number", value: next })}
              onCommit={(next) => onValue({ kind: "number", value: next })}
            />
          </label>
        );
      }

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

      // Convert only at the editor boundary; the tool keeps canonical m/s.
      if (spec.unit === "speed") {
        return (
          <label>
            {spec.label}
            <NumberField
              min={0}
              max={spec.max === null ? null : units.speedFromMps(spec.max)}
              step={1}
              value={units.speedFromMps(value.value)}
              format={(v) => String(Math.round(v * 100) / 100)}
              onCommit={(speed) => onValue({ kind: "number", value: units.speedToMps(speed) })}
            />
            {units.speedUnit}
          </label>
        );
      }

      const size = spec.unit === "kilometres";
      const ground = size && state.unit !== "px";
      const shown = (v: number) => ground ? units.distanceFromKm(v) : v;
      return (
        <label>
          {spec.label}
          {/*
            The bounds are the schema's, not a floor of 1: a modifier's amount
            is *signed* — intensify/reduce, diverge/converge, a turn either way
            — and a hard-coded minimum made every one of them one-directional
            (spec.md 6.3).
          */}
          <NumberField
            min={shown(spec.min ?? 1)}
            max={size && state.unit === "px" ? 2000 : spec.max === null ? null : shown(spec.max)}
            step={size ? (state.unit === "px" ? 5 : 50) : 1}
            value={shown(value.value)}
            format={(v) => String(Math.round(v * 100) / 100)}
            onCommit={(next) => onValue({ kind: "number", value: ground ? units.distanceToKm(next) : next })}
          />
          {size && showUnit && (
            <select
              value={state.unit}
              onChange={(e) => {
              onUnit(unitOf(e.target.value));
              releaseFocus(e);
            }}
              title={UNIT_TITLE}
            >
              <option value="km">{units.distanceUnit}</option>
              <option value="px">px</option>
            </select>
          )}
          {size && !showUnit && (ground ? units.distanceUnit : state.unit)}
          {!size && units.suffix(spec.unit)}
        </label>
      );
    }
  }
}

/**
 * Memoised: the map re-renders on things the bar does not show — the pointer
 * readout, the busy flag — and rebuilding every control each time was a cost
 * paid on every pointer move for no change on screen.
 */
export default memo(ToolOptions);

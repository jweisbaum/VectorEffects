import { useEffect, useState } from "react";

import { api } from "../ipc";
import type { ProjectSummary } from "../generated/ProjectSummary";
import type { PropertyValue } from "../generated/PropertyValue";
import type { PropertyView } from "../generated/PropertyView";
import { knotsFromMps, mpsFromKnots } from "../project/format";
import type { PositionPick } from "../picking";
import { toShownAngle } from "./inspectorAngle";

/** Suffix shown after a property's editor. */
function unitLabel(unit: string): string {
  switch (unit) {
    case "speed":
      return "kt";
    case "kilometres":
      return "km";
    case "degrees":
      return "°";
    case "percent":
      return "%";
    default:
      return "";
  }
}

/** Converts a stored number into the one shown. */
function toDisplay(unit: string, value: number): number {
  return unit === "speed" ? knotsFromMps(value) : value;
}

/** Converts an entered number back into the stored one. */
function toStored(unit: string, value: number): number {
  return unit === "speed" ? mpsFromKnots(value) : value;
}

/** Rounds for display without hiding meaningful precision. */
function tidy(value: number): string {
  return Number.isInteger(value) ? String(value) : value.toFixed(2);
}

/**
 * Property editor for the selected object.
 *
 * Built entirely from what the backend reports, which is built from the schema.
 * Adding a property to a tool makes it appear here with no change to this file
 * (`CLAUDE.md`, "adding a property"). The only thing hard-coded is how each
 * *kind* of value is edited.
 */
export default function Inspector({
  project,
  selection,
  step,
  picking,
  onPick,
  onChanged,
}: {
  project: ProjectSummary;
  selection: number[];
  step: number;
  picking: PositionPick | null;
  onPick: (pick: PositionPick | null) => void;
  onChanged: (project: ProjectSummary) => void;
}) {
  // Editing shows one object's values. A multi-selection is transformed on the
  // map rather than edited field by field, and showing one member's numbers as
  // though they applied to all of them would be a lie.
  const object = selection.length === 1 ? (selection[0] ?? null) : null;
  const [properties, setProperties] = useState<PropertyView[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (object === null) {
      setProperties(null);
      return;
    }
    api
      .objectProperties(object, step)
      .then(setProperties)
      .catch((err: unknown) => setError(String(err)));
  }, [object, project.revision, step]);

  if (object === null) {
    return (
      <div className="panel-empty muted">
        {selection.length > 1
          ? `${selection.length} objects selected. Drag the handles to transform them, or select one to edit its properties.`
          : "Select an object to edit its properties."}
      </div>
    );
  }
  if (!properties) {
    return <div className="panel-empty muted">{error ?? "Loading…"}</div>;
  }

  const write = (property: string, value: PropertyValue) => {
    setError(null);
    api
      .setObjectProperty(object, property, value)
      .then(onChanged)
      .catch((err: unknown) => setError(String(err)));
  };

  return (
    <div className="inspector">
      <header>
        <h2>Properties</h2>
      </header>

      <div className="properties">
        {properties.map((property) => {
          const suffix = unitLabel(property.unit);
          return (
            <label
              key={property.id}
              // Two coordinates and a picker do not fit beside a label in this
              // panel's width, so a position takes the next line for itself.
              className={
                property.value.kind === "position" ? "property stacked" : "property"
              }
            >
              <span className="property-label">
                {property.label}
                {property.animated && (
                  <span className="keyed" title="Has keyframes">
                    ◆
                  </span>
                )}
              </span>

              {property.value.kind === "number" && (
                <span className="property-editor">
                  <input
                    type="number"
                    step="any"
                    min={
                      property.min !== null
                        ? toDisplay(property.unit, property.min)
                        : undefined
                    }
                    max={
                      property.max !== null
                        ? toDisplay(property.unit, property.max)
                        : undefined
                    }
                    defaultValue={tidy(toDisplay(property.unit, property.value.value))}
                    key={`${property.id}-${project.revision}`}
                    onBlur={(e) =>
                      write(property.id, {
                        kind: "number",
                        value: toStored(property.unit, Number(e.target.value) || 0),
                      })
                    }
                  />
                  {suffix && <span className="suffix muted">{suffix}</span>}
                </span>
              )}

              {property.value.kind === "angle" && (
                <span className="property-editor">
                  <input
                    type="number"
                    step="any"
                    key={`${property.id}-${project.revision}`}
                    defaultValue={tidy(
                      toShownAngle(
                        property.unit,
                        project.direction_convention,
                        property.value.degrees,
                      ),
                    )}
                    onBlur={(e) =>
                      write(property.id, {
                        kind: "angle",
                        // The conversion is its own inverse, so one function
                        // serves both ways.
                        degrees: toShownAngle(
                          property.unit,
                          project.direction_convention,
                          Number(e.target.value) || 0,
                        ),
                      })
                    }
                  />
                  <span className="suffix muted">
                    °{property.unit === "direction" && ` (${project.direction_convention})`}
                  </span>
                </span>
              )}

              {property.value.kind === "bool" && (
                <span className="property-editor">
                  <input
                    type="checkbox"
                    checked={property.value.value}
                    onChange={(e) =>
                      write(property.id, { kind: "bool", value: e.target.checked })
                    }
                  />
                </span>
              )}

              {property.value.kind === "choice" && (
                <span className="property-editor">
                  <select
                    value={property.value.index}
                    onChange={(e) =>
                      write(property.id, {
                        kind: "choice",
                        index: Number(e.target.value),
                      })
                    }
                  >
                    {property.variants.map((variant, index) => (
                      <option key={variant} value={index}>
                        {variant.replace(/_/g, " ")}
                      </option>
                    ))}
                  </select>
                </span>
              )}

              {property.value.kind === "position" && (
                <span className="property-editor position">
                  <input
                    type="number"
                    step="any"
                    key={`${property.id}-lon-${project.revision}`}
                    defaultValue={property.value.lon.toFixed(3)}
                    title="Longitude"
                    onBlur={(e) => {
                      if (property.value.kind !== "position") return;
                      write(property.id, {
                        kind: "position",
                        lon: Number(e.target.value) || 0,
                        lat: property.value.lat,
                      });
                    }}
                  />
                  <input
                    type="number"
                    step="any"
                    key={`${property.id}-lat-${project.revision}`}
                    defaultValue={property.value.lat.toFixed(3)}
                    title="Latitude"
                    onBlur={(e) => {
                      if (property.value.kind !== "position") return;
                      write(property.id, {
                        kind: "position",
                        lon: property.value.lon,
                        lat: Number(e.target.value) || 0,
                      });
                    }}
                  />
                  {/*
                    Coordinates are exact but no use for a point you are looking
                    at, which is how an aim point, an anchor or a clone source
                    is actually chosen. Every position property gets this, from
                    the value's kind alone — nothing here knows what a target is.
                  */}
                  <button
                    className={picking?.property === property.id ? "active" : ""}
                    title="Click the map to place this point"
                    onClick={(e) => {
                      // The row is a `<label>`, and a click inside one is
                      // forwarded to its first input: without this the button
                      // also drops the caret into the longitude box.
                      e.preventDefault();
                      if (property.value.kind !== "position") return;
                      onPick(
                        picking?.property === property.id
                          ? null
                          : {
                              object,
                              property: property.id,
                              label: property.label,
                              lon: property.value.lon,
                              lat: property.value.lat,
                            },
                      );
                    }}
                  >
                    {picking?.property === property.id ? "Click the map…" : "Pick"}
                  </button>
                </span>
              )}
            </label>
          );
        })}
      </div>

      {error !== null && <p className="error">{error}</p>}
    </div>
  );
}

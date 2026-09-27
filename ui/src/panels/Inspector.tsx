import ToolSelect from "../ToolSelect";
import { useEffect, useState } from "react";

import CentredSlider, { readoutFor } from "../CentredSlider";
import NumberField from "../NumberField";

import { reportError } from "../hint";
import { api } from "../ipc";
import type { LayerNode } from "../generated/LayerNode";
import type { ProjectSummary } from "../generated/ProjectSummary";
import type { PropertyValue } from "../generated/PropertyValue";
import type { PropertyView } from "../generated/PropertyView";
import { KIND_LABELS, kindOf } from "../kind";
import { formatUtcHour } from "./historyRange";
import { useUnits } from "../settings/units";
import type { PositionPick } from "../picking";
import { toShownAngle } from "./inspectorAngle";
import { msg, useT } from "../i18n";
import { variantLabel } from "../map/ToolOptions";

/**
 * The archives' names as the backend writes them (`ve_zarr::Archive::label`),
 * marked so the catalogues hold them; shown through `t` like any other label.
 */
export const ARCHIVE_NAMES = [msg("ERA5 10 m wind"), msg("GlobCurrent surface current")] as const;

/**
 * What a layer is, for the panel to say when no object is selected (M55).
 *
 * Facts and not controls. The things a layer's row already edits — its name,
 * its eye, its lock, its speed filter, its opacity — stay in the layer panel,
 * where they sit beside the layer they belong to; putting a second copy here
 * would be two places to change one thing. What this adds is what nothing
 * showed: where an imported field came from, how much of the timeline it
 * covers, and, for a history layer, which archive and which hours — the
 * provenance those layers have carried since M38 with nowhere to display it.
 */
function LayerFacts({ layer }: { layer: LayerNode | null }) {
  const t = useT();
  if (layer === null) {
    return (
      <div className="panel-empty muted">
        {t("Select an object to edit its properties, or a layer to see what it holds.")}
      </div>
    );
  }
  const rows: [string, string][] = [];
  const kindOfLayer =
    layer.source === "painted"
      ? t("Painted")
      : layer.source === "image"
        ? t("Image")
        : layer.grib?.history
          ? t("History")
          : t("Imported field");
  rows.push([t("Layer"), layer.name]);
  rows.push([t("Holds"), kindOfLayer]);
  if (layer.source !== "image") rows.push([t("Field"), t(KIND_LABELS[kindOf(layer.parameter)])]);
  if (!layer.visible) rows.push([t("Shown"), t("hidden")]);
  if (layer.locked) rows.push([t("Locked"), t("yes")]);

  const grib = layer.grib;
  if (grib) {
    if (grib.history) {
      rows.push([t("Archive"), t(grib.history.label)]);
      rows.push([
        t("Hours"),
        t("{start} to {end} UTC", {
          start: formatUtcHour(grib.history.start_unix_s).replace("T", " "),
          end: formatUtcHour(grib.history.end_unix_s).replace("T", " "),
        }),
      ]);
    }
    rows.push([layer.source === "zarr" && !grib.history ? t("Directory") : t("File"), grib.path]);
    rows.push([
      t("Frames"),
      grib.loaded
        ? t("{count} over {hours} h", { count: grib.frame_count, hours: grib.span_hours })
        : t("not read — the file is missing or unreadable"),
    ]);
    const covered = grib.covered_steps.filter(Boolean).length;
    rows.push([
      t("Steps covered"),
      t("{covered} of {total}", { covered, total: grib.covered_steps.length }),
    ]);
    if (grib.speed_min_mps !== null || grib.speed_max_mps !== null) {
      rows.push([t("Speed filter"), t("on — see the layer panel")]);
    }
  }

  const image = layer.image;
  if (image) {
    rows.push([t("File"), image.path]);
    rows.push([
      t("Size"),
      image.loaded
        ? `${image.width} × ${image.height} px`
        : t("not read — missing or unreadable"),
    ]);
    rows.push([t("Opacity"), `${Math.round(image.opacity * 100)}%`]);
  }

  if (layer.source === "painted") {
    rows.push([t("Objects"), String(layer.objects.length)]);
  }

  return (
    <div className="layer-facts" data-feature="inspector:layer-facts">
      {rows.map(([label, value]) => (
        <div className="layer-fact" key={label}>
          <span className="muted">{label}</span>
          <span className="layer-fact-value" title={value}>
            {value}
          </span>
        </div>
      ))}
    </div>
  );
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
  activeLayer,
  autoKey,
  picking,
  onPick,
  onChanged,
}: {
  project: ProjectSummary;
  selection: number[];
  step: number;
  /** The layer the panel describes when no object is selected (M55). */
  activeLayer: number | null;
  /** Whether an edit keys the current step rather than the base (spec.md 9.3). */
  autoKey: boolean;
  picking: PositionPick | null;
  onPick: (pick: PositionPick | null) => void;
  onChanged: (project: ProjectSummary) => void;
}) {
  const t = useT();
  const { toDisplay, toStored, suffix: unitLabel } = useUnits();
  // Editing shows one object's values. A multi-selection is transformed on the
  // map rather than edited field by field, and showing one member's numbers as
  // though they applied to all of them would be a lie.
  const object = selection.length === 1 ? (selection[0] ?? null) : null;
  const [loaded, setLoaded] = useState<{ object: number; step: number; properties: PropertyView[] } | null>(null);
  const properties = loaded?.object === object && loaded.step === step ? loaded.properties : null;
  // The active layer, for the panel to describe when no object is selected.
  // Read here rather than passed down: the tree is the one place that knows
  // what a layer holds, and the panel already refetches on every revision.
  const [layer, setLayer] = useState<LayerNode | null>(null);
  useEffect(() => {
    if (object !== null || activeLayer === null) {
      setLayer(null);
      return;
    }
    let live = true;
    api
      .documentTree(step)
      .then((tree) => {
        if (live) setLayer(tree.layers.find((node) => node.id === activeLayer) ?? null);
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, [object, activeLayer, project.revision, step]);
  // Errors go to the status bar's hint area (M25), not a line of their own.
  const setError = reportError;

  useEffect(() => {
    if (object === null) {
      setLoaded(null);
      return;
    }
    let live = true;
    api
      .objectProperties(object, step)
      .then((properties) => { if (live) setLoaded({ object, step, properties }); })
      .catch((err: unknown) => {
        if (!live) return;
        setLoaded(null);
        // Undo/deletion can win the race with this read. The layer tree
        // retires that selection; it is not a failed user edit.
        if (typeof err === "object" && err !== null && "kind" in err && err.kind === "missing-object") return;
        setError(String(err));
      });
    return () => { live = false; };
  }, [object, project.revision, step]);

  if (object === null) {
    // More than one selected is a transform, not an edit: showing one
    // member's numbers as though they applied to all of them would be a lie.
    if (selection.length > 1) {
      return (
        <div className="panel-empty muted">
          {t(
            "{count} objects selected. Drag the handles to transform them, or select one to edit its properties.",
            { count: selection.length },
          )}
        </div>
      );
    }
    // With nothing selected the panel describes the layer instead (M55).
    // Empty, it was the one panel that never had anything to say — while the
    // facts about a layer that are not editable anywhere, a history layer's
    // archive and hours above all, had nowhere to be seen at all.
    return <LayerFacts layer={layer} />;
  }
  if (!properties) {
    return <div className="panel-empty muted">{t("Loading…")}</div>;
  }

  const write = (property: string, value: PropertyValue) => {
    setError(null);
    api
      // Keys this step when the property is animated or auto-key is on, and
      // edits the base otherwise (spec.md 9.3).
      .setObjectProperty(object, property, value, step, autoKey)
      .then(onChanged)
      .catch((err: unknown) => setError(String(err)));
  };

  /** Keys the property at this step, or removes the key that is there. */
  const toggleKey = (property: string, keyed: boolean) => {
    setError(null);
    (keyed ? api.removeKeyframe(object, property, step) : api.setKeyframe(object, property, step))
      .then(onChanged)
      .catch((err: unknown) => setError(String(err)));
  };

  return (
    <div className="inspector" data-feature="inspector:properties">
      <div className="properties">
        {properties.map((property) => {
          const suffix = unitLabel(property.unit);
          // The schema's words are English from the backend; they are shown,
          // and read back by the slider's readout, in the interface language.
          const slider = property.slider && {
            ...property.slider,
            low_label: t(property.slider.low_label),
            high_label: t(property.slider.high_label),
          };
          return (
            <label
              key={property.id}
              // Each property is findable by its identifier: `Enabled` is the one
              // labelled Visible (the Help search registers the ones people ask for).
              data-feature={`inspector:property-${property.id.toLowerCase()}`}
              // Two coordinates and a picker do not fit beside a label in this
              // panel's width, so a position takes the next line for itself.
              className={
                property.value.kind === "position" || property.value.kind === "offset" || property.slider ? "property stacked" : "property"
              }
            >
              <span className="property-label">
                {t(property.label)}
                {/* The diamond: filled when this step is keyed, hollow when the
                    value is interpolated, dim otherwise. Clicking keys or
                    unkeys this step (spec.md 9.3).

                    Absent where the property cannot be keyed at all (M60): a
                    mode that decides which other properties the object has is
                    edited like anything else and animated by nothing. A
                    diamond that refuses is worse than no diamond. */}
                {property.keyable && (
                  <button
                    type="button"
                    className={`key-here${property.keyed_here ? " on" : property.interpolated_here ? " between" : ""}`}
                    data-feature="inspector:keyframe"
                    title={
                      property.keyed_here
                        ? t("Keyed at this step — click to remove the key")
                        : property.interpolated_here
                          ? t("Interpolated at this step — click to key it here")
                          : t("Click to key this property at this step")
                    }
                    onClick={(event) => {
                      event.preventDefault();
                      toggleKey(property.id, property.keyed_here);
                    }}
                  >
                    ◆
                  </button>
                )}
              </span>

              {property.value.kind === "number" && slider && (
                <span className="property-editor slider">
                  <CentredSlider
                    value={property.value.value}
                    min={property.min ?? -100}
                    max={property.max ?? 100}
                    lowLabel={slider.low_label}
                    highLabel={slider.high_label}
                    reversed={slider.reversed}
                    format={readoutFor(slider, property.unit)}
                    // On release, not on every tick: an inspector write is a
                    // document edit and an undo entry (spec.md 8.4).
                    onCommit={(next) => write(property.id, { kind: "number", value: next })}
                  />
                </span>
              )}

              {property.value.kind === "number" && !property.slider && (
                <span className="property-editor">
                  <NumberField
                    step="any"
                    min={property.min !== null ? toDisplay(property.unit, property.min) : null}
                    max={property.max !== null ? toDisplay(property.unit, property.max) : null}
                    value={toDisplay(property.unit, property.value.value)}
                    format={tidy}
                    // On blur, not on every keystroke: an inspector write is a
                    // document edit, an undo entry and a re-render of the field
                    // (spec.md 8.4).
                    commitWhileTyping={false}
                    onCommit={(shown) =>
                      write(property.id, {
                        kind: "number",
                        value: toStored(property.unit, shown),
                      })
                    }
                  />
                  {suffix && <span className="suffix muted">{suffix}</span>}
                </span>
              )}

              {property.value.kind === "angle" && (
                <span className="property-editor">
                  <NumberField
                    step="any"
                    value={toShownAngle(
                      property.unit,
                      project.direction_convention,
                      property.value.degrees,
                    )}
                    format={tidy}
                    commitWhileTyping={false}
                    onCommit={(shown) =>
                      write(property.id, {
                        kind: "angle",
                        // The conversion is its own inverse, so one function
                        // serves both ways.
                        degrees: toShownAngle(
                          property.unit,
                          project.direction_convention,
                          shown,
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
                  <ToolSelect
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
                        {variantLabel(variant)}
                      </option>
                    ))}
                  </ToolSelect>
                </span>
              )}

              {property.value.kind === "offset" && (
                <span className="property-editor position">
                  {(["x", "y"] as const).map((axis) => <NumberField key={axis}
                    step="any"
                    value={toDisplay("kilometres", property.value.kind === "offset" ? property.value[axis] : 0)}
                    format={(v) => v.toFixed(3)} title={t("{axis} displacement ({unit})", { axis: axis.toUpperCase(), unit: unitLabel("kilometres") })}
                    commitWhileTyping={false}
                    onCommit={(v) => {
                      if (property.value.kind === "offset") write(property.id, {...property.value, [axis]: toStored("kilometres", v)});
                    }} />)}
                  <span>{unitLabel("kilometres")}</span>
                </span>
              )}
              {property.value.kind === "position" && (
                <span className="property-editor position">
                  <NumberField
                    step="any"
                    value={property.value.lon}
                    format={(v) => v.toFixed(3)}
                    title={t("Longitude")}
                    commitWhileTyping={false}
                    onCommit={(lon) => {
                      if (property.value.kind !== "position") return;
                      write(property.id, { kind: "position", lon, lat: property.value.lat });
                    }}
                  />
                  <NumberField
                    step="any"
                    min={-90}
                    max={90}
                    value={property.value.lat}
                    format={(v) => v.toFixed(3)}
                    title={t("Latitude")}
                    commitWhileTyping={false}
                    onCommit={(lat) => {
                      if (property.value.kind !== "position") return;
                      write(property.id, { kind: "position", lon: property.value.lon, lat });
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
                    data-feature="inspector:pick-position"
                    title={t("Click the map to place this point")}
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
                              label: t(property.label),
                              lon: property.value.lon,
                              lat: property.value.lat,
                            },
                      );
                    }}
                  >
                    {picking?.property === property.id ? t("Click the map…") : t("Pick")}
                  </button>
                </span>
              )}
            </label>
          );
        })}
      </div>


    </div>
  );
}

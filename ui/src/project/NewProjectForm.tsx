import { useEffect, useState } from "react";

import NumberField from "../NumberField";
import { onReveal } from "../help/highlight";
import { useT } from "../i18n";
import type { NewProjectRequest } from "../generated/NewProjectRequest";
import type { RegionRequest } from "../generated/RegionRequest";
import RegionPicker from "./RegionPicker";
import { DEFAULT_REGION, regionNodes, regionProblem, snapEdges } from "./regionPick";
import {
  MAX_STEPS,
  RESOLUTIONS,
  STEP_HOURS,
  estimatedGribBytes,
  formatBytes,
  gridSize,
} from "./format";

/**
 * The settings a new project is created with.
 *
 * Shared by the start screen and the New Project dialog rather than written
 * twice: field kind, grid resolution and time step are immutable once a project
 * exists (spec.md 4.1), so the consequences of each choice — grid dimensions
 * and the export size they imply — have to be shown wherever the choice is
 * made, and two copies of that would drift.
 */
export default function NewProjectForm({
  disabled = false,
  submitLabel,
  onSubmit,
  viewBounds,
}: {
  disabled?: boolean;
  submitLabel: string;
  onSubmit: (request: NewProjectRequest) => void;
  /** The open map's view, for *Use current view*: only with a project open. */
  viewBounds?: (() => [number, number, number, number] | null) | undefined;
}) {
  const t = useT();
  const [name, setName] = useState(() => t("Untitled"));
  const [resolution, setResolution] = useState<string>("0.25");
  const [stepHours, setStepHours] = useState(3);
  const [stepCount, setStepCount] = useState(24);
  // Global or a region (spec.md 4.2, R9). The region is kept while Global is
  // chosen, so switching back does not lose a box drawn a moment ago.
  const [regional, setRegional] = useState(false);
  const [region, setRegion] = useState<RegionRequest>(DEFAULT_REGION);

  // The Help search's way to the region's controls.
  useEffect(() => onReveal("new:regional", () => setRegional(true)), []);

  const resolutionDeg = Number(resolution);
  // The edges as they will be on this resolution's lattice.
  const shown = snapEdges(region, resolutionDeg);
  const chosen = regional ? shown : null;
  const problem = chosen === null ? null : regionProblem(chosen, resolutionDeg);
  const { ni, nj } = chosen === null ? gridSize(resolutionDeg) : regionNodes(chosen, resolutionDeg);
  const exportBytes = estimatedGribBytes(resolutionDeg, stepCount, chosen);
  const durationHours = stepHours * (stepCount - 1);

  const submit = () =>
    onSubmit({
      name,
      // A layer says which field it is part of (M29); the project no
      // longer does. Wind is the default a new layer takes.
      field_kind: "wind",
      resolution,
      step_hours: stepHours,
      step_count: stepCount,
      // As drawn: Rust snaps it again and is the authority on the lattice.
      region: regional ? region : null,
    });

  return (
    <>
      <label data-feature="new:name">
        {t("Name")}
        <input value={name} onChange={(e) => setName(e.target.value)} spellCheck={false} />
      </label>

      <label data-feature="new:resolution">
        {t("Resolution")}
        <select value={resolution} onChange={(e) => setResolution(e.target.value)}>
          {RESOLUTIONS.map((r) => (
            <option key={r.value} value={r.value}>
              {r.label}
            </option>
          ))}
        </select>
      </label>

      <label data-feature="new:extent">
        {t("Extent")}
        <select value={regional ? "regional" : "global"} onChange={(e) => setRegional(e.target.value === "regional")}>
          <option value="global">{t("Global")}</option>
          <option value="regional">{t("Regional")}</option>
        </select>
      </label>

      {regional && (
        <RegionPicker value={shown} resolution={resolution} onChange={setRegion} viewBounds={viewBounds} />
      )}

      <label data-feature="new:time-step">
        {t("Time step")}
        <select value={stepHours} onChange={(e) => setStepHours(Number(e.target.value))}>
          {STEP_HOURS.map((h) => (
            <option key={h} value={h}>
              {h === 1 ? t("1 hour") : t("{count} hours", { count: h })}
            </option>
          ))}
        </select>
      </label>

      <label data-feature="new:steps">
        {t("Steps")}
        <NumberField min={1} max={MAX_STEPS} value={stepCount} onCommit={setStepCount} />
      </label>

      <p className="start-implications muted">
        {/* A box that is no region has no grid to describe. */}
        {problem !== "no-height" && problem !== "no-width" && (
          <>
            {t("{ni} × {nj} grid · covers {hours} h · export about {size}", {
              ni, nj, hours: durationHours, size: formatBytes(exportBytes),
            })}
            <br />
          </>
        )}
        <span className="warn-note">
          {regional
            ? t("Resolution, time step and region cannot be changed later.")
            : t("Resolution and time step cannot be changed later.")}
        </span>
      </p>

      {problem === "whole-earth" && <p className="error">{t("Choose Global for the whole earth")}</p>}
      {problem === "no-height" && <p className="error">{t("North must be above South")}</p>}
      {problem === "no-width" && <p className="error">{t("West and East must differ")}</p>}

      <button className="primary" onClick={submit} disabled={disabled || problem !== null}>
        {submitLabel}
      </button>
    </>
  );
}

import { useState } from "react";

import NumberField from "../NumberField";
import type { NewProjectRequest } from "../generated/NewProjectRequest";
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
}: {
  disabled?: boolean;
  submitLabel: string;
  onSubmit: (request: NewProjectRequest) => void;
}) {
  const [name, setName] = useState("Untitled");
  const [fieldKind, setFieldKind] = useState<"wind" | "current">("wind");
  const [resolution, setResolution] = useState<string>("0.25");
  const [stepHours, setStepHours] = useState(3);
  const [stepCount, setStepCount] = useState(24);

  const resolutionDeg = Number(resolution);
  const { ni, nj } = gridSize(resolutionDeg);
  const exportBytes = estimatedGribBytes(resolutionDeg, stepCount);
  const durationHours = stepHours * (stepCount - 1);

  const submit = () =>
    onSubmit({
      name,
      field_kind: fieldKind,
      resolution,
      step_hours: stepHours,
      step_count: stepCount,
    });

  return (
    <>
      <label>
        Name
        <input value={name} onChange={(e) => setName(e.target.value)} spellCheck={false} />
      </label>

      <label>
        Field
        <select
          value={fieldKind}
          onChange={(e) => setFieldKind(e.target.value === "current" ? "current" : "wind")}
        >
          <option value="wind">Wind (10 m)</option>
          <option value="current">Ocean current (surface)</option>
        </select>
      </label>

      <label>
        Resolution
        <select value={resolution} onChange={(e) => setResolution(e.target.value)}>
          {RESOLUTIONS.map((r) => (
            <option key={r.value} value={r.value}>
              {r.label}
            </option>
          ))}
        </select>
      </label>

      <label>
        Time step
        <select value={stepHours} onChange={(e) => setStepHours(Number(e.target.value))}>
          {STEP_HOURS.map((h) => (
            <option key={h} value={h}>
              {h} hour{h === 1 ? "" : "s"}
            </option>
          ))}
        </select>
      </label>

      <label>
        Steps
        <NumberField min={1} max={MAX_STEPS} value={stepCount} onCommit={setStepCount} />
      </label>

      <p className="start-implications muted">
        {ni} × {nj} grid · covers {durationHours} h · export about {formatBytes(exportBytes)}
        <br />
        <span className="warn-note">
          Field, resolution and time step cannot be changed later.
        </span>
      </p>

      <button className="primary" onClick={submit} disabled={disabled}>
        {submitLabel}
      </button>
    </>
  );
}

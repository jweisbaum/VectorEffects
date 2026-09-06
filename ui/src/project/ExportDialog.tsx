import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import { KIND_LABELS, kindOf } from "../kind";
import NumberField from "../NumberField";
import { startDraftFrom } from "../timeline/Timeline";
import { api, IpcError } from "../ipc";
import type { ExportEstimate } from "../generated/ExportEstimate";
import type { ExportProgress } from "../generated/ExportProgress";
import type { ProjectSummary } from "../generated/ProjectSummary";
import { formatBytes } from "./format";
import { pickGribDestination } from "./dialogs";

/** Today's date, as the form's default reference time. */
/**
 * The export dialog.
 *
 * Shows the size before the user commits: a 0.1° project over ten days is
 * around 12 GB, which is far better learned here than after a long wait
 * (spec.md 12.4).
 */
export default function ExportDialog({
  project,
  onClose,
}: {
  project: ProjectSummary;
  onClose: () => void;
}) {
  // The project's start time when it has one, else now rounded to the
  // nearest hour, UTC (M29): the dialog still asks, it just starts right.
  const start = startDraftFrom(project.start_unix_s);
  const [year, setYear] = useState(start.year);
  const [month, setMonth] = useState(start.month);
  const [day, setDay] = useState(start.day);
  const [hour, setHour] = useState(start.hour);
  const [centre, setCentre] = useState(255);
  /**
   * Bits per packed value (spec.md 12.3, M19). Sixteen is what every export
   * wrote before this was a choice; 8 halves the file at steps of about half a
   * knot, which the estimate says beside it.
   */
  const [bits, setBits] = useState(16);

  const [estimate, setEstimate] = useState<ExportEstimate | null>(null);
  const [progress, setProgress] = useState<ExportProgress | null>(null);
  const [running, setRunning] = useState(false);
  const [done, setDone] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.exportEstimate(bits).then(setEstimate).catch(() => setEstimate(null));
  }, [bits]);

  useEffect(() => {
    const pending = listen<ExportProgress>("export://progress", (event) =>
      setProgress(event.payload),
    );
    return () => {
      void pending.then((unlisten) => unlisten());
    };
  }, []);

  const run = async () => {
    setError(null);
    setDone(null);
    const path = await pickGribDestination(project.name);
    if (path === null) return;

    setRunning(true);
    setProgress(null);
    try {
      const result = await api.exportGrib({ path, year, month, day, hour, centre, bits });
      setDone(
        `Wrote ${result.messages} messages, ${formatBytes(result.bytes)}, ` +
          `in ${(result.elapsed_ms / 1000).toFixed(1)} s`,
      );
    } catch (err) {
      // Cancelling is a choice, not a failure.
      const message = err instanceof IpcError ? err.message : String(err);
      setError(err instanceof IpcError && err.kind === "cancelled" ? null : message);
      if (err instanceof IpcError && err.kind === "cancelled") setDone("Export cancelled");
    } finally {
      setRunning(false);
      setProgress(null);
    }
  };

  const percent =
    progress && progress.total > 0 ? Math.round((progress.step / progress.total) * 100) : 0;

  return (
    <div className="modal-backdrop" onClick={running ? undefined : onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>Export GRIB2</h2>

        <p className="muted modal-summary">
          {project.kinds_present.map((kind) => KIND_LABELS[kindOf(kind)]).join(" + ") ||
            "no field"}{" "}
          · {project.grid_ni} × {project.grid_nj} ·{" "}
          {project.step_count} steps every {project.step_hours} h
          {estimate && <> · about {formatBytes(estimate.bytes)}</>}
        </p>

        <fieldset disabled={running}>
          <legend>Forecast start (UTC)</legend>
          <div className="modal-row">
            <label>
              Year
              <NumberField min={1900} max={2999} value={year} onCommit={setYear} />
            </label>
            <label>
              Month
              <NumberField min={1} max={12} value={month} onCommit={setMonth} />
            </label>
            <label>
              Day
              <NumberField min={1} max={31} value={day} onCommit={setDay} />
            </label>
            <label>
              Hour
              <NumberField min={0} max={23} value={hour} onCommit={setHour} />
            </label>
          </div>

          <label className="modal-centre">
            Precision
            <select value={bits} onChange={(event) => setBits(Number(event.target.value))}>
              {[8, 12, 16, 24].map((width) => (
                <option key={width} value={width}>
                  {width} bits per value
                </option>
              ))}
            </select>
            <span className="muted">
              {estimate
                ? `steps of about ${estimate.step_knots >= 0.1 ? estimate.step_knots.toFixed(2) : estimate.step_knots.toFixed(4)} kt over ±60 m/s · ${formatBytes(estimate.bytes)}`
                : ""}
              {bits === 16 ? " · the default, and what earlier exports used" : ""}
            </span>
          </label>

          <label className="modal-centre">
            Originating centre
            <NumberField min={0} max={65535} value={centre} onCommit={setCentre} />
            <span className="muted">
              {centre === 255
                ? "255 = missing, the honest default"
                : centre === 7
                  ? "7 = NCEP; a fiction some readers prefer"
                  : ""}
            </span>
          </label>
        </fieldset>

        {running && (
          <div className="progress">
            <div className="progress-bar">
              <div className="progress-fill" style={{ width: `${percent}%` }} />
            </div>
            <span className="muted">
              {progress ? `step ${progress.step} of ${progress.total}` : "starting…"}
            </span>
          </div>
        )}

        {done !== null && <p className="accent">{done}</p>}
        {error !== null && <p className="error">{error}</p>}

        <div className="modal-actions">
          {running ? (
            <button onClick={() => void api.cancelExport()}>Cancel export</button>
          ) : (
            <>
              <button onClick={onClose}>Close</button>
              <button className="primary" onClick={() => void run()}>
                Choose location and export…
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

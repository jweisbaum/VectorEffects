import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import { api, IpcError } from "../ipc";
import type { ExportEstimate } from "../generated/ExportEstimate";
import type { ExportProgress } from "../generated/ExportProgress";
import type { ProjectSummary } from "../generated/ProjectSummary";
import { formatBytes } from "./format";
import { pickGribDestination } from "./dialogs";

/** Today's date, as the form's default reference time. */
function today(): { year: number; month: number; day: number } {
  const now = new Date();
  return { year: now.getUTCFullYear(), month: now.getUTCMonth() + 1, day: now.getUTCDate() };
}

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
  const start = today();
  const [year, setYear] = useState(start.year);
  const [month, setMonth] = useState(start.month);
  const [day, setDay] = useState(start.day);
  const [hour, setHour] = useState(0);
  const [centre, setCentre] = useState(255);

  const [estimate, setEstimate] = useState<ExportEstimate | null>(null);
  const [progress, setProgress] = useState<ExportProgress | null>(null);
  const [running, setRunning] = useState(false);
  const [done, setDone] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.exportEstimate().then(setEstimate).catch(() => setEstimate(null));
  }, []);

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
      const result = await api.exportGrib({ path, year, month, day, hour, centre });
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
          {project.field_kind} · {project.grid_ni} × {project.grid_nj} ·{" "}
          {project.step_count} steps every {project.step_hours} h
          {estimate && <> · about {formatBytes(estimate.bytes)}</>}
        </p>

        <fieldset disabled={running}>
          <legend>Forecast start (UTC)</legend>
          <div className="modal-row">
            <label>
              Year
              <input type="number" min={1900} max={2999} value={year}
                onChange={(e) => setYear(Number(e.target.value) || start.year)} />
            </label>
            <label>
              Month
              <input type="number" min={1} max={12} value={month}
                onChange={(e) => setMonth(Number(e.target.value) || 1)} />
            </label>
            <label>
              Day
              <input type="number" min={1} max={31} value={day}
                onChange={(e) => setDay(Number(e.target.value) || 1)} />
            </label>
            <label>
              Hour
              <input type="number" min={0} max={23} value={hour}
                onChange={(e) => setHour(Number(e.target.value) || 0)} />
            </label>
          </div>

          <label className="modal-centre">
            Originating centre
            <input type="number" min={0} max={65535} value={centre}
              onChange={(e) => setCentre(Number(e.target.value) || 0)} />
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

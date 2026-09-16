import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import NumberField from "../NumberField";
import { startDraftFrom } from "../timeline/Timeline";
import { setHint } from "../hint";
import { api, IpcError } from "../ipc";
import type { ExportProgress } from "../generated/ExportProgress";
import type { ProjectSummary } from "../generated/ProjectSummary";
import { formatBytes } from "./format";
import { pickZarrDestination } from "./dialogs";

/** The Zarr V3 export dialog. */
export default function ExportZarrDialog({
  project,
  onClose,
}: {
  project: ProjectSummary;
  onClose: () => void;
}) {
  const start = startDraftFrom(project.start_unix_s);
  const [year, setYear] = useState(start.year);
  const [month, setMonth] = useState(start.month);
  const [day, setDay] = useState(start.day);
  const [hour, setHour] = useState(start.hour);
  const [progress, setProgress] = useState<ExportProgress | null>(null);
  const [running, setRunning] = useState(false);
  const [done, setDone] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

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
    const path = await pickZarrDestination(project.name);
    if (path === null) return;
    setRunning(true);
    setProgress(null);
    try {
      const result = await api.exportZarr({ path, year, month, day, hour });
      setHint(
        `Exported Zarr V3 (${result.chunks} chunks, ${formatBytes(result.bytes)}) ` +
          `in ${(result.elapsed_ms / 1000).toFixed(1)} s`,
      );
      setRunning(false);
      setProgress(null);
      onClose();
    } catch (err) {
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
      <div className="modal" onClick={(event) => event.stopPropagation()}>
        <h2>Export Zarr V3</h2>
        <p className="muted modal-summary">
          u/v 10 m wind + u/v total surface current · Float16 · Zstd · land masked
          <br />
          {project.grid_ni} × {project.grid_nj} · {project.step_count} steps every {project.step_hours} h
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

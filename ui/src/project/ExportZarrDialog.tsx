import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import NumberField from "../NumberField";
import { startDraftFrom } from "../timeline/Timeline";
import { setHint } from "../hint";
import { useT } from "../i18n";
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
  const t = useT();
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
        t("Exported Zarr V3 ({chunks} chunks, {size}) in {seconds} s", {
          chunks: result.chunks,
          size: formatBytes(result.bytes),
          seconds: (result.elapsed_ms / 1000).toFixed(1),
        }),
      );
      setRunning(false);
      setProgress(null);
      onClose();
    } catch (err) {
      const message = err instanceof IpcError ? err.message : String(err);
      setError(err instanceof IpcError && err.kind === "cancelled" ? null : message);
      if (err instanceof IpcError && err.kind === "cancelled") setDone(t("Export cancelled"));
    } finally {
      setRunning(false);
      setProgress(null);
    }
  };

  // The store is always the global routing layout, whose rows stop one short
  // of the south pole; a regional project fills only its own chunks of it.
  const storeNi = Math.round(360 / project.resolution_deg);
  const storeNj = Math.round(180 / project.resolution_deg);

  const percent =
    progress && progress.total > 0 ? Math.round((progress.step / progress.total) * 100) : 0;

  return (
    <div className="modal-backdrop" onClick={running ? undefined : onClose}>
      <div className="modal" onClick={(event) => event.stopPropagation()}>
        <h2>{t("Export Zarr V3")}</h2>
        <p className="muted modal-summary">
          {t("u/v 10 m wind + u/v total surface current · Float16 · Zstd · land masked")}
          <br />
          {storeNi} × {storeNj} ·{" "}
          {t("{count} steps every {hours} h", { count: project.step_count, hours: project.step_hours })}
          {project.region !== null && (
            <>
              <br />
              {t("Only the region's chunks are written")}
            </>
          )}
        </p>

        <fieldset disabled={running}>
          <legend>{t("Forecast start (UTC)")}</legend>
          <div className="modal-row">
            <label>
              {t("Year")}
              <NumberField min={1900} max={2999} value={year} onCommit={setYear} />
            </label>
            <label>
              {t("Month")}
              <NumberField min={1} max={12} value={month} onCommit={setMonth} />
            </label>
            <label>
              {t("Day")}
              <NumberField min={1} max={31} value={day} onCommit={setDay} />
            </label>
            <label>
              {t("Hour")}
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
              {progress ? t("step {step} of {total}", { step: progress.step, total: progress.total }) : t("starting…")}
            </span>
          </div>
        )}
        {done !== null && <p className="accent">{done}</p>}
        {error !== null && <p className="error">{error}</p>}

        <div className="modal-actions">
          {running ? (
            <button onClick={() => void api.cancelExport()}>{t("Cancel export")}</button>
          ) : (
            <>
              <button onClick={onClose}>{t("Close")}</button>
              <button className="primary" onClick={() => void run()}>
                {t("Choose location and export…")}
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

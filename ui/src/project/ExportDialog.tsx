import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import { KIND_LABELS, kindOf } from "../kind";
import NumberField from "../NumberField";
import { startDraftFrom } from "../timeline/Timeline";
import { setHint } from "../hint";
import { useT } from "../i18n";
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
  const t = useT();
  const start = startDraftFrom(project.start_unix_s);
  const [year, setYear] = useState(start.year);
  const [month, setMonth] = useState(start.month);
  const [day, setDay] = useState(start.day);
  const [hour, setHour] = useState(start.hour);
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
      const result = await api.exportGrib({ path, year, month, day, hour });
      // The export is what the dialog was opened to do, so finishing it
      // closes the dialog (M48). What it wrote goes to the status bar, which
      // is where everything else the application has to say goes: leaving the
      // result behind a modal makes the user dismiss a box to get back to the
      // map they were already looking at.
      setHint(
        t("Exported {messages} messages, {size}, in {seconds} s", {
          messages: result.messages,
          size: formatBytes(result.bytes),
          seconds: (result.elapsed_ms / 1000).toFixed(1),
        }),
      );
      setRunning(false);
      setProgress(null);
      onClose();
      return;
    } catch (err) {
      // Cancelling is a choice, not a failure.
      const message = err instanceof IpcError ? err.message : String(err);
      setError(err instanceof IpcError && err.kind === "cancelled" ? null : message);
      if (err instanceof IpcError && err.kind === "cancelled") setDone(t("Export cancelled"));
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
        <h2>{t("Export GRIB2")}</h2>

        <p className="muted modal-summary">
          {project.kinds_present.map((kind) => t(KIND_LABELS[kindOf(kind)])).join(" + ") ||
            t("no field")}{" "}
          · {project.grid_ni} × {project.grid_nj} ·{" "}
          {t("{count} steps every {hours} h", { count: project.step_count, hours: project.step_hours })}
          {estimate && <> · {t("about {size}", { size: formatBytes(estimate.bytes) })}</>}
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

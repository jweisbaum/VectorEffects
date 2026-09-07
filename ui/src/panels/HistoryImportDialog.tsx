/**
 * Asking for a range of past hours to import (spec 4.10, M38).
 *
 * Two instants and a choice of archives. What it fetches is what the layers
 * then hold for good: the hours are written to a GRIB2 file each and read
 * back like any other import, so this dialog is the only moment the
 * application reaches the network (invariant 5).
 *
 * **It counts steps, not hours.** A step shows an imported message only where
 * the file has one for that step's forecast hour (spec 4.8), so the import
 * strides by the project's own step and the dialog says how many steps that
 * is. On a three-hourly project a day of range is nine downloads, not
 * twenty-five, and the wait is a third of what it was.
 *
 * **Rendered through a portal.** The layer panel sits in a stacking context
 * of its own, so a dialog rendered inside it is painted under the timeline
 * whatever its `z-index` says. A modal belongs to the window, not to the
 * panel whose button opened it.
 */
import { useState } from "react";
import { createPortal } from "react-dom";

import type { ProjectSummary } from "../generated/ProjectSummary";
import { ARCHIVES, MAX_FETCHED_STEPS, defaultRange, rangeState } from "./historyRange";

/** What the dialog hands back when the user commits. */
export interface HistoryChoice {
  /** Archive identifiers, as `ve_zarr::Archive::id` spells them. */
  archives: string[];
  /** First hour, in UTC seconds. */
  startUnixS: number;
  /** Last hour, in UTC seconds. */
  endUnixS: number;
}

export default function HistoryImportDialog({
  project,
  now,
  onImport,
  onClose,
}: {
  /** For the project's step, which decides what is worth fetching. */
  project: ProjectSummary;
  /** The current time in Unix seconds, so the default range is testable. */
  now: number;
  onImport: (choice: HistoryChoice) => void;
  onClose: () => void;
}) {
  const initial = defaultRange(now);
  const [start, setStart] = useState(initial.start);
  const [end, setEnd] = useState(initial.end);
  const [chosen, setChosen] = useState<string[]>(ARCHIVES.map((a) => a.id));

  const state = rangeState(start, end, chosen, project.step_hours, project.step_count);
  const ready = state.problem === null && state.start !== null && state.end !== null;
  const downloads = state.steps * chosen.length;

  const toggle = (id: string) =>
    setChosen((held) => (held.includes(id) ? held.filter((x) => x !== id) : [...held, id]));

  return createPortal(
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal modal-narrow history-import"
        role="dialog"
        aria-label="Import history"
        onClick={(event) => event.stopPropagation()}
      >
        <h2>Import history</h2>
        <p className="muted">
          Fetches past times from the public archives and adds one layer for each. Times are
          UTC and land on the hour.
        </p>

        <label className="field">
          <span>Start (UTC)</span>
          <input
            type="datetime-local"
            step={3600}
            value={start}
            onChange={(event) => setStart(event.target.value)}
          />
        </label>
        <label className="field">
          <span>End (UTC)</span>
          <input
            type="datetime-local"
            step={3600}
            value={end}
            onChange={(event) => setEnd(event.target.value)}
          />
        </label>

        <fieldset className="field">
          <legend>Archives</legend>
          {ARCHIVES.map((archive) => (
            <label key={archive.id} className="check">
              <input
                type="checkbox"
                checked={chosen.includes(archive.id)}
                onChange={() => toggle(archive.id)}
              />
              <span>
                {archive.label} <span className="muted">— {archive.detail}</span>
              </span>
            </label>
          ))}
        </fieldset>

        {/*
          The download count is what the wait is proportional to, so it is
          shown before the button rather than discovered by pressing it. The
          project's step is named because it is what makes the count smaller
          than the number of hours in the range.
        */}
        <p className={state.problem === null ? "muted" : "error"}>
          {state.problem ??
            `${state.steps} step${state.steps === 1 ? "" : "s"} at ${project.step_hours} h, ` +
              `${downloads} download${downloads === 1 ? "" : "s"} in all.`}
        </p>

        <div className="modal-actions">
          <button onClick={onClose}>Cancel</button>
          <button
            disabled={!ready}
            title={
              ready
                ? `Fetch ${state.steps} steps from each archive`
                : `Up to ${MAX_FETCHED_STEPS} steps at a time`
            }
            onClick={() => {
              if (state.start === null || state.end === null || state.problem !== null) return;
              onImport({
                archives: ARCHIVES.map((a) => a.id).filter((id) => chosen.includes(id)),
                startUnixS: state.start,
                endUnixS: state.end,
              });
            }}
          >
            Import
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}

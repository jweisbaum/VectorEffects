/**
 * Asking for a range of past hours to import (spec 4.10, M38).
 *
 * Two instants and a choice of archives. What it fetches is what the layers
 * then hold for good: the hours are written to a GRIB2 file each and read
 * back like any other import, so this dialog is the only moment the
 * application reaches the network (invariant 5).
 */
import { useState } from "react";

import {
  ARCHIVES,
  MAX_HOURS,
  defaultRange,
  rangeState,
} from "./historyRange";

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
  now,
  onImport,
  onClose,
}: {
  /** The current time in Unix seconds, so the default range is testable. */
  now: number;
  onImport: (choice: HistoryChoice) => void;
  onClose: () => void;
}) {
  const initial = defaultRange(now);
  const [start, setStart] = useState(initial.start);
  const [end, setEnd] = useState(initial.end);
  const [chosen, setChosen] = useState<string[]>(ARCHIVES.map((a) => a.id));

  const state = rangeState(start, end, chosen);
  const ready = state.problem === null && state.start !== null && state.end !== null;

  const toggle = (id: string) =>
    setChosen((held) => (held.includes(id) ? held.filter((x) => x !== id) : [...held, id]));

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal modal-narrow"
        role="dialog"
        aria-label="Import history"
        onClick={(event) => event.stopPropagation()}
      >
        <h2>Import history</h2>
        <p className="muted">
          Fetches past hours from the public archives and adds one layer for each. Times are
          UTC, and land on the hour.
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
          The hour count is what the wait is proportional to, so it is shown
          before the button rather than discovered by pressing it.
        */}
        <p className={state.problem === null ? "muted" : "error"}>
          {state.problem ??
            `${state.hours} hour${state.hours === 1 ? "" : "s"} from ` +
              `${chosen.length} archive${chosen.length === 1 ? "" : "s"}. ` +
              `This can take several minutes.`}
        </p>

        <div className="modal-actions">
          <button onClick={onClose}>Cancel</button>
          <button
            disabled={!ready}
            title={ready ? `Fetch ${state.hours} hours` : `Up to ${MAX_HOURS} hours at a time`}
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
    </div>
  );
}

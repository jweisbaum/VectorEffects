/**
 * Asking for the last few days of observed data (spec 4.10, M89).
 *
 * The sibling of `HistoryImportDialog`: that one names two instants in the
 * past, this one names a number of days and always ends *now*. What it
 * fetches is what the layers then hold for good — every time is written to
 * disk and read back like any other import — so pressing Import is the only
 * moment this reaches the network (invariant 5).
 *
 * **It asks for days, and the backend decides the hours.** The request
 * carries a count and no instant: the import reads its own clock when it
 * runs, so a dialog left open over an hour boundary still fetches up to the
 * present. The period shown here is `nrtRange.ts` doing the same arithmetic,
 * to say how far back the days reach and what they will cost.
 *
 * **It offers to lengthen the timeline, and only when that is needed.** A
 * period of days rarely fits a project made for one, and times past the last
 * step have nowhere to be shown — so the box is there exactly when the period
 * needs more steps than the project has, and the cost line follows it.
 *
 * **Rendered through a portal**, for the reason the history dialog is: the
 * layer panel is a stacking context of its own, and a modal belongs to the
 * window, not to the panel whose button opened it.
 */
import { useState } from "react";
import { createPortal } from "react-dom";

import NumberField from "../NumberField";
import type { NrtRequest } from "../generated/NrtRequest";
import type { ProjectSummary } from "../generated/ProjectSummary";
import { useT } from "../i18n";
import { formatUtcHour } from "./historyRange";
import {
  COPERNICUS_CREDIT,
  DEFAULT_DAYS,
  NRT_GROUPS,
  NRT_PRODUCTS,
  clampDays,
  costOf,
  maxDays,
  periodOf,
} from "./nrtRange";

export default function NrtImportDialog({
  project,
  now,
  onImport,
  onClose,
}: {
  /** For the project's step and length, which decide what is worth fetching. */
  project: ProjectSummary;
  /** The current time in Unix seconds, so the period shown is testable. */
  now: number;
  onImport: (request: NrtRequest) => void;
  onClose: () => void;
}) {
  const t = useT();
  const dated = project.start_unix_s !== null;
  const most = maxDays(now, project.step_hours);
  const [days, setDays] = useState(() => clampDays(DEFAULT_DAYS, most));
  const [chosen, setChosen] = useState<string[]>(NRT_PRODUCTS.map((p) => p.id));
  // Ticked whether or not the project has a date. The period ends now, so a
  // timeline that starts anywhere else shows these times on the wrong steps
  // or on none; keeping the old start is the choice that has to be made.
  const [setStartTime, setSetStartTime] = useState(true);
  const [extend, setExtend] = useState(true);

  // The hour may have turned since the dialog opened, and with it the most
  // days a timeline takes; what is asked for is held to what can be.
  const asked = clampDays(days, most);
  const period = periodOf(now, asked, project.step_hours);
  const startText = formatUtcHour(period.startUnixS).replace("T", " ");
  const canExtend = period.steps > project.step_count;
  const extending = canExtend && extend;
  const stepCount = extending ? Math.max(project.step_count, period.steps) : project.step_count;
  const cost = costOf(chosen, period, project.step_hours, stepCount);
  const ready = chosen.length > 0;

  const toggle = (id: string) =>
    setChosen((held) => (held.includes(id) ? held.filter((x) => x !== id) : [...held, id]));

  return createPortal(
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal modal-narrow history-import nrt-import"
        role="dialog"
        aria-label={t("Near-real-time data")}
        data-feature="nrt-import:dialog"
        onClick={(event) => event.stopPropagation()}
      >
        <h2>{t("Near-real-time data")}</h2>
        <p className="muted">
          {t("Fetches the last days of observed data, up to now, and adds one layer for each product. Times are UTC.")}
        </p>

        <label className="field" data-feature="nrt-import:days">
          <span>{t("Days")}</span>
          <NumberField
            value={asked}
            min={1}
            max={most}
            step={1}
            onCommit={(value) => setDays(clampDays(value, most))}
          />
          <span className="muted">{t("From {start} UTC to now.", { start: startText })}</span>
        </label>

        <label className="check" data-feature="nrt-import:set-start">
          <input
            type="checkbox"
            checked={setStartTime}
            onChange={(event) => setSetStartTime(event.target.checked)}
          />
          <span>
            {dated
              ? t("Move the timeline’s start to {time} UTC", { time: startText })
              : t("Set the timeline’s start to {time} UTC", { time: startText })}
            {dated && (
              <span className="muted">
                {" "}
                {t("— it is {time} now", {
                  time: formatUtcHour(project.start_unix_s ?? 0).replace("T", " "),
                })}
              </span>
            )}
          </span>
        </label>

        {canExtend && (
          <label className="check" data-feature="nrt-import:extend">
            <input
              type="checkbox"
              checked={extend}
              onChange={(event) => setExtend(event.target.checked)}
            />
            <span>
              {t("Lengthen the timeline to {steps} steps", { steps: period.steps })}
              <span className="muted">
                {" "}
                {t("— it has {steps} now", { steps: project.step_count })}
              </span>
            </span>
          </label>
        )}

        <fieldset className="field" data-feature="nrt-import:products">
          <legend>{t("Products")}</legend>
          {NRT_GROUPS.map((group) => (
            <div key={group.id} className="nrt-group">
              <span className="nrt-group-heading">{t(group.heading)}</span>
              {NRT_PRODUCTS.filter((product) => product.group === group.id).map((product) => (
                <label key={product.id} className="check">
                  <input
                    type="checkbox"
                    checked={chosen.includes(product.id)}
                    onChange={() => toggle(product.id)}
                  />
                  <span>
                    {t(product.label)} <span className="muted">— {t(product.detail)}</span>
                  </span>
                </label>
              ))}
            </div>
          ))}
          {/* The attribution the products' licence asks for, as it words it. */}
          <span className="muted nrt-credit">{COPERNICUS_CREDIT}</span>
        </fieldset>

        {/*
          The download count is what the wait is proportional to and the size
          is what the disk gives up, so both are shown before the button
          rather than discovered by pressing it.
        */}
        <p className="muted">
          {cost.downloads === 1
            ? t("1 download, about {megabytes} MB on disk.", { megabytes: cost.megabytes })
            : t("{downloads} downloads, about {megabytes} MB on disk.", {
                downloads: cost.downloads,
                megabytes: cost.megabytes,
              })}{" "}
          {t("Each product trails the present by about a day, so the newest steps are usually empty.")}
        </p>

        <div className="modal-actions">
          <button onClick={onClose}>{t("Cancel")}</button>
          <button
            disabled={!ready}
            data-feature="nrt-import:import"
            onClick={() => {
              if (!ready) return;
              onImport({
                products: NRT_PRODUCTS.map((p) => p.id).filter((id) => chosen.includes(id)),
                days: asked,
                set_start_time: setStartTime,
                extend_timeline: extending,
              });
            }}
          >
            {t("Import")}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}

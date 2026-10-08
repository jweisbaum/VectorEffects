import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { useBusy } from "./busy";
import { msg, useT } from "./i18n";
import type { HistoryProgress } from "./generated/HistoryProgress";

const PHASES: Record<string, string> = {
  preparing: msg("Checking archive"),
  indexing: msg("Reading archive index"),
  downloading: msg("Downloading history"),
  decoding: msg("Decoding history"),
  resampling: msg("Preparing fields"),
  writing: msg("Writing history"),
  importing: msg("Adding history layers"),
  complete: msg("History ready"),
};

/**
 * How far a fetch has got (spec.md 4.10, M38): the history import's, or the
 * near-real-time import's (M89), which reports the same way.
 *
 * A fetch is minutes of network with nothing else to look at, and a spinner
 * that only turns cannot tell a slow archive from a stalled one. The backend
 * reports actual work. Hindsight includes download bytes and batch phases;
 * other sources use completed steps. Preparation has no known fraction.
 *
 * It is bounded by the busy store rather than by the last event: an import
 * that fails leaves its final progress behind, and the bar has to go when the
 * command does, whichever way it ended. That is also what tells the two
 * imports' bars apart — each shows only while its own command's label is in
 * the busy set, and listens only to its own event.
 *
 * Its own component, and its own subscription, so a progress event re-renders
 * this bar and not the shell.
 */
export default function FetchProgressBar({
  busyLabel,
  event,
  waiting,
}: {
  /** The command's entry in `LONG_RUNNING`: the bar shows while it is busy. */
  busyLabel: string;
  /** The event the backend reports on; its payload is a `HistoryProgress`. */
  event: string;
  /** What to say before the first report lands. English, via `msg`. */
  waiting: string;
}) {
  const t = useT();
  const busy = useBusy();
  const [progress, setProgress] = useState<HistoryProgress | null>(null);
  const running = busy.labels.includes(busyLabel);

  useEffect(() => {
    const pending = listen<HistoryProgress>(event, (report) => {
      setProgress(report.payload);
    });
    return () => {
      void pending.then((unlisten) => unlisten());
    };
  }, [event]);

  // Cleared on the way out, so the next fetch does not open on the last
  // one's bar before its first event lands.
  useEffect(() => {
    if (!running) setProgress(null);
  }, [running]);

  if (!running) return null;
  const total = progress?.total ?? 0;
  const work = progress?.work;
  const indeterminate = progress === null || work?.phase === "preparing";
  const fraction = work?.fraction ?? (total > 0 ? (progress?.done ?? 0) / total : 0);
  const percent = Math.min(100, Math.max(0, Math.floor(fraction * 100)));
  const label =
    progress === null
      ? t(waiting)
      : work
        ? indeterminate
          ? t("{source} · {phase}", { source: progress.archive, phase: t(PHASES[work.phase] ?? "Checking archive") })
          : t("{source} · {phase} · {percent}% · {megabytes} MB", {
              source: progress.archive,
              phase: t(PHASES[work.phase] ?? "Downloading history"),
              percent,
              megabytes: (work.downloaded_bytes / 1_000_000).toFixed(1),
            })
        : `${progress.archive} · ${progress.done}/${progress.total}`;
  return (
    <span className="history-progress" title={label} aria-label={label}>
      <span className="progress-bar" role="progressbar" aria-label={label}
        aria-valuemin={0} aria-valuemax={100} aria-valuenow={indeterminate ? undefined : percent}>
        <span className={`progress-fill${indeterminate ? " indeterminate" : ""}`}
          style={{ width: indeterminate ? "35%" : `${percent}%` }} />
      </span>
      <span className="muted">{label}</span>
    </span>
  );
}

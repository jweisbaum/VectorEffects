import { useEffect, useState } from "react";

import { api } from "../ipc";
import type { HistoryView } from "../generated/HistoryView";
import type { ProjectSummary } from "../generated/ProjectSummary";

/**
 * The undo stack, as a list you can jump around in.
 *
 * Entries past the cursor are dimmed: they have been undone but are still
 * available to redo, until the next edit discards that branch.
 */
export default function HistoryPanel({
  project,
  onChanged,
}: {
  project: ProjectSummary;
  onChanged: (project: ProjectSummary) => void;
}) {
  const [history, setHistory] = useState<HistoryView | null>(null);

  useEffect(() => {
    api.historyView().then(setHistory).catch(() => setHistory(null));
  }, [project.revision, project.can_undo, project.can_redo]);

  if (!history || history.entries.length === 0) {
    return null;
  }

  return (
    <div className="history-panel">
      <ol className="history">
        <li
          className={history.cursor === 0 ? "entry current" : "entry"}
          onClick={() => void api.jumpToHistory(0).then(onChanged)}
        >
          <span className="muted">Opened</span>
        </li>
        {history.entries.map((entry) => (
          <li
            key={entry.index}
            className={[
              "entry",
              entry.applied ? "" : "undone",
              history.cursor === entry.index + 1 ? "current" : "",
            ]
              .filter(Boolean)
              .join(" ")}
            onClick={() => void api.jumpToHistory(entry.index + 1).then(onChanged)}
            title="Jump to this point"
          >
            {entry.label}
          </li>
        ))}
      </ol>
    </div>
  );
}

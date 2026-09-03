import { useEffect } from "react";

import type { UnsavedChoice } from "./saveGuard";

/**
 * Offers to save before the open project is replaced or closed.
 *
 * Three choices, not two: a plain confirm can only ask "discard?", which makes
 * saving a separate thing the user has to think of first. Cancel is the default
 * — Escape and a click outside both take it — because it is the only one of the
 * three that cannot lose anything.
 */
export default function UnsavedChangesDialog({
  name,
  onChoose,
}: {
  name: string;
  onChoose: (choice: UnsavedChoice) => void;
}) {
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onChoose("cancel");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onChoose]);

  return (
    <div className="modal-backdrop" onClick={() => onChoose("cancel")}>
      <div className="modal modal-narrow" onClick={(e) => e.stopPropagation()}>
        <h2>Unsaved changes</h2>
        <p className="modal-summary">
          “{name}” has changes that have not been saved.
        </p>

        <div className="modal-actions">
          <button onClick={() => onChoose("discard")} className="danger">
            Don’t save
          </button>
          <span className="spacer" />
          <button onClick={() => onChoose("cancel")}>Cancel</button>
          <button className="primary" autoFocus onClick={() => onChoose("save")}>
            Save
          </button>
        </div>
      </div>
    </div>
  );
}

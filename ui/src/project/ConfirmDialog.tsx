import { useEffect } from "react";

/**
 * Asks before something irreversible.
 *
 * Cancel is the default — Escape and a click outside both take it — because it
 * is the choice that cannot lose anything, the same reasoning as
 * `UnsavedChangesDialog`. The confirming button carries `danger` so the
 * destructive choice is never the one the eye lands on first.
 */
export default function ConfirmDialog({
  title,
  body,
  confirmLabel,
  onConfirm,
  onCancel,
}: {
  title: string;
  body: string;
  confirmLabel: string;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onCancel();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);

  return (
    <div className="modal-backdrop" onClick={onCancel}>
      <div className="modal modal-narrow" onClick={(e) => e.stopPropagation()}>
        <h2>{title}</h2>
        <p className="modal-summary">{body}</p>

        <div className="modal-actions">
          <button onClick={onConfirm} className="danger">
            {confirmLabel}
          </button>
          <span className="spacer" />
          <button className="primary" autoFocus onClick={onCancel}>
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
}

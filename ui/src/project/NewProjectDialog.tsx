import { useEffect, useState } from "react";

import { api, IpcError } from "../ipc";
import type { NewProjectRequest } from "../generated/NewProjectRequest";
import type { ProjectSummary } from "../generated/ProjectSummary";
import NewProjectForm from "./NewProjectForm";

/**
 * Creates a project without going back to the start screen.
 *
 * `discardUnsaved` is the answer the user already gave to the unsaved-changes
 * prompt, carried through to the command that acts on it. Nothing is dropped
 * before this dialog's Create: cancel here and the old project is still open,
 * unsaved.
 */
export default function NewProjectDialog({
  discardUnsaved,
  onCreated,
  onCancel,
}: {
  discardUnsaved: boolean;
  onCreated: (project: ProjectSummary) => void;
  onCancel: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !busy) {
        event.preventDefault();
        onCancel();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [busy, onCancel]);

  const create = (request: NewProjectRequest) => {
    setBusy(true);
    setError(null);
    api
      .newProject(request, discardUnsaved)
      .then(onCreated)
      .catch((err) => setError(err instanceof IpcError ? err.message : String(err)))
      .finally(() => setBusy(false));
  };

  return (
    <div className="modal-backdrop" onClick={busy ? undefined : onCancel}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>New project</h2>
        <div className="start-new">
          <NewProjectForm disabled={busy} submitLabel="Create project" onSubmit={create} />
        </div>
        {error !== null && <p className="error">{error}</p>}
        <div className="modal-actions">
          <button onClick={onCancel} disabled={busy}>
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
}

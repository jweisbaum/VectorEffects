/**
 * The interface following the MCP service (spec 8.8, M76).
 *
 * The service emits `document://changed` after every write. This applies it
 * the way the app applies the result of its own call: an edit replaces the
 * summary and the panels refresh by revision; a different project resets
 * what `openPath` resets.
 */
import type { DocumentChanged } from "../generated/DocumentChanged";
import type { ProjectSummary } from "../generated/ProjectSummary";

export interface FollowActions {
  setProject: (project: ProjectSummary | null) => void;
  setStep: (step: number) => void;
  setSelection: (objects: number[]) => void;
  setShapeEditing: (object: number | null) => void;
  setActiveLayer: (layer: number | null) => void;
  /** What `openPath` clears with `reportError(null)` (App.tsx, M76). */
  clearError: () => void;
}

export function applyDocumentChanged(payload: DocumentChanged, actions: FollowActions): void {
  if (payload.opened || payload.project === null) {
    actions.setSelection([]);
    actions.setShapeEditing(null);
    actions.setActiveLayer(null);
    actions.setStep(0);
    actions.clearError();
  }
  actions.setProject(payload.project);
}

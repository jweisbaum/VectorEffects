import { useEffect } from "react";
import { api } from "./ipc";
import type { ProjectSummary } from "./generated/ProjectSummary";

/** Selection belongs to the document, even when the layer panel is closed. */
export function useSelectionLifecycle(
  project: ProjectSummary | null,
  selection: number[],
  onSelect: (ids: number[]) => void,
): void {
  useEffect(() => {
    if (!project || selection.length === 0) return;
    let live = true;
    void api.documentTree(0).then((tree) => {
      if (!live) return;
      const present = new Set(tree.layers.flatMap((layer) => layer.objects.map((object) => object.id)));
      const kept = selection.filter((id) => present.has(id));
      if (kept.length !== selection.length) onSelect(kept);
    }).catch(() => { /* The panels report read failures; retain selection on failure. */ });
    return () => { live = false; };
  }, [project?.image_token, project?.revision, selection, onSelect]);
}

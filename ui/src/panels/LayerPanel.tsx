import { useEffect, useState } from "react";

import { api } from "../ipc";
import type { DocumentTree } from "../generated/DocumentTree";
import type { ProjectSummary } from "../generated/ProjectSummary";

/** What is being dragged, while a reorder is in progress. */
type Dragging =
  | { kind: "layer"; id: number; index: number }
  | { kind: "object"; id: number };

/**
 * Layers and their objects.
 *
 * Presented top-first, the way a user thinks about a stack, while the document
 * stores them bottom-first — the order they composite in (spec.md 4.3). The
 * reversal happens here and nowhere else, which is why every drop handler
 * converts a displayed position back into a document index before calling the
 * backend.
 */
export default function LayerPanel({
  project,
  step,
  selection,
  activeLayer,
  onSelect,
  onActivateLayer,
  onChanged,
}: {
  project: ProjectSummary;
  step: number;
  selection: number[];
  activeLayer: number | null;
  onSelect: (objects: number[]) => void;
  onActivateLayer: (layer: number | null) => void;
  onChanged: (project: ProjectSummary) => void;
}) {
  const [tree, setTree] = useState<DocumentTree | null>(null);
  const [renaming, setRenaming] = useState<number | null>(null);
  const [draft, setDraft] = useState("");
  const [dragging, setDragging] = useState<Dragging | null>(null);
  const [dropTarget, setDropTarget] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  // The tree is derived from the document, so it is refetched whenever the
  // revision moves rather than being patched in place.
  useEffect(() => {
    api
      .documentTree(step)
      .then(setTree)
      .catch((err: unknown) => setError(String(err)));
  }, [project.revision, step]);

  const run = (action: Promise<ProjectSummary>) => {
    setError(null);
    action.then(onChanged).catch((err: unknown) => setError(String(err)));
  };

  const commitRename = (id: number, isLayer: boolean) => {
    const name = draft.trim();
    setRenaming(null);
    if (name.length === 0) return;
    run(isLayer ? api.renameLayer(id, name) : api.renameObject(id, name));
  };

  /** Click semantics: plain replaces the selection, accel adds or removes. */
  const clickObject = (id: number, layer: number, event: React.MouseEvent) => {
    onActivateLayer(layer);
    if (event.metaKey || event.ctrlKey) {
      onSelect(
        selection.includes(id) ? selection.filter((x) => x !== id) : [...selection, id],
      );
    } else if (event.shiftKey && selection.length > 0) {
      onSelect(selection.includes(id) ? selection : [...selection, id]);
    } else {
      onSelect([id]);
    }
  };

  const endDrag = () => {
    setDragging(null);
    setDropTarget(null);
  };

  if (!tree) {
    return <div className="panel-empty muted">{error ?? "Loading…"}</div>;
  }

  // Top of the stack first: index 0 is the bottom of the document.
  const layers = [...tree.layers].reverse();

  /** Moves the dragged layer to where `targetIndex` currently sits. */
  const dropOnLayer = (targetIndex: number, targetId: number) => {
    const drag = dragging;
    endDrag();
    if (!drag) return;
    if (drag.kind === "layer") {
      if (drag.index !== targetIndex) run(api.moveLayer(drag.index, targetIndex));
      return;
    }
    // An object dropped on a layer header joins the top of that layer.
    const layer = tree.layers.find((l) => l.id === targetId);
    if (layer) run(api.moveObject(drag.id, targetId, layer.objects.length));
  };

  /** Moves the dragged object to a position in the object list. */
  const dropOnObject = (layerId: number, documentIndex: number) => {
    const drag = dragging;
    endDrag();
    if (drag?.kind !== "object") return;
    run(api.moveObject(drag.id, layerId, documentIndex));
  };

  return (
    <div className="layer-panel">
      <header>
        <h2>Layers</h2>
        <button title="Add a layer" onClick={() => run(api.addLayer(""))}>
          +
        </button>
      </header>

      <ul className="layers">
        {layers.map((layer, reversed) => {
          const index = tree.layers.length - 1 - reversed;
          return (
            <li key={layer.id} className="layer">
              <div
                className={[
                  "layer-header",
                  activeLayer === layer.id ? "active" : "",
                  dropTarget === layer.id ? "drop-target" : "",
                ]
                  .filter(Boolean)
                  .join(" ")}
                onClick={() => onActivateLayer(layer.id)}
                title="Click to make this the active layer"
                draggable
                onDragStart={(e) => {
                  e.dataTransfer.effectAllowed = "move";
                  // Some payload is required for a drag to start at all.
                  e.dataTransfer.setData("text/plain", String(layer.id));
                  setDragging({ kind: "layer", id: layer.id, index });
                }}
                onDragEnd={endDrag}
                onDragOver={(e) => {
                  if (!dragging) return;
                  e.preventDefault();
                  e.dataTransfer.dropEffect = "move";
                  setDropTarget(layer.id);
                }}
                onDragLeave={() => setDropTarget((t) => (t === layer.id ? null : t))}
                onDrop={(e) => {
                  e.preventDefault();
                  dropOnLayer(index, layer.id);
                }}
              >
                <button
                  className={layer.visible ? "eye on" : "eye"}
                  title={layer.visible ? "Hide" : "Show"}
                  onClick={() => run(api.setLayerVisible(layer.id, !layer.visible))}
                >
                  {layer.visible ? "◉" : "○"}
                </button>
                <button
                  className={layer.locked ? "lock on" : "lock"}
                  title={layer.locked ? "Unlock" : "Lock"}
                  onClick={() => run(api.setLayerLocked(layer.id, !layer.locked))}
                >
                  {layer.locked ? "🔒" : "🔓"}
                </button>

                {renaming === layer.id ? (
                  <input
                    autoFocus
                    value={draft}
                    onChange={(e) => setDraft(e.target.value)}
                    onBlur={() => commitRename(layer.id, true)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") commitRename(layer.id, true);
                      if (e.key === "Escape") setRenaming(null);
                    }}
                  />
                ) : (
                  <span
                    className="name"
                    onDoubleClick={() => {
                      setRenaming(layer.id);
                      setDraft(layer.name);
                    }}
                    title="Drag to reorder · double-click to rename"
                  >
                    {layer.name}
                  </span>
                )}

                <span className="spacer" />
                <button
                  title="Move up"
                  disabled={index === tree.layers.length - 1}
                  onClick={() => run(api.moveLayer(index, index + 1))}
                >
                  ↑
                </button>
                <button
                  title="Move down"
                  disabled={index === 0}
                  onClick={() => run(api.moveLayer(index, index - 1))}
                >
                  ↓
                </button>
                <button
                  title="Delete layer"
                  disabled={tree.layers.length <= 1}
                  onClick={() => run(api.removeLayer(layer.id))}
                >
                  ×
                </button>
              </div>

              <ul
                className="objects"
                onDragOver={(e) => {
                  if (dragging?.kind !== "object") return;
                  e.preventDefault();
                  e.dataTransfer.dropEffect = "move";
                }}
                onDrop={(e) => {
                  // Dropping on the list's empty space, rather than on a row,
                  // means "the bottom of this layer".
                  if (e.currentTarget !== e.target) return;
                  e.preventDefault();
                  dropOnObject(layer.id, 0);
                }}
              >
                {/* Objects are also shown top-first. */}
                {[...layer.objects].reverse().map((object, reversedObject) => {
                  const documentIndex = layer.objects.length - 1 - reversedObject;
                  return (
                    <li
                      key={object.id}
                      className={[
                        "object",
                        selection.includes(object.id) ? "selected" : "",
                        object.active_here ? "" : "inactive",
                        dropTarget === object.id ? "drop-target" : "",
                      ]
                        .filter(Boolean)
                        .join(" ")}
                      draggable
                      onDragStart={(e) => {
                        e.stopPropagation();
                        e.dataTransfer.effectAllowed = "move";
                        e.dataTransfer.setData("text/plain", String(object.id));
                        setDragging({ kind: "object", id: object.id });
                      }}
                      onDragEnd={endDrag}
                      onDragOver={(e) => {
                        if (dragging?.kind !== "object") return;
                        e.preventDefault();
                        e.stopPropagation();
                        e.dataTransfer.dropEffect = "move";
                        setDropTarget(object.id);
                      }}
                      onDragLeave={() =>
                        setDropTarget((t) => (t === object.id ? null : t))
                      }
                      onDrop={(e) => {
                        e.preventDefault();
                        e.stopPropagation();
                        dropOnObject(layer.id, documentIndex);
                      }}
                      onClick={(e) => clickObject(object.id, layer.id, e)}
                    >
                      {renaming === object.id ? (
                        <input
                          autoFocus
                          value={draft}
                          onChange={(e) => setDraft(e.target.value)}
                          onBlur={() => commitRename(object.id, false)}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") commitRename(object.id, false);
                            if (e.key === "Escape") setRenaming(null);
                          }}
                        />
                      ) : (
                        <span
                          className="name"
                          onDoubleClick={() => {
                            setRenaming(object.id);
                            setDraft(object.name);
                          }}
                          title={`${object.tool_label} · steps ${object.start_step}–${object.end_step}`}
                        >
                          {object.name}
                        </span>
                      )}
                      <span className="tool muted">{object.tool_label}</span>
                      <button
                        title="Duplicate"
                        onClick={(e) => {
                          e.stopPropagation();
                          run(api.duplicateObject(object.id));
                        }}
                      >
                        ⧉
                      </button>
                      <button
                        title="Delete"
                        onClick={(e) => {
                          e.stopPropagation();
                          onSelect(selection.filter((id) => id !== object.id));
                          run(api.removeObject(object.id));
                        }}
                      >
                        ×
                      </button>
                    </li>
                  );
                })}
                {layer.objects.length === 0 && (
                  <li className="object empty muted">empty</li>
                )}
              </ul>
            </li>
          );
        })}
      </ul>

      {error !== null && <p className="error">{error}</p>}
    </div>
  );
}

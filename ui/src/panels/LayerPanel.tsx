import { useEffect, useState } from "react";

import { reportError } from "../hint";
import { api } from "../ipc";
import type { DocumentTree } from "../generated/DocumentTree";
import type { ProjectSummary } from "../generated/ProjectSummary";
import type { ImageLayerView } from "../generated/ImageLayerView";
import { pickGribToImport, pickImageToImport } from "../project/dialogs";
import { EyeIcon } from "./EyeIcon";
import SpeedFilter from "./SpeedFilter";

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
  viewBounds,
}: {
  project: ProjectSummary;
  step: number;
  selection: number[];
  activeLayer: number | null;
  onSelect: (objects: number[]) => void;
  onActivateLayer: (layer: number | null) => void;
  onChanged: (project: ProjectSummary) => void;
  /**
   * The visible map as `[west, north, east, south]`, for an image that carries
   * no georeference: it lands filling the view, so its control points are
   * somewhere the user can reach them (spec.md 4.9, M18).
   */
  viewBounds?: () => [number, number, number, number] | null;
}) {
  const [tree, setTree] = useState<DocumentTree | null>(null);
  /**
   * The revision `tree` was fetched at. The speed filter compares it with
   * the revision its write returned, to know when the tree it is reading has
   * caught up with the band it wrote.
   */
  const [treeRevision, setTreeRevision] = useState(0);
  const [renaming, setRenaming] = useState<number | null>(null);
  const [draft, setDraft] = useState("");
  const [dragging, setDragging] = useState<Dragging | null>(null);
  const [dropTarget, setDropTarget] = useState<number | null>(null);
  // Errors go to the status bar's hint area (M25), not a line of their own.
  const setError = reportError;
  /**
   * Layers whose object lists are folded away (M25). Panel state: which
   * lists are open is how this person is looking at the project, not a fact
   * about it.
   */
  const [folded, setFolded] = useState<Set<number>>(new Set());
  const toggleFold = (id: number) =>
    setFolded((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  // The tree is derived from the document, so it is refetched whenever the
  // revision moves rather than being patched in place. A fetch overtaken by
  // a newer one is dropped: two in flight are not guaranteed to answer in
  // order, and the older tree landing last would show a band the document no
  // longer holds.
  useEffect(() => {
    let stale = false;
    const revision = project.revision;
    api
      .documentTree(step)
      .then((next) => {
        if (stale) return;
        setTree(next);
        setTreeRevision(revision);
      })
      .catch((err: unknown) => {
        if (!stale) setError(String(err));
      });
    return () => {
      stale = true;
    };
  }, [project.revision, step]);

  /**
   * Runs a document write and hands the summary back to the app. Resolves to
   * the summary, or null when the write was refused — the error has already
   * gone to the status bar, so a caller that needs to know only needs that.
   */
  const run = (
    action: Promise<ProjectSummary>,
    done?: () => void,
  ): Promise<ProjectSummary | null> => {
    setError(null);
    return action
      .then((summary) => {
        onChanged(summary);
        return summary;
      })
      .catch((err: unknown) => {
        setError(String(err));
        return null;
      })
      .finally(() => done?.());
  };

  /** Picks a GRIB2 file and imports it as a layer, or two if it holds both kinds. */
  const importGrib = async () => {
    setError(null);
    const path = await pickGribToImport();
    if (path === null) return;
    run(api.importGrib(path));
  };

  /** Picks an image and lays it under the field (spec.md 4.9, M18). */
  const importImage = async () => {
    setError(null);
    const path = await pickImageToImport();
    if (path === null) return;
    run(api.importImage(path, viewBounds?.() ?? null));
  };

  const commitRename = (id: number, isLayer: boolean) => {
    const name = draft.trim();
    setRenaming(null);
    if (name.length === 0) return;
    run(isLayer ? api.renameLayer(id, name) : api.renameObject(id, name));
  };

  /**
   * A shift-press on a row extends the selection and must not also start a
   * text selection across the rows, which is the browser's default for a
   * shift-click (M28). Belt to the stylesheet's `user-select` braces.
   */
  const noTextSelect = (event: React.MouseEvent) => {
    if (event.shiftKey) event.preventDefault();
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
    return <div className="panel-empty muted">Loading…</div>;
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
        <button
          className="import-grib"
          title="Import a GRIB2 file as a layer"
          onClick={() => void importGrib()}
        >
          Import GRIB
        </button>
        <button
          className="import-grib"
          title="Lay a georeferenced image under the field. A GeoTIFF or an image with a world file lands where it says; anything else lands on the view, to be placed by its corners."
          onClick={() => void importImage()}
        >
          Import image
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
                onMouseDown={noTextSelect}
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
                  // An imported layer takes no objects (D66): a layer can be
                  // dropped beside it, but an object dropped onto it would be
                  // refused, so the target is not offered.
                  if (dragging.kind === "object" && layer.grib !== null) return;
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
                  className={folded.has(layer.id) ? "fold" : "fold open"}
                  title={folded.has(layer.id) ? "Show this layer's objects" : "Hide this layer's objects"}
                  aria-label={folded.has(layer.id) ? "Show objects" : "Hide objects"}
                  aria-expanded={!folded.has(layer.id)}
                  onClick={(e) => {
                    e.stopPropagation();
                    toggleFold(layer.id);
                  }}
                >
                  ▾
                </button>
                <button
                  className={layer.visible ? "eye on" : "eye"}
                  title={layer.visible ? "Hide layer" : "Show layer"}
                  aria-label={layer.visible ? "Hide layer" : "Show layer"}
                  aria-pressed={layer.visible}
                  onClick={() => run(api.setLayerVisible(layer.id, !layer.visible))}
                >
                  <EyeIcon open={layer.visible} />
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
                {layer.grib && !layer.grib.loaded && (
                  <span
                    className="grib-missing"
                    title={`${layer.grib.path}\nThe file could not be read, so this layer contributes nothing.`}
                  >
                    file missing
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

              {layer.grib?.loaded && (
                <SpeedFilter
                  grib={layer.grib}
                  treeRevision={treeRevision}
                  onChange={(min, max, gesture) =>
                    run(api.setLayerSpeedRange(layer.id, min, max, gesture)).then(
                      (summary) => summary?.revision ?? null,
                    )
                  }
                />
              )}

              {layer.image && (
                <ImageControls
                  image={layer.image}
                  onOpacity={(value) => run(api.setImageOpacity(layer.id, value))}
                  onReset={() => run(api.resetImagePlacement(layer.id))}
                />
              )}

              {!folded.has(layer.id) && (
              <ul
                className="objects"
                onDragOver={(e) => {
                  if (dragging?.kind !== "object" || layer.grib !== null) return;
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
                      onMouseDown={noTextSelect}
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
                {/* A GRIB layer's field is its content; only a painted layer
                    with nothing on it is empty. */}
                {layer.objects.length === 0 && !layer.grib && (
                  <li className="object empty muted">empty</li>
                )}
              </ul>
              )}
            </li>
          );
        })}
      </ul>

    </div>
  );
}

/**
 * An image layer's own controls (spec.md 4.9, M18).
 *
 * Opacity, because an image under a field is a reference and a reference at
 * full strength hides what it is a reference *for*. And a way back to the
 * file's own georeference — offered only when the file has one, since a
 * hand-placed image has nothing to go back to.
 */
function ImageControls({
  image,
  onOpacity,
  onReset,
}: {
  image: ImageLayerView;
  onOpacity: (opacity: number) => void;
  onReset: () => void;
}) {
  if (!image.loaded) {
    return (
      <div className="image-controls">
        <span className="grib-missing" title={`${image.path}\nThe file could not be read.`}>
          image missing
        </span>
      </div>
    );
  }
  return (
    <div className="image-controls">
      <label>
        Opacity
        <input
          type="range"
          min={0}
          max={100}
          value={Math.round(image.opacity * 100)}
          onChange={(event) => onOpacity(Number(event.target.value) / 100)}
          title="How strongly the image shows through"
        />
      </label>
      <span className="muted">
        {image.width}×{image.height}
        {image.georeferenced ? " · georeferenced" : " · placed by hand"}
      </span>
      {image.georeferenced && (
        <button onClick={onReset} title="Put the image back where its own file says it goes">
          Reset place
        </button>
      )}
    </div>
  );
}

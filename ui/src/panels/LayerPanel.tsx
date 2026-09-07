import { useEffect, useRef, useState } from "react";

import { reportError } from "../hint";
import { api } from "../ipc";
import type { DocumentTree } from "../generated/DocumentTree";
import type { ProjectSummary } from "../generated/ProjectSummary";
import type { ImageLayerView } from "../generated/ImageLayerView";
import { pickGribToImport, pickImageToImport } from "../project/dialogs";
import { CalendarIcon } from "./CalendarIcon";
import { EyeIcon } from "./EyeIcon";
import HistoryImportDialog, { type HistoryChoice } from "./HistoryImportDialog";
import { dropSide, layerDropIndex } from "./reorder";
import { KIND_LABELS, KINDS, type FieldKindName, kindOf } from "../kind";
import SpeedFilter from "./SpeedFilter";

/** Which side of a row a drop lands on: above or below a layer, or into it. */
type DropWhere = "above" | "below" | "into";

/** A row a drop would land on, read off the DOM under the pointer. */
type DropAt =
  | { row: "layer"; id: number; index: number; where: DropWhere }
  | { row: "object"; id: number; layer: number; index: number };

/** How far the pointer must travel before a press becomes a drag, in px. */
const DRAG_SLACK_PX = 4;

/**
 * The row under the pointer, and which side of it a layer would land on.
 *
 * Read from the DOM rather than from React's own hit testing, because the
 * pointer is captured by the row the press began on: every move after that
 * is delivered there whatever it is over. `elementFromPoint` is what says
 * where it actually is.
 *
 * **Reordering is pointer-driven and not HTML5 drag-and-drop** (M33). WebKit
 * will not begin a native drag from an element it considers unselectable,
 * and the panel's rows are `user-select: none` so that a shift-click extends
 * the selection instead of selecting text across them — so `draggable` rows
 * simply never started a drag in the app, however correct the drop handlers
 * were. Pointer events have no such rule and are the same events the map
 * already reorders and transforms with.
 */
function dropAt(x: number, y: number, dragging: Dragging["kind"]): DropAt | null {
  const at = document.elementFromPoint(x, y);
  if (!(at instanceof Element)) return null;
  const objectRow = dragging === "object" ? at.closest<HTMLElement>("[data-object-id]") : null;
  if (objectRow) {
    return {
      row: "object",
      id: Number(objectRow.dataset.objectId),
      layer: Number(objectRow.dataset.objectLayer),
      index: Number(objectRow.dataset.objectIndex),
    };
  }
  const block = at.closest<HTMLElement>("[data-layer-id]");
  if (!block) return null;
  // Above or below is which half of the row the pointer is over — of the
  // header where it is over one, and of the whole layer otherwise, so the
  // space beside a layer's objects is still a place to drop beside it.
  const header = at.closest<HTMLElement>(".layer-header") ?? block;
  const rect = header.getBoundingClientRect();
  return {
    row: "layer",
    id: Number(block.dataset.layerId),
    index: Number(block.dataset.layerIndex),
    where: dropSide(y, rect.top, rect.height),
  };
}

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
  onActiveKind,
  onChanged,
  viewBounds,
}: {
  project: ProjectSummary;
  step: number;
  selection: number[];
  activeLayer: number | null;
  onSelect: (objects: number[]) => void;
  onActivateLayer: (layer: number | null) => void;
  /** The active layer's kind of field, so the map turns to it (M29). */
  onActiveKind: (kind: FieldKindName) => void;
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
  /** The row a drop would land on, and — for a layer — which side of it. */
  const [dropTarget, setDropTarget] = useState<{ id: number; where: DropWhere } | null>(null);
  // Errors go to the status bar's hint area (M25), not a line of their own.
  const setError = reportError;
  /**
   * Layers whose object lists are folded away (M25). Panel state: which
   * lists are open is how this person is looking at the project, not a fact
   * about it.
   */
  const [folded, setFolded] = useState<Set<number>>(new Set());
  /** Whether the history range dialog is up (M38). */
  const [historyOpen, setHistoryOpen] = useState(false);
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

  // The map shows the active layer's kind of field (M29): making a current
  // layer active turns the map to the currents, since that is what a stroke
  // in it will paint.
  useEffect(() => {
    const active = tree?.layers.find((layer) => layer.id === activeLayer);
    if (active && active.source !== "image") onActiveKind(kindOf(active.parameter));
  }, [activeLayer, onActiveKind, tree]);

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

  /**
   * Fetches a range of past hours and lands one layer per archive
   * (spec.md 4.10, M38). The one action in the application that reaches the
   * network, and only from this button.
   */
  const importHistory = (choice: HistoryChoice) => {
    setHistoryOpen(false);
    setError(null);
    run(api.importHistory(choice.archives, choice.startUnixS, choice.endUnixS));
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

  /**
   * The press in progress, and whether it has become a drag.
   *
   * A press that never travels is a click — activating a layer, selecting an
   * object — so the click handlers stand down only once one actually has.
   */
  const press = useRef<{ drag: Dragging; x: number; y: number; moved: boolean } | null>(null);
  const dragged = useRef(false);
  /**
   * What a release does, kept in a ref because it needs the layer tree and
   * the hooks here run before the tree is known to exist.
   */
  const dropRef = useRef<(drag: Dragging, at: DropAt | null) => void>(() => {});

  const startPress = (event: React.PointerEvent, drag: Dragging) => {
    if (event.button !== 0) return;
    press.current = { drag, x: event.clientX, y: event.clientY, moved: false };
    dragged.current = false;
  };

  useEffect(() => {
    const move = (event: PointerEvent) => {
      const held = press.current;
      if (!held) return;
      if (
        !held.moved &&
        Math.hypot(event.clientX - held.x, event.clientY - held.y) < DRAG_SLACK_PX
      ) {
        return;
      }
      if (!held.moved) {
        held.moved = true;
        dragged.current = true;
        setDragging(held.drag);
      }
      const at = dropAt(event.clientX, event.clientY, held.drag.kind);
      const shown =
        at === null
          ? null
          : at.row === "object"
            ? { id: at.id, where: "into" as DropWhere }
            : { id: at.id, where: held.drag.kind === "object" ? ("into" as DropWhere) : at.where };
      setDropTarget((current) =>
        current?.id === shown?.id && current?.where === shown?.where ? current : shown,
      );
    };
    const up = (event: PointerEvent) => {
      const held = press.current;
      press.current = null;
      if (!held) return;
      if (held.moved) {
        dropRef.current(held.drag, dropAt(event.clientX, event.clientY, held.drag.kind));
      }
      setDragging(null);
      setDropTarget(null);
      // The click that follows a drag is not a click on the row it ended on;
      // cleared afterwards so the next press is one again.
      window.setTimeout(() => {
        dragged.current = false;
      }, 0);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
    };
  }, []);

  if (!tree) {
    return <div className="panel-empty muted">Loading…</div>;
  }

  // Top of the stack first: index 0 is the bottom of the document.
  const layers = [...tree.layers].reverse();

  /**
   * What a release does, with the tree it needs (M33). Reassigned every
   * render, so the drop that runs is the one for the panel as it stands.
   *
   * A layer lands above or below the row it was let go on — the halves of
   * that row, so a layer can be put on either side of any other in one drag
   * (M29). An object lands in the layer of whatever it was let go on: at that
   * object's place among its siblings, or at the top of a layer dropped on
   * directly.
   */
  dropRef.current = (drag, at) => {
    if (at === null) return;
    if (drag.kind === "layer") {
      if (at.row !== "layer") return;
      const to = layerDropIndex(drag.index, at.index, at.where !== "below");
      if (drag.index !== to) run(api.moveLayer(drag.index, to));
      return;
    }
    if (at.row === "object") {
      dropOnObject(at.layer, at.index);
      return;
    }
    const layer = tree.layers.find((l) => l.id === at.id);
    if (layer) run(api.moveObject(drag.id, at.id, layer.objects.length));
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
        <button
          className="import-grib icon-button"
          title="Import past hours from the ERA5 and GlobCurrent archives as layers. This is the only action that reaches the network."
          aria-label="Import history"
          onClick={() => setHistoryOpen(true)}
        >
          <CalendarIcon />
        </button>
      </header>

      {historyOpen && (
        <HistoryImportDialog
          project={project}
          now={Date.now() / 1000}
          onImport={importHistory}
          onClose={() => setHistoryOpen(false)}
        />
      )}

      <ul className="layers">
        {layers.map((layer, reversed) => {
          const index = tree.layers.length - 1 - reversed;
          return (
            <li key={layer.id} className="layer" data-layer-id={layer.id} data-layer-index={index}>
              <div
                className={[
                  "layer-header",
                  activeLayer === layer.id ? "active" : "",
                  dropTarget?.id === layer.id ? `drop-${dropTarget.where}` : "",
                ]
                  .filter(Boolean)
                  .join(" ")}
                onClick={() => {
                  if (dragged.current) return;
                  onActivateLayer(layer.id);
                }}
                onMouseDown={noTextSelect}
                onPointerDown={(e) => startPress(e, { kind: "layer", id: layer.id, index })}
                title="Click to make this the active layer; drag to reorder"
              >
                <span className="grip" aria-hidden="true" title="Drag to reorder">
                  ⋮⋮
                </span>
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

              {/*
                Which field the layer is part of (M29). A painted layer
                chooses; an imported one is its file's; an image has none.
              */}
              {layer.source === "painted" && (
                <div className="layer-parameter">
                  <label title="Which field this layer's objects are part of. Wind layers export together as the wind messages, current layers as the current messages.">
                    Field
                    <select
                      value={layer.parameter}
                      onChange={(e) =>
                        run(api.setLayerParameter(layer.id, kindOf(e.target.value)))
                      }
                    >
                      {KINDS.map((kind) => (
                        <option key={kind} value={kind}>
                          {KIND_LABELS[kind]}
                        </option>
                      ))}
                    </select>
                  </label>
                </div>
              )}
              {(layer.source === "raster" || layer.source === "zarr") && (
                <div className="layer-parameter muted">
                  Field: {KIND_LABELS[kindOf(layer.parameter)]} ·{" "}
                  {layer.source === "zarr" ? "from the archive" : "from the file"}
                </div>
              )}
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
              <ul className="objects">
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
                        dropTarget?.id === object.id ? "drop-target" : "",
                      ]
                        .filter(Boolean)
                        .join(" ")}
                      data-object-id={object.id}
                      data-object-layer={layer.id}
                      data-object-index={documentIndex}
                      onMouseDown={noTextSelect}
                      onPointerDown={(e) => startPress(e, { kind: "object", id: object.id })}
                      onClick={(e) => {
                        if (dragged.current) return;
                        clickObject(object.id, layer.id, e);
                      }}
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

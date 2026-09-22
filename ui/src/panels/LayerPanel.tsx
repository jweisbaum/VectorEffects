import { useEffect, useRef, useState } from "react";

import { reportError } from "../hint";
import { api } from "../ipc";
import type { DocumentTree } from "../generated/DocumentTree";
import type { ProjectSummary } from "../generated/ProjectSummary";
import type { ImageLayerView } from "../generated/ImageLayerView";
import type { GisLayerView } from "../generated/GisLayerView";
import {
  isGeoRaster,
  pickGisToImport,
  pickGribToImport,
  pickImageToImport,
  pickZarrToImport,
} from "../project/dialogs";
import { layerForSelection, layerToActivate } from "./activeLayer";
import { CalendarIcon } from "./CalendarIcon";
import { AlignIcon } from "./AlignIcon";
import { EyeIcon } from "./EyeIcon";
import HistoryImportDialog, { type HistoryChoice } from "./HistoryImportDialog";
import { dropSide, layerDropIndex, objectDropIndex } from "./reorder";
import { KIND_LABELS, KINDS, type FieldKindName, kindOf } from "../kind";
import SpeedFilter from "./SpeedFilter";

/**
 * Whether a layer source carries a field of its own.
 *
 * An image and a GIS layer are display only (spec.md 4.9, 4.11): they are
 * drawn under the field, make none, and reach no export — so there is no
 * speed to threshold and the speed filter is not offered for them.
 *
 * Written as the sources that *do* carry a field rather than the two that do
 * not, so a new display-only source is not handed a control it cannot honour.
 */
const hasField = (source: string) =>
  source === "painted" || source === "raster" || source === "zarr";

/** A drop lands above or below a row, or into a layer. */
type DropWhere = "above" | "below" | "into";

/** A row a drop would land on, read off the DOM under the pointer. */
type DropAt =
  | { row: "layer"; id: number; where: DropWhere }
  | { row: "object"; id: number; layer: number; where: "above" | "below" };

/** How far the pointer must travel before a press becomes a drag, in px. */
const DRAG_SLACK_PX = 4;

/**
 * The row under the pointer, and which side of it a drop would land on.
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
    const rect = objectRow.getBoundingClientRect();
    return {
      row: "object",
      id: Number(objectRow.dataset.objectId),
      layer: Number(objectRow.dataset.objectLayer),
      where: dropSide(y, rect.top, rect.height),
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
    where: dropSide(y, rect.top, rect.height),
  };
}

/** What is being dragged, while a reorder is in progress. */
type Dragging =
  | { kind: "layer"; id: number }
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
  onAlign,
  canAlign,
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
  /**
   * Arms the map's image alignment mode for a layer (Task 7): the "Align…"
   * button in `ImageControls`. The map owns the interaction, since the map
   * is where the control points are placed.
   */
  onAlign: (layer: number) => void;
  /**
   * Whether the projection on show can be aligned on (spec.md 4.9): the
   * cylindrical maps only. `App` decides; the panel only carries it.
   */
  canAlign: boolean;
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
  const opening = useRef<number | null>(project.image_token);
  opening.current = project.image_token;
  useEffect(() => {
    opening.current = project.image_token;
    return () => { opening.current = null; };
  }, [project.image_token]);
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

  // A layer is always selected while a project is open (M75). A tool picked
  // up with nothing active had no layer of its own to draw into; it fell
  // through to "the top of the stack", which is right as a default and wrong
  // as a thing the user cannot see. Done here rather than on the tool change
  // that prompted the report: the panel is what knows the layers, and a layer
  // chosen as soon as the tree lands is already chosen by the time any tool
  // is picked up. It also covers a layer deleted out from under the
  // selection, which is the same absence arriving a different way.
  useEffect(() => {
    if (!tree) return;
    if (tree.layers.length === 0) {
      if (activeLayer !== null) onActivateLayer(null);
      return;
    }
    const wanted = layerToActivate(activeLayer, tree.layers);
    if (wanted !== null) onActivateLayer(wanted);
  }, [activeLayer, onActivateLayer, tree]);

  // And a selected object's layer is the active one (M83). Clicking an object
  // in this panel already does both in one gesture; selecting one on the *map*
  // moved the selection and left the active layer where it was, so the handles
  // described an object in one layer while the next stroke would land in
  // another. Here rather than beside the selection, for the same reason as
  // above: the panel is what knows which layer holds what.
  useEffect(() => {
    if (!tree) return;
    const wanted = layerForSelection(activeLayer, selection, tree.layers);
    if (wanted !== null) onActivateLayer(wanted);
  }, [activeLayer, onActivateLayer, selection, tree]);

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

  const importZarr = async () => {
    setError(null);
    const path = await pickZarrToImport();
    if (path === null) return;
    run(api.importZarr(path));
  };

  /**
   * Fetches a range of past hours and lands one layer per archive
   * (spec.md 4.10, M38). The one action in the application that reaches the
   * network, and only from this button.
   */
  const importHistory = (choice: HistoryChoice) => {
    const token = project.image_token;
    if (opening.current !== token) return;
    setHistoryOpen(false);
    setError(null);
    void api.importHistory(
        choice.archives,
        choice.startUnixS,
        choice.endUnixS,
        choice.setStartTime,
      ).then((summary) => {
        if (opening.current === token) onChanged(summary);
      }).catch((err: unknown) => {
        if (opening.current === token) reportError(String(err), null, () => importHistory(choice));
      });
  };

  /** Picks an image and lays it under the field (spec.md 4.9, M18). */
  const importImage = async () => {
    setError(null);
    const path = await pickImageToImport();
    if (path === null) return;
    run(api.importImage(path, viewBounds?.() ?? null));
  };

  /**
   * Picks a GIS file and lays it under the field (spec.md 4.11).
   *
   * A georeferenced raster goes to the image import instead: a GeoTIFF is a
   * picture, and the application already places one. The user chooses a
   * file, not a kind — which of the two it is, is the file's own business.
   */
  const importGis = async () => {
    setError(null);
    const path = await pickGisToImport();
    if (path === null) return;
    run(isGeoRaster(path) ? api.importImage(path, viewBounds?.() ?? null) : api.importGis(path));
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

  /**
   * Makes a layer the active one, and drops any selection that is not in it
   * (M47).
   *
   * A selected object in another layer is a selection the panel is no longer
   * showing and the tools no longer act on: the next edit goes to the layer
   * just chosen, while the handles and the inspector still describe something
   * in a layer the user has moved away from. Whichever of the two you follow,
   * the other is wrong, so the selection follows the layer.
   *
   * Clicking an object is the exception — that activates the object's own
   * layer and selects it in the same gesture, so nothing is dropped.
   */
  const activateLayer = (layer: number) => {
    onActivateLayer(layer);
    const mine = new Set(tree?.layers.find((l) => l.id === layer)?.objects.map((o) => o.id) ?? []);
    const kept = selection.filter((id) => mine.has(id));
    if (kept.length !== selection.length) onSelect(kept);
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

  /**
   * The press in progress, and whether it has become a drag.
   *
   * A press that never travels is a click — activating a layer, selecting an
   * object — so the click handlers stand down only once one actually has.
   */
  const press = useRef<{
    drag: Dragging;
    x: number;
    y: number;
    moved: boolean;
    pointerId: number;
    element: HTMLElement;
  } | null>(null);
  const dragged = useRef(false);
  /**
   * What a release does, kept in a ref because it needs the layer tree and
   * the hooks here run before the tree is known to exist.
   */
  const dropRef = useRef<(drag: Dragging, at: DropAt | null) => void>(() => {});

  const startPress = (event: React.PointerEvent<HTMLElement>, drag: Dragging) => {
    if (event.button !== 0 || !event.isPrimary || press.current) return;
    if (event.target instanceof Element && event.target.closest("button, input, select, textarea")) return;
    press.current = {
      drag, x: event.clientX, y: event.clientY, moved: false,
      pointerId: event.pointerId, element: event.currentTarget,
    };
    dragged.current = false;
  };

  useEffect(() => {
    const move = (event: PointerEvent) => {
      const held = press.current;
      if (!held || held.pointerId !== event.pointerId) return;
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
        // Capture only once it is a drag, preserving name clicks and rename
        // double-clicks. Keep receiving the release outside the panel too.
        try {
          held.element.setPointerCapture(held.pointerId);
        } catch {
          // Window listeners still handle the gesture if capture is unavailable.
        }
      }
      event.preventDefault();
      const at = dropAt(event.clientX, event.clientY, held.drag.kind);
      const shown =
        at === null
          ? null
          : at.row === "object"
            ? { id: at.id, where: at.where }
            : { id: at.id, where: held.drag.kind === "object" ? ("into" as DropWhere) : at.where };
      setDropTarget((current) =>
        current?.id === shown?.id && current?.where === shown?.where ? current : shown,
      );
    };
    const finish = (event?: PointerEvent) => {
      const held = press.current;
      if (!held || (event && held.pointerId !== event.pointerId)) return;
      press.current = null;
      if (event?.type === "pointerup" && held.moved) {
        dropRef.current(held.drag, dropAt(event.clientX, event.clientY, held.drag.kind));
      }
      if (held.element.hasPointerCapture?.(held.pointerId)) {
        held.element.releasePointerCapture(held.pointerId);
      }
      setDragging(null);
      setDropTarget(null);
      // The click that follows a drag is not a click on the row it ended on;
      // cleared afterwards so the next press is one again.
      window.setTimeout(() => {
        dragged.current = false;
      }, 0);
    };
    const cancel = () => finish();
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape" && press.current) {
        event.preventDefault();
        finish();
      }
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", finish);
    window.addEventListener("pointercancel", finish);
    window.addEventListener("lostpointercapture", finish);
    window.addEventListener("blur", cancel);
    window.addEventListener("keydown", key);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", finish);
      window.removeEventListener("pointercancel", finish);
      window.removeEventListener("lostpointercapture", finish);
      window.removeEventListener("blur", cancel);
      window.removeEventListener("keydown", key);
      const held = press.current;
      press.current = null;
      if (held?.element.hasPointerCapture?.(held.pointerId)) {
        held.element.releasePointerCapture(held.pointerId);
      }
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
   * (M29). Objects use the same above/below rule among their siblings, or
   * land at the top of a layer when dropped on its header.
   */
  dropRef.current = (drag, at) => {
    if (at === null) return;
    if (drag.kind === "layer") {
      if (at.row !== "layer") return;
      // Resolve identities against the current tree if an edit landed while
      // the pointer was held, rather than moving a different layer by index.
      const from = tree.layers.findIndex((layer) => layer.id === drag.id);
      const target = tree.layers.findIndex((layer) => layer.id === at.id);
      if (from < 0 || target < 0) return;
      const to = layerDropIndex(from, target, at.where !== "below");
      if (from !== to) run(api.moveLayer(from, to));
      return;
    }
    const source = tree.layers.find((layer) => layer.objects.some((object) => object.id === drag.id));
    const destination = tree.layers.find((layer) => layer.id === (at.row === "object" ? at.layer : at.id));
    if (!source || !destination) return;
    const from = source.id === destination.id
      ? source.objects.findIndex((object) => object.id === drag.id)
      : null;
    // The backend removes the object before inserting it. Resolve the target
    // by identity, then account for that removal within the same layer.
    let to = destination.objects.length - Number(from !== null);
    if (at.row === "object") {
      const target = destination.objects.findIndex((object) => object.id === at.id);
      if (target < 0) return;
      to = objectDropIndex(from, target, at.where === "above");
    }
    if (from !== to) run(api.moveObject(drag.id, destination.id, to));
  };

  return (
    <div
      className={dragging ? "layer-panel reordering" : "layer-panel"}
      onDragStartCapture={(event) => event.preventDefault()}
    >
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
          + GRIB
        </button>
        <button
          className="import-grib"
          title="Import wind and currents from a routing Zarr directory as layers"
          onClick={() => void importZarr()}
        >
          + Zarr
        </button>
        <button
          className="import-grib"
          title="Lay a georeferenced image under the field. A GeoTIFF or an image with a world file lands where it says; anything else lands on the view, to be placed by hand."
          onClick={() => void importImage()}
        >
          + image
        </button>
        <button
          className="import-grib"
          title="Lay GIS data under the field: a shapefile, GeoJSON, KML/KMZ, a GPX route or track, or a georeferenced raster. Display only — it makes no wind and reaches no export."
          onClick={() => void importGis()}
        >
          + GIS
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
                  dragging?.kind === "layer" && dragging.id === layer.id ? "reordering" : "",
                  activeLayer === layer.id ? "active" : "",
                  dropTarget?.id === layer.id ? `drop-${dropTarget.where}` : "",
                ]
                  .filter(Boolean)
                  .join(" ")}
                onClick={() => {
                  if (dragged.current) return;
                  activateLayer(layer.id);
                }}
                onMouseDown={noTextSelect}
                onPointerDown={(e) => startPress(e, { kind: "layer", id: layer.id })}
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
                    title={`${layer.name}\nDrag to reorder · double-click to rename`}
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
                  {layer.grib?.history ? "from the archive" : "from the file"}
                </div>
              )}
              {hasField(layer.source) && (
                <SpeedFilter
                  grib={layer.speed_filter ?? layer.grib ?? { speed_min_mps: null, speed_max_mps: null, speed_ceiling_mps: 60 }}
                  treeRevision={treeRevision}
                  onChange={(min, max, gesture) =>
                    run(api.setLayerSpeedRange(layer.id, min, max, gesture)).then(
                      (summary) => summary?.revision ?? null,
                    )
                  }
                />
              )}

              {layer.gis && (
                <GisControls
                  gis={layer.gis}
                  onStyle={(style) => run(api.setGisStyle(layer.id, style))}
                />
              )}

              {layer.image && (
                <ImageControls
                  image={layer.image}
                  onOpacity={(value) => run(api.setImageOpacity(layer.id, value))}
                  onReset={() => run(api.resetImagePlacement(layer.id))}
                  onAlign={() => onAlign(layer.id)}
                  canAlign={canAlign}
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
                        dragging?.kind === "object" && dragging.id === object.id ? "reordering" : "",
                        selection.includes(object.id) ? "selected" : "",
                        object.active_here ? "" : "inactive",
                        dropTarget?.id === object.id ? `drop-${dropTarget.where}` : "",
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
                      title="Drag to change stacking order"
                    >
                      <span className="grip" aria-hidden="true" title="Drag to reorder">
                        ⋮⋮
                      </span>
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
                          title={`${object.tool_label} · steps ${object.start_step}–${object.end_step} · drag to reorder · double-click to rename`}
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
                {/* An imported layer's file is its content — a GRIB's field,
                    a picture, 1,368 GIS features — so only a painted layer
                    with nothing on it is empty. The condition used to name
                    the GRIB alone and called a loaded chart empty. */}
                {layer.objects.length === 0 && layer.source === "painted" && (
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
/**
 * What a GIS layer offers (spec.md 4.11): its colour, its line width and how
 * strongly its areas are filled.
 *
 * Display only, so there is no speed filter and no step bar — it makes no
 * field. What it says instead is what the file turned out to hold, which is
 * the question a survey laid under the map actually raises: did it read, and
 * how much is in it.
 */
function GisControls({
  gis,
  onStyle,
}: {
  gis: GisLayerView;
  onStyle: (style: { colour?: string; widthPx?: number; fillOpacity?: number }) => void;
}) {
  if (!gis.loaded) {
    return (
      <div className="grib-info">
        <span className="grib-missing" title={`${gis.path}\n${gis.error ?? ""}`}>
          {gis.error ?? "The file could not be read."}
        </span>
      </div>
    );
  }
  return (
    <div className="grib-info">
      <label className="layer-filter-row" title="Line and point colour.">
        <span className="layer-filter-label">Colour</span>
        <input
          type="color"
          value={gis.colour}
          onChange={(event) => onStyle({ colour: event.target.value })}
        />
      </label>
      <label className="layer-filter-row" title="Line width, in screen pixels.">
        <span className="layer-filter-label">Width</span>
        <input
          type="range"
          min={2}
          max={60}
          value={Math.round(gis.width_px * 10)}
          onChange={(event) => onStyle({ widthPx: Number(event.target.value) / 10 })}
        />
      </label>
      {/*
        The fill is offered only when the file holds an area to fill. A route,
        a track or a coastline is all lines, and a slider that changes nothing
        on the map is a control that lies about what it does.
      */}
      {gis.areas > 0 && (
        <label
          className="layer-filter-row"
          title="How strongly areas are filled. At nothing, only their outlines are drawn — which is what a boundary over a field usually wants. Lines and marks are never filled."
        >
          <span className="layer-filter-label">Fill</span>
          <input
            type="range"
            min={0}
            max={100}
            value={Math.round(gis.fill_opacity * 100)}
            onChange={(event) => onStyle({ fillOpacity: Number(event.target.value) / 100 })}
          />
        </label>
      )}
      <span className="layer-filter-summary muted" title={gis.path}>
        {gis.features.toLocaleString()} features · display only
      </span>
    </div>
  );
}

/** Exported for direct testing (review finding 3(b): the reset button's visibility). */
export function ImageControls({
  image,
  onOpacity,
  onReset,
  onAlign,
  canAlign,
}: {
  image: ImageLayerView;
  onOpacity: (opacity: number) => void;
  onReset: () => void;
  /** Arms the map's alignment mode for this layer (Task 7). */
  onAlign: () => void;
  /**
   * Whether the projection on show can be aligned on: the cylindrical maps
   * only. Off the button is not offered at all — the globe and the general
   * presets curve the picture's own edge across the screen, and clicking a
   * place in a picture whose true edge you cannot see is a georeference
   * nobody can aim.
   */
  canAlign: boolean;
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
    <div className="image-controls" title="Hand tool: drag a corner to resize proportionally, a side to stretch, or the picture to move it.">
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
        {image.georeferenced && " · georeferenced"}
      </span>
      {canAlign && (
        <button
          className="align"
          onClick={onAlign}
          aria-label="Align by pointing"
          title="Align by pointing: click a place in the picture, then the same place on the map. Repeat as often as you like, then press Enter. Escape cancels; Backspace drops the last pair."
        >
          <AlignIcon />
        </button>
      )}
      {(image.georeferenced || image.warped || image.control_points.length > 0) && (
        <button
          onClick={onReset}
          title={
            image.georeferenced
              ? "Put the image back where its own file says it goes, and clear every control point"
              : "Clear every control point and put the image back where it was before any were placed"
          }
        >
          Reset place
        </button>
      )}
    </div>
  );
}

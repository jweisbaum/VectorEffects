/**
 * The typed command surface.
 *
 * Types under `./generated` are produced from the Rust definitions by
 * `npm run bindings` and must never be hand-edited. This module is the only
 * place `invoke` is called, so every IPC failure is normalised into one type.
 */
import { invoke } from "@tauri-apps/api/core";

import type { AppErrorPayload } from "./generated/AppErrorPayload";
import type { AppInfo } from "./generated/AppInfo";
import type { FieldSample } from "./generated/FieldSample";
import type { NewProjectRequest } from "./generated/NewProjectRequest";
import type { BrushStroke } from "./generated/BrushStroke";
import type { NewObject } from "./generated/NewObject";
import type { ObjectTracks } from "./generated/ObjectTracks";
import type { InterpolationView } from "./generated/InterpolationView";
import type { OperatorOutline } from "./generated/OperatorOutline";
import type { ShrinkImpact } from "./generated/ShrinkImpact";
import type { TrackSamples } from "./generated/TrackSamples";
import type { TileAddress } from "./generated/TileAddress";
import type { TimelineReadiness } from "./generated/TimelineReadiness";
import type { Tool } from "./generated/Tool";
import type { ToolSchema } from "./generated/ToolSchema";
import type { DocumentTree } from "./generated/DocumentTree";
import type { ClipboardState } from "./generated/ClipboardState";
import type { HistoryView } from "./generated/HistoryView";
import type { PropertyValue } from "./generated/PropertyValue";
import type { PropertyView } from "./generated/PropertyView";
import type { ExportEstimate } from "./generated/ExportEstimate";
import type { ExportRequest } from "./generated/ExportRequest";
import type { ExportResult } from "./generated/ExportResult";
import type { ProjectSummary } from "./generated/ProjectSummary";
import type { RecentProject } from "./generated/RecentProject";
import type { SelectionTransform } from "./generated/SelectionTransform";
import type { TransformPreview } from "./generated/TransformPreview";
import type { TransformKind } from "./generated/TransformKind";

/** An error raised by a Rust command, carrying its machine-readable kind. */
export class IpcError extends Error {
  readonly kind: string;

  constructor(payload: AppErrorPayload) {
    super(payload.message);
    this.name = "IpcError";
    this.kind = payload.kind;
  }
}

export function isErrorPayload(value: unknown): value is AppErrorPayload {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as AppErrorPayload).kind === "string" &&
    typeof (value as AppErrorPayload).message === "string"
  );
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (raw) {
    if (isErrorPayload(raw)) throw new IpcError(raw);
    // A command that panicked, or a Tauri-level failure, arrives as a bare string.
    throw new IpcError({ kind: "unknown", message: String(raw) });
  }
}

/** Every Rust command reachable from the frontend. */
export const api = {
  /** Build and installation facts for the About panel. */
  appInfo: () => call<AppInfo>("app_info"),

  /** The bundled basemap asset, as raw bytes. */
  basemap: async (): Promise<ArrayBuffer> => {
    const bytes = await call<ArrayBuffer | number[]>("basemap");
    // Raw responses arrive as an ArrayBuffer; older shapes as a number array.
    return bytes instanceof ArrayBuffer ? bytes : new Uint8Array(bytes).buffer;
  },

  /** URL prefix for `ve-tile://` requests. */
  tileBaseUrl: () => call<string>("tile_base_url"),

  /** Samples the field at a position, for the cursor readout. */
  sampleField: (lon: number, lat: number, step: number) =>
    call<FieldSample>("sample_field", { lon, lat, step }),

  /**
   * Writes a rendered frame to the log directory. Development only.
   *
   * Base64 rather than a byte array: a megabyte of pixels as a JSON array of a
   * million numbers is pathologically slow to serialise.
   */
  saveDebugCapture: (name: string, pngBase64: string) =>
    call<string>("save_debug_capture", { name, pngBase64 }),

  /** Records a frontend message in the application log. */
  frontendLog: (level: "info" | "warn" | "error", message: string) =>
    call<void>("frontend_log", { level, message }),

  // --- Project lifecycle ---

  /** Creates a project and makes it the open one. */
  newProject: (request: NewProjectRequest, discardUnsaved = false) =>
    call<ProjectSummary>("new_project", { request, discardUnsaved }),

  /** Opens a project from disk. */
  openProject: (path: string, discardUnsaved = false) =>
    call<ProjectSummary>("open_project", { path, discardUnsaved }),

  /** Creates a project shaped by a GRIB2 file and imports the file into it (spec 4.8). */
  newProjectFromGrib: (path: string, discardUnsaved = false) =>
    call<ProjectSummary>("new_project_from_grib", { path, discardUnsaved }),

  /** Saves the open project to its existing path. */
  saveProject: () => call<ProjectSummary>("save_project"),

  /** Saves the open project to a new path. */
  saveProjectAs: (path: string) => call<ProjectSummary>("save_project_as", { path }),

  /** Closes the open project without saving. */
  closeProject: () => call<void>("close_project"),

  /** The open project, or null. */
  currentProject: () => call<ProjectSummary | null>("current_project"),

  /** Recently opened projects, newest first. */
  recentProjects: () => call<RecentProject[]>("recent_projects"),

  // --- Editing ---

  /**
   * Every tool, with its options, its gesture and whether it has a hover
   * indicator (spec.md 6.2).
   *
   * Fetched rather than written out in the frontend: the option bar is the same
   * question the inspector asks, and one answer keeps them from disagreeing.
   */
  toolPalette: () => call<ToolSchema[]>("tool_palette"),

  /** Adds an object drawn with any tool. */
  createObject: (object: NewObject) => call<ProjectSummary>("create_object", { object }),

  /** Adds a painted stroke to the topmost layer. */
  addBrushStroke: (stroke: BrushStroke) =>
    call<ProjectSummary>("add_brush_stroke", { stroke }),

  /** Reverses the most recent change. */
  undo: () => call<ProjectSummary>("undo"),

  /** Reapplies the most recently undone change. */
  redo: () => call<ProjectSummary>("redo"),

  // --- Export ---

  /** Approximate size of the export, before committing to it. */
  exportEstimate: () => call<ExportEstimate>("export_estimate"),

  /** Writes the open project to a GRIB2 file. */
  exportGrib: (request: ExportRequest) => call<ExportResult>("export_grib", { request }),

  /** Asks a running export to stop. */
  cancelExport: () => call<void>("cancel_export"),

  // --- Document structure ---

  /** Layers and objects, bottom of the stack first. */
  documentTree: (step: number) => call<DocumentTree>("document_tree", { step }),

  /** Every property of one object, in schema order. */
  objectProperties: (object: number, step: number) =>
    call<PropertyView[]>("object_properties", { object, step }),

  /**
   * Changes one property of one object.
   *
   * `gesture` groups consecutive changes into a single undo entry; pass the
   * same key throughout a drag and call `endGesture` on release.
   */
  setObjectProperty: (
    object: number,
    property: string,
    value: PropertyValue,
    step: number,
    autoKey: boolean,
    gesture?: string,
  ) =>
    call<ProjectSummary>("set_object_property", {
      object,
      property,
      value,
      step,
      autoKey,
      gesture: gesture ?? null,
    }),

  // --- Animation (spec.md 9) ---

  /** An object's tracks: base, keys and allowed easings per property. */
  objectTracks: (object: number, step: number) =>
    call<ObjectTracks>("object_tracks", { object, step }),

  /**
   * One property's value at every step, for the track's value graph.
   *
   * Sampled by the model, not interpolated here: the easings, the shortest-arc
   * angle and the great-circle position are the domain's (spec.md 9.3). Does
   * not depend on the current step, only on the revision.
   */
  trackSamples: (object: number, property: string) =>
    call<TrackSamples>("track_samples", { object, property }),

  /** Adds or replaces a key; without a value, pins what the step shows. */
  setKeyframe: (object: number, property: string, step: number, value?: PropertyValue) =>
    call<ProjectSummary>("set_keyframe", { object, property, step, value: value ?? null }),

  removeKeyframe: (object: number, property: string, step: number) =>
    call<ProjectSummary>("remove_keyframe", { object, property, step }),

  moveKeyframe: (object: number, property: string, from: number, to: number, gesture?: string) =>
    call<ProjectSummary>("move_keyframe", {
      object,
      property,
      from,
      to,
      gesture: gesture ?? null,
    }),

  setInterpolation: (object: number, property: string, step: number, interp: InterpolationView) =>
    call<ProjectSummary>("set_interpolation", { object, property, step, interp }),

  /** What shrinking the timeline would delete (spec.md 4.1). */
  stepCountImpact: (stepCount: number) =>
    call<ShrinkImpact>("step_count_impact", { stepCount }),

  setStepCount: (stepCount: number) => call<ProjectSummary>("set_step_count", { stepCount }),

  setStartTime: (startUnixS: number | null) =>
    call<ProjectSummary>("set_start_time", { startUnixS }),

  /** Queues the viewport's tiles for the steps around the playhead (spec.md 9.5). */
  renderAhead: (current: number, tiles: TileAddress[]) =>
    call<void>("render_ahead", { current, tiles }),

  /** How ready every step is, for the viewport. */
  frameReadiness: (tiles: TileAddress[]) =>
    call<TimelineReadiness>("frame_readiness", { tiles }),

  endGesture: () => call<void>("end_gesture"),

  /** The selected object's transform, for the on-map handles. */
  /** Where a selection's handles go. One object or many; the pivot is theirs. */
  selectionTransform: (objects: number[], step: number) =>
    call<SelectionTransform | null>("selection_transform", { objects, step }),

  /**
   * The footprints the map may need to draw at a step.
   *
   * Objects drawn with `tool`, for the edge highlighted under the pointer, and
   * the objects in `objects`, for the selection outline (spec.md 6.1, 6.2,
   * 6.3). Bounded by the tool in hand rather than by the size of the project.
   * Fetched per revision, step, tool and selection, never per frame: the hit
   * test that finds the edge under the pointer is the map's own.
   */
  objectOutlines: (step: number, tool: Tool | null, objects: number[]) =>
    call<OperatorOutline[]>("object_outlines", { step, tool, objects }),
  /**
   * Captures the drag baseline. `lon`/`lat` is where the pointer went down, so
   * rotation and scale are measured from there rather than snapping.
   */
  beginTransform: (
    objects: number[],
    step: number,
    kind: TransformKind,
    lon: number,
    lat: number,
    autoKey: boolean,
  ) =>
    call<SelectionTransform | null>("begin_transform", {
      objects,
      step,
      kind,
      lon,
      lat,
      autoKey,
    }),
  /**
   * Where the drag would leave the selection, without writing anything.
   *
   * What the pointer follows. Applying the drag instead bumps the revision, and
   * the revision addresses every tile, so each pointer report would invalidate
   * the whole visible field (spec.md 8.2).
   */
  previewTransform: (lon: number, lat: number) =>
    call<TransformPreview | null>("preview_transform", { lon, lat }),
  /** Applies the drag. Idempotent: the same pointer position gives the same result. */
  dragTransform: (lon: number, lat: number) =>
    call<ProjectSummary>("drag_transform", { lon, lat }),
  /** Objects meeting a lat/lon rectangle. `layer` scopes it; null reaches across. */
  objectsInRegion: (
    west: number,
    south: number,
    east: number,
    north: number,
    step: number,
    layer: number | null,
  ) => call<number[]>("objects_in_region", { west, south, east, north, step, layer }),

  addLayer: (name: string) => call<ProjectSummary>("add_layer", { name }),
  /** Imports a GRIB2 file as one layer per field kind it holds (spec 4.8). */
  importGrib: (path: string) => call<ProjectSummary>("import_grib", { path }),
  removeLayer: (layer: number) => call<ProjectSummary>("remove_layer", { layer }),
  renameLayer: (layer: number, name: string) =>
    call<ProjectSummary>("rename_layer", { layer, name }),
  /**
   * Which speeds an imported field keeps, in m/s. `null` clears the filter.
   *
   * m/s below the boundary and knots above it, like every other speed
   * (spec.md 3.4).
   */
  setLayerSpeedRange: (layer: number, minMps: number | null, maxMps: number | null) =>
    call<ProjectSummary>("set_layer_speed_range", { layer, minMps, maxMps }),

  setLayerVisible: (layer: number, visible: boolean) =>
    call<ProjectSummary>("set_layer_visible", { layer, visible }),
  setLayerLocked: (layer: number, locked: boolean) =>
    call<ProjectSummary>("set_layer_locked", { layer, locked }),
  moveLayer: (from: number, to: number) => call<ProjectSummary>("move_layer", { from, to }),

  renameObject: (object: number, name: string) =>
    call<ProjectSummary>("rename_object", { object, name }),
  removeObject: (object: number) => call<ProjectSummary>("remove_object", { object }),
  moveObject: (object: number, layer: number, index: number) =>
    call<ProjectSummary>("move_object", { object, layer, index }),
  duplicateObject: (object: number) => call<ProjectSummary>("duplicate_object", { object }),
  setActiveRange: (object: number, start: number, end: number) =>
    call<ProjectSummary>("set_active_range", { object, start, end }),

  /** The topmost selectable object covering a position. */
  objectAt: (lon: number, lat: number, step: number) =>
    call<number | null>("object_at", { lon, lat, step }),

  // --- Clipboard and history ---

  copyObjects: (objects: number[], step: number) =>
    call<ClipboardState>("copy_objects", { objects, step }),
  cutObjects: (objects: number[], step: number) =>
    call<ProjectSummary>("cut_objects", { objects, step }),
  /** `absoluteTiming` keeps the original step numbers instead of moving them. */
  pasteObjects: (layer: number | null, step: number, absoluteTiming: boolean) =>
    call<ProjectSummary>("paste_objects", { layer, step, absoluteTiming }),
  clipboardState: () => call<ClipboardState>("clipboard_state"),

  historyView: () => call<HistoryView>("history_view"),
  jumpToHistory: (target: number) => call<ProjectSummary>("jump_to_history", { target }),
};

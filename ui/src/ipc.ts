/**
 * The typed command surface.
 *
 * Types under `./generated` are produced from the Rust definitions by
 * `npm run bindings` and must never be hand-edited. This module is the only
 * place `invoke` is called, so every IPC failure is normalised into one type.
 */
import { invoke } from "@tauri-apps/api/core";

import { beginBusy } from "./busy";

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
import type { ClipboardKind } from "./generated/ClipboardKind";
import type { AutosaveMode } from "./generated/AutosaveMode";
import type { AppSettings } from "./generated/AppSettings";
import type { CaptureMode } from "./generated/CaptureMode";
import type { CaptureState } from "./generated/CaptureState";
import type { MacroLibrary } from "./generated/MacroLibrary";
import type { Shortcut } from "./generated/Shortcut";
import type { FrameClipboardState } from "./generated/FrameClipboardState";
import type { RegionShape } from "./generated/RegionShape";
import type { HistoryView } from "./generated/HistoryView";
import type { PropertyValue } from "./generated/PropertyValue";
import type { PropertyView } from "./generated/PropertyView";
import type { ExportEstimate } from "./generated/ExportEstimate";
import type { ExportRequest } from "./generated/ExportRequest";
import type { ExportResult } from "./generated/ExportResult";
import type { Autosave } from "./generated/Autosave";
import type { MeasurementKind } from "./generated/MeasurementKind";
import type { MeasurementView } from "./generated/MeasurementView";
import type { NewMeasurement } from "./generated/NewMeasurement";
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

/**
 * The commands that can take seconds, and what the status bar's spinner says
 * while each runs. Everything else answers within a frame and is not shown:
 * a spinner that spins for every hit test spins forever.
 */
const LONG_RUNNING: Readonly<Record<string, string>> = {
  import_grib: "Importing GRIB",
  new_project_from_grib: "Opening GRIB",
  import_image: "Importing image",
  open_project: "Opening project",
  recover_autosave: "Recovering project",
  save_project: "Saving",
  save_project_as: "Saving",
  export_grib: "Exporting GRIB",
  capture_region: "Copying the field",
  paste_capture: "Pasting the field",
  preview_capture: "Baking the macro",
  finish_capture: "Saving the macro",
  insert_macro: "Inserting the macro",
  set_step_count: "Changing the duration",
};

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const label = LONG_RUNNING[command];
  const done = label === undefined ? null : beginBusy(label);
  try {
    return await invoke<T>(command, args);
  } catch (raw) {
    if (isErrorPayload(raw)) throw new IpcError(raw);
    // A command that panicked, or a Tauri-level failure, arrives as a bare string.
    throw new IpcError({ kind: "unknown", message: String(raw) });
  } finally {
    done?.();
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
  /**
   * Puts the open project down. Refused when it is dirty unless
   * `discardUnsaved`, the same guard `newProject` and `openProject` use.
   */
  closeProject: (discardUnsaved: boolean) =>
    call<void>("close_project", { discardUnsaved }),

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
  /** The size and resolution an export would have at a width (spec.md 12.3). */
  exportEstimate: (bits?: number) => call<ExportEstimate>("export_estimate", { bits: bits ?? null }),

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
  /** Renames the project: a document write, undoable (M25). */
  renameProject: (name: string) => call<ProjectSummary>("rename_project", { name }),
  renameLayer: (layer: number, name: string) =>
    call<ProjectSummary>("rename_layer", { layer, name }),
  /**
   * Which speeds an imported field keeps, in m/s. `null` clears the filter.
   *
   * m/s below the boundary and knots above it, like every other speed
   * (spec.md 3.4).
   */
  /**
   * Which speeds an imported field keeps. `gesture` is the coalescing key a
   * slider drag sends on every tick so the drag is one undo; a typed value
   * sends null.
   */
  setLayerSpeedRange: (
    layer: number,
    minMps: number | null,
    maxMps: number | null,
    gesture: string | null = null,
  ) => call<ProjectSummary>("set_layer_speed_range", { layer, minMps, maxMps, gesture }),

  setLayerVisible: (layer: number, visible: boolean) =>
    call<ProjectSummary>("set_layer_visible", { layer, visible }),
  setLayerLocked: (layer: number, locked: boolean) =>
    call<ProjectSummary>("set_layer_locked", { layer, locked }),
  moveLayer: (from: number, to: number) => call<ProjectSummary>("move_layer", { from, to }),

  renameObject: (object: number, name: string) =>
    call<ProjectSummary>("rename_object", { object, name }),
  removeObject: (object: number) => call<ProjectSummary>("remove_object", { object }),
  /** Deletes a whole selection as one history entry. */
  removeObjects: (objects: number[]) => call<ProjectSummary>("remove_objects", { objects }),
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
  /**
   * `still` pastes the copy as it was at the step it was copied from, with no
   * animation at all (spec.md 8.5, M27); it wins over `absoluteTiming`.
   */
  pasteObjects: (layer: number | null, step: number, absoluteTiming: boolean, still = false) =>
    call<ProjectSummary>("paste_objects", { layer, step, absoluteTiming, still }),

  /**
   * Copies a run of an imported layer's steps (spec.md 4.8, M20). The layer
   * travels with them: the paste goes back to it whatever layer is active.
   */
  copyGribFrames: (layer: number, steps: number[]) =>
    call<FrameClipboardState>("copy_grib_frames", { layer, steps }),
  /** Pastes the copied frames, the first landing on `at`. */
  pasteGribFrames: (at: number) => call<ProjectSummary>("paste_grib_frames", { at }),
  /**
   * `Delete` on a run of steps: a pasted frame goes back to the file's own
   * message, and the file's own message is hidden.
   */
  deleteGribFrames: (layer: number, steps: number[]) =>
    call<ProjectSummary>("delete_grib_frames", { layer, steps }),
  /**
   * Puts one of an object's own movements into the field it paints, or takes
   * it out again (spec.md 9.3, M13). One track at a time: the checkbox is on
   * the track row because the track is where the movement is.
   */
  setMotion: (object: number, property: string, on: boolean) =>
    call<ProjectSummary>("set_motion", { object, property, on }),

  /**
   * Makes one object's position or rotation follow another's, or clears the
   * link when `primary` is null (spec.md 9.3, M13). Linking never moves
   * anything: the offset is read from where the two objects already are.
   */
  setFollow: (object: number, property: string, primary: number | null, step: number) =>
    call<ProjectSummary>("set_follow", { object, property, primary, step }),

  /**
   * Captures the visible composite inside a region (spec.md 8.5, M14).
   * Evaluated on the CPU, like an export: a capture is a value the user keeps
   * rather than a frame they are looking at.
   */
  captureRegion: (region: RegionShape, step: number) =>
    call<CaptureState>("capture_region", { region, step }),
  /**
   * Pastes the captured field as a patch, at a position or where it came
   * from, into `layer` (null: the top of the stack). Its run of frames begins
   * at `step` (D65).
   */
  pasteCapture: (
    lon: number | null,
    lat: number | null,
    step: number,
    layer: number | null,
    still = false,
  ) => call<ProjectSummary>("paste_capture", { lon, lat, step, layer, still }),
  /** Which of the two things a paste would put down: objects, or a captured field. */
  clipboardKind: () => call<ClipboardKind>("clipboard_kind"),
  /** The application's settings: shortcuts, display defaults, macros (M15). */
  appSettings: () => call<AppSettings>("app_settings", {}),
  /** Rebinds one shortcut. A collision or a reserved key is refused. */
  setShortcut: (binding: Shortcut) => call<AppSettings>("set_shortcut", { binding }),
  /** Puts every shortcut back to its default. */
  resetShortcuts: () => call<AppSettings>("reset_shortcuts", {}),
  /** The colour-ramp top a *new* project of each kind gets, in knots. */
  setDefaultScales: (windKnots: number, currentKnots: number) =>
    call<AppSettings>("set_default_scales", { windKnots, currentKnots }),
  // --- Crash recovery (spec.md 4.2, M10) ---
  /** Snapshots of unsaved work left by an unclean shutdown, newest first. */
  autosaves: () => call<Autosave[]>("autosaves"),
  /** Opens a snapshot as the project it was taken from: dirty, at its old path. */
  recoverAutosave: (id: number, discardUnsaved: boolean) =>
    call<ProjectSummary>("recover_autosave", { id, discardUnsaved }),
  /** Drops a snapshot the user does not want back. */
  discardAutosave: (id: number) => call<Autosave[]>("discard_autosave", { id }),

  // --- Image layers (spec.md 4.9, M18) ---
  /**
   * Adds a georeferenced image under the field.
   *
   * `view` is the visible map as `[west, north, east, south]`, used only when
   * the file says nothing about where it goes — the image then lands filling
   * the view, where its control points can be reached.
   */
  importImage: (path: string, view: [number, number, number, number] | null) =>
    call<ProjectSummary>("import_image", { path, view }),
  /** Moves an image by its three control points. */
  setImageCorners: (
    layer: number,
    topLeft: [number, number],
    topRight: [number, number],
    bottomLeft: [number, number],
    gesture: string | null,
  ) =>
    call<ProjectSummary>("set_image_corners", {
      layer,
      topLeft,
      topRight,
      bottomLeft,
      gesture,
    }),
  /** Sets how strongly an image shows. */
  setImageOpacity: (layer: number, opacity: number) =>
    call<ProjectSummary>("set_image_opacity", { layer, opacity }),
  /** Puts an image back where its own file says it goes. */
  resetImagePlacement: (layer: number) =>
    call<ProjectSummary>("reset_image_placement", { layer }),

  // --- Measurements (spec.md 10, M8) ---
  /** Every measurement laid over the map, drawn and labelled by the backend. */
  measurements: () => call<MeasurementView[]>("measurements"),
  /** Places one. */
  addMeasurement: (measurement: NewMeasurement) =>
    call<MeasurementView[]>("add_measurement", { measurement }),
  /** Moves one placed point. Coalesces, so a drag is one undo. */
  moveMeasurementHandle: (id: number, index: number, at: [number, number]) =>
    call<MeasurementView[]>("move_measurement_handle", { id, index, at }),
  /** Adds a leg to a chain. */
  extendMeasurement: (id: number, at: [number, number]) =>
    call<MeasurementView[]>("extend_measurement", { id, at }),
  /** Sets a ring set's spacing and count. */
  setMeasurementRings: (id: number, intervalKm: number, count: number) =>
    call<MeasurementView[]>("set_measurement_rings", { id, intervalKm, count }),
  /** Removes one. */
  removeMeasurement: (id: number) =>
    call<MeasurementView[]>("remove_measurement", { id }),
  /** Clears one tool's measurements, or every one of them. */
  clearMeasurements: (kind: MeasurementKind | null) =>
    call<MeasurementView[]>("clear_measurements", { kind }),

  /** How the map lays the world out (M11). A view preference only. */
  setProjection: (projection: string) =>
    call<AppSettings>("set_projection", { projection }),
  /** Whether the colour ramp follows the field in view (spec.md 5.3, M27). */
  setAutoScale: (on: boolean) => call<AppSettings>("set_auto_scale", { on }),
  /** What the autosave thread does with unsaved work (D70). */
  setAutosaveMode: (mode: AutosaveMode) => call<AppSettings>("set_autosave_mode", { mode }),
  /** Where the macro library lives. */
  setMacroDirectory: (directory: string) =>
    call<AppSettings>("set_macro_directory", { directory }),
  /** The open project's colour scale: a document write, undoable. */
  setColourScale: (maxKnots: number) =>
    call<ProjectSummary>("set_colour_scale", { maxKnots }),

  /** The macro library (spec.md 8.7, M16). */
  macroLibrary: () => call<MacroLibrary>("macro_library", {}),
  /** Deletes one macro, or every one. Inserted macros keep working (D52). */
  deleteMacros: (id: string | null) => call<MacroLibrary>("delete_macros", { id }),
  /** Begins a capture over a region. Every document write is then refused. */
  startCapture: (region: RegionShape, step: number, recordMovement: boolean) =>
    call<CaptureMode>("start_capture", { region, step, recordMovement }),
  /** Moves the capture's region at one step. Each frame holds its own place. */
  placeCapture: (step: number, lon: number, lat: number) =>
    call<CaptureMode>("place_capture", { step, lon, lat }),
  /** Keys the region at a step where it stands: visiting a step records it (D72). */
  visitCapture: (step: number) => call<CaptureMode>("visit_capture", { step }),
  /** Removes the region's position key at a step; the gap interpolates (D72). */
  unplaceCapture: (step: number) => call<CaptureMode>("unplace_capture", { step }),
  /** Bakes the capture into the session and shows it alone on the map (D71). */
  previewCapture: (lastStep: number) => call<CaptureMode>("preview_capture", { lastStep }),
  /** Moves the preview's stamp. */
  stampPreview: (lon: number, lat: number) => call<CaptureMode>("stamp_preview", { lon, lat }),
  /** Back from the preview to recording, keys intact. */
  editCapture: () => call<CaptureMode>("edit_capture", {}),
  /** Abandons a capture, writing nothing. */
  cancelCapture: () => call<CaptureMode>("cancel_capture", {}),
  /**
   * Whether a capture is running, and where its region sits at `step`.
   *
   * The step is optional because two callers want different things: one asks
   * whether a capture exists at all, the other is scrubbing and needs the
   * position this frame holds.
   */
  captureMode: (step?: number) => call<CaptureMode>("capture_mode", { step: step ?? null }),
  /** Bakes the capture under a name. Creates no object. */
  finishCapture: (name: string, lastStep: number) =>
    call<MacroLibrary>("finish_capture", { name, lastStep }),
  /**
   * Inserts a library macro as an object, centred on a position, into
   * `layer` (null: the top of the stack). Its frames run from `step`.
   */
  insertMacro: (id: string, lon: number, lat: number, step: number, layer: number | null) =>
    call<ProjectSummary>("insert_macro", { id, lon, lat, step, layer }),

  /** What the capture clipboard holds. */
  captureState: () => call<CaptureState>("capture_state", {}),

  /** What the frame clipboard holds. */
  frameClipboardState: () => call<FrameClipboardState>("frame_clipboard_state", {}),
  clipboardState: () => call<ClipboardState>("clipboard_state"),

  historyView: () => call<HistoryView>("history_view"),
  jumpToHistory: (target: number) => call<ProjectSummary>("jump_to_history", { target }),
};

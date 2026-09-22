import ProjectionPicker from "./ProjectionPicker";
import ToolSelect from "../ToolSelect";
import { hitShapePoint, movedShapePoint, type ShapePointHit } from "./shapeEditing";
import type { ShapeControls } from "../generated/ShapeControls";
import { useUnits, type DisplayUnits } from "../settings/units";
import {
  type Ref,
  useCallback,
  useEffect,
  useImperativeHandle,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";

import { api } from "../ipc";
import { listen } from "@tauri-apps/api/event";
import NumberField from "../NumberField";
import type { Gesture } from "../generated/Gesture";
import type { PathPoint } from "../generated/PathPoint";
import type { ProjectSummary } from "../generated/ProjectSummary";
import type { OperatorOutline } from "../generated/OperatorOutline";
import type { ObjectOutline } from "../generated/ObjectOutline";
import type { TileAddress } from "../generated/TileAddress";
import type { Tool } from "../generated/Tool";
import type { AppSettings } from "../generated/AppSettings";
import type { ChartStatus } from "../generated/ChartStatus";
import { BackdropCache } from "./backdrops";
import type { BrushShape } from "../generated/BrushShape";
import type { CaptureMode } from "../generated/CaptureMode";
import type { CaptureRequest } from "../generated/CaptureRequest";
import type { MacroLibrary } from "../generated/MacroLibrary";
import type { LayerNode } from "../generated/LayerNode";
import type { MacroOutline } from "../generated/MacroOutline";
import type { ShortcutAction } from "../generated/ShortcutAction";
import type { ToolSchema } from "../generated/ToolSchema";
import { actionFor, chordOf, toolChord } from "../settings/bindings";
import { cursorFor, pointerPriorityFor } from "./cursor";
import { createPortal } from "react-dom";
import { reportError, setActivity, setHint } from "../hint";
import { IconSvg, REDO_ICON, UNDO_ICON } from "./ToolIcon";
import type { PositionPick } from "../picking";
import type { SelectionTransform } from "../generated/SelectionTransform";
import type { TransformPreview } from "../generated/TransformPreview";
import type { TransformKind } from "../generated/TransformKind";
import { displayDirection, knotsFromMps, mpsFromKnots } from "../project/format";
import {
  type Camera,
  type Viewport,
  clampCamera,
  cameraForProjection,
  validGeo,
  minPxPerDeg,
  normalizeLon,
  panBy,
  projectionFor,
  visibleBounds,
  project as toScreen,
  unproject,
  visibleTiles,
  zoomAbout,
  onProjectedMeshReady,
  projectedMeshWanted,
  refreshProjectedMesh,
} from "./camera";
import {
  addFootprint,
  buildFootprintPath,
  buildStrokePath,
  footprintOfOutline,
  extendStrokePath,
  footprintHead,
  footprintRadii,
  freshSweptPath,
  type SweptPathProgress,
} from "./footprint";
import { destination, distanceM } from "./geo";
import { type Placement as GhostPlacement, ghostMatrix, ghostRegion } from "./ghost";
import {
  extendLatticeUnderStroke,
  freshLattice,
  glyphGeometry,
  type LatticeProgress,
  latticeUnder,
} from "./glyph";
import { GLYPH_SUBDIVISIONS, spacedGlyphs } from "./glyphPlacement";
import { DEFAULT_GLYPHS, glyphDisplayLayout, glyphOpacity } from "./glyphAppearance";
import {
  DEFAULT_PROJECTION,
  type ProjectionId,
  projectionOf,
} from "./projection";
import {
  drawMeasurements,
  HANDLE_REACH_CSS,
  type HandlePick,
  handleUnder,
  MEASURE_LABELS,
  pointsNeeded,
  projectPath,
} from "./measure";
import type { MeasurementView } from "../generated/MeasurementView";
import type { MeasurementKind } from "../generated/MeasurementKind";
import type { NewMeasurement } from "../generated/NewMeasurement";
import { showsHoverIndicator, showsMagnifier } from "./hover";
import { KIND_LABELS, KINDS, type FieldKindName, kindOf } from "../kind";
import { trackKeyframes } from "./macroTrack";
import { legendKnots } from "./legend";
import { rampColour, rampCss, rampStops } from "./ramp";
import { parseBasemap } from "./format";
import { marqueeBounds } from "./marquee";
import {
  overlayPlan,
  previewHasLanded,
  SETTLE_TIMEOUT_MS,
  type HeldPreview,
  type Settling,
  type FieldPreview,
} from "./preview";
import {
  finished,
  gestureLatitude,
  hoverGesture,
  type InProgress,
  isComplete,
  press,
  release,
  shapeNode,
} from "./gesture";
import ToolIcon from "./ToolIcon";
import ToolOptions, { type ToolPick } from "./ToolOptions";
import {
  type ActiveTool,
  CAPTURE,
  ERASE,
  INSERT,
  MEASURE,
  operatorOf,
  defaultState,
  drawsObjects,
  liveOptions,
  footprintOf,
  gestureKind,
  HAND,
  newObject,
  SELECT,
  positionOf,
  previewField,
  eraserStamp,
  sampled,
  spaceFor,
  type ToolState,
} from "./tools";
import {
  type Region,
  editsRegion,
  recentred,
  regionAnchor,
  regionContains,
  regionGesture,
  regionShape,
  type RegionMode,
  regionFromDrag,
  regionFromLasso,
  regionOfView,
  regionRing,
  wholeMap,
} from "./region";
import { ImageCache } from "./images";
import { imageUnder, imageResizeHandles, imageResizeHandleAt } from "./place";
import { imageGesture } from "./imageGesture";
import { MAX_CONTROL_POINTS, createAlignStore, imagePixelAt, outlinePath, pictureToMap, type AlignStore } from "./align";
import { playbackPreparation } from "../timeline/metrics";
import type { ImageLayerView } from "../generated/ImageLayerView";
import { MapRenderer, type OperatorPreview, type RenderState } from "./renderer";
import { uniqueTiles } from "../timeline/playback";
import { preparationTargets, type PlaybackMap, type PreparationRequest } from "../timeline/preparation";
import { playbackPresented, playbackTime } from "../timeline/metrics";
import { TileCache } from "./tiles";
import { beneathToken, frameToken, onlyToken, parseFrameToken } from "./frameToken";
import { focusMapForGesture, releaseFocus } from "./focus";
import { imagesOverField } from "./imageStack";
import {
  aimedOffTheMap,
  layerTakes,
  type LayerSourceName,
  type ToolKindOfWork,
} from "./allowed";
import { macroFitsLayer } from "./allowed";
import { knownGradients, loadGradients, stopsOf } from "../gradients";

interface Readout {
  lon: number;
  lat: number;
  /** Speed in knots internally; readouts use the global speed preference. */
  speedKnots: number;
  /** Direction, already converted to the project's convention. */
  directionDeg: number;
  /** The stored azimuth, toward, for drawing a glyph (spec.md 3.3). */
  azimuthTowardDeg: number;
  /** Whether any layer wrote the cell (M31, D58). */
  defined: boolean;
  /** The kind of the layer that wins the cell. */
  kind: FieldKindName;
}

/** What the readout shows: the sample under the cursor, and the zoom. */
interface ReadoutSnapshot {
  sample: Readout | null;
  zoomPercent: number;
}

/**
 * The readout's state, kept outside React.
 *
 * It changes on every pointer move, and as component state it re-rendered the
 * whole map view — two thousand lines of hooks and a toolbar — at pointer rate
 * to refresh four spans at the bottom of the screen. As a store, only the
 * component that subscribes to it renders.
 */
function createReadoutStore() {
  let snapshot: ReadoutSnapshot = { sample: null, zoomPercent: 100 };
  const listeners = new Set<() => void>();
  return {
    get: () => snapshot,
    set(next: Partial<ReadoutSnapshot>) {
      snapshot = { ...snapshot, ...next };
      for (const listener of listeners) listener();
    },
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
  };
}

type ReadoutStore = ReturnType<typeof createReadoutStore>;

/**
 * A macro's outline as a region centred at a position, for the insert hover
 * (M24). The outline is degrees about its centre; a region is degrees on the
 * map — the same thing, so the region's own ring builder draws it.
 */
function macroRegion(outline: MacroOutline, lon: number, lat: number): Region {
  switch (outline.kind) {
    case "rect":
      return {
        kind: "rect",
        centre: [lon, lat],
        halfWidthDeg: outline.half_width_deg,
        halfHeightDeg: outline.half_height_deg,
      };
    case "disc":
      return { kind: "disc", centre: [lon, lat], radiusDeg: outline.radius_deg };
    case "polygon":
      return {
        kind: "polygon",
        points: outline.points.map(([dx, dy]) => [lon + dx, lat + dy]),
      };
  }
}

/** The eyedropper's loupe: how much larger the map is inside the ring, and the ring's radius. */
const MAGNIFIER_ZOOM = 2.5;
const MAGNIFIER_RADIUS_CSS = 44;

/** A retained basemap or this frame's WebGL canvas, enlarged at the pointer. */
function drawMagnifier(context: CanvasRenderingContext2D, source: HTMLCanvasElement,
  cursor: { x: number; y: number }, dpr: number): void {
  const radius = MAGNIFIER_RADIUS_CSS * dpr;
  const span = radius * 2 / MAGNIFIER_ZOOM;
  context.save();
  context.beginPath();
  context.arc(cursor.x, cursor.y, radius, 0, Math.PI * 2);
  context.clip();
  context.imageSmoothingEnabled = false;
  context.drawImage(source, cursor.x - span / 2, cursor.y - span / 2, span, span,
    cursor.x - radius, cursor.y - radius, radius * 2, radius * 2);
  context.restore();
  context.save();
  context.strokeStyle = "rgba(20, 28, 44, 0.9)";
  context.lineWidth = 3.5 * dpr;
  context.beginPath();
  context.arc(cursor.x, cursor.y, radius + 2.5 * dpr, 0, Math.PI * 2);
  context.stroke();
  context.strokeStyle = "rgba(255, 255, 255, 0.95)";
  context.lineWidth = 1.5 * dpr;
  context.beginPath();
  context.arc(cursor.x, cursor.y, radius, 0, Math.PI * 2);
  context.stroke();
  const arm = 6 * dpr;
  context.lineWidth = dpr;
  context.beginPath();
  context.moveTo(cursor.x - arm, cursor.y); context.lineTo(cursor.x + arm, cursor.y);
  context.moveTo(cursor.x, cursor.y - arm); context.lineTo(cursor.x, cursor.y + arm);
  context.stroke();
  context.restore();
}

/**
 * Whether a frame token names the macro preview's scene: its revision has
 * bit 62 set (D71), which no document revision reaches.
 */
/** Wind is always drawn as barbs and a current always as arrows (M30). */
function glyphStyleOf(kind: FieldKindName): "arrow" | "barb" {
  return kind === "wind" ? "barb" : "arrow";
}

/** The range of speeds a kind's last frame drew, in m/s, or null for none. */
type SeenRange = { min: number; max: number } | null;

/** A kind's colour ramp, in m/s for the shader and display units for the legend. */
interface Ramp {
  min: number;
  max: number;
  minKnots: string;
  maxKnots: string;
  auto: boolean;
}

function rampOf(scaleKnots: number, seen: SeenRange, units: DisplayUnits): Ramp {
  if (seen === null) {
    return {
      min: 0,
      max: mpsFromKnots(scaleKnots),
      minKnots: "0",
      maxKnots: legendKnots(units.speedFromKnots(scaleKnots), units.speedFromKnots(scaleKnots)),
      auto: false,
    };
  }
  const max = Math.max(seen.max, seen.min + AUTO_SCALE_MIN_SPAN_MPS);
  const lowKnots = units.speedFromMps(seen.min);
  const highKnots = units.speedFromMps(max);
  const span = highKnots - lowKnots;
  return {
    min: seen.min,
    max,
    minKnots: legendKnots(lowKnots, span),
    maxKnots: legendKnots(highKnots, span),
    auto: true,
  };
}

function isPreviewFrame(frame: string): boolean {
  return Number(frame.split("/")[0]) >= 2 ** 62;
}

/**
 * The narrowest span the auto scale gives the ramp, m/s (spec.md 5.3, M27):
 * a field of one speed would otherwise put its whole range into one colour
 * stop, and a ramp that goes nowhere says nothing.
 */
const AUTO_SCALE_MIN_SPAN_MPS = 0.5;

/** How far the seen range may drift before the auto scale follows it: 2%, and never under 0.1 m/s. */
function autoScaleTolerance(range: { min: number; max: number }): number {
  return Math.max(0.1, 0.02 * (range.max - range.min));
}

/**
 * How far one nudge moves the selection, in CSS pixels (spec.md 8.2, M23).
 *
 * Three, down from eight (M27): a nudge is for the last few pixels of a
 * placement, and eight was most of the way to a drag. Holding the key
 * repeats it, so a long way is still reachable.
 */
const NUDGE_PX_CSS = 3;

/** The cursor readout: position, field, zoom. */
function MapReadout({ store, convention }: { store: ReadoutStore; convention: string }) {
  const units = useUnits();
  const { sample, zoomPercent } = useSyncExternalStore(store.subscribe, store.get);
  return (
    <div className="map-readout">
      {sample ? (
        <>
          <span>{formatDegrees(sample.lat, "N", "S")}</span>
          <span>{formatDegrees(normalizeLon(sample.lon), "E", "W")}</span>
          {sample.defined ? (
            <>
              <span className="muted">{sample.kind === "wind" ? "wind" : "current"}</span>
              <span className="accent">{units.speedFromKnots(sample.speedKnots).toFixed(1)} {units.speedUnit}</span>
              <span>
                {Math.round(sample.directionDeg)}° ({convention})
              </span>
            </>
          ) : (
            <span className="muted">no field</span>
          )}
          <span className="muted">{zoomPercent}%</span>
        </>
      ) : (
        <span className="muted">move the cursor over the map</span>
      )}
    </div>
  );
}

/**
 * A stroke preview's swept path and lattice, kept between pointer reports.
 *
 * Keyed by the stroke's own point array, which a gesture in progress mutates
 * in place — so the entry survives from one report to the next and only the
 * new segment is walked. `key` names everything else the geometry depends on;
 * a change to any of it rebuilds from nothing.
 */
interface SweptEntry {
  key: string;
  path: Path2D;
  progress: SweptPathProgress;
  lattice: LatticeProgress;
}

/** Formats a latitude or longitude with a hemisphere suffix. */
function formatDegrees(value: number, positive: string, negative: string): string {
  const suffix = value >= 0 ? positive : negative;
  return `${Math.abs(value).toFixed(2)}° ${suffix}`;
}

/**
 * How near an object's edge the pointer counts as being on it, in CSS pixels.
 */
const EDGE_GRAB_CSS = 6;

/**
 * The tools that highlight the edge under the pointer (spec.md 6.1).
 *
 * The operators, which are invisible on the map and need an edge to be found
 * at all, and the brush, whose strokes are many and whose edges say which one
 * the pointer is over. Not the tools that place a single shape: a circle or a
 * polygon shows its own outline in the field it paints.
 */
const HOVER_TOOLS: ReadonlySet<string> = new Set([
  "brush",
  "mask",
  "intensity",
  "divergence",
  "turn",
  "warp",
]);
/** Hit radius of a transform handle, in CSS pixels. */
const HANDLE_RADIUS_CSS = 6;

/**
 * Floor under a preview's opacity.
 *
 * The field fades calm out entirely so the basemap stays readable, but a
 * preview the user cannot see is not a preview.
 */
const PREVIEW_MIN_ALPHA = 0.28;

/**
 * Resolution of the live-gesture mask, as a fraction of the framebuffer.
 *
 * The mask is uploaded on every pointer report, and a full-resolution one is
 * about 19 MB of texture at 2880x1684 — a cost paid many times a second, in the
 * middle of the interaction the frame budget exists to protect (spec.md 13).
 * Halving each axis quarters it.
 *
 * What it costs is a slightly soft edge on the masked region, sampled back with
 * linear filtering. The preview is a proxy and is allowed to approximate
 * (spec.md 7.9); the field that lands when the gesture commits is exact.
 */
const MASK_SCALE = 0.5;

/**
 * The operation a committed gesture is still waiting to see landed.
 *
 * The most recent, because two masks in flight at once both apply and the
 * later one is what the map has not caught up with. There is never more than
 * one in practice: a gesture is a pointer drag, and there is one pointer.
 */
function heldOperator(held: readonly HeldPreview[]): OperatorPreview | null {
  for (let i = held.length - 1; i >= 0; i--) {
    const operator = held[i]?.operator;
    if (operator) return operator;
  }
  return null;
}

/**
 * Most glyphs one stroke preview will draw.
 *
 * A stroke around the world at a fine lattice would ask for far more than is
 * worth drawing every pointer move; the preview is a proxy, not the field.
 */
const MAX_PREVIEW_GLYPHS = 600;

/** What the map does for the timeline (spec.md 9.4). */
export interface MapHandle extends PlaybackMap {
  /**
   * Starts fetching a step's tiles for the current viewport onto the GPU, and
   * says whether they are all there. Playback advances into a step only once
   * they are; the backend holding them rendered is not enough.
   */
  warm(step: number): boolean;
  /**
   * The visible map as `[west, north, east, south]`, or null before the map
   * has a size.
   *
   * For an image with no georeference of its own: it lands filling the view,
   * where its control points can be reached (spec.md 4.9, M18). The map is the
   * only thing that knows where it is looking.
   */
  bounds(): [number, number, number, number] | null;
  /**
   * Drops the selected region. Object selection and region selection are
   * mutually exclusive (spec.md 8.2, M23): the app calls this when objects
   * are selected, and the map clears the objects when a region is drawn.
   */
  clearRegion(): void;
  /**
   * `Cmd`-`C` with a region selected: captures the field inside it and says
   * so. False when there is no region, in which case the key means the
   * objects and the app copies them (spec.md 8.5).
   */
  copyRegion(): boolean;
  /**
   * `Cmd`-`V` when the clipboard holds a captured field: pastes it under the
   * pointer when the pointer is over the map, otherwise back where it was
   * taken, into `layer`. `still` keeps the copied frame alone (spec.md 8.5).
   */
  pasteCapture(layer: number | null, still: boolean): void;
  /** Takes a capture mode answered elsewhere — the timeline's key removal. */
  setCapture(mode: CaptureMode): void;
  /**
   * Pans to a place, optionally at a zoom, for the MCP service's
   * `view_focus` (spec 8.8). Clamped like every other camera move.
   */
  focus(lon: number, lat: number, pxPerDeg?: number): void;
  /**
   * Arms the image alignment mode for `layer` (Task 7, spec.md 4.9 §2, §5):
   * the "Align…" button beside Opacity in `ImageControls`. Every click on
   * the map from here is a control point's picture or map half, alternating,
   * until `Enter` applies them, `Escape` cancels, or `Backspace` drops the
   * last pair.
   */
  beginAlign(layer: number): void;
  /**
   * Whether the alignment mode is armed, for `App`'s own `Backspace` — which
   * otherwise deletes the selection — to stand down while it is (spec.md
   * 8.4): the same key means something else here, and both listen on
   * `window`.
   */
  isAligning(): boolean;
}

export default function MapView({
  ref,
  project,
  step,
  selection,
  activeLayer,
  picking,
  onPicked,
  onProjectChanged,
  settings,
  onSettings,
  onRecording,
  onStepChange,
  onSelect,
  onViewport,
  autoKey,
  viewSlot,
  activeKind,
  libraryRevision,
  shapeEditing = null,
  onExitShapeEditing,
}: {
  ref?: Ref<MapHandle>;
  shapeEditing?: number | null;
  onExitShapeEditing?: () => void;
  /**
   * The title bar's centre, which the view controls — the capture and
   * measure tools, glyphs, projection, graticule, undo and redo — are
   * rendered into through a portal (M25, D68). The state stays here, where
   * the draw loop reads it; only the buttons move.
   */
  viewSlot: HTMLElement | null;
  /**
   * The kind of field the active layer paints (M29, M30): what the
   * eyedropper and the readout sample, what a region copy or a macro capture
   * takes, and the glyph style of a gesture's preview. The map itself shows
   * every kind the project holds.
   */
  activeKind: FieldKindName;
  /**
   * Bumped when the macro library has changed elsewhere (M35): the insert
   * tool re-reads it rather than going on offering what is no longer there.
   */
  libraryRevision: number;
  project: ProjectSummary;
  step: number;
  selection: number[];
  activeLayer: number | null;
  /** A position property waiting for a map click, armed by the inspector. */
  picking: PositionPick | null;
  onPicked: () => void;
  onProjectChanged: (project: ProjectSummary) => void;
  /**
   * The application's bindings table (spec.md 8.6, M15). Null until it has
   * loaded, in which case no shortcut fires — which is better than firing the
   * wrong one.
   */
  settings: AppSettings | null;
  /** Called when the map changes a view preference — the projection (M11). */
  onSettings: (settings: AppSettings) => void;
  /**
   * Called whenever a macro capture starts, changes or ends (spec.md 8.7).
   *
   * The timeline greys itself out and marks the visited frames from this; the
   * app stands its edit shortcuts down. The map owns the capture because the
   * map is where the region is placed.
   */
  onRecording: (mode: CaptureMode | null) => void;
  onStepChange: (step: number) => void;
  onSelect: (objects: number[]) => void;
  /**
   * Called when the set of visible tiles changes, for render-ahead and
   * readiness (spec.md 9.5). The map is the only thing that knows its
   * viewport; the timeline is the only thing that needs it.
   */
  onViewport: (tiles: TileAddress[]) => void;
  /** Whether a drag keys the current step rather than the base (spec.md 9.3). */
  autoKey: boolean;
}) {
  const units = useUnits();
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const rendererRef = useRef<MapRenderer | null>(null);
  const glyphSettingsRef = useRef(settings?.glyphs ?? DEFAULT_GLYPHS);
  const tilesRef = useRef<TileCache | null>(null);
  /** Textures for the image layers (spec.md 4.9, M18). */
  const imagesRef = useRef<ImageCache | null>(null);
  /**
   * The image layers, mirrored into a ref `draw` can read.
   *
   * They come from the document tree, so they change when the project does —
   * which is also when the revision changes, and the revision is part of the
   * texture's address.
   */
  const imageLayersRef = useRef<ImageLayerView[]>([]);
  /** The image layers that sit above every visible field layer (M49). */
  const imagesOverRef = useRef<ReadonlySet<number>>(new Set());
  /**
   * What each layer holds, for the tools it will take (M51).
   *
   * The whole table rather than the active layer's own answer: the active
   * layer changes without the tree being refetched, so a single remembered
   * answer would be the previous layer's.
   */
  const layerSourcesRef = useRef<ReadonlyMap<number, LayerSourceName>>(new Map());
  /**
   * The stack, bottom first, as far as a gesture needs to know it (M68).
   *
   * A gesture aimed at a hidden layer is refused by the backend, but the
   * refusal arrives on release: the preview would run for the whole stroke
   * first, showing the edit happening to the layers that *are* visible. So
   * the map has to know before the pointer goes down.
   */
  const layerStackRef = useRef<ReadonlyArray<LayerNode>>([]);

  /** Whether a gesture now would land off the map (`allowed.ts`, M68). */
  const activeLayerHidden = useCallback(
    (): boolean => aimedOffTheMap(activeLayerRef.current, layerStackRef.current),
    [],
  );
  /**
   * The last frame whose tiles were all on screen. A frame that is not yet
   * draws its missing tiles from this one, dimmed, rather than blank.
   */
  const shownFrameRef = useRef<string | null>(null);
  const needsTilesRef = useRef(true);
  const cameraRef = useRef<Camera>({ centerLon: 0, centerLat: 20, pxPerDeg: 3 });
  const viewRef = useRef<Viewport>({ width: 1, height: 1 });
  /**
   * Pending redraw handles.
   *
   * Both a frame callback and a timer, because WKWebView suspends
   * `requestAnimationFrame` whenever the window is not being composited --
   * occluded by another window, minimised, or on another space. Without the
   * timer the map simply never draws, and nothing reports why. Whichever fires
   * first cancels the other.
   */
  const scheduled = useRef<{ raf: number; timer: number } | null>(null);
  /** `requestDraw`, for `draw` itself, which is defined before it. */
  const requestDrawRef = useRef<() => void>(() => undefined);
  /** The wait for the pointer to rest before a plane mesh is rebuilt. */
  const meshSettle = useRef<number | null>(null);
  const dragging = useRef<{ x: number; y: number } | null>(null);
  /** Where the pointer went down, to tell a click from a pan. */
  const pressOrigin = useRef<{ x: number; y: number } | null>(null);
  /** Whether the press in progress began in a macro preview (M33). */
  const previewPress = useRef(false);
  const overlayRef = useRef<HTMLCanvasElement | null>(null);
  const loggedFirstDraw = useRef(false);
  /**
   * The project, mirrored into a ref.
   *
   * `draw` reads only refs so it never needs rebuilding, and the tile address
   * depends on the revision — reading it from the closure would keep painting
   * the pre-edit field.
   */
  const projectRef = useRef(project);
  /**
   * The gesture in progress, in the shape the backend will receive.
   *
   * One ref for every tool: what is being drawn is a gesture, not a brush
   * stroke, and holding it in the wire type means the preview and the commit
   * are looking at exactly the same thing.
   *
   * A `point` gesture never appears here — a click commits it at once — so the
   * in-progress kinds are the four that take more than an instant.
   */
  const gestureRef = useRef<InProgress | null>(null);
  /**
   * Strokes that have been committed but whose field is not on screen yet.
   *
   * A commit is an IPC round trip and a tile render. Clearing the overlay at
   * pointer-up leaves a gap in which the stroke exists in the document and
   * nothing on screen shows it, which reads as the paint blinking out. Each
   * entry is held until the revision it produced has been drawn.
   */
  const settling = useRef<HeldPreview[]>([]);
  /** The current `drawOverlay`, for callers that only read refs. */
  const drawOverlayRef = useRef<() => void>(() => {});
  /**
   * The current `finishGesture`, for the key handler.
   *
   * The handler is bound once and lives above the callback it needs, and
   * rebinding it on every option change would put a listener churn on the
   * window for no gain.
   */
  const finishGestureRef = useRef<() => void>(() => {});
  /** Drops a gesture's live preview, for callers that only read refs. */
  const abandonRef = useRef<() => void>(() => {});
  const cursorRef = useRef<{ x: number; y: number } | null>(null);
  /**
   * The gesture in progress, when it operates on the field rather than adding
   * one — the mask and the clone stamp (spec.md 6.2).
   *
   * Held in a ref and read by `draw`, because the map itself has to change: the
   * overlay sits above the field and can add pixels, never take them away, so
   * there is no way to show a removal from up there.
   */
  const operatorRef = useRef<OperatorPreview | null>(null);
  /**
   * Where the gesture's coverage is rasterised, for the mask.
   *
   * One canvas reused across the drag rather than one per pointer move: it is
   * the size of the framebuffer, and allocating that per report is the kind of
   * thing that shows up as jank on a fast stroke.
   */
  const maskCanvas = useRef<HTMLCanvasElement | null>(null);
  /**
   * Display options mirrored into refs.
   *
   * `draw` runs from an animation frame or a timer, so it must read the current
   * values rather than whatever its closure captured. Setting React state and
   * drawing in the same tick would otherwise render the previous options.
   */
  /**
   * The gradient catalogue (spec.md 5.3, M42), for the draw loop, the legend
   * and every preview. Fetched once; a ref because the draw loop holds no
   * dependencies, and state as well so a legend re-renders when it lands.
   */
  const [gradientCatalogue, setGradientCatalogue] = useState(knownGradients);
  const gradientsRef = useRef(gradientCatalogue);
  gradientsRef.current = gradientCatalogue;
  useEffect(() => {
    let dropped = false;
    void loadGradients()
      .then((list) => {
        if (!dropped) {
          setGradientCatalogue(list);
          requestDraw();
        }
      })
      .catch((err: unknown) => void api.frontendLog("error", String(err)));
    return () => {
      dropped = true;
    };
  }, []);
  /** The stops each kind is painted with, for this frame. */
  const gradientStops = (summary: ProjectSummary) => ({
    wind: stopsOf(gradientsRef.current, summary.wind_gradient),
    current: stopsOf(gradientsRef.current, summary.current_gradient),
  });

  const showGlyphsRef = useRef(true);
  const showGraticuleRef = useRef(true);
  const stepRef = useRef(0);
  /** The active layer's kind, for the paths that read refs (M29). */
  const activeKindRef = useRef(activeKind);
  activeKindRef.current = activeKind;

  const loggedDrawError = useRef(false);

  /**
   * The measurements laid over the map (spec.md 10, M8).
   *
   * Held as state *and* as a ref: the option bar renders from the state, and
   * `drawOverlay` — which runs from an animation frame — reads the ref, since
   * its closure would otherwise draw whatever list it captured.
   *
   * Everything in them is computed by Rust. Nothing here measures.
   */
  const [measurements, setMeasurements] = useState<MeasurementView[]>([]);
  const measurementsRef = useRef<MeasurementView[]>([]);
  const [measureKind, setMeasureKind] = useState<MeasurementKind>("dividers");
  const [ringIntervalKm, setRingIntervalKm] = useState(100);
  const [ringCount, setRingCount] = useState(3);
  /**
   * The ring set the bar is editing.
   *
   * Spec.md 10 asks for the interval and count to be editable, and a ring set
   * has no handle for either — they are numbers, not positions. So the bar
   * edits the set most recently placed or touched, and the same two fields are
   * the defaults for the next one. Null when there is no ring set to edit,
   * which is when the fields are only defaults.
   */
  const [activeRings, setActiveRings] = useState<number | null>(null);
  /** A point placed but not yet joined to a second one. */
  const pendingPoint = useRef<[number, number] | null>(null);
  /**
   * The leg being drawn, read out as the placed one will be (spec.md 10,
   * M29): from the pending point, or the open chain's last point, to the
   * pointer. One request in flight, the newest position waiting — the
   * readout's pattern — and the answer drawn with the placed measurements.
   */
  const measurePreview = useRef<MeasurementView | null>(null);
  const measurePreviewFlight = useRef<{ inFlight: boolean; queued: NewMeasurement | null }>({
    inFlight: false,
    queued: null,
  });
  /**
   * The chain still being built, so the next click extends it rather than
   * starting another.
   *
   * A chain is open from the click that creates it until Escape, Enter, a tool
   * change or a click on something else — the same rule the polygon follows,
   * because it is the same gesture: a shape built up click by click has no
   * pointer-up to end it.
   */
  const openChain = useRef<number | null>(null);
  /**
   * The capture region being dragged, while a capture runs (spec.md 8.7).
   *
   * A *drag*, by delta from where the pointer went down, and not a click that
   * recentres the region on the pointer: the region has to follow the hand at
   * pointer resolution, and a click-to-centre jumps the whole region to
   * wherever the click landed.
   */
  const captureDrag = useRef<{ pointer: [number, number]; anchor: [number, number] } | null>(
    null,
  );
  /** The placement in flight, and the one waiting behind it. */
  const captureMove = useRef<{ inFlight: boolean; queued: [number, number] | null }>({
    inFlight: false,
    queued: null,
  });
  /** The measurement handle being dragged. */
  const measureDrag = useRef<HandlePick | null>(null);
  /**
   * The handle move in flight, and the position waiting behind it.
   *
   * One request at a time, latest wins — the same pattern the readout and the
   * drag preview use, for the same reason: the pointer reports faster than a
   * round trip completes.
   */
  const measureMove = useRef<{ inFlight: boolean; queued: [number, number] | null }>({
    inFlight: false,
    queued: null,
  });

  const [ready, setReady] = useState(false);
  const [error, setErrorState] = useState<string | null>(null);
  // The map's errors go to the status bar's hint area (M25); the state is
  // kept only so the basemap's loading line knows to stand down.
  const setError = useCallback((message: string | null) => {
    setErrorState(message);
    reportError(message);
  }, []);
  /**
   * The glyph style of a gesture's preview: wind is always barbs and a
   * current always arrows (M30), so it follows the layer being painted.
   */
  const glyphStyle: "arrow" | "barb" = glyphStyleOf(activeKind);
  const [showGlyphs, setShowGlyphs] = useState(true);
  const [showGraticule, setShowGraticule] = useState(true);
  // The backdrops (spec.md 4.11): what the map draws *under* everything.
  // View state like the graticule, not layers — the layer panel shows
  // nothing for them, because neither is part of the document.
  const [showCharts, setShowCharts] = useState(false);
  const [showOsm, setShowOsm] = useState(false);
  const showChartsRef = useRef(false);
  const showOsmRef = useRef(false);
  const backdropsRef = useRef<BackdropCache | null>(null);
  /** The chart directory's token, or 0 when none is chosen. */
  const chartTokenRef = useRef(0);
  /** The visible GIS layers, bottom of the stack first (spec.md 4.11). */
  const gisLayersRef = useRef<{ layer: number; token: number }[]>([]);
  /** What the chart directory holds, so the checkbox can say. */
  const [chartStatus, setChartStatus] = useState<ChartStatus | null>(null);

  // The directory is a setting, so the status is re-asked whenever the
  // settings change — choosing a directory in the dialog turns the
  // checkbox on the moment it finds cells.
  useEffect(() => {
    let live = true;
    void api
      .chartStatus()
      .then((status) => { if (live) setChartStatus(status); })
      .catch(() => { if (live) setChartStatus(null); });
    return () => { live = false; };
  }, [settings?.chart_directory]);

  useEffect(() => {
    chartTokenRef.current = chartStatus?.cells ? Number(chartStatus.token) : 0;
    requestDrawRef.current();
  }, [chartStatus]);

  // A directory that goes away takes the charts off the map with it.
  useEffect(() => {
    if (showCharts && (!chartStatus?.directory || chartStatus.cells === 0)) setShowCharts(false);
  }, [chartStatus, showCharts]);
  /**
   * The two boxes over the map: the colour legend and the cursor readout
   * (M77).
   *
   * View state like the glyphs and the graticule, and kept the same way — in
   * the map, not in the project and not in the settings file. What is drawn
   * over the map is how one person is looking at it, not a fact about the
   * document (spec.md 5.2).
   */
  const [showLegend, setShowLegend] = useState(true);
  const [showReadout, setShowReadout] = useState(true);
  const readoutStore = useRef<ReadoutStore | null>(null);
  readoutStore.current ??= createReadoutStore();
  /**
   * The image alignment mode (Task 7), kept the same way as the readout:
   * external, because a click is rarer than a pointer move but still
   * changes outside anything React would otherwise re-render for.
   */
  const alignStore = useRef<AlignStore | null>(null);
  // Retained separately from WebGL's transient drawing buffer: moving the
  // alignment loupe only redraws the overlay, without rendering the field.
  const alignBasemap = useRef<HTMLCanvasElement | null>(null);
  const imageDrag = useRef<{
    cursor: string;
    push: (point: { lon: number; lat: number }) => void;
    finish: () => void;
  } | null>(null);
  const imageWriteTail = useRef<Promise<void>>(Promise.resolve());
  const imageGestureId = useRef(0);
  useEffect(() => () => imageDrag.current?.finish(), []);
  alignStore.current ??= createAlignStore();
  /**
   * The field sample in flight for the readout, and the position waiting
   * behind it.
   *
   * One request at a time, latest wins: the pointer reports more often than a
   * round trip completes, and a request per report queued up behind itself
   * until the readout was answering positions the cursor had left long ago.
   */
  const sampling = useRef<{ inFlight: boolean; queued: { lon: number; lat: number } | null }>({
    inFlight: false,
    queued: null,
  });
  /** Swept previews being extended, keyed by their stroke's point array. */
  const sweptCache = useRef(new WeakMap<object, SweptEntry>());
  /** An overlay-only redraw waiting for the next frame. */
  const overlayScheduled = useRef<number | null>(null);
  /** The last viewport reported, so the same one is not reported per frame. */
  const reportedViewport = useRef("");
  const onViewportRef = useRef(onViewport);
  onViewportRef.current = onViewport;
  const [pending, setPending] = useState(0);
  // Tiles still rendering, said in the status bar rather than the title bar
  // (M27): the count comes and goes on every edit, and the status bar is
  // where the eye already goes for what is happening.
  useEffect(() => {
    setActivity(pending > 0 ? `rendering ${pending}…` : null);
    return () => setActivity(null);
  }, [pending]);
  const [tool, setActiveTool] = useState<ActiveTool>(HAND);
  const setTool = useCallback((next: ActiveTool) => {
    onExitShapeEditing?.();
    setActiveTool(next);
  }, [onExitShapeEditing]);
  const shapeControls = useRef<ShapeControls | null>(null);
  const shapeDrag = useRef<{ hit: ShapePointHit; before: ShapeControls; pointer: number; moved: boolean } | null>(null);
  const shapeModeRef = useRef(shapeEditing);
  shapeModeRef.current = shapeEditing;
  useEffect(() => {
    if (shapeEditing !== null) setActiveTool(HAND);
  }, [shapeEditing]);
  useEffect(() => {
    let cancelled = false;
    shapeControls.current = null;
    shapeDrag.current = null;
    drawOverlayRef.current();
    if (shapeEditing !== null) {
      void api.shapeControls(shapeEditing, step).then((controls) => {
        if (cancelled) return;
        shapeControls.current = controls;
        drawOverlayRef.current();
      }).catch((error: unknown) => {
        if (cancelled) return;
        onExitShapeEditing?.();
        if (typeof error === "object" && error !== null && "kind" in error && error.kind === "missing-object") return;
        reportError(String(error));
      });
    }
    return () => { cancelled = true; };
  }, [shapeEditing, step, project.revision, onExitShapeEditing]);
  useEffect(() => {
    const cancel = () => {
      const drag = shapeDrag.current;
      if (!drag) return;
      shapeControls.current = drag.before;
      shapeDrag.current = null;
      drawOverlayRef.current();
    };
    window.addEventListener("blur", cancel);
    return () => window.removeEventListener("blur", cancel);
  }, []);
  /*
    The selected region, and how the select tool draws one (spec.md 8.2, M14).
    Session state: not document, not history — a region is a way of pointing,
    and pointing is not an edit.
  */
  const [region, setRegion] = useState<Region | null>(null);
  /**
   * Selects a region, and with it deselects every object: the two are
   * mutually exclusive (spec.md 8.2, M23). The app does the converse when an
   * object is selected, through `MapHandle.clearRegion`. Clearing a region
   * leaves the objects alone.
   */
  const selectRegion = useCallback(
    (next: Region | null) => {
      setRegion(next);
      if (next !== null) onSelect([]);
    },
    [onSelect],
  );
  /**
   * The macro capture in progress, if any (spec.md 8.7, M16).
   *
   * While it is active the backend refuses *every* document write — the
   * history lock — so the map's job is to say so, to let the region be placed
   * at each step, and to offer the two ways out.
   */
  const [recording, setRecording] = useState<CaptureMode | null>(null);
  const recordingRef = useRef<CaptureMode | null>(null);
  recordingRef.current = recording;
  const previewing = recording !== null && recording.phase === "previewing";
  useEffect(() => {
    onRecording(recording);
  }, [onRecording, recording]);
  /**
   * The revision tiles are addressed by: the macro preview's while one is
   * shown, the document's otherwise (D71). One function, read by the draw
   * loop and by `warm`, so playback and the frame agree about which scene
   * they are looking at.
   */
  const frameRevision = () =>
    recordingRef.current?.preview_revision ?? projectRef.current.revision;
  /**
   * A tile frame's address: revision, then step. An edit changes the
   * revision, so a cached tile can never show a field that no longer exists.
   * One tile holds every kind of field (M31).
   */
  const frameOf = (step: number): string => frameToken(frameRevision(), step);
  /**
   * The same frame with one layer left out (spec.md 6.2, M40).
   *
   * The eraser acts on one layer, but the live remove acts on the composited
   * tile, so removing where the gesture covers took the whole stack with it:
   * a stroke on a lower layer blanked every layer above it until the button
   * came up. The map draws this frame back into the hole, which is the field
   * the erase actually leaves behind.
   */
  const beneathOf = (step: number, layer: number): string =>
    beneathToken(frameRevision(), step, layer);
  /**
   * The layer alone (spec.md 6.2, M45).
   *
   * What a clone stamp reads its source from. The commit samples the field
   * beneath the stamp *in the layer the stamp is in*, so a preview drawn from
   * the whole composited stack showed a different field arriving under the
   * brush from the one that would land — the top layer's, wherever a layer
   * above covered the source.
   */
  const onlyOf = (step: number, layer: number): string =>
    onlyToken(frameRevision(), step, layer);
  /** Whether movement is recorded by the *next* capture. */
  const [recordMovement, setRecordMovement] = useState(false);
  /** The macro library, for the insert tool's bar. */
  const [library, setLibrary] = useState<MacroLibrary | null>(null);
  /** Which macro the insert tool will place. */
  const [macroId, setMacroId] = useState<string | null>(null);
  const canInsertMacro = useCallback(() => macroFitsLayer(
    library?.entries.find(entry => entry.id === macroId) ?? null,
    activeLayerRef.current,
    layerStackRef.current,
  ), [library, macroId]);
  /** The name being typed for a capture being finished. */
  const [captureName, setCaptureName] = useState<string | null>(null);
  const [regionMode, setRegionMode] = useState<RegionMode>("rect");
  /** The region drag in flight, in geographic degrees. */
  const regionDrag = useRef<{ from: [number, number]; points: Array<[number, number]> } | null>(
    null,
  );
  /**
   * The tool catalogue, as the backend describes it.
   *
   * Fetched rather than written out here: the options a tool has, which of them
   * a mode makes inert, and which gesture drives it are all facts the property
   * system already holds, and a second copy in the frontend is a second copy to
   * get wrong (spec.md 6.1).
   */
  const [palette, setPalette] = useState<ToolSchema[]>([]);
  /**
   * What each tool is currently set to, keyed by tool.
   *
   * Per tool rather than shared: options freeze onto the object at creation, so
   * they are the *tool's* settings, and switching away and back must not lose
   * them.
   */
  const [toolStates, setToolStates] = useState<Record<string, ToolState>>({});
  /**
   * A position option of the *tool* waiting for a map click.
   *
   * The inspector's `picking` does the same for an existing object. Two arms
   * rather than one because they place different things — a tool option that no
   * object holds yet, and a property of one that does.
   */
  const [toolPick, setToolPick] = useState<ToolPick | null>(null);
  /**
   * Whether the eyedropper is armed: the next map click takes the speed and
   * direction from the field rather than painting (spec.md 6.1).
   */
  const [eyedropper, setEyedropper] = useState(false);
  const eyedropperRef = useRef(false);
  eyedropperRef.current = eyedropper;
  const [busy, setBusy] = useState(false);
  /**
   * Where the selection's handles go.
   *
   * One object or many: the backend reports the pivot, which is the object's
   * anchor for one and the collective centroid for a group (spec.md 8.2).
   */
  const [committedTransform, setTransform] = useState<SelectionTransform | null>(null);
  /** The same, for `draw`, which reads refs only. */
  const committedTransformRef = useRef<SelectionTransform | null>(null);
  committedTransformRef.current = committedTransform;
  /**
   * The document revision `committedTransform` was fetched for.
   *
   * A drag's held preview retires when the field it produced is on screen —
   * but the committed handles are fetched separately, and if every tile was
   * already cached the field lands before the fetch returns. Dropping the
   * preview then draws the handles from the *previous* revision: they snap back
   * to where the drag started and jump forward again a round trip later. The
   * preview is held until this has caught up as well.
   */
  const transformRevision = useRef(-1);
  /**
   * The drag in progress.
   *
   * The backend holds the baseline; this only remembers what kind of drag it is
   * and where the pointer is. **Nothing is written until the pointer comes up.**
   * Every write bumps the revision, and the revision is part of the tile
   * address, so a write per pointer report invalidated every visible tile and
   * asked for a re-render costing tens to hundreds of milliseconds — the object
   * then appeared to move only when the drag ended. What follows the pointer is
   * a preview drawn on the overlay, which evaluates nothing (spec.md 8.2).
   */
  const handleDrag = useRef<{
    kind: TransformKind;
    /** Where the pointer last was, for the one write at the end. */
    pointer: { lon: number; lat: number };
    /** True while a preview request is in flight, so they never queue up. */
    asking: boolean;
    /** A pointer position that arrived while one was in flight. */
    queued: { lon: number; lat: number } | null;
  } | null>(null);
  /**
   * The selection's footprints as the drag has them, from the backend.
   *
   * Held in a ref rather than state: it changes on every pointer report, and a
   * re-render per report would cost more than the drawing does.
   */
  const dragPreview = useRef<TransformPreview | null>(null);
  /**
   * The drag's last preview, held until its field is on screen.
   *
   * The write at pointer-up re-renders every visible tile, and dropping the
   * outline before those arrive would snap the object back to where it started
   * for as long as that takes. The same rule retires it as retires a stroke's
   * preview.
   */
  const settlingDrag = useRef<(TransformPreview & Settling) | null>(null);
  /**
   * A transparent rendering of the selected objects at the start of a drag.
   * Kept until the drag commits and the replacement tiles arrive.
   */
  const ghost = useRef<{
    image: HTMLCanvasElement;
    left: number;
    top: number;
    from: GhostPlacement;
  } | null>(null);
  /** Whether a drag is waiting for the next frame to take its ghost. */
  const ghostWanted = useRef(false);
  /**
   * The selection's edges as the document had them when the drag began.
   *
   * What the ghost was lifted out of, and so what is dimmed while it is being
   * carried. Held from the start of the drag rather than read from the current
   * outlines: the write at pointer-up moves those, and dimming *them* would
   * shade the place the object has just arrived at.
   */
  const ghostSource = useRef<OperatorOutline[]>([]);
  /**
   * The tool option whose marker is being dragged, if any.
   *
   * A placed position is a handle and takes precedence over what is under it,
   * on the same rule as the transform handles (spec.md 8.1). Without that a
   * placed point could never be adjusted on the map, only retyped or re-picked.
   */
  const markerDrag = useRef<string | null>(null);
  /**
   * A warp being pulled: which object, and where the pointer is now.
   *
   * Shift with the warp tool grabs the warp under the pointer instead of
   * painting a new one, and dragging says where its field goes (spec.md 6.3).
   * Previewed locally and written once on release, like every other drag: a
   * write per pointer report re-renders the map (D29).
   */
  const pushDrag = useRef<{
    object: number;
    from: { lon: number; lat: number };
    to: { lon: number; lat: number };
  } | null>(null);
  /**
   * Set while a path node's handles are being pulled out.
   *
   * The pen places a node on the press and shapes it while the button is held,
   * so the drag belongs to the node just placed rather than to the gesture.
   */
  const nodeDrag = useRef(false);
  /** The rubber band, in device pixels, while one is being dragged. */
  const marquee = useRef<{
    from: { x: number; y: number };
    to: { x: number; y: number };
    crossLayer: boolean;
  } | null>(null);

  /** The active tool's description, or null while a non-drawing tool is chosen. */
  const schemaTool: Tool | null = drawsObjects(tool) ? tool : null;
  /**
   * Whether the tool in hand edits the field that is already there, rather
   * than adding one of its own (spec.md 6.2). Those are the tools whose live
   * preview has to be scoped to the layer being edited (M44), so it is what
   * decides whether the map fetches the frame to compare against.
   *
   * Read off the tool's declared preview kind, so a tool added later is
   * covered by having declared what it is rather than by being listed here.
   * The eraser has no schema of its own and is named.
   */
  const editsFieldRef = useRef(false);
  /** The active tool's schema, for the cursor's own callback (M51). */
  const schemaRef = useRef<ToolSchema | null>(null);
  /**
   * Whether the tool in hand reads the field from somewhere else and brings
   * it in — the clone stamp, and a warp's push (M45). Only those need the
   * edited layer on its own, so only those pay for fetching it.
   */
  const clonesFieldRef = useRef(false);
  const schema = palette.find((entry) => entry.tool === schemaTool) ?? null;
  schemaRef.current = schema;
  editsFieldRef.current = tool === ERASE || (schema !== null && schema.preview !== "field");
  clonesFieldRef.current = schema?.preview === "clone" || schema?.preview === "warp";

  /**
   * What the active tool is set to.
   *
   * Falls back to the schema defaults so the bar renders on the first frame
   * after the palette arrives, before any option has been touched.
   */
  const toolState: ToolState =
    (schema ? toolStates[schema.tool] : undefined) ??
    (schema ? defaultState(schema) : { values: {}, unit: "km" });

  const setToolState = useCallback(
    (next: ToolState) => {
      if (!schema) return;
      setToolStates((current) => ({ ...current, [schema.tool]: next }));
    },
    [schema],
  );
  /**
   * The tool's own state, for a write that lands after an await.
   *
   * The eyedropper's sample is a round trip, and the values it merges into
   * must be the ones the bar holds when the answer arrives — not the ones it
   * held when the click happened.
   */
  const toolStateRef = useRef(toolState);
  toolStateRef.current = toolState;
  // The draw loop holds no dependencies of its own, so anything a frame reads
  // reaches it through a ref. Which tool is in hand and which layer it writes
  // to decide the frame drawn beneath an erase (M40).
  const toolRef = useRef(tool);
  toolRef.current = tool;
  const activeLayerRef = useRef(activeLayer);
  activeLayerRef.current = activeLayer;

  // An armed mode belongs to the tool that armed it. Switching tools — by key
  // or by button — drops both, so a click with the new tool is an ordinary one.
  useEffect(() => {
    setEyedropper(false);
    setToolPick(null);
  }, [tool]);

  // The project's own scale (spec.md 5.3, M15): two people opening one file
  // see the same map, and the application's preference is only the default a
  // new project got.
  /**
   * The colour ramp's span. The project's own scale from calm (spec.md 5.3,
   * M15) — or, with the auto scale on (M27), the slowest to the fastest
   * speed among the tiles the last frame drew, which the renderer reports
   * and the next frame paints with. A frame that drew no field falls back
   * to the project's scale rather than to nothing.
   */
  // One ramp per kind (M29, M31): wind and current are an order of magnitude
  // apart, and the map shows both, each on its own scale, with a legend
  // entry for each kind the project holds.
  // A macro preview shows the kinds the capture holds and no others (M33,
  // M34): a legend offering a scale for a kind the preview has nothing of is
  // a scale for nothing, and a capture takes every kind under its region.
  const kindsShown: FieldKindName[] =
    previewing && recording && recording.kinds.length > 0
      ? recording.kinds.map(kindOf)
      : project.kinds_present.map(kindOf);
  const autoScale = settings?.auto_scale ?? false;
  const [seenRanges, setSeenRanges] = useState<Record<FieldKindName, SeenRange>>({
    wind: null,
    current: null,
  });
  const ramps: Record<FieldKindName, Ramp> = {
    wind: rampOf(project.wind_scale_knots, autoScale ? seenRanges.wind : null, units),
    current: rampOf(project.current_scale_knots, autoScale ? seenRanges.current : null, units),
  };
  // The active layer's ramp, which a gesture's preview paints with.
  const rampMin = ramps[activeKind].min;
  const rampMax = ramps[activeKind].max;
  const rampsKey = KINDS.map((kind) => `${ramps[kind].min}/${ramps[kind].max}`).join(",");
  // What the last frame reported, to compare the next against without a
  // render in between: a change smaller than the eye can see is not applied,
  // or the ramp would breathe with every tile that lands.
  const seenRef = useRef<Record<FieldKindName, SeenRange>>({ wind: null, current: null });
  const autoScaleRef = useRef(autoScale);
  autoScaleRef.current = autoScale;
  // `draw` is built once and reads its inputs through refs; the ramp went in
  // by closure and so stood still after the first frame — a colour scale
  // changed in the settings reached the map only when something else rebuilt
  // the callback. Through a ref, with a redraw when either end moves (M27).
  const rampRef = useRef(ramps);
  rampRef.current = ramps;
  const lastStep = Math.max(0, project.step_count - 1);

  /**
   * The palette, fetched once.
   *
   * The tools and their options are the property system's answer, not the
   * frontend's (spec.md 6.1), so they arrive over IPC like everything else the
   * document knows.
   */
  useEffect(() => {
    let live = true;
    void api
      .toolPalette()
      .then((entries) => {
        if (!live) return;
        setPalette(entries);
        setToolStates((current) => {
          const next = { ...current };
          for (const entry of entries) next[entry.tool] ??= defaultState(entry);
          return next;
        });
      })
      .catch((err: unknown) =>
        void api.frontendLog("error", `tool palette failed: ${String(err)}`),
      );
    return () => {
      live = false;
    };
  }, []);

  /** Whether a screen point is within grabbing distance of a marker. */
  const near = useCallback(
    (point: { x: number; y: number }, marker: { x: number; y: number }) =>
      Math.hypot(point.x - marker.x, point.y - marker.y) <=
      HANDLE_RADIUS_CSS * 2 * (window.devicePixelRatio || 1),
    [],
  );

  /**
   * The position option whose marker is under `point`, if any.
   *
   * Checked across every live position option of the tool, so the rule that a
   * placed marker is a handle holds for all of them at once (spec.md 6.1).
   */
  const markerUnder = useCallback(
    (point: { x: number; y: number }): string | null => {
      if (!schema) return null;
      for (const spec of liveOptions(schema, toolState.values)) {
        if (spec.default.kind !== "position") continue;
        const [lon, lat] = positionOf(toolState.values, spec.property);
        if (near(point, toScreen(cameraRef.current, viewRef.current, { lon, lat }))) {
          return spec.property;
        }
      }
      return null;
    },
    [near, schema, toolState],
  );

  /**
   * The nodes and vertices placed so far, for a gesture built point by point.
   *
   * The footprint preview shows what a polygon or a curve will *paint*; this
   * shows what has been *placed*, which for the first two vertices is all there
   * is. Bézier handles are drawn as well, so the pen's pull is visible while it
   * is being made rather than only in the curve it produced.
   */
  const drawPlacedPoints = useCallback(
    (
      context: CanvasRenderingContext2D,
      drawing: Extract<InProgress, { kind: "ring" | "path" }>,
      dpr: number,
      cursor: { x: number; y: number } | null,
    ) => {
      const camera = cameraRef.current;
      const view = viewRef.current;
      const at = (lon: number, lat: number) => toScreen(camera, view, { lon, lat });

      context.save();
      context.strokeStyle = "rgba(160, 232, 255, 0.95)";
      context.fillStyle = "rgba(160, 232, 255, 0.95)";
      context.lineWidth = Math.max(1, dpr);

      const nodes =
        drawing.kind === "ring"
          ? drawing.points.map((point) => ({ at: point }) as PathPoint)
          : drawing.nodes;

      // The chain as placed, closed for a ring because its last edge is
      // implied and a user cannot see an edge nobody drew.
      context.beginPath();
      nodes.forEach((node, index) => {
        const point = at(node.at[0], node.at[1]);
        if (index === 0) context.moveTo(point.x, point.y);
        else context.lineTo(point.x, point.y);
      });
      if (drawing.kind === "ring" && nodes.length > 2) context.closePath();
      context.stroke();

      // The edge the next click would add. Without it there is no way to see
      // where a vertex is going until it has been placed, which for a polygon
      // is most of the gesture.
      const last = nodes[nodes.length - 1];
      if (cursor && last) {
        const from = at(last.at[0], last.at[1]);
        context.save();
        context.setLineDash([5 * dpr, 4 * dpr]);
        context.beginPath();
        context.moveTo(from.x, from.y);
        context.lineTo(cursor.x, cursor.y);
        context.stroke();
        context.restore();
      }

      for (const node of nodes) {
        const point = at(node.at[0], node.at[1]);
        context.beginPath();
        context.arc(point.x, point.y, 3.5 * dpr, 0, Math.PI * 2);
        context.fill();

        for (const handle of [node.in_handle, node.out_handle]) {
          if (!handle) continue;
          const end = at(handle[0], handle[1]);
          context.beginPath();
          context.moveTo(point.x, point.y);
          context.lineTo(end.x, end.y);
          context.stroke();
          context.beginPath();
          context.arc(end.x, end.y, 2.5 * dpr, 0, Math.PI * 2);
          context.stroke();
        }
      }
      context.restore();
    },
    [],
  );

  /** Render only the selected objects into a transparent, bounded drag image. */
  const takeGhost = useCallback(() => {
    const canvas = canvasRef.current;
    const placement = committedTransformRef.current;
    if (!canvas || !placement) return;
    const camera = cameraRef.current;
    if (projectionFor(camera).general) return;
    const view = viewRef.current;
    const at = toScreen(camera, view, { lon: placement.lon, lat: placement.lat });
    const { rx, ry } = footprintRadii(camera, placement.lat, placement.radius_m / 1000);
    const region = ghostRegion(at, { rx, ry }, { width: canvas.width, height: canvas.height });
    if (region === null) return;
    const image = document.createElement("canvas");
    image.width = region.width;
    image.height = region.height;
    const context = image.getContext("2d");
    if (context === null) return;
    ghostWanted.current = false;
    const source = ghostSource.current;
    const projection = projectionFor(camera);
    const scale = Math.min(1, 384 / Math.max(region.width, region.height));
    const width = Math.max(1, Math.ceil(region.width * scale));
    const height = Math.max(1, Math.ceil(region.height * scale));
    const west = camera.centerLon + (region.left - view.width / 2) / camera.pxPerDeg;
    const north = projection.yOf(camera.centerLat) - (region.top - view.height / 2) / camera.pxPerDeg;
    void api.selectionPreview({
      objects: source.map((o) => o.object), step: stepRef.current, revision: projectRef.current.revision,
      west, north, across: region.width / camera.pxPerDeg, down: region.height / camera.pxPerDeg,
      space: projection.mode + 1, width, height,
    }).then((bytes) => {
      if (ghostSource.current !== source) return;
      const data = new Float32Array(bytes);
      const pixels = new ImageData(width, height);
      const summary = projectRef.current;
      const gradients = { wind: stopsOf(gradientsRef.current, summary.wind_gradient), current: stopsOf(gradientsRef.current, summary.current_gradient) };
      for (let i = 0; i < width * height; i++) {
        const at = i * 4;
        const kind = data[at + 3]! > 0.5 ? "wind" : "current";
        const ramp = rampRef.current[kind];
        const speed = Math.hypot(data[at]!, data[at + 1]!);
        const color = rampColour((speed - ramp.min) / Math.max(0.001, ramp.max - ramp.min), gradients[kind]);
        for (let channel = 0; channel < 3; channel++) pixels.data[at + channel] = Math.round(color[channel]! * 255);
        pixels.data[at + 3] = Math.round(data[at + 2]! * 230);
      }
      const small = document.createElement("canvas");
      small.width = width; small.height = height;
      small.getContext("2d")?.putImageData(pixels, 0, 0);
      context.drawImage(small, 0, 0, image.width, image.height);
      ghost.current = { image, left: region.left, top: region.top,
        from: { pivot: at, rotationDeg: placement.rotation_deg, radiusM: placement.radius_m },
      };
      drawOverlayRef.current();
    }).catch((error: unknown) => {
      void api.frontendLog("warn", `Selection preview: ${String(error)}`);
    });
  }, []);

  const draw = useCallback(() => {
    const drawStarted = performance.now();
    const renderer = rendererRef.current;
    if (!renderer) return;

    // This frame draws the overlay too, so one waiting on its own is redundant.
    if (overlayScheduled.current !== null) {
      cancelAnimationFrame(overlayScheduled.current);
      overlayScheduled.current = null;
    }

    const frame = frameOf(stepRef.current);
    // The last frame fully on screen stands in for this one's missing tiles —
    // but never across the preview's boundary (M28): the document's tiles
    // held under the preview kept every layer on the map until the first
    // click, and the preview's under the document kept the macro alone.
    const previous = shownFrameRef.current;
    const shown =
      previous !== null && isPreviewFrame(previous) === isPreviewFrame(frame) ? previous : null;
    // The tiles this frame draws, which the backdrops are asked for too: a
    // backdrop that fetched a different set would be a second viewport.
    const viewTiles = visibleTiles(cameraRef.current, viewRef.current);
    const alignment = alignStore.current!.get();
    const mapPoint = alignment.layer !== null && alignment.expecting === "map";
    if (!mapPoint) alignBasemap.current = null;
    const state: RenderState = {
      camera: cameraRef.current,
      view: viewRef.current,
      hiddenImageLayer: mapPoint ? alignment.layer : null,
      frame,
      heldFrame: shown !== null && shown !== frame ? shown : null,
      ramps: {
        wind: { min: rampRef.current.wind.min, max: rampRef.current.wind.max },
        current: { min: rampRef.current.current.min, max: rampRef.current.current.max },
      },
      gradients: gradientStops(projectRef.current),
      showGlyphs: showGlyphsRef.current,
      glyphs: glyphSettingsRef.current,
      showGraticule: showGraticuleRef.current,
      pixelRatio: window.devicePixelRatio || 1,
      // The gesture's own operation while it is being drawn, and the one it
      // committed while its tiles are still on their way.
      operator: operatorRef.current ?? heldOperator(settling.current),
      // The stack without the layer being edited (M40, M44). It is what the
      // erase leaves behind, and it is also what tells every other live edit
      // which pixels belong to the layer it is aimed at.
      //
      // Named whenever a field-editing tool has a layer in hand rather than
      // only while the button is down, so the frame is warmed in the pause
      // before the first stroke rather than during it. A tile the layer does
      // not reach hashes the same with the layer left out as with it, so
      // those are already resident and only the tiles the layer covers are
      // rendered.
      belowFrame:
        editsFieldRef.current && activeLayerRef.current !== null
          ? beneathOf(stepRef.current, activeLayerRef.current)
          : null,
      // The same scope for the frame being *held* (M82). A commit
      // re-addresses the beneath frame along with everything else, so for the
      // round trip that follows a stroke the scope the live edit is drawn
      // with names tiles nobody has fetched — and an edit whose scope has not
      // arrived applies to nothing (M72). The held removal stopped being
      // drawn there and the field it had taken came back until the new tiles
      // landed: the flash at every release. The frame on screen during that
      // window is the one before the commit, and its beneath frame is
      // resident, having been warmed throughout the stroke.
      belowHeldFrame:
        editsFieldRef.current && activeLayerRef.current !== null && shown !== null && shown !== frame
          ? (() => {
              const was = parseFrameToken(shown);
              return was === null
                ? null
                : beneathToken(was.revision, was.step, activeLayerRef.current);
            })()
          : null,
      // Where a clone's source is read from (M45): the edited layer alone,
      // which is what its commit samples.
      sourceFrame:
        clonesFieldRef.current && activeLayerRef.current !== null
          ? onlyOf(stepRef.current, activeLayerRef.current)
          : null,
      // Georeferenced images, above the land and below the field (M18). The
      // revision is part of the texture's address, so an import or a reopen
      // makes the old one unreachable rather than stale.
      // A preview shows the macro on the basemap alone: no images either.
      // The backdrops, under everything (spec.md 4.11). A macro preview
      // shows the field on the basemap alone, so none of them are drawn
      // there either.
      backdrops: recordingRef.current?.preview_revision
        ? []
        : (backdropsRef.current?.draws(
            {
              charts: showChartsRef.current && chartTokenRef.current
                ? { token: chartTokenRef.current }
                : undefined,
              osm: showOsmRef.current,
              gis: gisLayersRef.current,
            },
            viewTiles,
          ) ?? []),
      images:
        recordingRef.current?.preview_revision !== undefined &&
        recordingRef.current?.preview_revision !== null
          ? []
          : (imagesRef.current?.draws(
              projectRef.current.image_token,
              imageLayersRef.current,
              imagesOverRef.current,
            ) ?? []),
    };

    if (mapPoint) state.onBasemap = () => {
      const source = canvasRef.current;
      if (!source) return;
      const copy = alignBasemap.current ??= document.createElement("canvas");
      if (copy.width !== source.width) copy.width = source.width;
      if (copy.height !== source.height) copy.height = source.height;
      copy.getContext("2d")?.drawImage(source, 0, 0);
    };

    // Tell the timeline which tiles are on screen, once per change rather
    // than per frame: a pan delivers many frames and one viewport.
    const unique = uniqueTiles(viewTiles);
    // A worker can replace the projection mesh under an unchanged camera.
    // Preparation and the ruler must use the tiles this draw actually sees.
    warmTilesRef.current = { camera: state.camera, view: state.view, tiles: unique };
    const key = unique.map((t) => `${t.z}/${t.x}/${t.y}`).join(",");
    if (key !== reportedViewport.current) {
      reportedViewport.current = key;
      onViewportRef.current(unique);
    }

    // Hold the whole timeline of this viewport, so a playback loop pays the
    // fetch-and-upload crossing once rather than on every lap (spec.md 9.4).
    // Called every frame rather than only on a viewport change, because the step
    // count can change under a still camera; `reserve` returns at once when
    // the answer has not moved.
    tilesRef.current?.reserve(projectRef.current.step_count, unique.length);

    // The field beneath the edit, warmed before a stroke needs it (M40).
    // The hole a stroke opens is filled from that frame, and it is also what
    // tells every live edit which pixels belong to the layer it is aimed at.
    // Asked for while the tool is merely in hand, so the round trip happens
    // in the pause between choosing it and using it. `get` is a lookup once
    // the tile is resident, so repeating it per frame costs nothing.
    //
    // **During the stroke as well** (M72). This used to stop the moment the
    // button went down, on the theory that the frame was already warm by
    // then. It is not, after the first stroke: every commit moves the
    // revision, which re-addresses the beneath frame too, so the second
    // stroke and every one after it asked for a frame nothing had fetched.
    // The scope was then unavailable for the whole stroke and the edit was
    // drawn over every layer.
    {
      const cache = tilesRef.current;
      for (const warm of [state.belowFrame, state.sourceFrame]) {
        if (cache && warm) for (const tile of unique) cache.get(warm, tile.z, tile.x, tile.y);
      }
    }

    try {
      const ranges = renderer.render(state);
      // A fixed general projection may have drawn through a mesh it would
      // replace: one zoom band coarse, or with the view nearing its edge
      // (`projectedMesh` in camera.ts). Building one is a fifth of a second,
      // which mid-gesture is a dropped frame and at rest is nothing, so it
      // waits for the pointer to stop. Every frame that still wants one
      // pushes the wait back.
      if (meshSettle.current !== null) window.clearTimeout(meshSettle.current);
      meshSettle.current = null;
      if (projectedMeshWanted()) {
        meshSettle.current = window.setTimeout(() => {
          meshSettle.current = null;
          refreshProjectedMesh(cameraRef.current, viewRef.current);
          requestDrawRef.current();
        }, 180);
      }
      // The auto scale follows what was drawn (spec.md 5.3, M27), per kind
      // (M31). Applied only when a range moved by more than the eye can see,
      // and through state, so the next frame paints with it and the legend
      // says so.
      if (autoScaleRef.current) {
        let moved = false;
        KINDS.forEach((kind) => {
          const seen = ranges[kind];
          const last = seenRef.current[kind];
          const changed =
            seen === null
              ? last !== null
              : last === null ||
                Math.abs(seen.min - last.min) > autoScaleTolerance(last) ||
                Math.abs(seen.max - last.max) > autoScaleTolerance(last);
          if (changed) {
            seenRef.current[kind] = seen;
            moved = true;
          }
        });
        if (moved) setSeenRanges({ ...seenRef.current });
      }
      // Once every tile of this frame is on screen it is the one to hold.
      const tiles = tilesRef.current;
      needsTilesRef.current = !tiles || tiles.residentCount(frame, unique) !== unique.length;
      if (!needsTilesRef.current) {
        shownFrameRef.current = frame;
        playbackPresented(frame);
      }
    } catch (err) {
      // A GL failure inside an animation frame is easy to lose. Report it once
      // and stop drawing rather than flooding the log every frame.
      if (!loggedDrawError.current) {
        loggedDrawError.current = true;
        const message = err instanceof Error ? `${err.message}\n${err.stack ?? ""}` : String(err);
        void api.frontendLog("error", `render failed: ${message}`);
        setError(message);
      }
      return;
    }

    const stats = tilesRef.current?.stats();

    // Retire the previews whose field has arrived — meaning the *map* is
    // showing it, not merely that the document holds it (M80). `shownFrame`
    // is set above only once every tile of this frame is resident, which is
    // the one signal that says so; a count of pending tiles said nothing
    // during the key round trip an edit starts, when no tile has been asked
    // for yet.
    const onScreen = shownFrameRef.current === frame;
    if (settling.current.length > 0) {
      const now = performance.now();
      const held = settling.current.filter(
        (entry) => !previewHasLanded(entry, projectRef.current.revision, onScreen, now),
      );
      settling.current = held;
    }

    const settled = settlingDrag.current;
    if (
      settled &&
      previewHasLanded(settled, projectRef.current.revision, onScreen, performance.now()) &&
      // ...and the handles it hands back to describe the same revision. The
      // timeout inside `previewHasLanded` still bounds the wait.
      (settled.revision === null ||
        transformRevision.current >= settled.revision ||
        performance.now() - settled.at >= SETTLE_TIMEOUT_MS)
    ) {
      settlingDrag.current = null;
      ghost.current = null;
      ghostSource.current = [];
      ghostWanted.current = false;
    }

    // Start the selected-object preview once this frame establishes the view.
    if (ghostWanted.current && ghost.current === null) takeGhost();

    // The overlay is drawn as part of the same frame, from the same camera.
    //
    // Handles, outlines and the brush's footprint are all positioned by the
    // camera, so any arrangement that redraws them *separately* lets the two
    // canvases disagree until something else happens to schedule an overlay
    // redraw. That is not hypothetical: the wheel handler remembered to ask for
    // one and the pan and resize handlers did not, so a selected object's
    // handles sat still while the map moved under them and only caught up on
    // pointer-up. Drawn here, they cannot come apart.
    drawOverlayRef.current();

    if (!loggedFirstDraw.current) {
      loggedFirstDraw.current = true;
      void api.frontendLog(
        "info",
        `first draw: view ${state.view.width}x${state.view.height}, ` +
          `pxPerDeg ${state.camera.pxPerDeg.toFixed(2)}, tiles ${JSON.stringify(stats)}`,
      );
    }
    setPending(stats?.pending ?? 0);
    playbackTime("drawMs", performance.now() - drawStarted);
    // Reading refs only, so this callback never needs to be rebuilt.
  }, []);

  const requestDraw = useCallback(() => {
    if (scheduled.current !== null) return;
    const run = () => {
      const pending = scheduled.current;
      scheduled.current = null;
      if (pending) {
        cancelAnimationFrame(pending.raf);
        window.clearTimeout(pending.timer);
      }
      draw();
    };
    scheduled.current = {
      raf: requestAnimationFrame(run),
      timer: window.setTimeout(run, 200),
    };
  }, [draw]);
  requestDrawRef.current = requestDraw;

  // A plane mesh built off the main thread has arrived (camera.ts): draw
  // through it. One listener, because there is one map.
  useEffect(() => {
    onProjectedMeshReady(() => {
      warmTilesRef.current = null;
      requestDrawRef.current();
    });
    return () => onProjectedMeshReady(null);
  }, []);

  /**
   * Redraws the overlay on the next frame.
   *
   * Pointer reports arrive faster than frames are shown, and an overlay drawn
   * synchronously on each one was drawn twice per frame on a fast drag. Nothing
   * a pointer report changes is visible before the next frame anyway, so the
   * draw waits for it. A pending GL frame already draws the overlay in the same
   * frame — the rule that keeps the two canvases together — so nothing is
   * scheduled behind one.
   */
  const requestOverlay = useCallback(() => {
    if (scheduled.current !== null || overlayScheduled.current !== null) return;
    overlayScheduled.current = requestAnimationFrame(() => {
      overlayScheduled.current = null;
      drawOverlayRef.current();
    });
  }, []);

  // Alternating picture/map clicks changes image visibility as well as the
  // overlay. Undo, cancel and commit restore it through the same draw path.
  useEffect(() => alignStore.current!.subscribe(requestDraw), [requestDraw]);

  // --- Set up GL once ---
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const gl = canvas.getContext("webgl2", {
      antialias: true,
      alpha: false,
      // Left off deliberately: it slows every frame. The capture path reads
      // pixels back inside the same frame instead.
      preserveDrawingBuffer: false,
    });
    if (!gl) {
      setError("WebGL2 is unavailable, so the map cannot be drawn.");
      return;
    }

    let disposed = false;
    let renderer: MapRenderer | null = null;
    let tiles: TileCache | null = null;
    let pictures: ImageCache | null = null;

    void (async () => {
      const trace = (stage: string) => void api.frontendLog("info", `map init: ${stage}`);
      try {
        trace("requesting assets");
        const buffer = await api.basemap();
        trace(`basemap received, ${buffer.byteLength} bytes`);
        const baseUrl = await api.tileBaseUrl();
        trace(`tile base url ${baseUrl}`);
        const info = await api.appInfo();
        if (disposed) return;

        const basemap = parseBasemap(buffer);
        trace(`basemap parsed, ${basemap.lods.length} lods`);
        tiles = new TileCache(gl, baseUrl);
        // A frame's tiles are kept by key, and the keys come from the
        // backend (spec.md 7.10, M31): the token is `<revision>/<step>`.
        tiles.resolver = (frame, wanted) => {
          const at = parseFrameToken(frame);
          if (!at) return Promise.resolve(wanted.map(() => ""));
          const layer = at.scope.kind === "whole" ? null : at.scope.layer;
          return api.tileKeys(at.revision, at.step, [...wanted], at.scope.kind, layer);
        };
        backdropsRef.current = new BackdropCache(gl, baseUrl);
        backdropsRef.current.onChange = () => requestDraw();
        pictures = new ImageCache(gl, baseUrl);
        pictures.onChange = () => requestDraw();
        pictures.onError = (message) => void api.frontendLog("error", message);
        imagesRef.current = pictures;
        renderer = new MapRenderer(gl, basemap, tiles);
        tiles.onChange = () => {
          // Preparing another step must not repaint an already complete map.
          // Missing viewport tiles and live/settling edits still redraw as they land.
          if (needsTilesRef.current || shownFrameRef.current !== frameOf(stepRef.current)
            || gestureRef.current || settling.current.length || settlingDrag.current) requestDraw();
        };
        let reported = 0;
        tiles.onError = (message) => {
          // Report the first few only; a failing scheme fails for every tile.
          if (reported++ < 3) void api.frontendLog("error", message);
        };
        tilesRef.current = tiles;
        rendererRef.current = renderer;
        trace("renderer ready");
        setReady(true);
        requestDraw();

        if (info.debug_capture) {
          window.setTimeout(() => void runCaptureSuite(), 1200);
        }
      } catch (err) {
        const message = err instanceof Error ? `${err.message}\n${err.stack ?? ""}` : String(err);
        if (!disposed) setError(message);
        void api.frontendLog("error", `map init failed: ${message}`);
      }
    })();

    return () => {
      disposed = true;
      if (scheduled.current) {
        cancelAnimationFrame(scheduled.current.raf);
        window.clearTimeout(scheduled.current.timer);
        scheduled.current = null;
      }
      renderer?.dispose();
      tiles?.dispose();
      pictures?.dispose();
      rendererRef.current = null;
      tilesRef.current = null;
      imagesRef.current = null;
    };
    // Set up once; `requestDraw` is stable enough for this purpose and
    // re-running would tear down the GL context on every option change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // --- Track the element size, in device pixels ---
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const resize = () => {
      const dpr = window.devicePixelRatio || 1;
      const width = Math.max(1, Math.round(canvas.clientWidth * dpr));
      const height = Math.max(1, Math.round(canvas.clientHeight * dpr));
      canvas.width = width;
      canvas.height = height;
      if (overlayRef.current) {
        overlayRef.current.width = width;
        overlayRef.current.height = height;
      }
      viewRef.current = { width, height };
      cameraRef.current = clampCamera(cameraRef.current, viewRef.current);
      requestDraw();
    };

    resize();
    const observer = new ResizeObserver(resize);
    observer.observe(canvas);
    return () => observer.disconnect();
  }, [requestDraw]);

  /**
   * A tool's shortcut as its button should show it.
   *
   * From the bindings table rather than from the palette's own letter, so a
   * rebound key appears in the tooltip — which is the half of a rebind that is
   * otherwise forgotten (M15).
   */
  const chord = (tool: string) => (settings ? toolChord(settings, tool) : "unbound");

  /** The actions that move the camera rather than choosing a tool. */
  const PAN_ZOOM = new Set<ShortcutAction>([
    "pan_left",
    "pan_right",
    "pan_up",
    "pan_down",
    "zoom_in",
    "zoom_out",
  ]);

  /**
   * Pans or zooms by a keystroke (spec.md 8.6, M15).
   *
   * A pan is a fixed fraction of the viewport rather than a fixed number of
   * degrees, so one press covers the same amount of what you can see at every
   * zoom. Through `requestDraw`, like every other camera change, so the
   * overlay moves with the map rather than after it.
   */
  const nudgeCamera = useCallback(
    (action: ShortcutAction) => {
      const view = viewRef.current;
      const camera = cameraRef.current;
      const stepPx = 0.2;
      if (action === "zoom_in" || action === "zoom_out") {
        const centre = { x: view.width / 2, y: view.height / 2 };
        cameraRef.current = zoomAbout(
          camera,
          view,
          centre,
          action === "zoom_in" ? 1.25 : 0.8,
        );
      } else {
        const dx = (action === "pan_right" ? 1 : action === "pan_left" ? -1 : 0) * stepPx;
        const dy = (action === "pan_down" ? 1 : action === "pan_up" ? -1 : 0) * stepPx;
        cameraRef.current = panBy(camera, view, view.width * dx, view.height * dy);
      }
      requestDraw();
    },
    [requestDraw],
  );

  /** The actions that move the selection rather than the camera. */
  const NUDGE = new Set<ShortcutAction>(["nudge_left", "nudge_right", "nudge_up", "nudge_down"]);

  /**
   * Moves the selection one step across the screen by a keystroke (spec.md
   * 8.2, M23).
   *
   * With a region selected the region moves; otherwise the selected objects
   * do, as one rigid move through the same transform a drag is — begin at
   * the pivot, drag to the pivot plus the step, end the gesture — so each
   * press is one history entry and a follower goes where its primary does. A
   * screen distance rather than degrees, so a press moves the same amount of
   * what you can see at every zoom and every latitude.
   */
  const nudgeSelection = useCallback(
    (action: ShortcutAction) => {
      const dpr = window.devicePixelRatio || 1;
      const px = NUDGE_PX_CSS * dpr;
      const dx = action === "nudge_right" ? px : action === "nudge_left" ? -px : 0;
      const dy = action === "nudge_down" ? px : action === "nudge_up" ? -px : 0;
      const camera = cameraRef.current;
      const view = viewRef.current;
      const shifted = (lon: number, lat: number) => {
        const at = toScreen(camera, view, { lon, lat });
        return unproject(camera, view, { x: at.x + dx, y: at.y + dy });
      };
      if (region !== null) {
        const anchor = regionAnchor(region);
        if (anchor === null) return;
        const moved = shifted(anchor[0], anchor[1]);
        setRegion(recentred(region, moved.lon, moved.lat));
        requestOverlay();
        return;
      }
      const pivot = committedTransform;
      if (pivot === null || selection.length === 0 || handleDrag.current !== null) return;
      const to = shifted(pivot.lon, pivot.lat);
      void api
        .beginTransform(selection, step, "move", pivot.lon, pivot.lat, autoKey)
        .then(() => api.dragTransform(to.lon, to.lat))
        .then((summary) => {
          onProjectChanged(summary);
          requestDraw();
        })
        .catch((err: unknown) => void api.frontendLog("error", `nudge failed: ${String(err)}`))
        .finally(() => void api.endGesture());
    },
    [autoKey, committedTransform, onProjectChanged, region, requestDraw, requestOverlay, selection, step],
  );

  /**
   * Whether a macro capture is running (spec.md 8.7, M16).
   *
   * Asked once on mount and kept by the commands that change it. A capture
   * survives a reload of this component — it lives in the session — so the map
   * has to *ask* rather than assume it starts with none.
   */
  useEffect(() => {
    let live = true;
    void api
      .captureMode()
      .then((mode) => {
        if (live) setRecording(mode.active ? mode : null);
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, []);

  /**
   * Scrubbing to a step while recording keys the region there where it
   * stands and shows it there (D72): the region at each step is the key, or
   * the great circle between keys, and the map draws that step's place.
   */
  useEffect(() => {
    if (recording === null || recording.phase !== "recording") return;
    let live = true;
    void api
      .visitCapture(step)
      .then((mode) => {
        if (!live || !mode.active) return;
        setRecording(mode);
        const at = mode.position;
        if (at) {
          setRegion((current) => (current === null ? current : recentred(current, at[0], at[1])));
          requestOverlay();
        }
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
    // Per step and per phase; the mode itself is what this sets.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [step, recording?.phase]);

  /**
   * The tool's hint, for the status bar (M25). What the capture bar and the
   * insert bar used to say beside their controls: a hint is about the next
   * thing to do, and the status bar is where the eye goes for that.
   */
  useEffect(() => {
    if (tool === CAPTURE && recording === null && region === null) {
      setHint("Draw a region first — it is what gets recorded.");
    } else if (recording !== null && recording.phase === "previewing") {
      setHint(
        "Click the map to see a copy of the macro there, or drag to pan. Save keeps the macro in the library; Edit goes back to recording; the copies go with the preview.",
      );
    } else if (recording !== null) {
      setHint(
        "Scrub the ruler and drag the region into place at each step; every step visited is a key.",
      );
    } else if (tool === ERASE) {
      setHint(
        "Drag to erase what the brush covers in the active layer. Hold Shift to erase from this frame only.",
      );
    } else if (tool === INSERT && recording === null) {
      setHint(
        (library?.entries.length ?? 0) === 0
          ? "The macro library is empty. Capture a run of frames first."
          : "Click the map to place the macro.",
      );
    } else {
      setHint(null);
    }
  }, [library, recording, region, tool]);

  /** The macro library, for the insert tool's bar. */
  const readLibrary = useCallback(() => {
    void api
      .macroLibrary()
      .then((held) => {
        setLibrary(held);
        // Keep a choice that is still there; otherwise fall to the newest,
        // which is the one just captured.
        setMacroId((current) =>
          current !== null && held.entries.some((entry) => entry.id === current)
            ? current
            : (held.entries[0]?.id ?? null),
        );
      })
      .catch(() => undefined);
  }, []);
  useEffect(() => {
    if (tool === INSERT) readLibrary();
  }, [readLibrary, tool, libraryRevision]);

  /**
   * The image layers, re-read whenever the document changes.
   *
   * From the document tree, because that is where a layer's own view of itself
   * lives and an image layer is a layer. It carries where each picture sits,
   * so a drag lands here — the texture itself is addressed by the opening's
   * token and stays put (M37), and the picture moves because its placement
   * did.
   */
  useEffect(() => {
    let live = true;
    void api
      .documentTree(0)
      .then((tree) => {
        if (!live) return;
        // Hidden layers are not on the map, images included (M49). Dropped
        // here rather than at the draw, so a hidden picture also offers no
        // control points and cannot be dragged: an eye that hides a layer
        // hides all of it.
        // A GIS layer is drawn under the field like a picture, but as tiles
        // on the map's own grid rather than as one placed image: a survey
        // may span the world, and its tiles follow the projection.
        gisLayersRef.current = tree.layers
          .filter((layer) => layer.visible && layer.gis?.loaded)
          .map((layer) => ({ layer: layer.id, token: projectRef.current.revision }));
        imageLayersRef.current = tree.layers
          .filter((layer) => layer.visible)
          .flatMap((layer) => (layer.image ? [{ ...layer.image,
            speedRange: layer.speed_filter?.speed_min_mps != null && layer.speed_filter.speed_max_mps != null
              ? [layer.speed_filter.speed_min_mps, layer.speed_filter.speed_max_mps] as [number, number] : undefined,
          }] : []));
        layerSourcesRef.current = new Map(
          tree.layers.map((layer) => [layer.id, layer.source as LayerSourceName]),
        );
        layerStackRef.current = tree.layers;
        // And which of them draw over the field rather than under it.
        imagesOverRef.current = imagesOverField(
          tree.layers.map((layer) => ({
            id: layer.id,
            visible: layer.visible,
            isImage: layer.image !== null,
          })),
        );
        requestDraw();
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, [project, requestDraw]);

  /**
   * The measurements, re-read whenever the document changes.
   *
   * Keyed on the whole summary rather than on the revision, because a
   * measurement deliberately does *not* bump it — the revision addresses tiles,
   * and a pair of dividers changes no pixel of the field, so bumping it would
   * throw the whole tile cache away. Undo and redo do change the measurements
   * and are ordinary document changes, so this is what catches them. The call
   * reads a short list under one lock; it is off the render path.
   */
  useEffect(() => {
    let live = true;
    void api
      .measurements()
      .then((views) => {
        if (!live) return;
        measurementsRef.current = views;
        setMeasurements(views);
        requestOverlay();
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, [project, requestOverlay, units.distanceUnit]);

  /** Takes a new set of measurements from the backend and redraws. */
  const tookMeasurements = useCallback(
    (views: MeasurementView[]) => {
      measurementsRef.current = views;
      setMeasurements(views);
      requestOverlay();
      return views;
    },
    [requestOverlay],
  );

  /**
   * Closes the chain being built, so the next click starts a new one.
   *
   * Also drops a lone point that never found its partner: a half-placed
   * measurement is not a measurement, and leaving the mark on screen after the
   * gesture has ended is a mark nothing will ever join.
   */
  const endMeasuring = useCallback(() => {
    if (openChain.current === null && pendingPoint.current === null) return false;
    openChain.current = null;
    pendingPoint.current = null;
    requestOverlay();
    return true;
  }, [requestOverlay]);

  // Single-key tool shortcuts, as in every other paint application.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      // Never steal a keystroke from a field the user is typing in.
      if (target && /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName)) return;

      // The image alignment mode (Task 7, spec.md 4.9 §2, §5) takes these
      // three keys ahead of everything below, including the shape editor's
      // and the timeline's own `Escape`/`Backspace`: while it is armed they
      // mean the session, not a gesture or the selection.
      if (alignStore.current!.get().layer !== null) {
        if (event.key === "Enter") {
          event.preventDefault();
          // `commit` ends the session only once the write it is given
          // succeeds (align.ts): zero pairs behaves as Escape, and a
          // rejected write — any failure, not only the cap above, which
          // should stop the reachable case before this is ever reached —
          // leaves every pair exactly where it was, so Backspace can trim
          // and Enter can be tried again instead of starting the whole
          // chart over. An earlier version of this cleared the store
          // *before* the write resolved and threw every pair away whether
          // it succeeded or not — the bug this replaces.
          void alignStore.current!
            .commit((layer, pairs) =>
              api.setImageControlPoints(layer, pairs.map((p) => [p.u, p.v, p.lon, p.lat])),
            )
            .then((summary) => {
              setHint(null);
              if (summary) onProjectChanged(summary);
              requestOverlay();
            })
            .catch((err: unknown) => setError(String(err)));
          return;
        }
        if (event.key === "Escape") {
          event.preventDefault();
          alignStore.current!.cancel();
          setHint(null);
          requestOverlay();
          return;
        }
        if (event.key === "Backspace" || event.key === "Delete") {
          event.preventDefault();
          alignStore.current!.undoLast();
          return;
        }
      }

      // Region selection (spec.md 8.2, M14). `Cmd`-`A` enters the select tool
      // with the view selected and `Cmd`-`Shift`-`A` the whole map, so the
      // key that means "select everything" everywhere else means it here too;
      // `Cmd`-`D` clears. None of the three was bound before.
      if ((event.metaKey || event.ctrlKey) && !event.altKey) {
        const key = event.key.toLowerCase();
        if (key === "a") {
          event.preventDefault();
          setTool(SELECT);
          selectRegion(
            event.shiftKey
              ? wholeMap()
              : regionOfView(cameraRef.current, viewRef.current.width, viewRef.current.height),
          );
          requestOverlay();
          return;
        }
        if (key === "d") {
          event.preventDefault();
          selectRegion(null);
          requestOverlay();
          return;
        }
        // Copy and paste are the app's (spec.md 8.5): it asks this map to
        // copy the region through `MapHandle.copyRegion` and to paste a
        // capture through `MapHandle.pasteCapture`, so one key press can
        // never mean two things.
        return;
      }

      // Every binding comes from one table, which the settings dialog edits
      // and the tooltips read (spec.md 8.6, M15) — so a rebound key selects
      // its tool and says so in the same breath, and two actions cannot
      // quietly share a chord.
      const chord = chordOf(event);
      const bound = chord !== null && settings ? actionFor(settings, chord) : null;
      if (bound?.action === "tool") {
        event.preventDefault();
        setTool(bound.tool as ActiveTool);
        return;
      }
      if (bound !== null && PAN_ZOOM.has(bound.action)) {
        event.preventDefault();
        nudgeCamera(bound.action);
        return;
      }
      if (bound !== null && NUDGE.has(bound.action)) {
        event.preventDefault();
        nudgeSelection(bound.action);
        return;
      }
      // Deselect (M47). Bound to accel-D by default, which is what a hand
      // reaches for. It drops the objects and the rubber band together —
      // both are "what is selected", and leaving one of them would make the
      // key mean half of what it says.
      if (bound?.action === "deselect") {
        event.preventDefault();
        if (selection.length > 0) onSelect([]);
        if (marquee.current !== null) {
          marquee.current = null;
          requestOverlay();
        }
        return;
      }

      // Enter closes a gesture built up click by click — the polygon and the
      // curve, which have no pointer-up to end them, and the dividers, which
      // are the same gesture with a running total.
      if (event.key === "Enter") {
        if (endMeasuring()) return;
        finishGestureRef.current();
        return;
      }

      // Escape abandons a gesture in progress before it abandons the tool: the
      // first press is what a half-drawn polygon needs, and dropping the tool
      // as well would be two steps at once.
      if (event.key === "Escape") {
        if (shapeModeRef.current !== null) { onExitShapeEditing?.(); return; }
        if (gestureRef.current) {
          gestureRef.current = null;
          nodeDrag.current = false;
          abandonRef.current();
          return;
        }
        if (regionDrag.current) {
          regionDrag.current = null;
          requestOverlay();
          return;
        }
        // A running capture is the biggest thing Escape can be about: while it
        // runs, every edit in the application is refused (spec.md 8.7).
        if (recording !== null) {
          setCaptureName(null);
          void api
            .cancelCapture()
            .then(() => {
              setRecording(null);
              requestOverlay();
            })
            .catch(() => undefined);
          return;
        }
        // A chain being built ends before the tool does, for the same reason a
        // half-drawn polygon does: the first press is what the gesture needs.
        if (endMeasuring()) return;
        setTool(HAND);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [
    endMeasuring,
    recording,
    nudgeCamera,
    nudgeSelection,
    onProjectChanged,
    palette,
    requestOverlay,
    selectRegion,
    setError,
    settings,
  ]);

  // The selection's handles, so the map shows what the panels are pointing at
  // and where a drag would act.
  const selectionKey = selection.join(",");
  useEffect(() => {
    if (selection.length === 0) {
      setTransform(null);
      return;
    }
    let cancelled = false;
    const revision = project.revision;
    api
      .selectionTransform(selection, step)
      .then((value) => {
        if (cancelled) return;
        transformRevision.current = revision;
        setTransform(value);
      })
      .catch(() => { if (!cancelled) setTransform(null); });
    return () => {
      cancelled = true;
    };
    // `selectionKey` rather than `selection`: a new array of the same ids is a
    // new value every render and would refetch forever.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project.revision, selectionKey, step]);

  /**
   * Every visible operator's edge: the masks and the modifiers (spec.md 6.2,
   * 6.3).
   *
   * None of them paints a field of its own, so there is nothing on the map to
   * say where one is, which side of its edge it covers, or that a click landed
   * on one. The map draws the edge — for a selected one, and for the one under
   * the pointer while such a tool is in hand.
   *
   * Fetched only when one of those two could be drawn, and only per revision
   * and per step. The pointer hit test below runs on the outline this already
   * holds; asking the backend per pointer report would be a round trip per
   * frame for an answer that does not change between them.
   */
  const [outlineList, setOutlineList] = useState<OperatorOutline[]>([]);
  // The tool in hand, for the edge under the pointer — the brush included, so
  // hovering a stroke says which one it is and where it ends (spec.md 6.1).
  // Plus the selection, so a selected operator is outlined whatever tool is in
  // hand. Nothing else: the answer is bounded by one tool's objects rather than
  // by the size of the project.
  const hoverTool = drawsObjects(tool) && HOVER_TOOLS.has(tool) ? tool : null;
  // The eraser acts on anything in the active layer, so it sees every object
  // there, whatever its tool (spec.md 8.1, M28).
  const erasing = tool === ERASE;
  useEffect(() => {
    if (hoverTool === null && selection.length === 0 && !erasing) {
      setOutlineList([]);
      return;
    }
    let cancelled = false;
    api
      .objectOutlines(step, hoverTool, selection, activeLayer, erasing)
      .then((outlines) => {
        if (!cancelled) setOutlineList(outlines);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
    // `selectionKey` stands in for the array, which is new every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project.revision, step, hoverTool, selectionKey, erasing, activeLayer]);

  /**
   * The operator whose edge the pointer is on, if any.
   *
   * A ref and not state: it changes with every pointer report, and this
   * component is two thousand lines of hooks (see `createReadoutStore`). The
   * overlay is asked to redraw when the answer actually changes.
   */
  const hoveredOperator = useRef<number | null>(null);
  /**
   * The eraser's sweep (spec.md 8.1, M28): every object the pointer has
   * passed over since it went down, and whether `Ctrl` was held — this frame
   * only — when it did. Committed as one write on release, so one undo
   * returns the whole sweep; marked in pink meanwhile, so the hand sees what
   * it has taken as it takes it.
   */
  const eraseDrag = useRef<{
    points: Array<[number, number]>;
    step: number | null;
    radiusKm: number;
  } | null>(null);
  /**
   * The eraser's brush (spec.md 8.1, M29): a size in km or px, a disc or a
   * square, a feather — the brush's own options, since the eraser is a
   * brush that takes away. Local state: the eraser makes no object for a
   * schema to describe.
   */
  const [eraser, setEraser] = useState<{
    size: number;
    unit: "km" | "px";
    shape: BrushShape;
    feather: number;
  }>({ size: 400, unit: "km", shape: "circle", feather: 0 });
  const eraserRef = useRef(eraser);
  eraserRef.current = eraser;
  const operatorOutlinesRef = useRef<OperatorOutline[]>([]);
  operatorOutlinesRef.current = outlineList;

  useEffect(() => {
    drawOverlay();
  }, [committedTransform]);

  useEffect(() => {
    projectRef.current = project;
    requestDraw();
  }, [project, requestDraw]);
  // The preview is served under its own revision (D71): its tiles are other
  // tiles, so the map redraws when it comes, moves and goes.
  useEffect(() => {
    requestDraw();
  }, [recording, requestDraw]);

  // Changing tools disarms a pick and abandons a gesture in progress: the
  // panel that shows a pick is armed goes away with the tool, and a half-drawn
  // polygon has no meaning under a different one.
  useEffect(() => {
    setToolPick(null);
    gestureRef.current = null;
    abandonRef.current();
  }, [tool]);

  /**
   * The viewport's unique tiles, rebuilt only when the camera or the view
   * changes.
   *
   * Playback preparation and presentation reuse this list for the current
   * viewport. Both refs are replaced wholesale rather than mutated (`panBy`,
   * `zoomAbout`, `clampCamera` all return new cameras), so identity is a sound
   * key between draws. A mesh completion invalidates it separately, and
   * every draw refreshes it from the actual visible tiles.
   */
  const warmTilesRef = useRef<{
    camera: Camera;
    view: Viewport;
    tiles: ReturnType<typeof uniqueTiles>;
  } | null>(null);
  const viewportTiles = useCallback(() => {
    const camera = cameraRef.current;
    const view = viewRef.current;
    const held = warmTilesRef.current;
    if (held && held.camera === camera && held.view === view) return held.tiles;
    const tiles = uniqueTiles(visibleTiles(camera, view));
    warmTilesRef.current = { camera, view, tiles };
    return tiles;
  }, []);

  // What the timeline asks of the map (spec.md 9.4).
  const warm = useCallback(
    (target: number): boolean => {
      const tiles = tilesRef.current;
      if (!tiles) return true;
      return tiles.prefetch(frameOf(target), viewportTiles());
    },
    [viewportTiles],
  );
  const prepare = useCallback((request: PreparationRequest) => {
    const cache = tilesRef.current;
    const view = viewportTiles();
    if (!cache || view.length === 0) return { ready: 0, total: 1, streaming: false };
    cache.reserve(projectRef.current.step_count, view.length);
    const plan = preparationTargets(request, view.length, cache.capacity, cache.preparationMs);
    const protect = plan.protected.map(frameOf);
    if (shownFrameRef.current) protect.push(shownFrameRef.current);
    cache.protect(protect, view);
    const frames = plan.targets.filter((step) => request.states[step] === "solid").map(frameOf);
    const ready = cache.prepare(frames, view);
    // Keep the existing diagnostic until the reported persistent 2/3 stall
    // reproduces. Include residency/failures to distinguish it from a wait
    // for the backend or a projection viewport that changed underneath it.
    playbackPreparation({
      targets: [...plan.targets],
      states: plan.targets.map((step) => request.states[step] ?? "(missing)"),
      framesAsked: frames.length,
      ready,
      total: plan.targets.length,
      streaming: plan.streaming,
      tilesPerFrame: view.length,
      capacity: cache.capacity,
      statesLength: request.states.length,
      resident: ready < plan.targets.length
        ? plan.targets.map((step) => cache.residentCount(frameOf(step), view)) : undefined,
      cache: ready < plan.targets.length ? cache.stats() : undefined,
    });
    return { ready, total: plan.targets.length, streaming: plan.streaming };
  }, [viewportTiles]);
  const present = useCallback((target: number): boolean => {
    const cache = tilesRef.current;
    const view = viewportTiles();
    if (!cache || view.length === 0 || !cache.prefetch(frameOf(target), view)) return false;
    stepRef.current = target;
    draw();
    return shownFrameRef.current === frameOf(target);
  }, [draw, viewportTiles]);
  const bounds = useCallback((): [number, number, number, number] | null => {
    const view = viewRef.current;
    if (view.width <= 1 || view.height <= 1) return null;
    const seen = visibleBounds(cameraRef.current, view);
    return [seen.west, seen.north, seen.east, seen.south];
  }, []);
  const focus = useCallback(
    (lon: number, lat: number, pxPerDeg?: number) => {
      cameraRef.current = clampCamera(
        {
          ...cameraRef.current,
          centerLon: lon,
          centerLat: lat,
          pxPerDeg: pxPerDeg ?? cameraRef.current.pxPerDeg,
        },
        viewRef.current,
      );
      requestDraw();
    },
    [requestDraw],
  );
  /**
   * Arms the image alignment mode for `layer` (Task 7): the button beside
   * Opacity in `ImageControls`. Every click on the map is a control point's
   * picture or map half until `Enter`, `Escape` or `Backspace` ends it.
   */
  const beginAlign = useCallback(
    (layer: number) => {
      alignStore.current!.begin(layer);
      setHint(
        "Click a feature in the picture, then the same place on the map. Repeat as often as " +
          "you like, then press Enter. Escape cancels; Backspace drops the last pair.",
      );
      requestOverlay();
    },
    [requestOverlay],
  );
  /** Whether the alignment mode is armed, for a key that must mean something else while it is. */
  const isAligning = useCallback(() => alignStore.current!.get().layer !== null, []);
  const clearRegion = useCallback(() => {
    setRegion((current) => {
      if (current !== null) requestOverlay();
      return null;
    });
  }, [requestOverlay]);
  const copyRegion = useCallback((): boolean => {
    if (region === null) return false;
    // The visible composite inside the region, from this step to the end of
    // the timeline (spec.md 8.5, D65).
    void api
      .captureRegion(regionShape(region), stepRef.current, activeKindRef.current)
      .catch((err: unknown) => setError(String(err)));
    return true;
  }, [region]);
  const pasteCapture = useCallback(
    (layer: number | null, still: boolean) => {
      // Under the pointer when it is over the map, because that is where the
      // user is pointing; otherwise back where it was taken, nudged.
      const at = cursorRef.current
        ? unproject(cameraRef.current, viewRef.current, cursorRef.current)
        : null;
      void api
        .pasteCapture(at?.lon ?? null, at?.lat ?? null, stepRef.current, layer, still)
        .then(onProjectChanged)
        .catch((err: unknown) => setError(String(err)));
    },
    [onProjectChanged],
  );
  const setCapture = useCallback(
    (mode: CaptureMode) => {
      setRecording(mode.active ? mode : null);
      requestOverlay();
    },
    [requestOverlay],
  );
  useImperativeHandle(
    ref,
    () => ({
      warm,
      prepare,
      present,
      bounds,
      clearRegion,
      copyRegion,
      pasteCapture,
      setCapture,
      focus,
      beginAlign,
      isAligning,
    }),
    [
      beginAlign,
      bounds,
      clearRegion,
      copyRegion,
      focus,
      isAligning,
      pasteCapture,
      setCapture,
      warm,
      prepare,
      present,
    ],
  );

  // A project change can shorten the timeline.
  useEffect(() => {
    if (step > lastStep) onStepChange(lastStep);
  }, [lastStep, onStepChange, step]);

  /**
   * The map projection, taken from the settings onto the camera (M11).
   *
   * It rides on the camera because everything that needs it — the renderer,
   * the pointer, the overlay — is already handed one. Re-clamped on the way,
   * since a projection change moves where the poles are and the current centre
   * may no longer be a legal one.
   */
  useEffect(() => {
    const wanted = projectionOf(
      (settings?.projection ?? DEFAULT_PROJECTION) as ProjectionId,
    ).id;
    if (cameraRef.current.projection === wanted) return;
    cameraRef.current = cameraForProjection(cameraRef.current, viewRef.current, wanted);
    requestDraw();
  }, [requestDraw, settings?.projection]);

  /**
   * A projection that cannot be aligned on cancels an alignment under way.
   *
   * The panel offers the button on the cylindrical maps only (spec.md 4.9),
   * so switching to the globe mid-session would otherwise leave the mode
   * armed and swallowing every click with nothing on screen to say why, and
   * no way to reach the button that would have turned it off. Cancelling
   * discards the pairs rather than writing them — the session was never
   * committed, and `Escape` does the same thing.
   */
  useEffect(() => {
    const armed = alignStore.current?.get().layer ?? null;
    if (armed === null) return;
    if (!projectionOf((settings?.projection ?? DEFAULT_PROJECTION) as ProjectionId).general) return;
    alignStore.current?.cancel();
    setHint("");
    reportError(null);
    requestOverlay();
  }, [requestOverlay, settings?.projection]);

  // Mirror display state into the refs `draw` reads, then redraw.
  useEffect(() => {
    glyphSettingsRef.current = settings?.glyphs ?? DEFAULT_GLYPHS;
    requestDraw();
  }, [requestDraw, settings?.glyphs]);

  // Mirror display state into the refs `draw` reads, then redraw.
  useEffect(() => {
    const changed = stepRef.current !== step || showGlyphsRef.current !== showGlyphs
      || showGraticuleRef.current !== showGraticule || showChartsRef.current !== showCharts
      || showOsmRef.current !== showOsm;
    showGlyphsRef.current = showGlyphs;
    showGraticuleRef.current = showGraticule;
    showChartsRef.current = showCharts;
    showOsmRef.current = showOsm;
    stepRef.current = step;
    if (changed) requestDraw();
  }, [requestDraw, step, showGlyphs, showGraticule, showCharts, showOsm]);

  /**
   * Draws preview glyphs at a set of geographic positions.
   *
   * Every glyph goes into one path, stroked once: at a fine lattice a stroke
   * can cover hundreds of them, and a `stroke()` call each would cost more than
   * working out where they go.
   */
  const drawGlyphs = useCallback(
    (
      context: CanvasRenderingContext2D,
      at: ReadonlyArray<readonly [number, number]>,
      knots: number,
      azimuthAt: (lon: number, lat: number) => number,
      dpr: number,
    ) => {
      if (at.length === 0) return;
      const camera = cameraRef.current;
      const view = viewRef.current;
      // The map's lattice, and this kind's length on it (M31), held to the
      // size the style asks for where the lattice is wider than it wanted
      // (M33) — the same cap the map's own glyphs take.
      const appearance = glyphSettingsRef.current[glyphStyle];
      const { lengthPx } = glyphDisplayLayout(glyphStyle, camera.pxPerDeg, dpr, appearance);
      const width = appearance.stroke_width_px * dpr;

      const strokes = new Path2D();
      const fills = new Path2D();

      for (const [lon, lat] of at) {
        const point = toScreen(camera, view, { lon, lat });
        if (!Number.isFinite(point.x)) continue;
        let angle = azimuthAt(lon, lat);
        if (projectionFor(camera).general) {
          const ahead = toScreen(camera, view, destination({lon, lat}, angle, 100));
          if (!Number.isFinite(ahead.x)) continue;
          angle = Math.atan2(ahead.x - point.x, point.y - ahead.y) * 180 / Math.PI;
        }
        const geometry = glyphGeometry(
          glyphStyle,
          angle,
          knots,
          lengthPx,
          lat,
          width,
          appearance.size_percent / 100,
        );

        const trace = (
          path: Path2D,
          points: ReadonlyArray<readonly [number, number]>,
        ) => {
          path.moveTo(point.x + points[0]![0], point.y + points[0]![1]);
          for (let i = 1; i < points.length; i++) {
            path.lineTo(point.x + points[i]![0], point.y + points[i]![1]);
          }
        };

        for (const line of geometry.lines) trace(strokes, line);
        for (const shape of geometry.fills) {
          trace(fills, shape);
          fills.closePath();
        }
      }

      context.save();
      context.globalAlpha *= glyphOpacity(appearance, knots / 1.9438444924406046);
      context.lineWidth = width;
      context.lineCap = "round";
      context.lineJoin = "round";
      if (appearance.shadow.enabled) {
        context.save();
        context.globalAlpha *= appearance.shadow.opacity_percent / 100;
        context.translate(appearance.shadow.offset_x_px * dpr, appearance.shadow.offset_y_px * dpr);
        context.strokeStyle = appearance.shadow.color;
        context.fillStyle = appearance.shadow.color;
        context.stroke(strokes);
        context.fill(fills);
        context.restore();
      }
      context.strokeStyle = appearance.color;
      context.fillStyle = appearance.color;
      context.stroke(strokes);
      context.fill(fills);
      context.restore();
    },
    [glyphStyle],
  );

  /**
   * The field a gesture will paint: its footprint in the speed colour, with
   * direction glyphs over it.
   *
   * Glyphs sit on the map's own globe-anchored lattice, so what the preview
   * shows is what the field will show once the gesture is committed -- same
   * positions, same mark, same direction (spec.md 6.1).
   *
   * Generic over the footprint, so a tool previews the field it paints by
   * saying what it paints. There is no per-tool preview to write, and so no
   * per-tool preview to get wrong.
   */
  const drawFieldPreview = useCallback(
    (context: CanvasRenderingContext2D, preview: FieldPreview, dpr: number) => {
      const camera = cameraRef.current;
      const view = viewRef.current;

      // Every piece is its own subpath, so filling once merges overlapping
      // footprints into a single silhouette instead of drawing a chain of
      // outlines on top of each other -- and leaves a ring its hole.
      const { stepDeg } = glyphDisplayLayout(glyphStyle, camera.pxPerDeg, dpr, glyphSettingsRef.current[glyphStyle]);
      const fineStep = stepDeg / GLYPH_SUBDIVISIONS;
      const candidateLimit = MAX_PREVIEW_GLYPHS * GLYPH_SUBDIVISIONS ** 2;
      const footprint = preview.footprint;
      let covered: Array<[number, number]>;

      if (footprint.kind === "swept") {
        // A stroke gains a point per pointer report. Its path and lattice are
        // extended by the new segment rather than rebuilt from the first point
        // — the same walk, resumed — so a long stroke costs the same per
        // report as a short one. The key holds everything the geometry depends
        // on besides the points; a zoom mid-stroke rebuilds from nothing.
        const key = [
          camera.centerLon, camera.centerLat, camera.pxPerDeg, camera.projection,
          view.width, view.height,
          footprint.radiusKm, footprint.shape, footprint.space,
          stepDeg, MAX_PREVIEW_GLYPHS,
        ].join("|");
        let entry = sweptCache.current.get(footprint.points);
        if (!entry || entry.key !== key) {
          entry = { key, path: new Path2D(), progress: freshSweptPath(), lattice: freshLattice() };
          sweptCache.current.set(footprint.points, entry);
        }
        extendStrokePath(
          entry.path, camera, view, footprint.points, entry.progress,
          footprint.radiusKm, footprint.shape, footprint.space,
        );
        context.fillStyle = preview.paint;
        context.fill(entry.path);
        if (!showGlyphs) return;
        covered = extendLatticeUnderStroke(
          footprint.points, entry.lattice, footprint.radiusKm, fineStep,
          candidateLimit, footprint.shape, footprint.space,
        );
      } else {
        const region = new Path2D();
        buildFootprintPath(region, camera, view, footprint);
        context.fillStyle = preview.paint;
        context.fill(region);
        if (!showGlyphs) return;
        covered = latticeUnder(footprint, fineStep, candidateLimit);
      }
      covered = spacedGlyphs(covered.map(([lon, lat]) => ({
        lon: camera.centerLon + normalizeLon(lon - camera.centerLon), lat,
      })), stepDeg, camera).slice(0, MAX_PREVIEW_GLYPHS).map(({ lon, lat }) => [lon, lat]);
      // A footprint narrower than the lattice can cover no point at all. It
      // still has a direction, and a preview showing none of it is worse than
      // one glyph off the lattice, so a point on the shape stands in.
      const head = footprintHead(preview.footprint);
      const at = covered.length > 0 ? covered : head === null ? [] : [head];
      if (at.length === 0) return;
      drawGlyphs(context, at, preview.knots, preview.azimuthAt, dpr);
    },
    [drawGlyphs, glyphStyle, showGlyphs],
  );

  /**
   * One mask's edge as a screen path.
   *
   * The same builder the drag outline and the footprint preview use, so what
   * is drawn, what is hit-tested and what the object actually covers are one
   * shape rather than three that resemble each other.
   */
  const maskPath = useCallback((outline: ObjectOutline, insetPx = 0): Path2D => {
    const path = new Path2D();
    for (const footprint of footprintOfOutline(outline)) {
      buildFootprintPath(path, cameraRef.current, viewRef.current, footprint, insetPx);
    }
    return path;
  }, []);

  /**
   * Draws a band along the *outline of a footprint's union*, `widthCss` wide.
   *
   * A footprint is a union of stamps and `Path2D` has no union operator, so
   * stroking one traces every stamp's own circle and leaves a chain of rings
   * where a single edge belongs. Filling it and then knocking out a copy inset
   * by the band's width leaves exactly the union's boundary: the outer fill
   * covers the shape, the inset fill removes everything but the rim.
   *
   * `destination-out` erases what is under it, so this must run before anything
   * else is drawn on the frame. A ring outline is a single closed polygon with
   * no union to take, and is stroked.
   */
  const drawEdgeBand = useCallback(
    (
      context: CanvasRenderingContext2D,
      outline: ObjectOutline,
      colour: string,
      widthCss: number,
      dpr: number,
      erased: readonly ObjectOutline[] = [],
    ) => {
      const width = Math.max(1, widthCss * dpr);
      // `destination-out` removes destination alpha in proportion to the
      // *source's*, so anything knocking a hole in the band has to be opaque
      // (M79). Reusing the band's own colour — 0.95, or 0.16 for the wide
      // faint one — removed only that fraction and left the rest of it lying
      // over everything the knockout was meant to clear: a wash of the band's
      // colour across the object's whole interior, invisible over the field
      // it covers and plain to see wherever the eraser had taken the field
      // away. The colour is for what is drawn; what is erased takes this.
      const ERASING = "#000";
      context.save();
      if (outline.kind === "ring" || outline.kind === "contours") {
        context.strokeStyle = colour;
        context.lineWidth = width;
        context.stroke(maskPath(outline));
        // What the eraser took is no longer there to outline (M33).
        context.globalCompositeOperation = "destination-out";
        context.fillStyle = ERASING;
        for (const hole of erased) context.fill(maskPath(hole, width / 2));
        context.restore();
        return;
      }
      // Centred on the edge, half out and half in, which is where a stroke of
      // the same width would put it — a band that sat entirely inside read as
      // an outline shrunk away from the paint it belongs to.
      context.fillStyle = colour;
      context.fill(maskPath(outline, -width / 2));
      context.globalCompositeOperation = "destination-out";
      context.fillStyle = ERASING;
      context.fill(maskPath(outline, width / 2));
      // The eraser takes pieces out of an object without changing the shape
      // it was drawn with (spec.md 8.1), so the edge has to be told: the band
      // is cut where a piece is gone, and the piece's own rim is drawn inside
      // the object in its place (M33).
      for (const hole of erased) context.fill(maskPath(hole, width / 2));
      context.restore();
      if (erased.length === 0) return;
      // The rim of the *union* of the holes, by the same trick the object's
      // own band uses above: every hole grown, then every hole inset knocked
      // out of that. Taken one hole at a time it was the rim of each, which
      // is a different shape — a stroke crossing ground an earlier stroke had
      // already taken drew its edge there anyway, a line with no field behind
      // it, and only ever on the later of the two because a hole carved only
      // its own inside (M78). The union has one boundary and no interior
      // edges, and it does not depend on the order they were drawn in.
      context.save();
      context.clip(maskPath(outline, width / 2));
      context.fillStyle = colour;
      context.globalCompositeOperation = "source-over";
      for (const hole of erased) context.fill(maskPath(hole, -width / 2));
      context.globalCompositeOperation = "destination-out";
      context.fillStyle = ERASING;
      for (const hole of erased) context.fill(maskPath(hole, width / 2));
      context.restore();
    },
    [maskPath],
  );

  /**
   * The selection's footprints where a drag would leave them.
   *
   * Drawn as a selection affordance rather than as a field: it says where the
   * objects are going, and it has no speed or direction to show — those come
   * back with the tiles once the drag lands.
   */
  const drawDragOutlines = useCallback(
    (context: CanvasRenderingContext2D, outlines: ObjectOutline[], dpr: number) => {
      // The perimeter, as a band (M24). Stroking the silhouette traced every
      // stamp of a swept chain — the ring trail the static outline was cured
      // of — so the drag outline goes through the same band: the union's
      // boundary, and nothing inside it. Bands use `destination-out`, so they
      // are drawn before the faint fill that says which side is the object.
      for (const outline of outlines) {
        drawEdgeBand(context, outline, "rgba(255, 214, 102, 0.85)", 1.5, dpr);
      }
      context.save();
      context.fillStyle = "rgba(255, 214, 102, 0.14)";
      for (const outline of outlines) {
        context.fill(maskPath(outline, Math.max(1, 1.5 * dpr) / 2), outline.kind === "contours" ? "evenodd" : "nonzero");
      }
      context.restore();
    },
    [drawEdgeBand, maskPath],
  );

  /**
   * The handles as they are on screen right now.
   *
   * A drag in flight, then a drag whose field has not landed, then the
   * selection as the document has it. Drawing and hit-testing both read this,
   * and only this: the bug it closes is a handle drawn in one place and grabbed
   * in another, which is what happened when the overlay followed the preview
   * and the hit test followed the document.
   */
  const shownTransform = useCallback(
    (): SelectionTransform | null =>
      dragPreview.current?.handles ?? settlingDrag.current?.handles ?? committedTransform,
    [committedTransform],
  );

  /**
   * Tool feedback, drawn on a 2D canvas over the WebGL one.
   *
   * Separate from the renderer because it changes on every pointer move and has
   * nothing to do with the field: mixing it into the GL pass would mean
   * redrawing the whole map to move a cursor outline.
   */
  /**
   * A macro's footprint at a pointer: its region outline, dashed, where a
   * click would put it — and, for one that recorded movement, **where it
   * goes**: the outline again at every keyframe of its track, fainter, with
   * the path its centre follows drawn through them (spec.md 8.7, M27). The
   * keys are read off the track (`trackKeyframes`): a saved macro holds no
   * keys, only frames, and the frames bend only where a key was placed.
   * Static: the hover says the whole motion at once, and nothing animates
   * under the pointer. What the insert tool's hover and the preview's stamp
   * hover both draw.
   */
  const drawMacroFootprint = useCallback(
    (
      context: CanvasRenderingContext2D,
      ring: Array<[number, number]>,
      track: Array<[number, number]> | null,
      at: { lon: number; lat: number },
      dpr: number,
    ) => {
      const camera = cameraRef.current;
      const view = viewRef.current;
      const outlineAt = (dx: number, dy: number, stroke: string, fill: string | null) => {
        const outline = ring.map((p) =>
          toScreen(camera, view, { lon: p[0] + dx, lat: p[1] + dy }),
        );
        const first = outline[0];
        if (!first) return;
        context.beginPath();
        context.moveTo(first.x, first.y);
        for (const point of outline.slice(1)) context.lineTo(point.x, point.y);
        context.closePath();
        if (fill !== null) {
          context.fillStyle = fill;
          context.fill();
        }
        context.strokeStyle = stroke;
        context.stroke();
      };
      context.save();
      context.lineWidth = Math.max(1, dpr);
      context.setLineDash([6 * dpr, 4 * dpr]);
      if (track && track.length > 1) {
        // The later keys first, so the box at the pointer is drawn on top.
        const keys = trackKeyframes(track);
        for (const index of keys.slice(1).reverse()) {
          const [dx, dy] = track[index] as [number, number];
          outlineAt(dx, dy, "rgba(150, 255, 200, 0.55)", null);
        }
      }
      outlineAt(0, 0, "rgba(150, 255, 200, 0.95)", "rgba(140, 255, 190, 0.08)");
      context.setLineDash([]);
      if (track && track.length > 1) {
        // The path the centre follows, through every frame, so a curve reads
        // as a curve and not as a chord between its keys.
        const path = track.map(([dx, dy]) =>
          toScreen(camera, view, { lon: at.lon + dx, lat: at.lat + dy }),
        );
        const start = path[0];
        if (start) {
          context.beginPath();
          context.moveTo(start.x, start.y);
          for (const point of path.slice(1)) context.lineTo(point.x, point.y);
          context.strokeStyle = "rgba(150, 255, 200, 0.75)";
          context.stroke();
        }
      }
      context.restore();
    },
    [],
  );

  const drawOverlay = useCallback(() => {
    const canvas = overlayRef.current;
    const context = canvas?.getContext("2d");
    if (!canvas || !context) return;

    const view = viewRef.current;
    context.clearRect(0, 0, view.width, view.height);

    const camera = cameraRef.current;
    const dpr = window.devicePixelRatio || 1;

    // The macro preview (spec.md 8.7, M27) shows the macro and nothing of the
    // document: no edges, handles, measurements, image corners or region.
    // The pointer carries the macro's region outline and its track, as the
    // insert tool's does, since a click stamps it there; the green frame
    // round the map is CSS.
    const stamped = recordingRef.current;
    if (stamped?.phase === "previewing") {
      const at = cursorRef.current;
      if (at && region !== null) {
        const geo = unproject(camera, view, at);
        drawMacroFootprint(
          context,
          regionRing(recentred(region, geo.lon, geo.lat)),
          stamped.track.length > 1 ? stamped.track : null,
          geo,
          dpr,
        );
      }
      return;
    }

    if (shapeEditing !== null) {
      const controls = shapeControls.current;
      if (controls && controls.object === shapeEditing && controls.step === step) {
        const outline: ObjectOutline = { kind: "contours", rings: controls.rings };
        const cuts = outlineList.find((o) => o.object === shapeEditing)?.erased ?? [];
        drawEdgeBand(context, outline, "#ffd666", 1.5, dpr, cuts);
        for (let r = 0; r < controls.rings.length; r++) {
          for (let p = 0; p < controls.rings[r]!.length; p++) {
            const [lon, lat] = controls.rings[r]![p]!;
            const at = toScreen(camera, view, { lon, lat });
            context.beginPath();
            context.arc(at.x, at.y, 4 * dpr, 0, Math.PI * 2);
            context.fillStyle = controls.keyed[r]?.[p] ? "#ffd666" : "#1c2638";
            context.strokeStyle = "#ffd666";
            context.lineWidth = 1.5 * dpr;
            context.fill(); context.stroke();
          }
        }
      }
      return;
    }

    // Object edges (spec.md 6.1, 6.2, 6.3): the one under the pointer, and any
    // selected object with no field of its own to show where it is.
    //
    // **Drawn first, on the cleared canvas, deliberately.** An edge band is
    // made by knocking an inset copy out of a filled footprint, and
    // `destination-out` erases whatever is already on the canvas — first is the
    // one place where that is only ever the band's own interior. A drag's
    // outlines are bands too, so they come first of all.
    const live = dragPreview.current ?? settlingDrag.current;
    const dragLive = live !== null;
    if (live) drawDragOutlines(context, live.outlines, dpr);
    for (const outlined of outlineList) {
      const hovered = hoveredOperator.current === outlined.object;
      // The brush included: its field says roughly where it is, but a
      // selected stroke gets the same edge every other tool gets (M24).
      // While a drag is live the moving outline stands in for the selected
      // edge, which would otherwise mark where the object *was*.
      const selected = selection.includes(outlined.object) && !dragLive;
      if (!hovered && !selected) continue;
      // Pink for the edge under the pointer: a colour used for nothing else on
      // this map, so "the tool has found an edge" cannot be mistaken for a
      // selection or a preview.
      drawEdgeBand(
        context,
        outlined.outline,
        hovered ? "rgba(255, 110, 190, 0.95)" : "rgba(255, 214, 102, 0.85)",
        hovered ? 2.5 : 1.5,
        dpr,
        outlined.erased,
      );
      // An inverted mask covers everything *but* this, so a wide faint band
      // goes with it: an edge alone cannot say which side is covered.
      if (outlined.inverted) {
        drawEdgeBand(
          context,
          outlined.outline,
          "rgba(255, 110, 190, 0.16)",
          9,
          dpr,
          outlined.erased,
        );
      }
    }

    // The field the drag is carrying (M84).
    //
    // The document is not written until the pointer comes up, so the tiles
    // still hold the object where it was: without this the outline follows the
    // pointer and the object sits still, which for a macro — where the edge
    // and the field inside it are visibly two different things — reads as the
    // drag doing nothing. Evaluate the selected objects when the drag begins
    // and transform that transparent image until the committed tiles arrive.
    //
    // **After the edge bands, never before.** A band is made by knocking an
    // inset copy out of a filled footprint with `destination-out`, which would
    // take the ghost's interior with it.
    const carried = ghost.current;
    if (live && carried) {
      // Where it is leaving, dimmed: two copies of one object is a worse
      // reading of the gesture than none, and the shade says which is which.
      context.save();
      const vacated = new Path2D();
      for (const outlined of ghostSource.current) vacated.addPath(maskPath(outlined.outline));
      context.fillStyle = "rgba(9, 14, 24, 0.55)";
      context.fill(vacated);
      context.restore();

      const to = live.handles;
      const matrix = ghostMatrix(carried.from, {
        pivot: toScreen(camera, view, { lon: to.lon, lat: to.lat }),
        rotationDeg: to.rotation_deg,
        radiusM: to.radius_m,
      });
      context.save();
      const arriving = new Path2D();
      // Inset by the band, so the copy sits inside the outline rather than
      // over the edge that is showing where it lands.
      for (const outline of live.outlines) arriving.addPath(maskPath(outline, 1.5 * dpr));
      context.clip(arriving);
      // Keep the selected field slightly translucent to identify the preview.
      context.globalAlpha = 0.85;
      context.transform(matrix[0], matrix[1], matrix[2], matrix[3], matrix[4], matrix[5]);
      context.drawImage(carried.image, carried.left, carried.top);
      context.restore();
    }

    // A warp being pulled: from its anchor to the pointer, which is the push
    // the release will write (spec.md 6.3).
    const pull = pushDrag.current;
    if (pull) {
      const from = toScreen(camera, view, pull.from);
      const to = toScreen(camera, view, pull.to);
      context.save();
      context.strokeStyle = "rgba(255, 110, 190, 0.95)";
      context.lineWidth = Math.max(1, dpr) * 2;
      context.beginPath();
      context.moveTo(from.x, from.y);
      context.lineTo(to.x, to.y);
      context.stroke();
      // A head at the destination and a ring at the origin: which end is which
      // is the whole meaning of the gesture.
      context.beginPath();
      context.arc(from.x, from.y, 4 * dpr, 0, Math.PI * 2);
      context.stroke();
      context.beginPath();
      context.arc(to.x, to.y, 5 * dpr, 0, Math.PI * 2);
      context.fillStyle = "rgba(255, 110, 190, 0.95)";
      context.fill();
      // How far the pull is, beside its head (spec.md 6.3, M17): a push is
      // aimed by eye, and the number says what the eye chose.
      const distance = units.distanceFromKm(distanceM(pull.from, pull.to) / 1000);
      context.font = `${11 * dpr}px system-ui, sans-serif`;
      context.textAlign = "left";
      context.textBaseline = "bottom";
      context.fillText(
        `${distance >= 100 ? Math.round(distance) : distance.toFixed(1)} ${units.distanceUnit}`,
        to.x + 8 * dpr,
        to.y - 8 * dpr,
      );
      context.restore();
    }

    // While a drag is in flight the handles follow it rather than the document,
    // which is deliberately not being written until the pointer comes up.
    // The same answer the hit test uses, so a handle is grabbed where it is drawn.
    const transform = shownTransform();

    // Handles are drawn whatever tool is active: what the panels are editing
    // does not stop mattering while painting.
    if (transform) {
      const dpr = window.devicePixelRatio || 1;
      const anchor = { lon: transform.lon, lat: transform.lat };
      const at = toScreen(camera, view, anchor);

      const handleAt = (bearing: number) =>
        toScreen(
          camera,
          view,
          destination(anchor, bearing, transform.radius_m),
        );
      const rotateAt = handleAt(transform.rotation_deg);
      const scaleAt = handleAt(transform.rotation_deg + 90);

      // The object's reach, as an ellipse — a ground circle is not a screen
      // circle away from the equator.
      const { rx, ry } = footprintRadii(camera, transform.lat, transform.radius_m / 1000);
      context.strokeStyle = "rgba(255, 214, 102, 0.45)";
      context.lineWidth = Math.max(1, dpr);
      context.setLineDash([6 * dpr, 5 * dpr]);
      context.beginPath();
      context.ellipse(at.x, at.y, rx, ry, 0, 0, Math.PI * 2);
      context.stroke();
      context.setLineDash([]);

      // Spokes to the handles, so it is clear what they turn about.
      context.strokeStyle = "rgba(255, 214, 102, 0.55)";
      context.beginPath();
      context.moveTo(at.x, at.y);
      context.lineTo(rotateAt.x, rotateAt.y);
      context.moveTo(at.x, at.y);
      context.lineTo(scaleAt.x, scaleAt.y);
      context.stroke();

      const arm = 9 * dpr;
      context.strokeStyle = "rgba(255, 214, 102, 0.95)";
      context.lineWidth = Math.max(1.5, dpr * 1.5);
      context.beginPath();
      context.moveTo(at.x - arm, at.y);
      context.lineTo(at.x + arm, at.y);
      context.moveTo(at.x, at.y - arm);
      context.lineTo(at.x, at.y + arm);
      context.stroke();

      const knob = (point: { x: number; y: number }, fill: string) => {
        context.beginPath();
        context.arc(point.x, point.y, HANDLE_RADIUS_CSS * dpr, 0, Math.PI * 2);
        context.fillStyle = fill;
        context.fill();
        context.strokeStyle = "rgba(20, 28, 44, 0.9)";
        context.lineWidth = Math.max(1, dpr);
        context.stroke();
      };
      knob(rotateAt, "rgba(255, 214, 102, 0.95)");
      knob(scaleAt, "rgba(111, 217, 255, 0.95)");

      // A ring on the centre says the anchor itself can be dragged, which is
      // the only way to tell it apart from a plain crosshair. A group has no
      // anchor to repin, so its centre is left as the move target.
      if (transform.count === 1) {
        context.beginPath();
        context.arc(at.x, at.y, HANDLE_RADIUS_CSS * dpr, 0, Math.PI * 2);
        context.strokeStyle = "rgba(255, 214, 102, 0.85)";
        context.lineWidth = Math.max(1, dpr);
        context.stroke();
      } else {
        context.fillStyle = "rgba(255, 214, 102, 0.95)";
        context.font = `${11 * dpr}px system-ui, sans-serif`;
        context.textAlign = "left";
        context.textBaseline = "bottom";
        context.fillText(`${transform.count} objects`, at.x + arm + 4 * dpr, at.y - arm);
      }
    }

    // The selected region, and the one being drawn (spec.md 8.2, M14). Drawn
    // as marching ants — the outline every paint application uses for "an
    // area, not a thing" — in a colour used for nothing else here, so a region
    // cannot be mistaken for a selected object's edge.
    const shaping = regionDrag.current;
    const shown: Region | null = previewing
      ? null
      : shaping
      ? regionMode === "lasso"
        ? regionFromLasso(shaping.points)
        : regionFromDrag(
            regionMode,
            shaping.from,
            shaping.points[shaping.points.length - 1] ?? shaping.from,
          )
      : region;
    if (shown) {
      const ring = regionRing(shown).map((p) =>
        toScreen(camera, view, { lon: p[0], lat: p[1] }),
      );
      const first = ring[0];
      if (first) {
        context.save();
        context.beginPath();
        context.moveTo(first.x, first.y);
        for (const at of ring.slice(1)) context.lineTo(at.x, at.y);
        context.closePath();
        context.fillStyle = "rgba(140, 255, 190, 0.08)";
        context.fill();
        context.strokeStyle = "rgba(150, 255, 200, 0.95)";
        context.lineWidth = Math.max(1, dpr);
        context.setLineDash([6 * dpr, 4 * dpr]);
        context.stroke();
        context.setLineDash([]);
        context.restore();
      }
    }

    // The rubber band, drawn as it is dragged.
    const band = marquee.current;
    if (band) {
      const dpr = window.devicePixelRatio || 1;
      const x = Math.min(band.from.x, band.to.x);
      const y = Math.min(band.from.y, band.to.y);
      const w = Math.abs(band.to.x - band.from.x);
      const h = Math.abs(band.to.y - band.from.y);
      context.fillStyle = "rgba(111, 217, 255, 0.10)";
      context.fillRect(x, y, w, h);
      context.strokeStyle = "rgba(160, 232, 255, 0.9)";
      context.lineWidth = Math.max(1, dpr);
      context.setLineDash([5 * dpr, 4 * dpr]);
      context.strokeRect(x, y, w, h);
      context.setLineDash([]);
    }

    const cursor = cursorRef.current;

    // The active image layer's outline (spec.md 4.9, M18). Only the active
    // one: a project with several charts under it would otherwise stack
    // outlines from all of them, with no way to say which was meant.
    //
    const aligning = alignStore.current!.get();
    const placing = imageLayersRef.current.find((image) => image.layer === activeLayer);
    if (placing?.loaded && !(aligning.layer === placing.layer && aligning.expecting === "map")) {
      // Walked around the picture's edge and projected point by point, never
      // corner to corner: a straight screen line between two projected
      // corners is a chord, which on the globe cut through the planet rather
      // than lying on it. `projectPath` also unwraps longitude the short way
      // and marks the hidden hemisphere with non-finite points, which the
      // pen-up below turns into a break — the same shape `drawMeasurements`
      // strokes its two passage paths with.
      const screen = projectPath(camera, view, outlinePath(placing));
      if (screen.length > 1) {
        context.save();
        context.beginPath();
        let drawing = false;
        for (const at of screen) {
          if (!Number.isFinite(at.x) || !Number.isFinite(at.y)) {
            drawing = false;
            continue;
          }
          if (drawing) context.lineTo(at.x, at.y);
          else context.moveTo(at.x, at.y);
          drawing = true;
        }
        context.strokeStyle = "rgba(120, 200, 255, 0.85)";
        context.lineWidth = Math.max(1, dpr);
        context.lineJoin = "round";
        context.setLineDash([5 * dpr, 4 * dpr]);
        context.stroke();
        context.setLineDash([]);
        context.restore();
      }
    }

    if (placing?.loaded && tool === HAND && aligning.layer === null) {
      context.save();
      context.fillStyle = "#d8f3ff";
      context.strokeStyle = "#163c57";
      context.lineWidth = Math.max(1, dpr);
      for (const handle of imageResizeHandles(placing, camera, view)) {
        const size = (handle.index < 4 ? 8 : 6) * dpr;
        context.fillRect(handle.point.x - size / 2, handle.point.y - size / 2, size, size);
        context.strokeRect(handle.point.x - size / 2, handle.point.y - size / 2, size, size);
      }
      context.restore();
    }

    // The image alignment mode (Task 7, spec.md 4.9 §2, §5): every pair
    // placed this session, numbered, with a line from where it was clicked
    // in the picture to where it was told to belong. The picture end is
    // `pictureToMap`'s forward map of the pair's own pixel — the exact point
    // that was clicked, recovered from the pixel `imagePixelAt` inverted it
    // to, through the same (frozen, unwritten) warp — never the click's own
    // screen position kept aside, so undo and redo of a pair need remember
    // nothing but the pair. A picture click waiting on its map half draws the
    // same line to the pointer instead, the position picker's own cue for
    // "here to there".
    if (aligning.layer !== null) {
      const alignedView = imageLayersRef.current.find((v) => v.layer === aligning.layer);
      if (alignedView) {
        context.save();
        context.font = `${11 * dpr}px system-ui, sans-serif`;
        context.textAlign = "left";
        context.textBaseline = "bottom";
        const dot = (at: { x: number; y: number }) => {
          context.beginPath();
          context.arc(at.x, at.y, 4 * dpr, 0, Math.PI * 2);
          context.fillStyle = "rgba(255, 214, 102, 0.95)";
          context.fill();
        };
        aligning.pairs.forEach((pair, index) => {
          const pictureGeo = pictureToMap(alignedView, pair.u, pair.v);
          const from = toScreen(camera, view, pictureGeo);
          const to = toScreen(camera, view, { lon: pair.lon, lat: pair.lat });
          context.strokeStyle = "rgba(255, 214, 102, 0.9)";
          context.lineWidth = Math.max(1, dpr);
          context.beginPath();
          context.moveTo(from.x, from.y);
          context.lineTo(to.x, to.y);
          context.stroke();
          dot(from);
          dot(to);
          context.fillStyle = "rgba(255, 214, 102, 0.95)";
          const label = String(index + 1);
          context.fillText(label, from.x + 6 * dpr, from.y - 6 * dpr);
          context.fillText(label, to.x + 6 * dpr, to.y - 6 * dpr);
        });
        if (aligning.pending && cursor) {
          const pictureGeo = pictureToMap(alignedView, aligning.pending.u, aligning.pending.v);
          const from = toScreen(camera, view, pictureGeo);
          context.strokeStyle = "rgba(255, 214, 102, 0.6)";
          context.lineWidth = Math.max(1, dpr);
          context.setLineDash([4 * dpr, 4 * dpr]);
          context.beginPath();
          context.moveTo(from.x, from.y);
          context.lineTo(cursor.x, cursor.y);
          context.stroke();
          context.setLineDash([]);
          dot(from);
        }
        context.restore();
      }
    }

    if (aligning.layer !== null) {
      if (aligning.expecting === "map" && cursor && alignBasemap.current) {
        drawMagnifier(context, alignBasemap.current, cursor, dpr);
      }
      // Alignment owns the pointer; no brush or erase preview belongs here.
      return;
    }

    // The measurements (spec.md 10, M8). Drawn whatever the tool is, because
    // they are annotations: a passage measured with the dividers is still on
    // the chart while the brush is in hand, which is the whole point of
    // saving them. Everything drawn here was computed and formatted by Rust.
    drawMeasurements(context, camera, view, dpr, {
      views:
        tool === MEASURE && measurePreview.current !== null
          ? [...measurementsRef.current, measurePreview.current]
          : measurementsRef.current,
      pending: tool === MEASURE ? pendingPoint.current : null,
      cursor: tool === MEASURE ? cursor : null,
      active: measureDrag.current?.id ?? openChain.current,
    });

    // A position property waiting for a click: where it points now, and where
    // the cursor would move it to. Drawn whatever the tool is — the inspector
    // armed it, not the tool.
    if (picking !== null) {
      const from = toScreen(camera, view, { lon: picking.lon, lat: picking.lat });
      const arm = 9 * dpr;
      context.save();
      context.strokeStyle = "rgba(255, 168, 96, 0.55)";
      context.lineWidth = Math.max(1, dpr);
      context.setLineDash([4 * dpr, 4 * dpr]);
      context.beginPath();
      context.arc(from.x, from.y, arm, 0, Math.PI * 2);
      context.stroke();
      context.setLineDash([]);

      if (cursor) {
        // A line from the old place to the new one, so the move reads as a move
        // rather than as two unrelated marks.
        context.strokeStyle = "rgba(255, 214, 120, 0.95)";
        context.beginPath();
        context.moveTo(from.x, from.y);
        context.lineTo(cursor.x, cursor.y);
        context.moveTo(cursor.x - arm, cursor.y);
        context.lineTo(cursor.x + arm, cursor.y);
        context.moveTo(cursor.x, cursor.y - arm);
        context.lineTo(cursor.x, cursor.y + arm);
        context.stroke();

        context.fillStyle = "rgba(255, 214, 120, 0.95)";
        context.font = `${11 * dpr}px system-ui, sans-serif`;
        context.textAlign = "left";
        context.textBaseline = "bottom";
        context.fillText(picking.label, cursor.x + arm + 4 * dpr, cursor.y - arm);
      }
      context.restore();
    }

    // Strokes already committed, still waiting for their field. Drawn whatever
    // the tool is: they are finished strokes, and switching tools at pointer-up
    // must not blink the paint out either.
    for (const settled of settling.current) {
      if (!settled.silent) drawFieldPreview(context, settled, dpr);
    }

    // The insert tool's hover (M24): the macro's region outline at the
    // pointer, in the region's own colour, and for a macro that recorded
    // movement the track its centre will follow — one dot per frame.
    // Drawn *before* the schema check below: the insert tool has no schema,
    // and behind that return this was never reached (M27).
    const macro = library?.entries.find((entry) => entry.id === macroId) ?? null;
    if (cursor && tool === INSERT && recording === null && macro !== null && canInsertMacro()) {
      const at = unproject(camera, view, cursor);
      drawMacroFootprint(
        context,
        regionRing(macroRegion(macro.outline, at.lon, at.lat)),
        macro.moves ? macro.track : null,
        at,
        dpr,
      );
    }

    // The eraser's footprint (spec.md 8.1, M29): the stamp at the pointer,
    // in pink — the colour of what the tool will take. Before the schema
    // check, since the eraser has none.
    //
    // **Outline only: nothing the eraser touches is tinted.** The sweep and
    // the nib were filled with a translucent pink, which over the dark map
    // read as a purple wash sitting on the very field the tool was taking
    // away — and the wash was redundant from the moment the erase became
    // live (M32), since the field now leaves under the pointer as it moves.
    // What is left is the nib's own edge, which is what a brush cursor is
    // for: it says where the tool bites and how wide, and it is a silhouette
    // rather than a union so it may be stroked.
    //
    // **The sweep is never stroked either** (M33). A swept footprint is a
    // union of stamps and `Path2D` has no union, so stroking one traces every
    // stamp's own circle and leaves a chain of rings trailing the pointer.
    // Its true edge would need the band the outlines use, which is
    // `destination-out` and has to run before anything else on the frame —
    // and with the erase itself visible there is nothing left for it to say.
    if (tool === ERASE && recording === null) {
      const drag = eraseDrag.current;
      const brush = eraserRef.current;
      const at = cursor ? unproject(camera, view, cursor) : null;
      const points: Array<[number, number]> =
        drag?.points ?? (at ? [[at.lon, at.lat]] : []);
      const first = points[0];
      if (first !== undefined) {
        // px chooses a space here as it does on every other tool (M67): a
        // circle on the map, not a ground circle that flattens going north.
        const { radiusKm: fresh, space } = eraserStamp(brush, camera, first[1], first[0]);
        // Frozen at the press once a stroke is under way, as every px size is.
        const radiusKm = drag?.radiusKm ?? fresh;
        context.save();
        // The nib: one stamp, whose outline is its own silhouette and so may
        // be stroked. At the pointer while it is over the map, and at the
        // head of the stroke otherwise.
        const nib = at ?? { lon: first[0], lat: first[1] };
        const stamp = new Path2D();
        addFootprint(stamp, camera, view, nib.lon, nib.lat, radiusKm, brush.shape, space);
        context.strokeStyle = "rgba(255, 110, 190, 0.95)";
        context.lineWidth = Math.max(1, dpr);
        context.setLineDash([5 * dpr, 4 * dpr]);
        context.stroke(stamp);
        context.restore();
      }
    }

    if (tool === HAND || !schema) return;

    // Every `LonLat` option is placeable by pointing, on every tool that has
    // one (spec.md 6.1). The markers are drawn for all of them, so a clone
    // stamp's source and a brush's aim point are the same affordance rather
    // than two that happen to look alike.
    for (const spec of liveOptions(schema, toolState.values)) {
      if (spec.default.kind !== "position") continue;
      const armed = toolPick?.property === spec.property;
      const held = markerDrag.current === spec.property;
      const aim =
        armed && cursor
          ? unproject(camera, view, cursor)
          : (() => {
              const [lon, lat] = positionOf(toolState.values, spec.property);
              return { lon, lat };
            })();
      const at = toScreen(camera, view, aim);
      const arm = 9 * dpr;
      context.strokeStyle =
        armed || held ? "rgba(255, 214, 120, 0.95)" : "rgba(255, 168, 96, 0.9)";
      context.lineWidth = Math.max(1, dpr);
      context.beginPath();
      context.moveTo(at.x - arm, at.y);
      context.lineTo(at.x + arm, at.y);
      context.moveTo(at.x, at.y - arm);
      context.lineTo(at.x, at.y + arm);
      context.stroke();

      // A ring at the grab radius, so the marker reads as something to take
      // hold of rather than as a printed cross. Filled while it is held.
      const reach = HANDLE_RADIUS_CSS * 2 * dpr;
      context.beginPath();
      context.arc(at.x, at.y, reach, 0, Math.PI * 2);
      if (held || (!armed && cursor && near(cursor, at))) {
        context.fillStyle = "rgba(255, 168, 96, 0.22)";
        context.fill();
      }
      context.stroke();
    }

    // The colour the field will take once this gesture lands, from the same
    // ramp the map paints with. A floor under the alpha keeps a calm gesture
    // visible: the field fades calm out entirely, but a preview the user cannot
    // see is not a preview.
    const drawing = gestureRef.current;
    const inProgress =
      drawing === null || schemaTool === null
        ? null
        : footprintOf(schemaTool, toolState, drawing, camera);
    const field = previewField(schemaTool ?? "brush", toolState, inProgress);
    const paint = rampCss(
      mpsFromKnots(field.knots),
      rampMax,
      PREVIEW_MIN_ALPHA,
      rampMin,
      stopsOf(gradientsRef.current, projectRef.current[`${activeKindRef.current}_gradient`]),
    );

    // What the overlay draws is the tool's preview kind, decided in one place
    // (`overlayPlan`): the field for a tool that paints one, an outline for the
    // clone stamp, and — for the mask — nothing but the nib below, since the
    // map is already drawing the erasure through a mask.
    const plan = overlayPlan(schema.preview, drawing !== null);
    if (inProgress && plan.sweep === "outline") {
      const region = new Path2D();
      buildFootprintPath(region, camera, view, inProgress);
      context.strokeStyle = "rgba(160, 232, 255, 0.95)";
      context.lineWidth = Math.max(1, dpr);
      context.stroke(region);
    } else if (inProgress && plan.sweep === "field") {
      drawFieldPreview(context, { footprint: inProgress, paint, ...field }, dpr);
    }

    // A gesture built point by point shows the points it has so far, so the
    // user can see what has been placed before there is a shape to fill.
    if (drawing && (drawing.kind === "ring" || drawing.kind === "path")) {
      drawPlacedPoints(context, drawing, dpr, cursor);
    }

    // The hover indicator: the exact footprint a click would produce, in the
    // same colour and with the same glyph as the gesture preview (spec.md 6.1).
    // Only where the tool has one — a polygon and a curve are built up point by
    // point, so a single click produces nothing to show (spec.md 6.2) — and
    // only where the click *would* produce it: inside a selected region the
    // click fills the region and the cursor is the bucket (`showsHoverIndicator`).
    const magnifying = showsMagnifier({
      eyedropper,
      building: drawing?.kind === "ring" || drawing?.kind === "path",
    });
    const hoverGeo = cursor ? unproject(camera, view, cursor) : null;
    const hoverShown =
      hoverGeo !== null &&
      showsHoverIndicator({
        hasIndicator: schema.hover,
        nib: plan.nib,
        picking: toolPick !== null,
        magnifying,
        insideRegion:
          region !== null &&
          drawsObjects(tool) &&
          editsRegion(tool) &&
          regionContains(region, hoverGeo.lon, hoverGeo.lat),
      });
    if (hoverGeo && hoverShown) {
      const geo = hoverGeo;
      const hovered =
        schemaTool === null
          ? null
          : footprintOf(schemaTool, toolState, hoverGesture(schema, toolState, geo), camera);
      if (hovered) {
        const tip = new Path2D();
        buildFootprintPath(tip, camera, view, hovered);
        if (schema.preview === "field") {
          context.fillStyle = paint;
          context.fill(tip);
        }
        if (showGlyphs && schema.preview === "field") {
          const hoverField = previewField(schemaTool ?? "brush", toolState, hovered);
          const head = footprintHead(hovered);
          if (head) drawGlyphs(context, [head], hoverField.knots, hoverField.azimuthAt, dpr);
        }
        context.strokeStyle = "rgba(160, 232, 255, 0.95)";
        context.lineWidth = Math.max(1, dpr);
        context.stroke(tip);
      }
    }

    // The eyedropper's magnifier (M24): a ring with a plus at the pointer,
    // the field there as the project's glyph, and the numbers the click will
    // take. The numbers are the readout's — one sample in flight, the newest
    // position waiting — so arming the eyedropper costs no second stream.
    if (cursor && magnifying) {
      const sample = readoutStore.current?.get().sample ?? null;
      const radius = MAGNIFIER_RADIUS_CSS * dpr;
      const source = canvasRef.current;
      if (source) drawMagnifier(context, source, cursor, dpr);
      context.save();
      if (sample) {
        const here = unproject(camera, view, cursor);
        if (sample.speedKnots > 0.05) {
          drawGlyphs(
            context,
            [[here.lon, here.lat]],
            sample.speedKnots,
            () => sample.azimuthTowardDeg,
            dpr,
          );
        }
        context.font = `${11 * dpr}px system-ui, sans-serif`;
        context.textAlign = "left";
        context.textBaseline = "top";
        const text = `${units.speedFromKnots(sample.speedKnots).toFixed(1)} ${units.speedUnit} · ${Math.round(sample.directionDeg)}°`;
        const x = cursor.x + radius + 6 * dpr;
        const y = cursor.y - 7 * dpr;
        const width = context.measureText(text).width;
        context.fillStyle = "rgba(20, 28, 44, 0.85)";
        context.fillRect(x - 3 * dpr, y - 2 * dpr, width + 6 * dpr, 15 * dpr);
        context.fillStyle = "rgba(255, 255, 255, 0.95)";
        context.fillText(text, x, y);
      }
      context.restore();
    }
  }, [
    shapeEditing,
    step,
    canInsertMacro,
    drawDragOutlines,
    drawMacroFootprint,
    drawGlyphs,
    drawFieldPreview,
    drawPlacedPoints,
    eyedropper,
    previewing,
    glyphStyle,
    library,
    macroId,
    near,
    rampMax,
    rampMin,
    schema,
    showGlyphs,
    tool,
    toolPick,
    toolState,
    picking,
    shownTransform,
    outlineList,
    drawEdgeBand,
    selection,
    region,
    regionMode,
    units,
  ]);

  useEffect(() => {
    drawOverlayRef.current = drawOverlay;
    drawOverlay();
  }, [drawOverlay]);

  /**
   * Refreshes the live operator preview, and says whether the map must redraw.
   *
   * The mask and the clone stamp paint what is *already there*, so a preview
   * drawn on the overlay could only ever be a coloured guess at it (spec.md
   * 6.1). Instead the gesture's coverage is rasterised into a mask and the map
   * is drawn through it: the field is taken away where the mask covers, and
   * replaced from the source where the clone does.
   *
   * Nothing here is per tool. The footprint comes from the same builder the
   * overlay uses, so a new tool that operated on the field would inherit this
   * by declaring how it previews.
   */
  const refreshOperator = useCallback(
    (drawing: InProgress | null): boolean => {
      const had = operatorRef.current !== null;
      const kind = schema?.preview;
      // Every tool that does not paint a field of its own operates on the
      // one that is there, and is previewed by the map applying it (M32).
      const operates = kind !== undefined && kind !== "field";
      const footprint =
        drawing && operates && schemaTool !== null
          ? footprintOf(schemaTool, toolState, finished(drawing), cameraRef.current)
          : null;
      const spec =
        footprint && drawing && operates && schemaTool !== null
          ? operatorOf(schemaTool, toolState, finished(drawing), cameraRef.current, viewRef.current)
          : null;

      if (!footprint || !drawing || !spec) {
        operatorRef.current = null;
        return had;
      }

      const view = viewRef.current;
      const width = Math.max(1, Math.round(view.width * MASK_SCALE));
      const height = Math.max(1, Math.round(view.height * MASK_SCALE));
      const canvas = (maskCanvas.current ??= document.createElement("canvas"));
      if (canvas.width !== width || canvas.height !== height) {
        canvas.width = width;
        canvas.height = height;
      }
      const context = canvas.getContext("2d");
      if (!context) {
        operatorRef.current = null;
        return had;
      }
      context.setTransform(1, 0, 0, 1, 0, 0);
      context.clearRect(0, 0, width, height);
      // The footprint is built in framebuffer pixels, so the context is scaled
      // rather than the path: the shader samples by a normalised coordinate and
      // does not care what size the mask is.
      context.scale(MASK_SCALE, MASK_SCALE);
      const region = new Path2D();
      buildFootprintPath(region, cameraRef.current, view, footprint);
      // Only the alpha is read, so the colour is arbitrary; opaque white is
      // the one that says "fully covered" at a glance in a debugger.
      context.fillStyle = "#fff";
      context.fill(region);

      operatorRef.current = { mask: canvas, ...spec };
      return true;
    },
    [schema, tool, toolState],
  );

  /**
   * The eraser's live preview (M32): the field goes from under the stroke as
   * it is drawn, exactly as a mask's does. The eraser has no schema and no
   * gesture of the palette's kind, so it rasterises its own path.
   */
  const refreshEraser = useCallback((): boolean => {
    const had = operatorRef.current !== null;
    const drag = eraseDrag.current;
    if (!drag || drag.points.length === 0) {
      if (had) operatorRef.current = null;
      return had;
    }
    const view = viewRef.current;
    const width = Math.max(1, Math.round(view.width * MASK_SCALE));
    const height = Math.max(1, Math.round(view.height * MASK_SCALE));
    const canvas = (maskCanvas.current ??= document.createElement("canvas"));
    if (canvas.width !== width || canvas.height !== height) {
      canvas.width = width;
      canvas.height = height;
    }
    const context = canvas.getContext("2d");
    if (!context) return had;
    context.setTransform(1, 0, 0, 1, 0, 0);
    context.clearRect(0, 0, width, height);
    context.scale(MASK_SCALE, MASK_SCALE);
    const path = new Path2D();
    buildStrokePath(
      path,
      cameraRef.current,
      view,
      drag.points,
      drag.radiusKm,
      eraserRef.current.shape,
      spaceFor(eraserRef.current.unit, cameraRef.current),
    );
    context.fillStyle = "#fff";
    context.fill(path);
    operatorRef.current = { mask: canvas, kind: "remove" };
    return true;
  }, []);

  /** Screen positions of the handles, or null when nothing is selected. */
  const handlePositions = useCallback(() => {
    const transform = shownTransform();
    if (!transform) return null;
    const camera = cameraRef.current;
    const view = viewRef.current;
    const anchor = { lon: transform.lon, lat: transform.lat };
    return {
      pivot: anchor,
      centre: toScreen(camera, view, anchor),
      rotate: toScreen(
        camera,
        view,
        destination(anchor, transform.rotation_deg, transform.radius_m),
      ),
      scale: toScreen(
        camera,
        view,
        destination(anchor, transform.rotation_deg + 90, transform.radius_m),
      ),
      count: transform.count,
    };
  }, [shownTransform]);

  /**
   * Captures the drag baseline on the backend and starts a gesture.
   *
   * The baseline lives there rather than here so the geodesy — a rigid rotation
   * for a move, a turn about the centroid for a rotate — happens in one place
   * (spec.md 3.3), and so a drag survives a slow round trip without the handles
   * and the document disagreeing about where it started.
   */
  const startTransform = useCallback(
    (kind: TransformKind, geo: { lon: number; lat: number }) => {
      handleDrag.current = { kind, pointer: geo, asking: false, queued: null };
      dragPreview.current = null;
      // Snapshot the selected identities for the asynchronous field preview.
      //
      // Not for a repin, which leaves the geometry where it is on the ground —
      // nothing moves, so nothing should look as though it has. And not for a
      // group's rotation: a group has no orientation of its own, so its handles
      // report none, and a ghost built from them would orbit the pivot without
      // turning. Both of those keep the outline-only preview they had.
      ghost.current = null;
      ghostSource.current = [];
      ghostWanted.current = kind !== "anchor" && !(kind === "rotate" && selection.length > 1);
      ghostSource.current = outlineList.filter((o) => selection.includes(o.object));
      requestDraw();
      void api
        // Auto-key travels with the drag: with it on, or on any animated
        // property, the drag keys the current step rather than the base.
        .beginTransform(selection, step, kind, geo.lon, geo.lat, autoKey)
        .catch((err: unknown) => {
          handleDrag.current = null;
          ghost.current = null;
          ghostSource.current = [];
          ghostWanted.current = false;
          void api.frontendLog("error", `transform failed to start: ${String(err)}`);
        });
    },
    [autoKey, outlineList, requestDraw, selection, step],
  );

  /**
   * Asks where the drag would leave the selection, and draws it.
   *
   * One request in flight at a time: a newer pointer position replaces the
   * waiting one rather than joining a queue, so the preview is always answering
   * the current pointer and latency stays at one round trip however fast the
   * pointer moves.
   */
  const previewDrag = useCallback((geo: { lon: number; lat: number }) => {
    const drag = handleDrag.current;
    if (drag === null) return;
    drag.pointer = geo;
    if (drag.asking) {
      drag.queued = geo;
      return;
    }

    const ask = (at: { lon: number; lat: number }) => {
      const current = handleDrag.current;
      if (current === null) return;
      current.asking = true;
      void api
        .previewTransform(at.lon, at.lat)
        .then((preview) => {
          // The drag may have ended while this was in flight; its own commit
          // draws the result, and a stale preview would draw over it.
          if (handleDrag.current === null) return;
          dragPreview.current = preview;
          drawOverlayRef.current();
        })
        .catch((err: unknown) => void api.frontendLog("error", `preview failed: ${String(err)}`))
        .finally(() => {
          const still = handleDrag.current;
          if (still === null) return;
          still.asking = false;
          const next = still.queued;
          still.queued = null;
          if (next) ask(next);
        });
    };
    ask(geo);
  }, []);

  /**
   * Writes the drag, once, when the pointer comes up.
   *
   * No baseline travels with this: the backend captured it at pointer-down, so
   * the update is "where does the selection go with the pointer here", which
   * makes a repeated position a no-op rather than a nudge.
   */
  const commitDrag = useCallback(
    (geo: { lon: number; lat: number }): Promise<void> =>
      api
        .dragTransform(geo.lon, geo.lat)
        .then((summary) => {
          if (settlingDrag.current) settlingDrag.current.revision = summary.revision;
          onProjectChanged(summary);
          // If every tile was already cached nothing else would schedule the
          // draw that retires the held outline; and if they never arrive, the
          // backstop does.
          requestDraw();
          window.setTimeout(requestDraw, SETTLE_TIMEOUT_MS + 100);
        })
        .catch((err: unknown) => {
          // Nothing moved, so nothing should keep looking moved.
          settlingDrag.current = null;
          ghost.current = null;
          ghostSource.current = [];
          ghostWanted.current = false;
          drawOverlayRef.current();
          void api.frontendLog("error", `drag failed: ${String(err)}`);
        }),
    [onProjectChanged, requestDraw],
  );

  const toDevice = (event: { clientX: number; clientY: number }) => {
    const canvas = canvasRef.current;
    const dpr = window.devicePixelRatio || 1;
    const rect = canvas?.getBoundingClientRect();
    return {
      x: ((event.clientX - (rect?.left ?? 0))) * dpr,
      y: ((event.clientY - (rect?.top ?? 0))) * dpr,
    };
  };

  /**
   * The warp whose footprint the pointer is inside, topmost first.
   *
   * `isPointInPath` against the outline the overlay is already drawing, so what
   * can be grabbed is exactly what is shown — and its anchor comes back with
   * it, since that is the end of the push the drag does not move.
   */
  const warpUnder = (point: {
    x: number;
    y: number;
  }): { object: number; anchor: { lon: number; lat: number } } | null => {
    const context = overlayRef.current?.getContext("2d");
    if (!context) return null;
    for (let i = operatorOutlinesRef.current.length - 1; i >= 0; i -= 1) {
      const entry = operatorOutlinesRef.current[i];
      if (entry === undefined || entry.tool !== "warp") continue;
      if (!context.isPointInPath(maskPath(entry.outline), point.x, point.y, entry.outline.kind === "contours" ? "evenodd" : "nonzero")) continue;
      return { object: entry.object, anchor: { lon: entry.anchor[0], lat: entry.anchor[1] } };
    }
    return null;
  };

  /**
   * Applies the cursor rule (`cursorFor`) to the canvas.
   *
   * Written to the element rather than rendered as a class: the two inputs
   * that change per pointer report — inside the region, panning — would
   * otherwise re-render the map view at pointer rate, which is the failure
   * `createReadoutStore` exists to prevent.
   */
  const applyCursor = useCallback(
    (insideRegion: boolean, panning: boolean, onImage = false, imageCursor: string | null = null) => {
      const canvas = canvasRef.current;
      if (!canvas) return;
      if (shapeModeRef.current !== null) {
        canvas.style.cursor = shapeDrag.current ? "grabbing" : panning ? "grab" : "crosshair";
        return;
      }
      // What the tool would do to a layer, and whether this layer takes it
      // (M51). The eraser is an edit and has no schema of its own; a tool
      // that draws nothing is nobody's business.
      const work: ToolKindOfWork =
        tool === INSERT
          ? "adds"
          : tool === ERASE
          ? "edits"
          : !drawsObjects(tool)
            ? "neither"
            : schemaRef.current?.preview === "field" || schemaRef.current === null
              ? "adds"
              : "edits";
      const wanted = (tool === HAND && !picking && !toolPick && !recording && !eyedropper
        && alignStore.current!.get().layer === null && imageCursor) || cursorFor({
        tool,
        eyedropper,
        picking: picking !== null || toolPick !== null,
        insideRegion,
        panning,
        recording: recording !== null,
        aligning: alignStore.current!.get().layer !== null,
        onImage,
        // What the layer will not take, and what is not on the map at all
        // (M51, M68). Both are refusals the cursor can say on hover rather
        // than leaving the user to discover on release — which is the whole
        // point of `allowed.ts`. A tool that touches no field is exempt from
        // both: the hand and the selection work over a hidden layer as they
        // work over anything.
        forbidden:
          work !== "neither" &&
          (projectRef.current.layer_count === 0 || !layerTakes(
            layerSourcesRef.current.get(activeLayerRef.current ?? -1) ?? "painted",
            work,
          ) ||
            activeLayerHidden() ||
            (tool === INSERT && !canInsertMacro())),
      });
      if (canvas.style.cursor !== wanted) canvas.style.cursor = wanted;
    },
    [activeLayerHidden, canInsertMacro, eyedropper, picking, recording, tool, toolPick],
  );
  useEffect(() => {
    applyCursor(false, dragging.current !== null);
  }, [applyCursor]);
  // Either end of the ramp moving is a new frame: the tiles are the same,
  // the colours are not.
  useEffect(() => {
    requestDraw();
  }, [rampsKey, requestDraw]);

  const lastMapPointer = useRef<{x:number;y:number} | null>(null);

  const onPointerDown = (event: React.PointerEvent<HTMLCanvasElement>) => {
    if (imageDrag.current) return;
    const point = toDevice(event);
    if (!validGeo(unproject(cameraRef.current, viewRef.current, point))) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    lastMapPointer.current = point;

    if (shapeEditing !== null && picking === null) {
      const controls = shapeControls.current;
      const hit = controls && hitShapePoint(controls,
        ([lon, lat]) => toScreen(cameraRef.current, viewRef.current, { lon, lat }),
        point, 9 * (window.devicePixelRatio || 1));
      if (hit && controls && event.button === 0) {
        shapeDrag.current = { hit, before: controls, pointer: event.pointerId, moved: false };
      } else { dragging.current = point; }
      return;
    }

    // Which of a pick, a running capture or the alignment mode claims this
    // click, ahead of every tool and ahead of each other — the one place
    // this is decided. `cursorFor` reads the same function for the cursor it
    // shows, and `cursor.test.ts`'s own suite for it pins the return value
    // directly, rather than through the cursor string a review of this task
    // found could pass while the dispatch itself checked a different order
    // (a running capture is supposed to outrank the alignment mode; it did
    // on the cursor and did not, once, in the dispatch below).
    const pointerPriority = pointerPriorityFor({
      picking: picking !== null,
      recording: recording !== null,
      aligning: alignStore.current!.get().layer !== null,
    });

    // A pick armed in the inspector takes the click ahead of every tool: the
    // user asked for this one place, and painting or panning instead would
    // both lose the click and do something they did not ask for.
    // `pointerPriority` is derived from `picking !== null`, so the two can
    // never disagree; the redundant check is only for TypeScript's own
    // narrowing of `picking` inside the block.
    if (pointerPriority === "picking" && picking !== null) {
      onExitShapeEditing?.();
      const geo = unproject(cameraRef.current, viewRef.current, point);
      onPicked();
      void api
        .setObjectProperty(
          picking.object,
          picking.property,
          { kind: "position", lon: geo.lon, lat: geo.lat },
          step,
          autoKey,
        )
        .then(onProjectChanged)
        .catch((err: unknown) =>
          void api.frontendLog("error", `placing ${picking.property} failed: ${String(err)}`),
        );
      return;
    }

    // While a capture runs, the region can be *dragged* — and nothing else on
    // the map does anything (spec.md 8.7), the alignment mode included: an
    // image layer's corner-drag handles and its M36 whole-picture drag used
    // to be picked up here (spec.md 4.9, M18) and are gone with the
    // corner-placement model; this is their replacement, Task 7's alignment
    // interaction, gated below by `pointerPriority` rather than a capture
    // check of its own — `pointerPriorityFor` above already decided a
    // capture wins, so alignment's own block, further down, is only ever
    // reached when this one is not. By delta from where the pointer went
    // down, at pointer resolution: each frame holds its own position, and
    // this moves the step being viewed and no other. It is capture state,
    // never the document, which is why it is not an edit and not undoable.
    // Same redundant-looking narrowing as the picking block above, for the
    // same reason: `pointerPriority` and `recording !== null` cannot
    // disagree, since one is derived from the other.
    if (pointerPriority === "recording" && recording !== null) {
      const geo = unproject(cameraRef.current, viewRef.current, point);
      // In the preview a click places a *copy* of the macro there (spec.md
      // 8.7, M29): in the preview's own scene, looping with the original,
      // and in no layer of the document — the placements go with the
      // preview, whatever ends it.
      if (recording.phase === "previewing") {
        // A press pans and a click places (M33). Placing on the way down
        // meant a drag could never pan: the preview is a map to look around
        // before it is a surface to stamp on.
        previewPress.current = true;
        dragging.current = point;
        pressOrigin.current = point;
        applyCursor(false, true);
        return;
      }
      const anchor = region === null ? null : regionAnchor(region);
      if (region !== null && anchor !== null && regionContains(region, geo.lon, geo.lat)) {
        captureDrag.current = { pointer: [geo.lon, geo.lat], anchor };
      }
      return;
    }

    // The image alignment mode (Task 7, spec.md 4.9 §2, §5): every click here
    // is a control point's picture or map half, alternating, never a tool's —
    // ahead of the tools below, gated by `pointerPriority` rather than a
    // check of `alignStore` alone, so a running capture (handled above)
    // reliably wins the click even though both can be armed at once.
    //
    // `imagePixelAt` carries a picture click back to the image's own pixel
    // space through whatever warp is in force when the session began: frozen
    // for the whole session, since the document — and so the warp — is not
    // written until `Enter` (the amendment to this task's brief: a spline has
    // no closed-form inverse, so this searches the mesh already evaluated
    // forward rather than re-deriving the fit in TypeScript).
    //
    // A click that cannot be resolved to a real image pixel — outside the
    // coarse straight-edged quad `imageUnder` hit-tests, or in the gap
    // between that quad and the mesh's own, bowed true edge — is refused
    // with a hint rather than stored as an extrapolated guess: a wrong
    // control point written silently is far costlier to notice and undo
    // later than one more click now.
    if (pointerPriority === "aligning") {
      if (alignStore.current!.get().expecting === "picture") {
        const view = imageUnder(
          imageLayersRef.current,
          alignStore.current!.get().layer,
          cameraRef.current,
          viewRef.current,
          point,
        );
        const geo = unproject(cameraRef.current, viewRef.current, point);
        const pixel = view ? imagePixelAt(view, geo) : null;
        if (pixel) {
          // `pick` itself refuses a fifty-first pair (align.ts,
          // `MAX_CONTROL_POINTS`) — caught here, before it is ever stored,
          // rather than at `Enter` where the backend's own cap would reject
          // the whole set and cost every pair already placed.
          if (alignStore.current!.pick("picture", { lon: pixel[0], lat: pixel[1] })) {
            reportError(null);
          } else {
            reportError(
              `At most ${MAX_CONTROL_POINTS} control points are allowed — drop one with Backspace to add another.`,
            );
          }
        } else {
          reportError("That point is outside the picture — click a place inside it.");
        }
      } else {
        const geo = unproject(cameraRef.current, viewRef.current, point);
        alignStore.current!.pick("map", geo);
      }
      return;
    }

    // Inside the picture, the hand moves the whole thing (M36) — the same
    // tool that moves a selected object, and the same rule: what is selected
    // is what a drag moves, and a drag anywhere else pans. The active image
    // layer is the selection, which is why `imageUnder` looks at that layer
    // and no other: a stack of charts would otherwise move whichever
    // happened to be on top, with no way to say which was meant.
    //
    // Below the alignment branch on purpose. While the mode is armed every
    // click is a control point's, so a hand over the picture must not steal
    // one — `pointerPriority` has already decided that above.
    if (tool === HAND) {
      const activeImage = imageLayersRef.current.find((image) => image.layer === activeLayer);
      const handle = activeImage && imageResizeHandleAt(activeImage, cameraRef.current, viewRef.current,
        point, 9 * (window.devicePixelRatio || 1));
      const image = handle ? activeImage : imageUnder(imageLayersRef.current, activeLayer,
        cameraRef.current, viewRef.current, point);
      if (image) {
        const from = unproject(cameraRef.current, viewRef.current, point);
        const origin = [image.placement[2], image.placement[5]];
        const key = `image:${image.layer}:${++imageGestureId.current}`;
        const writer = imageGesture<{ lon: number; lat: number }>(imageWriteTail.current, async (at) => {
          const summary = handle
            ? await api.resizeImage(image.layer, handle.index, [from.lon, from.lat], [at.lon, at.lat], key)
            : await api.moveImage(image.layer, origin[0]! + normalizeLon(at.lon - from.lon),
              origin[1]! + at.lat - from.lat, key);
          onProjectChanged(summary);
        }, () => api.endGesture(), (error) => reportError(String(error)));
        imageWriteTail.current = writer.done;
        imageDrag.current = { cursor: handle?.cursor ?? "move", push: writer.push, finish: writer.finish };
        applyCursor(false, false, true, imageDrag.current.cursor);
        return;
      }
    }

    // The insert tool puts a library macro down where it is clicked
    // (spec.md 8.7, M16). One click, one object, like the fill tool: the macro
    // carries its own frames, so there is nothing to drag out.
    // The eraser (spec.md 8.1, M29): a brush stroke that takes away. `Shift`
    // says this frame only. A size in pixels becomes kilometres here, at the
    // latitude the stroke begins, as a brush's does.
    if ((drawsObjects(tool) || tool === ERASE || tool === INSERT) && projectRef.current.layer_count === 0) {
      setHint("Add a layer before using this tool.");
      return;
    }

    if (tool === ERASE) {
      const geo = unproject(cameraRef.current, viewRef.current, point);
      const brush = eraserRef.current;
      eraseDrag.current = {
        points: [[geo.lon, geo.lat]],
        step: event.shiftKey ? stepRef.current : null,
        radiusKm: eraserStamp(brush, cameraRef.current, geo.lat, geo.lon).radiusKm,
      };
      if (refreshEraser()) requestDraw();
      requestOverlay();
      return;
    }

    if (tool === INSERT) {
      if (macroId === null || !canInsertMacro()) return;
      const geo = unproject(cameraRef.current, viewRef.current, point);
      void api
        .insertMacro(macroId, geo.lon, geo.lat, stepRef.current, activeLayer)
        .then(onProjectChanged)
        .catch((err: unknown) => setError(String(err)));
      return;
    }

    // The capture tool draws its region with the select tool's own gestures:
    // the region *is* the select tool's, and a second way of drawing one would
    // be a second thing to keep in step.
    if (tool === CAPTURE) {
      const geo = unproject(cameraRef.current, viewRef.current, point);
      regionDrag.current = { from: [geo.lon, geo.lat], points: [[geo.lon, geo.lat]] };
      requestOverlay();
      return;
    }

    // The measurement tools (spec.md 10, M8). A click either grabs a handle
    // that is already there or places a point; nothing here is a drag, because
    // a measurement is a set of positions rather than a swept shape.
    if (tool === MEASURE) {
      const dpr = window.devicePixelRatio || 1;
      const grabbed = handleUnder(
        measurementsRef.current,
        cameraRef.current,
        viewRef.current,
        point,
        HANDLE_REACH_CSS * dpr,
      );
      if (grabbed !== null) {
        // Alt-click removes the measurement the handle belongs to, which is
        // spec.md 10's "individually clearable" — a measurement is a mark on a
        // chart and the way to get rid of one is to point at it.
        if (event.altKey) {
          void api
            .removeMeasurement(grabbed.id)
            .then((views) => {
              tookMeasurements(views);
              if (openChain.current === grabbed.id) openChain.current = null;
              if (activeRings === grabbed.id) setActiveRings(null);
            })
            .catch(() => undefined);
          return;
        }
        measureDrag.current = grabbed;
        const touched = measurementsRef.current.find((m) => m.id === grabbed.id);
        if (touched?.kind === "rings") setActiveRings(touched.id);
        requestOverlay();
        return;
      }

      const geo = unproject(cameraRef.current, viewRef.current, point);
      const at: [number, number] = [geo.lon, geo.lat];

      // A ring set needs only its centre: how big it is comes from the option
      // bar, where it can be typed and changed, rather than from a drag that
      // would have to be redone to correct it.
      if (measureKind === "rings") {
        void api
          .addMeasurement({
            kind: "rings",
            points: [at],
            interval_km: ringIntervalKm,
            count: ringCount,
          })
          .then((views) => {
            tookMeasurements(views);
            setActiveRings(views[views.length - 1]?.id ?? null);
          })
          .catch(() => undefined);
        return;
      }

      // A chain already open takes the click as its next leg.
      const open = openChain.current;
      if (open !== null && measureKind === "dividers") {
        measurePreview.current = null;
        void api.extendMeasurement(open, at).then(tookMeasurements).catch(() => undefined);
        return;
      }

      const pending = pendingPoint.current;
      if (pending === null) {
        pendingPoint.current = at;
        requestOverlay();
        return;
      }
      pendingPoint.current = null;
      measurePreview.current = null;
      const kind = measureKind;
      void api
        .addMeasurement({ kind, points: [pending, at], interval_km: 0, count: 0 })
        .then((views) => {
          tookMeasurements(views);
          // A chain stays open so the next click continues it; a passage is
          // complete at two points and has nothing to continue.
          const placed = views[views.length - 1];
          openChain.current = kind === "dividers" && placed ? placed.id : null;
        })
        .catch(() => undefined);
      return;
    }

    // The select tool draws a region of ground rather than an object
    // (spec.md 8.2, M14). A plain drag draws it, because the select tool is a
    // tool like the brush and the hand tool is where plain drag still pans
    // (D26).
    if (tool === SELECT) {
      const geo = unproject(cameraRef.current, viewRef.current, point);
      regionDrag.current = { from: [geo.lon, geo.lat], points: [[geo.lon, geo.lat]] };
      requestOverlay();
      return;
    }

    if (drawsObjects(tool) && schema) {
      const geo = unproject(cameraRef.current, viewRef.current, point);

      // While a pick is armed the click places that option rather than drawing
      // — one click, one position, and the mode ends itself so the next
      // gesture is an ordinary one.
      if (toolPick) {
        setToolState({
          ...toolState,
          values: {
            ...toolState.values,
            [toolPick.property]: { kind: "position", lon: geo.lon, lat: geo.lat },
          },
        });
        setToolPick(null);
        return;
      }

      // ...and while the eyedropper is armed, the click takes the field's own
      // speed and direction instead of painting. Sampled through the backend
      // rather than decoded from the tile under the pointer: the tile carries
      // the quantised value the map draws with, and this is the number the
      // object will hold. Only visible layers contribute, because that is what
      // the evaluator composites (spec.md 7.1) — pointing at what you can see
      // is the whole of the gesture.
      const building =
        gestureRef.current?.kind === "ring" || gestureRef.current?.kind === "path";
      if (schema && showsMagnifier({ eyedropper, building })) {
        setEyedropper(false);
        void api
          // Whatever is on top here, not whatever matches the layer being
          // painted into (M53). The eyedropper is a pointing gesture: it takes
          // the vector the user can *see*, and what they see at a point is the
          // topmost visible layer with coverage there. Restricting it to the
          // active layer's kind meant pointing at a current and being handed a
          // wind from further down the stack — a number from a layer the
          // pointer was never over.
          .sampleField(geo.lon, geo.lat, stepRef.current, null)
          .then((sample) => setToolState(sampled(toolStateRef.current, schema, sample)))
          .catch(() => undefined);
        return;
      }

      // Shift with the warp tool grabs the warp under the pointer rather than
      // painting another one, and the drag says where its field goes: the
      // object's anchor is where the field comes from and the release point is
      // where it lands, both of them animatable positions (spec.md 6.3).
      // Aiming a liquify by typing a distance and a bearing is guesswork; this
      // is the gesture the tool is named for.
      //
      // With no warp under the pointer it does *nothing* — it does not fall
      // through to painting. Shift says "act on the warp that is there", and a
      // modifier key that paints a new object when it misses is a way to draw
      // one by accident, in the middle of aiming another.
      if (tool === "warp" && event.shiftKey) {
        const grabbed = warpUnder(point);
        if (grabbed === null) return;
        onSelect([grabbed.object]);
        pushDrag.current = { object: grabbed.object, from: grabbed.anchor, to: geo };
        requestOverlay();
        return;
      }

      // With a region selected, a click *inside* it with the brush or one of
      // the four operators makes that object from the region's boundary
      // (spec.md 8.2). Inside, and not anywhere: a click outside is the
      // ordinary stroke, so a region left selected does not turn every stroke
      // into a region. A region is map space, so what is made from it is a
      // projected stamp (D55) — the px unit.
      if (region !== null && editsRegion(tool) && regionContains(region, geo.lon, geo.lat)) {
        void commitGesture(regionGesture(region), { ...toolState, unit: "px" }, schema, tool);
        return;
      }

      // A placed position is a handle, and a handle takes precedence over what
      // is under it — the same rule the transform handles follow (spec.md 8.1).
      // Without it a marker could be placed but never adjusted except by
      // arming the picker again or typing coordinates.
      const marker = markerUnder(point);
      if (marker !== null) {
        markerDrag.current = marker;
        return;
      }

      startGesture(geo, event);
      return;
    }

    const geo = unproject(cameraRef.current, viewRef.current, point);

    // A handle takes precedence over everything else under the pointer.
    const handles = handlePositions();
    if (handles) {
      const reach = HANDLE_RADIUS_CSS * 2 * (window.devicePixelRatio || 1);
      const near = (target: { x: number; y: number }) =>
        Math.hypot(point.x - target.x, point.y - target.y) <= reach;

      // The centre is the anchor for a single object — dragging it repins the
      // pivot without moving the geometry (spec.md 8.2). A group has no anchor
      // of its own, so its centre moves the whole selection instead.
      const kind: TransformKind | null = near(handles.rotate)
        ? "rotate"
        : near(handles.scale)
          ? "scale"
          : near(handles.centre)
            ? handles.count === 1
              ? "anchor"
              : "move"
            : null;

      if (kind) {
        startTransform(kind, geo);
        return;
      }
    }

    // Shift drags a rubber band instead of panning. Plain drag still pans
    // (spec.md 8.1); the modifier is what makes room for both on empty map.
    if (event.shiftKey) {
      marquee.current = {
        from: point,
        to: point,
        crossLayer: event.metaKey || event.ctrlKey,
      };
      drawOverlay();
      return;
    }

    dragging.current = point;
    pressOrigin.current = point;
    applyCursor(false, true);

    // Dragging a selected object moves it; dragging empty map pans (spec.md
    // 8.1). Which of the two this is takes a hit test, and a hit test is a
    // round trip — so panning starts immediately and converts to a move if the
    // answer comes back before the pointer has actually gone anywhere. Waiting
    // for the answer instead would put IPC latency in front of every pan.
    // A modifier click is a selection edit — add this one, drop that one —
    // and never the start of a move: the hit test below would otherwise
    // begin a drag of the whole selection on a `Cmd`-click meant to toggle
    // one member of it (M23).
    if (tool === "hand" && selection.length > 0 && !(event.metaKey || event.ctrlKey)) {
      const slack = 4 * (window.devicePixelRatio || 1);
      void api
        .objectAt(geo.lon, geo.lat, step)
        .then((hit) => {
          if (hit === null || !selection.includes(hit)) return;
          if (handleDrag.current || marquee.current) return;
          const origin = pressOrigin.current;
          const now = dragging.current;
          if (!origin || !now) return;
          if (Math.hypot(now.x - origin.x, now.y - origin.y) > slack) return;
          dragging.current = null;
          pressOrigin.current = null;
          startTransform("move", geo);
        })
        .catch(() => undefined);
    }
  };

  /**
   * Which operator's edge the pointer is on, in screen pixels.
   *
   * `isPointInStroke` against the same path the overlay draws, with a wide pen:
   * "near the edge" is a screen distance, and canvas will answer it exactly for
   * whatever shape the outline is — a swept chain, a ring, a rectangle — where
   * a hand-written distance test would need a case per shape and would drift
   * from what is drawn.
   */
  const operatorEdgeUnder = (point: { x: number; y: number }): number | null => {
    const canvas = overlayRef.current;
    const context = canvas?.getContext("2d");
    if (!context) return null;
    const dpr = window.devicePixelRatio || 1;
    context.save();
    context.lineWidth = EDGE_GRAB_CSS * 2 * dpr;
    let found: number | null = null;
    // Topmost first: the outlines arrive in z-order, and the edge a click would
    // reach is the one drawn last.
    for (let i = operatorOutlinesRef.current.length - 1; i >= 0; i -= 1) {
      const mask = operatorOutlinesRef.current[i];
      if (mask === undefined) continue;
      if (context.isPointInStroke(maskPath(mask.outline), point.x, point.y)) {
        found = mask.object;
        break;
      }
    }
    context.restore();
    return found;
  };

  /**
   * The topmost object of the outline list the pointer is on or in, for the
   * eraser: inside the footprint or near its edge, since a sweep crosses
   * bodies and a pointer parked on an edge means it too.
   */
  const objectUnder = (point: { x: number; y: number }): number | null => {
    const canvas = overlayRef.current;
    const context = canvas?.getContext("2d");
    if (!context) return null;
    const dpr = window.devicePixelRatio || 1;
    context.save();
    context.lineWidth = EDGE_GRAB_CSS * 2 * dpr;
    let found: number | null = null;
    for (let i = operatorOutlinesRef.current.length - 1; i >= 0; i -= 1) {
      const entry = operatorOutlinesRef.current[i];
      if (entry === undefined) continue;
      const path = maskPath(entry.outline);
      if (context.isPointInPath(path, point.x, point.y) || context.isPointInStroke(path, point.x, point.y)) {
        found = entry.object;
        break;
      }
    }
    context.restore();
    return found;
  };

  const onPointerMove = (event: React.PointerEvent<HTMLCanvasElement>) => {
    const point = toDevice(event);
    const geographic = validGeo(unproject(cameraRef.current, viewRef.current, point));
    cursorRef.current = geographic ? point : null;
    if (!geographic) {
      if (dragging.current) {
        const old = dragging.current; dragging.current = point;
        cameraRef.current = panBy(cameraRef.current, viewRef.current, old.x - point.x, old.y - point.y);
        requestDraw();
      }
      readoutStore.current?.set({sample: null});
      requestOverlay();
      return;
    }
    lastMapPointer.current = point;

    const pointDrag = shapeDrag.current;
    if (pointDrag && pointDrag.pointer === event.pointerId) {
      const geo = unproject(cameraRef.current, viewRef.current, point);
      shapeControls.current = movedShapePoint(pointDrag.before, pointDrag.hit, [geo.lon, geo.lat]);
      pointDrag.moved = true;
      drawOverlayRef.current();
      return;
    }

    // The cursor says what a click here would do: a bucket inside a selected
    // region with a tool that fills one — the same predicate the click uses —
    // and the tool's own cursor everywhere else (M24).
    //
    {
      const geo = unproject(cameraRef.current, viewRef.current, point);
      const image = imageLayersRef.current.find((image) => image.layer === activeLayer);
      const handle = tool === HAND && image && imageResizeHandleAt(image, cameraRef.current,
        viewRef.current, point, 9 * (window.devicePixelRatio || 1));
      applyCursor(
        region !== null &&
          drawsObjects(tool) &&
          editsRegion(tool) &&
          regionContains(region, geo.lon, geo.lat),
        dragging.current !== null,
        // The hand over the active picture moves it rather than panning
        // (M36), so the cursor has to say so on hover rather than on the
        // release. Only worth asking for the hand — no other tool changes
        // what it does over a picture.
        imageDrag.current !== null ||
          (tool === HAND &&
            imageUnder(
              imageLayersRef.current,
              activeLayer,
              cameraRef.current,
              viewRef.current,
              point,
            ) !== null),
        imageDrag.current?.cursor ?? (handle ? handle.cursor : null),
      );
    }

    const moving = imageDrag.current;
    if (moving) {
      const geo = unproject(cameraRef.current, viewRef.current, point);
      if (validGeo(geo)) moving.push(geo);
      return;
    }

    // A region being drawn follows the pointer. The lasso keeps every
    // coalesced position — a fast curve loses its corners otherwise, the same
    // reason a stroke reads them — and the other two need only the latest.
    const shaping = regionDrag.current;
    if (shaping) {
      const native = event.nativeEvent;
      const samples =
        typeof native.getCoalescedEvents === "function"
          ? native.getCoalescedEvents()
          : [native];
      for (const sample of samples) {
        const geo = unproject(cameraRef.current, viewRef.current, toDevice(sample));
        if (!validGeo(geo)) continue;
        shaping.points.push([geo.lon, geo.lat]);
      }
      requestOverlay();
      return;
    }

    // A warp being pulled follows the pointer; nothing else does while it is.
    if (pushDrag.current) {
      pushDrag.current.to = unproject(cameraRef.current, viewRef.current, point);
      requestOverlay();
      return;
    }

    // The eraser's stroke gains a point per coalesced report, thinned like a
    // brush stroke's, so a fast pass keeps its corners.
    if (eraseDrag.current) {
      const drag = eraseDrag.current;
      const native = event.nativeEvent;
      const samples =
        typeof native.getCoalescedEvents === "function"
          ? native.getCoalescedEvents()
          : [native];
      const spacing = 6 * (window.devicePixelRatio || 1);
      for (const sample of samples) {
        const geo = unproject(cameraRef.current, viewRef.current, toDevice(sample));
        if (!validGeo(geo)) continue;
        const last = drag.points[drag.points.length - 1];
        const moved =
          last === undefined ||
          Math.hypot(
            normalizeLon(geo.lon - last[0]) * cameraRef.current.pxPerDeg,
            (geo.lat - last[1]) * cameraRef.current.pxPerDeg,
          ) > spacing;
        if (moved) drag.points.push([geo.lon, geo.lat]);
      }
      // The map is the preview: the field leaves the stroke as it is drawn.
      if (refreshEraser()) requestDraw();
      requestOverlay();
      return;
    }

    // An operator tool highlights the edge it is over (spec.md 6.2, 6.3), and
    // the eraser the object it would take (M28). Kept in a ref and redrawn
    // only when the answer changes: this runs on every pointer report, and a
    // state change here would re-render the whole toolbar.
    if ((hoverTool !== null || tool === ERASE) && !gestureRef.current) {
      const over = tool === ERASE ? objectUnder(point) : operatorEdgeUnder(point);
      if (over !== hoveredOperator.current) {
        hoveredOperator.current = over;
        requestOverlay();
      }
    } else if (hoveredOperator.current !== null) {
      hoveredOperator.current = null;
      requestOverlay();
    }

    // Dragging a placed marker. The state change redraws the overlay: every
    // glyph aimed at this point swings round as it moves.
    const dragged = markerDrag.current;
    if (dragged !== null) {
      const geo = unproject(cameraRef.current, viewRef.current, point);
      setToolState({
        ...toolState,
        values: {
          ...toolState.values,
          [dragged]: { kind: "position", lon: geo.lon, lat: geo.lat },
        },
      });
      return;
    }

    if (handleDrag.current) {
      previewDrag(unproject(cameraRef.current, viewRef.current, point));
      return;
    }

    if (marquee.current) {
      marquee.current.to = point;
      requestOverlay();
      return;
    }

    const drawing = gestureRef.current;
    if (drawing?.kind === "stroke") {
      // Every position the OS captured, not just the one this frame delivered.
      // A `pointermove` fires about once per frame; a fast drag covers a lot of
      // ground between frames, and without the coalesced samples the stroke
      // records a coarse polyline that cuts the corners off fast curves.
      const native = event.nativeEvent;
      const samples =
        typeof native.getCoalescedEvents === "function"
          ? native.getCoalescedEvents()
          : [native];

      // Thin the path: a point every few pixels is plenty for a swept capsule
      // and keeps the stored geometry small.
      const spacing = 6 * (window.devicePixelRatio || 1);
      for (const sample of samples) {
        const geo = unproject(cameraRef.current, viewRef.current, toDevice(sample));
        if (!validGeo(geo)) continue;
        const last = drawing.points[drawing.points.length - 1];
        const moved =
          last === undefined ||
          Math.hypot(
            normalizeLon(geo.lon - last[0]) * cameraRef.current.pxPerDeg,
            (geo.lat - last[1]) * cameraRef.current.pxPerDeg,
          ) > spacing;
        if (moved) drawing.points.push([geo.lon, geo.lat]);
      }
      // For a tool that operates on the field, the map *is* the preview, so the
      // GL pass has to run too. No tile is refetched — the revision has not
      // moved — so this redraws textures that are already resident.
      if (refreshOperator(drawing)) requestDraw();
      requestOverlay();
      return;
    }

    // A preset is dragged out from its centre, so the pointer is its rim.
    if (drawing?.kind === "extent") {
      const geo = unproject(cameraRef.current, viewRef.current, point);
      drawing.rim = [geo.lon, geo.lat];
      if (refreshOperator(drawing)) requestDraw();
      requestOverlay();
      return;
    }

    // The pen pulls a node's handles out symmetrically while the button is
    // held, which is how every path tool behaves and what makes a smooth
    // corner possible without a second gesture.
    if (drawing?.kind === "path" && nodeDrag.current) {
      const geo = unproject(cameraRef.current, viewRef.current, point);
      shapeNode(drawing, [geo.lon, geo.lat]);
      requestOverlay();
      return;
    }

    // The capture region following the hand (spec.md 8.7). The local region
    // moves *now*, from the pointer's own delta, so there is no jump and no
    // wait on the wire; the backend is told one request at a time, latest
    // wins, and the reply is not used to move anything — it would arrive late
    // and put the region a report behind the hand.
    if (captureDrag.current) {
      const drag = captureDrag.current;
      const geo = unproject(cameraRef.current, viewRef.current, point);
      const lon = drag.anchor[0] + normalizeLon(geo.lon - drag.pointer[0]);
      const lat = Math.max(-90, Math.min(90, drag.anchor[1] + (geo.lat - drag.pointer[1])));
      setRegion((current) => (current === null ? current : recentred(current, lon, lat)));
      requestOverlay();

      const flight = captureMove.current;
      flight.queued = [lon, lat];
      if (!flight.inFlight) {
        const send = () => {
          const next = flight.queued;
          flight.queued = null;
          if (next === null) {
            flight.inFlight = false;
            return;
          }
          flight.inFlight = true;
          void api
            .placeCapture(stepRef.current, next[0], next[1])
            .then((mode) => setRecording(mode.active ? mode : null))
            .catch(() => undefined)
            .finally(send);
        };
        send();
      }
      return;
    }

    // The whole-picture drag (M36) and the image control-point drag
    // (spec.md 4.9, M18) used to be handled here. Both moved an image by its
    // three affine corners, which control points replace; Task 7's alignment
    // interaction is their successor and has its own gesture to add here.

    // A measurement handle being dragged (spec.md 10, M8). One request in
    // flight, latest wins: the pointer reports faster than a round trip, and a
    // request per report would queue up behind itself until the line was
    // following a position the cursor had left.
    if (measureDrag.current) {
      const grabbed = measureDrag.current;
      const geo = unproject(cameraRef.current, viewRef.current, point);
      const flight = measureMove.current;
      flight.queued = [geo.lon, geo.lat];
      if (!flight.inFlight) {
        const send = () => {
          const next = flight.queued;
          flight.queued = null;
          if (next === null) {
            flight.inFlight = false;
            return;
          }
          flight.inFlight = true;
          void api
            .moveMeasurementHandle(grabbed.id, grabbed.index, next)
            .then(tookMeasurements)
            .catch(() => undefined)
            .finally(send);
        };
        send();
      }
      return;
    }

    // The dividers' leg in progress reads out as it is drawn (M29): the
    // backend measures from the first point, or the open chain's last, to
    // the pointer, and the answer lands in the overlay with the rest.
    if (tool === MEASURE && !measureDrag.current) {
      const geo = unproject(cameraRef.current, viewRef.current, point);
      const from =
        pendingPoint.current ??
        (openChain.current !== null
          ? (measurementsRef.current.find((m) => m.id === openChain.current)?.handles.at(-1) ??
            null)
          : null);
      if (from !== null && measureKind !== "rings") {
        const leg: NewMeasurement = {
          kind: openChain.current !== null ? "dividers" : measureKind,
          points: [from, [geo.lon, geo.lat]],
          interval_km: 0,
          count: 0,
        };
        const flight = measurePreviewFlight.current;
        flight.queued = leg;
        if (!flight.inFlight) {
          const send = () => {
            const next = flight.queued;
            flight.queued = null;
            if (next === null) {
              flight.inFlight = false;
              return;
            }
            flight.inFlight = true;
            void api
              .previewMeasurement(next)
              .then((view) => {
                measurePreview.current = view;
                requestOverlay();
              })
              .catch(() => undefined)
              .finally(send);
          };
          send();
        }
      } else if (measurePreview.current !== null) {
        measurePreview.current = null;
      }
    }

    // A tool's hover indicator follows the cursor, and so does a pick's
    // crosshair — and so does the rubber line of a gesture being built up.
    // The magnifier copies the GL canvas, so it needs the whole frame drawn
    // again under it, not the overlay alone (M28).
    if (eyedropperRef.current) requestDraw();
    else if (tool !== HAND || picking !== null || alignStore.current!.get().layer !== null) requestOverlay();

    if (dragging.current) {
      const dx = point.x - dragging.current.x;
      const dy = point.y - dragging.current.y;
      dragging.current = point;
      // The camera moves against the pointer: the map follows the hand.
      cameraRef.current = panBy(cameraRef.current, viewRef.current, -dx, -dy);
      requestDraw();
    }

    readoutStore.current?.set({
      zoomPercent: Math.round(
        (cameraRef.current.pxPerDeg /
          minPxPerDeg(viewRef.current, projectionFor(cameraRef.current))) *
          100,
      ),
    });
    sampleAt(unproject(cameraRef.current, viewRef.current, point));
  };

  /**
   * Samples the field under the pointer for the readout — one request in
   * flight, the newest position waiting behind it.
   */
  const sampleAt = (geo: { lon: number; lat: number }) => {
    if (!validGeo(geo)) { readoutStore.current?.set({ sample: null }); return; }
    // Nobody is reading the number: the readout is hidden and the eyedropper
    // is not armed (M77). It costs a field evaluation per pointer report, so
    // it is not asked for. The magnifier shares this one stream deliberately,
    // which is why arming the eyedropper brings it back rather than starting
    // a second.
    if (!showReadout && !eyedropper) return;
    const state = sampling.current;
    if (state.inFlight) {
      state.queued = geo;
      return;
    }
    state.inFlight = true;
    const store = readoutStore.current;
    void api
      // Read from the refs rather than the closure: a queued request runs from
      // an earlier pointer report's promise, after the step may have moved.
      // The composite the map shows, whichever kind wins the cell (M31).
      .sampleField(geo.lon, geo.lat, stepRef.current, null)
      .then((sample) =>
        store?.set({
          sample: {
            lon: geo.lon,
            lat: geo.lat,
            speedKnots: knotsFromMps(sample.speed_mps),
            directionDeg: displayDirection(
              projectRef.current.direction_convention,
              sample.azimuth_toward_deg,
            ),
            azimuthTowardDeg: sample.azimuth_toward_deg,
            defined: sample.defined,
            kind: kindOf(sample.kind),
          },
        }),
      )
      // The eyedropper's magnifier shows this sample at the pointer, so it
      // is redrawn when the number lands rather than a pointer report later.
      .then(() => {
        if (eyedropperRef.current) requestOverlay();
      })
      .catch(() => store?.set({ sample: null }))
      .finally(() => {
        state.inFlight = false;
        const next = state.queued;
        state.queued = null;
        if (next) sampleAt(next);
      });
  };

  /**
   * The readout follows the timeline, not only the pointer.
   *
   * During playback the pointer stands still while the field moves under it,
   * so a readout sampled only on pointer reports keeps showing the step it
   * was last sampled at and disagrees with the map it sits on. Re-sampling on
   * every step change costs one field sample per frame, through the same
   * one-in-flight, latest-wins stream `sampleAt` already uses -- a timeline
   * faster than the backend answers drops samples rather than queueing them.
   *
   * `showReadout` and `eyedropper` are watched for the same reason: switching
   * either on with the pointer parked would otherwise show the sample from
   * whenever it was last on, until the pointer moved.
   */
  useEffect(() => {
    const point = cursorRef.current;
    if (!point) return;
    sampleAt(unproject(cameraRef.current, viewRef.current, point));
    // `sampleAt` is rebuilt every render and reads the rest from refs; the
    // step, and whether anyone is reading the number, are what this watches.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [step, showReadout, eyedropper]);

  /**
   * Commits a finished gesture.
   *
   * One path for every tool: the preview it holds, the layer it joins, the
   * settling rule that retires the preview and the error handling are all the
   * same question whatever was drawn, and asking it once is what stops a new
   * tool arriving without one of them (spec.md 6.1).
   */
  const commitGesture = useCallback(
    async (gesture: Gesture, state: ToolState, schema: ToolSchema, tool: Tool) => {
      const camera = cameraRef.current;
      const footprint = footprintOf(tool, state, gesture, camera);
      if (!footprint) return;

      // A pixel size becomes kilometres here and nowhere else: at the latitude
      // the gesture began, against the camera as it is now (spec.md 3.5). The
      // document only ever holds the kilometres.
      const lat = gestureLatitude(gesture);
      const field = previewField(tool, state, footprint);

      // Keep previewing until the field is drawn, with the values the gesture
      // froze rather than whatever the bar says by then (spec.md 6.1).
      const operator = operatorRef.current;
      const settled: HeldPreview = {
        footprint,
        paint: rampCss(
          mpsFromKnots(field.knots),
          rampMax,
          PREVIEW_MIN_ALPHA,
          rampMin,
          stopsOf(gradientsRef.current, projectRef.current[`${activeKindRef.current}_gradient`]),
        ),
        knots: field.knots,
        azimuthAt: field.azimuthAt,
        revision: null,
        at: performance.now(),
        // A tool that operates on the field keeps operating until its own field
        // arrives, and the overlay draws nothing for it; one that adds a field
        // keeps showing the field it added.
        ...(operator ? { operator, silent: true } : {}),
      };
      settling.current = [...settling.current, settled];

      setBusy(true);
      try {
        const created = await api.createObject(
          newObject(tool, gesture, state, schema, camera, lat, activeLayer),
        );
        settled.revision = created.project.revision;
        onProjectChanged(created.project);
        // What was just made is what the user is holding (M29): the panel
        // and the inspector turn to it, as they do to a clicked object.
        onSelect([created.object]);
        // The revision change re-addresses every tile, but if they were all
        // cached nothing else would schedule the draw that retires the preview.
        requestDraw();
        // ...and if the new tiles never arrive at all, the backstop does.
        window.setTimeout(requestDraw, SETTLE_TIMEOUT_MS + 100);
      } catch (err) {
        // Nothing was painted, so nothing should keep looking painted.
        settling.current = settling.current.filter((entry) => entry !== settled);
        drawOverlayRef.current();
        void api.frontendLog("error", `${tool} gesture failed: ${String(err)}`);
      } finally {
        setBusy(false);
      }
    },
    [activeLayer, onProjectChanged, onSelect, rampMax, rampMin, requestDraw],
  );

  /** Commits the gesture in progress, if it has enough placed to mean anything. */
  const finishGesture = useCallback(() => {
    const drawing = gestureRef.current;
    gestureRef.current = null;
    nodeDrag.current = false;

    if (!drawing || !schema || tool === HAND) return;
    // The commit picks the preview up synchronously, so the overlay redraw
    // never sees a moment with neither the gesture nor its field.
    // A press and release at one point describes a shape of no size, and one
    // with a pixel of tremor describes an object the user cannot see and did
    // not ask for. Both are no gesture at all.
    if (isComplete(drawing, cameraRef.current.pxPerDeg)) {
      // The commit takes the live operation over as a held one, so the map
      // never stops showing the erasure between the release and the tiles.
      if (schemaTool) void commitGesture(finished(drawing), toolState, schema, schemaTool);
    }
    // ...and once it has, the live one is done with either way.
    if (refreshOperator(null)) requestDraw();
    drawOverlayRef.current();
  }, [commitGesture, refreshOperator, requestDraw, schema, tool, toolState]);

  useEffect(() => {
    finishGestureRef.current = finishGesture;
  }, [finishGesture]);

  useEffect(() => {
    abandonRef.current = () => {
      if (refreshOperator(null)) requestDraw();
      drawOverlayRef.current();
    };
  }, [refreshOperator, requestDraw]);

  /**
   * Begins, extends or completes a gesture at a pointer press.
   *
   * Which of the three depends only on the gesture's kind, so a tool that draws
   * the way another one does behaves the way it does — the mask is brush-like
   * because both send a `stroke`, not because two branches were written alike.
   */
  /**
   * Begins, extends or completes a gesture at a pointer press.
   *
   * Which of the three depends only on the gesture's kind, so a tool that draws
   * the way another one does behaves the way it does — the mask is brush-like
   * because both send a `stroke`, not because two branches were written alike.
   */
  const startGesture = useCallback(
    (geo: { lon: number; lat: number }, event: React.PointerEvent<HTMLCanvasElement>) => {
      if (!schema || tool === HAND || !validGeo(geo)) return;
      // A stroke aimed at a hidden layer does nothing, and must be seen to do
      // nothing (M68). The backend refuses it, but only on release — and for
      // the whole drag before that the preview would show the edit happening
      // to the layers that *are* visible, a liquify's most of all, since its
      // preview displaces the rendered field itself and has no layer of its
      // own to displace.
      if (activeLayerHidden()) {
        setHint("That layer is hidden. Show it before drawing on it.");
        return;
      }
      const kind = gestureKind(schema, toolState.values);
      const at: [number, number] = [geo.lon, geo.lat];
      const current = gestureRef.current;

      // Whether the press landed on the ring's own first vertex, which is the
      // one thing the state machine cannot decide for itself: it is a distance
      // on screen, and only the camera knows that.
      const first = current?.kind === "ring" ? current.points[0] : undefined;
      const onFirstVertex =
        first !== undefined &&
        near(
          toDevice(event),
          toScreen(cameraRef.current, viewRef.current, { lon: first[0], lat: first[1] }),
        );

      const outcome = press(kind, current, at, onFirstVertex);
      switch (outcome.act) {
        case "commit":
          if (schemaTool) void commitGesture(outcome.gesture, toolState, schema, schemaTool);
          return;
        case "close":
          finishGesture();
          return;
        case "draw":
          gestureRef.current = outcome.drawing;
          nodeDrag.current = outcome.shapeHandles;
          break;
      }
      if (refreshOperator(gestureRef.current)) requestDraw();
      drawOverlay();
    },
    [
      activeLayerHidden,
      commitGesture,
      finishGesture,
      near,
      refreshOperator,
      requestDraw,
      schema,
      tool,
      toolState,
    ],
  );

  const endDrag = (event: React.PointerEvent<HTMLCanvasElement>) => {
    const rawPoint = toDevice(event);
    const releasePoint = validGeo(unproject(cameraRef.current, viewRef.current, rawPoint)) ? rawPoint : lastMapPointer.current ?? rawPoint;
    const pointDrag = shapeDrag.current;
    if (pointDrag?.pointer === event.pointerId) shapeDrag.current = null;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }

    if (pointDrag && pointDrag.pointer === event.pointerId) {
      const shown = shapeControls.current;
      const to = shown?.rings[pointDrag.hit.ring]?.[pointDrag.hit.point];
      if (event.type === "pointerup" && pointDrag.moved && to && shapeModeRef.current === pointDrag.before.object) {
        void api.moveShapePoint(pointDrag.before.object, pointDrag.before.step,
          pointDrag.hit.ring, pointDrag.hit.point, to[0], to[1], pointDrag.before.revision)
          .then(onProjectChanged).catch((error: unknown) => {
            shapeControls.current = pointDrag.before;
            drawOverlayRef.current();
            reportError(String(error));
          });
      } else { shapeControls.current = pointDrag.before; drawOverlayRef.current(); }
      return;
    }
    if (shapeEditing !== null) { dragging.current = null; pressOrigin.current = null; return; }

    // The picture has arrived. `finishGesture` breaks the coalescing key, so
    // the whole drag is one undo entry and the next drag starts another —
    // and it clears the baseline, which a stale one would make the next drag
    // compute from where this one started.
    if (imageDrag.current) {
      const moving = imageDrag.current;
      const at = unproject(cameraRef.current, viewRef.current, releasePoint);
      if (event.type !== "pointercancel" && validGeo(at)) moving.push(at);
      moving.finish();
      imageDrag.current = null;
      dragging.current = null;
      pressOrigin.current = null;
      applyCursor(false, false, true);
      return;
    }

    // In a macro preview a press that barely moved is a click, which places a
    // copy; one that travelled was a pan and places nothing (M33).
    if (previewPress.current) {
      previewPress.current = false;
      const origin = pressOrigin.current;
      pressOrigin.current = null;
      dragging.current = null;
      applyCursor(false, false);
      const point = releasePoint;
      const slack = 4 * (window.devicePixelRatio || 1);
      if (origin && Math.hypot(point.x - origin.x, point.y - origin.y) <= slack) {
        const geo = unproject(cameraRef.current, viewRef.current, point);
        void api
          .placePreview(geo.lon, geo.lat)
          .then(setCapture)
          .catch((err: unknown) => setError(String(err)));
      }
      return;
    }
    // The eraser's stroke lands as one write — one undo for the whole pass.
    if (eraseDrag.current) {
      const drag = eraseDrag.current;
      const brush = eraserRef.current;
      // The live removal is held until the erased tiles land, as a mask's is
      // (M32): dropping it at the release would let the field fill back in
      // for the round trip and then empty again.
      const operator = operatorRef.current;
      const held: HeldPreview | null = operator
        ? {
            footprint: {
              kind: "swept",
              points: drag.points,
              radiusKm: drag.radiusKm,
              shape: brush.shape,
              space: spaceFor(brush.unit, cameraRef.current),
            },
            paint: "transparent",
            knots: 0,
            azimuthAt: () => 0,
            revision: null,
            at: performance.now(),
            operator,
            silent: true,
          }
        : null;
      if (held) settling.current = [...settling.current, held];
      eraseDrag.current = null;
      operatorRef.current = null;
      void api
        .eraseStroke({
          points: drag.points,
          radius_km: drag.radiusKm,
          square: brush.shape === "square",
          space: spaceFor(brush.unit, cameraRef.current),
          feather: brush.feather,
          step: drag.step,
          at_step: stepRef.current,
          layer: activeLayer,
        })
        .then((summary) => {
          if (held) held.revision = summary.revision;
          onProjectChanged(summary);
          requestDraw();
          window.setTimeout(requestDraw, SETTLE_TIMEOUT_MS + 100);
        })
        .catch((err: unknown) => {
          if (held) settling.current = settling.current.filter((entry) => entry !== held);
          setError(String(err));
        });
      hoveredOperator.current = null;
      requestDraw();
      requestOverlay();
      return;
    }

    // The capture region lets go. Nothing to end on the backend: a placement
    // is capture state, not a history entry.
    if (captureDrag.current) {
      captureDrag.current = null;
      requestOverlay();
      return;
    }

    // A measurement handle lets go, and the coalescing group ends with it:
    // the next drag of the same handle must be its own undo entry rather than
    // merging into this one.
    if (measureDrag.current) {
      measureDrag.current = null;
      void api.endGesture().catch(() => undefined);
      requestOverlay();
      return;
    }

    // A region closes on release: a lasso becomes its own polygon, the other
    // two the shape the drag described. A drag too small to be a region is a
    // click, and a click on empty map clears the selection.
    const drawn = regionDrag.current;
    if (drawn) {
      regionDrag.current = null;
      const last = drawn.points[drawn.points.length - 1] ?? drawn.from;
      selectRegion(
        regionMode === "lasso"
          ? regionFromLasso(drawn.points)
          : regionFromDrag(regionMode, drawn.from, last),
      );
      requestOverlay();
      return;
    }
    // Finish a warp's pull: one write, at the step being viewed and through the
    // same path every other property edit takes — so it keys the current step
    // when the property is animated or auto-key is on, and both ends of the
    // push are keyframable (spec.md 6.3, 9.3).
    const pull = pushDrag.current;
    if (pull) {
      pushDrag.current = null;
      void api
        .setObjectProperty(
          pull.object,
          "PushTo",
          { kind: "position", lon: pull.to.lon, lat: pull.to.lat },
          step,
          autoKey,
        )
        .then(onProjectChanged)
        .catch((err: unknown) =>
          void api.frontendLog("error", `pulling the warp failed: ${String(err)}`),
        );
      requestOverlay();
      return;
    }

    // Finish a marker drag. Nothing to commit: a tool's placed position is an
    // option, not document state, until a gesture freezes it (spec.md 6.1).
    if (markerDrag.current !== null) {
      markerDrag.current = null;
      return;
    }

    // Finish a handle drag: one last write, then close the undo entry so the
    // whole drag collapses into a single step.
    const drag = handleDrag.current;
    if (drag) {
      handleDrag.current = null;
      const preview = dragPreview.current;
      dragPreview.current = null;
      // The overlay keeps the outline up until the field catches up, the same
      // way a committed stroke does: the write re-renders every visible tile,
      // and dropping it first would snap the object back to where it started
      // for as long as that takes.
      if (preview) settlingDrag.current = { ...preview, revision: null, at: performance.now() };

      // One more preview before the write, at the position the pointer actually
      // finished on: the last one in flight can be a round trip behind a fast
      // flick, and it is what stays on screen until the tiles arrive. It has to
      // go first — `endGesture` drops the baseline it reads.
      const release = unproject(cameraRef.current, viewRef.current, releasePoint);
      void api
        .previewTransform(release.lon, release.lat)
        .then((final) => {
          const held = settlingDrag.current;
          if (held && final) {
            settlingDrag.current = { ...final, revision: held.revision, at: held.at };
            drawOverlayRef.current();
          }
        })
        .catch(() => undefined)
        .finally(() => {
          // The write, *then* the end of the gesture — `endGesture` drops the
          // baseline the write reads, so the two must not be left to race.
          void commitDrag(release).finally(() => void api.endGesture());
        });
      return;
    }

    // Finish a rubber band: everything it touched joins the selection.
    const band = marquee.current;
    marquee.current = null;
    if (band) {
      const bounds = marqueeBounds(
        unproject(cameraRef.current, viewRef.current, band.from),
        unproject(cameraRef.current, viewRef.current, band.to),
        band.from.x <= band.to.x,
      );
      const layer = band.crossLayer ? null : activeLayer;
      void api
        .objectsInRegion(bounds.west, bounds.south, bounds.east, bounds.north, step, layer)
        .then((found) => onSelect([...new Set([...selection, ...found])]))
        .catch(() => undefined);
      drawOverlay();
      return;
    }

    // A press that barely moved is a click, not a pan: select what is under it.
    const origin = pressOrigin.current;
    pressOrigin.current = null;
    if (tool === "hand" && origin) {
      const point = releasePoint;
      const slack = 4 * (window.devicePixelRatio || 1);
      if (Math.hypot(point.x - origin.x, point.y - origin.y) <= slack) {
        const geo = unproject(cameraRef.current, viewRef.current, point);
        const toggle = event.metaKey || event.ctrlKey;
        void api
          .objectAt(geo.lon, geo.lat, step)
          .then((hit) => {
            if (hit === null) {
              // Clicking nothing clears, unless the modifier says the user is
              // building a selection and simply missed.
              if (!toggle) onSelect([]);
              return;
            }
            if (!toggle) {
              onSelect([hit]);
            } else {
              onSelect(
                selection.includes(hit)
                  ? selection.filter((id) => id !== hit)
                  : [...selection, hit],
              );
            }
          })
          .catch(() => onSelect([]));
      }
    }
    dragging.current = null;
    applyCursor(false, false);

    // A gesture that ends with the pointer is finished here; one built up click
    // by click keeps going until it is closed or cancelled.
    nodeDrag.current = false;
    // A gesture bounded by the pointer ends here; one built up click by click
    // is held until it is closed or abandoned.
    if (release(gestureRef.current) === "finish") finishGesture();
    drawOverlay();
  };

  const onWheel = (event: React.WheelEvent<HTMLCanvasElement>) => {
    const factor = Math.pow(2, -event.deltaY / 350);
    cameraRef.current = zoomAbout(
      cameraRef.current,
      viewRef.current,
      toDevice(event),
      factor,
    );
    requestDraw();
  };

  /**
   * The map's framebuffer with the overlay on it, as base64 PNG, once the
   * tiles have settled.
   *
   * Split out of `capture` (M76) because the MCP service's screenshot wants
   * the same picture and not the file: the debug capture writes it beside the
   * logs, the service hands it back over IPC.
   */
  const framebufferPng = useCallback(async (): Promise<string | null> => {
    // Waited for, not demanded: the renderer comes up asynchronously — a GL
    // context, then the basemap — while the effects that can ask for a capture
    // have already run. Giving up on the first look made an automated capture
    // a race against the map's own start-up (M71).
    let renderer = rendererRef.current;
    for (let attempt = 0; !renderer && attempt < 100; attempt++) {
      await new Promise((resolve) => window.setTimeout(resolve, 100));
      renderer = rendererRef.current;
    }
    if (!renderer) {
      void api.frontendLog("warn", "capture requested before the renderer was ready");
      return null;
    }
    // Wait for the tiles to settle: capturing mid-load photographs a
    // half-drawn map and looks like a rendering bug.
    //
    // **Settled means some arrived and none is outstanding** (M71). Waiting on
    // `pending === 0` alone was satisfied before the map had asked for
    // anything at all, so a capture taken just after a project opened
    // photographed the basemap with no field on it — which looks exactly like
    // a field that failed to render. A project whose viewport holds no tile
    // never satisfies it, so the attempts are still bounded and the capture
    // goes ahead either way.
    //
    // Bounded by the **clock**, not by a count of attempts (M78). A window
    // that is not composited has its timers throttled towards one a second,
    // so a hundred hundred-millisecond waits is ten seconds in front and a
    // minute and a half behind — which is exactly when a driver is taking the
    // picture.
    const settleBy = performance.now() + 10_000;
    for (;;) {
      const tiles = tilesRef.current?.stats();
      if (tiles && tiles.pending === 0 && tiles.ready > 0) break;
      if (performance.now() > settleBy) break;
      await new Promise((resolve) => window.setTimeout(resolve, 100));
    }

    const pendingCapture = renderer.captureNextFrame();
    requestDraw();
    // A frame may never arrive — `requestAnimationFrame` is suspended while
    // the window is not composited, and the scheduler's timer fallback is
    // throttled with everything else. Better to say so than to hang (M78).
    const image = await Promise.race([
      pendingCapture,
      new Promise<null>((resolve) => window.setTimeout(() => resolve(null), 20_000)),
    ]);
    if (!image) {
      void api.frontendLog("warn", "capture: no frame was drawn to read back");
      return null;
    }

    const scratch = document.createElement("canvas");
    scratch.width = image.width;
    scratch.height = image.height;
    const onto = scratch.getContext("2d");
    onto?.putImageData(image, 0, 0);
    // The overlay on top of the field (M78). Handles, outlines, brush
    // footprints and every gesture preview are drawn on a second canvas over
    // the GL one, so a capture of the framebuffer alone is the map with all
    // of that missing — which is most of what there is to look at when the
    // question is about an edge rather than about the field.
    if (onto && overlayRef.current) onto.drawImage(overlayRef.current, 0, 0);
    const blob = await new Promise<Blob | null>((resolve) =>
      scratch.toBlob(resolve, "image/png"),
    );
    if (!blob) return null;

    const dataUrl = await new Promise<string>((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(String(reader.result));
      reader.onerror = () => reject(reader.error ?? new Error("read failed"));
      reader.readAsDataURL(blob);
    });
    const base64 = dataUrl.slice(dataUrl.indexOf(",") + 1);
    return base64;
  }, [requestDraw]);

  /**
   * Reads the next frame back and writes it beside the logs, returning where.
   *
   * The path is returned as well as logged because automation needs it: a
   * driver asks for a capture and then reads that file (M71).
   */
  const capture = useCallback(
    async (name: string): Promise<string | null> => {
      const base64 = await framebufferPng();
      if (base64 === null) return null;
      const path = await api.saveDebugCapture(name, base64);
      void api.frontendLog("info", `capture written to ${path}`);
      return path;
    },
    [framebufferPng],
  );

  /**
   * The capture, reachable from a script injected into the webview (M71).
   *
   * Automation drives the real application, and what it wants a picture of is
   * the map — which is WebGL. The WebDriver plugin's own `/screenshot`
   * rasterises the *DOM* through an SVG `foreignObject`, and that does not
   * capture a canvas's contents at all: the map would come out blank. This
   * reads the framebuffer back instead, which is what the capture suite
   * already does, and waits for the tiles to settle first so a driver never
   * photographs a half-loaded map.
   *
   * **Development builds only.** `import.meta.env.DEV` is false in anything
   * `npm run build` produces, so a shipped bundle carries no such hook. It
   * writes a PNG beside the logs under a sanitised basename, which is not
   * dangerous, but a shipped application should not have a door in it that
   * exists for a test.
   */
  useEffect(() => {
    if (!import.meta.env.DEV) return;
    const hooks = window as unknown as { __veCapture?: (name: string) => Promise<string | null> };
    hooks.__veCapture = capture;
    return () => {
      delete hooks.__veCapture;
    };
  }, [capture]);

  // The MCP service's screenshot (spec 8.8). Not a window global and not
  // dev-only: it answers an event the backend emits only while the person
  // has the service on, with the same picture the capture suite takes.
  useEffect(() => {
    const pending = listen<CaptureRequest>("view://capture", async (event) => {
      const id = event.payload.id;
      try {
        const png = await framebufferPng();
        if (png === null) {
          // Say so at once: the tool would otherwise wait out its timeout
          // and report silence it cannot explain.
          await api.refuseCapture(id, "no frame to capture: is the window shown and the map settled?");
          return;
        }
        await api.deliverCapture(id, png);
      } catch (err) {
        // A throw anywhere above is still an answer the tool can use. The
        // refusal may itself fail if the request has already been forgotten;
        // that is not worth a second warning.
        void api.refuseCapture(id, `capture failed: ${String(err)}`).catch(() => {});
        void api.frontendLog("warn", `capture ${id} not answered: ${String(err)}`);
      }
    });
    return () => {
      void pending.then((unlisten) => unlisten());
    };
  }, [framebufferPng]);

  /**
   * Development capture suite.
   *
   * Renders a few deliberate scenarios and writes each to the log directory,
   * alongside the field values sampled at the same points. That turns "do the
   * glyphs point the right way" and "is the dateline seamless" into checks
   * against numbers rather than an impression of a screenshot.
   */
  const runCaptureSuite = useCallback(async () => {
    // Paint through the real command path, so the capture proves the whole
    // loop: stroke -> command -> history -> revision bump -> new tiles.
    if (projectRef.current.object_count === 0) {
      const arc: Array<[number, number]> = [];
      for (let i = 0; i <= 20; i++) {
        const t = i / 20;
        arc.push([-70 + t * 90, 20 + Math.sin(t * Math.PI) * 28]);
      }
      let summary = await api.addBrushStroke({
        points: arc,
        size_km: 900,
        speed_mps: 28,
        direction_toward_deg: 75,
        feather: 0.5,
        shape: "circle",
        space: "geodesic",
        direction_mode: "constant",
      });

      summary = await api.addBrushStroke({
        points: [
          [-30, -35],
          [10, -40],
          [50, -35],
        ],
        size_km: 1800,
        speed_mps: 14,
        direction_toward_deg: 300,
        feather: 0.8,
        // A square stamp and an aimed direction, so the capture exercises both
        // new brush options rather than only the defaults.
        shape: "square",
        space: "geodesic",
        direction_mode: "constant",
      });

      summary = await api.addBrushStroke({
        points: [[150, 45]],
        size_km: 2600,
        speed_mps: 40,
        direction_toward_deg: 180,
        feather: 0.2,
        shape: "circle",
        // A projected stamp, at 45 degrees where the two spaces differ
        // visibly: this one must come out round on screen, where a geodesic
        // stamp of the same size comes out half again as wide as it is tall.
        space: "projected",
        // Aimed away from a point due south of it, so the capture shows the
        // outward mode: every glyph in this stamp must point north.
        direction_mode: "away_from_point",
        target: [150, 0],
      });

      projectRef.current = summary;
      onProjectChanged(summary);
      void api.frontendLog(
        "info",
        `painted ${summary.object_count} strokes, revision ${summary.revision}`,
      );
      await new Promise((resolve) => window.setTimeout(resolve, 300));
    }

    const scenarios: Array<{
      name: string;
      camera: Camera;
      probes: Array<{ lon: number; lat: number }>;
    }> = [
      {
        name: "world",
        camera: { centerLon: 0, centerLat: 0, pxPerDeg: 8 },
        probes: [],
      },
      {
        name: "zoom",
        camera: { centerLon: -40, centerLat: 35, pxPerDeg: 60 },
        // The cyclone eye sits at -40, 35 at step 0. North and south of it the
        // rotation must be opposite; east and west likewise.
        probes: [
          { lon: -40, lat: 35 },
          { lon: -40, lat: 42 },
          { lon: -40, lat: 28 },
          { lon: -31, lat: 35 },
          { lon: -49, lat: 35 },
          { lon: -40, lat: 60 },
        ],
      },
      {
        // The zoom that previously showed glyphs clustered into blocks with
        // gaps at every tile boundary.
        name: "lattice",
        camera: { centerLon: -128, centerLat: 48, pxPerDeg: 34 },
        probes: [],
      },
      {
        name: "dateline",
        camera: { centerLon: 180, centerLat: 20, pxPerDeg: 24 },
        probes: [
          { lon: 179, lat: 20 },
          { lon: -179, lat: 20 },
        ],
      },
    ];

    for (const scenario of scenarios) {
      cameraRef.current = clampCamera(scenario.camera, viewRef.current);
      // Set the ref directly: a state update is async and would not reach the
      // draw that follows on this tick.
      showGlyphsRef.current = true;
      requestDraw();

      for (const probe of scenario.probes) {
        const sample = await api.sampleField(probe.lon, probe.lat, 0, "wind");
        void api.frontendLog(
          "info",
          `probe ${scenario.name} lon=${probe.lon} lat=${probe.lat} ` +
            `speed=${knotsFromMps(sample.speed_mps).toFixed(1)}kt ` +
            `toward=${sample.azimuth_toward_deg.toFixed(1)} ` +
            `shown=${displayDirection(project.direction_convention, sample.azimuth_toward_deg).toFixed(1)}`,
        );
      }

      await new Promise((resolve) => window.setTimeout(resolve, 400));
      await capture(scenario.name);
    }
    // Measure the draw cost directly. Simulating a pan rather than redrawing a
    // static frame, so tile selection and the wrap logic are included -- those
    // are the parts that change as the camera moves.
    const renderer = rendererRef.current;
    if (renderer) {
      const view = viewRef.current;
      const base: Camera = clampCamera(
        { centerLon: 0, centerLat: 20, pxPerDeg: 24 },
        view,
      );
      const frames = 120;
      // Warm up so shader and texture binds are not counted as steady state.
      for (let i = 0; i < 10; i++) {
        renderer.render({
          camera: { ...base, centerLon: base.centerLon + i * 0.25 },
          view,
          frame: `${projectRef.current.revision}/0`,
          heldFrame: null,
          ramps: {
            wind: { min: rampMin, max: rampMax },
            current: { min: rampMin, max: rampMax },
          },
          gradients: gradientStops(projectRef.current),
          showGlyphs: true, showGraticule: true,
          pixelRatio: window.devicePixelRatio || 1,
        });
      }

      const started = performance.now();
      for (let i = 0; i < frames; i++) {
        renderer.render({
          camera: { ...base, centerLon: base.centerLon + i * 0.25 },
          view,
          frame: `${projectRef.current.revision}/0`,
          heldFrame: null,
          ramps: {
            wind: { min: rampMin, max: rampMax },
            current: { min: rampMin, max: rampMax },
          },
          gradients: gradientStops(projectRef.current),
          showGlyphs: true, showGraticule: true,
          pixelRatio: window.devicePixelRatio || 1,
        });
      }
      const perFrame = (performance.now() - started) / frames;
      void api.frontendLog(
        "info",
        `draw benchmark: ${perFrame.toFixed(2)} ms/frame over ${frames} frames ` +
          `at ${view.width}x${view.height} (budget 16.7 ms for 60 fps)`,
      );
    }

    void api.frontendLog("info", "capture suite complete");
  }, [
    capture,
    onProjectChanged,
    project.direction_convention,
    rampMax,
    rampMin,
    requestDraw,
  ]);

  return (
    <div className="map">
      <canvas
        ref={canvasRef}
        className="map-canvas"
        tabIndex={0}
        onPointerDownCapture={(event) => focusMapForGesture(event.currentTarget)}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerLeave={() => {
          if (alignStore.current!.get().layer !== null) {
            cursorRef.current = null;
            requestOverlay();
          }
        }}
        onPointerUp={endDrag}
        onPointerCancel={endDrag}
        onLostPointerCapture={(event) => {
          const drag = shapeDrag.current;
          if (drag?.pointer !== event.pointerId) return;
          shapeControls.current = drag.before;
          shapeDrag.current = null;
          drawOverlayRef.current();
        }}
        onWheel={onWheel}
      />

      <canvas ref={overlayRef} className="map-overlay" />

      {error === null && !ready && <div className="map-status">Loading basemap…</div>}
      {previewing && <div className="map-preview-frame" aria-hidden="true" />}
      {previewing && <div className="map-preview-badge">Macro Preview</div>}
      {shapeEditing !== null && <div className="shape-edit-hint" role="status">
        Shape animation · Drag a dot to key frame {step}. Choose a tool or press Escape to finish.
      </div>}

      <div className="map-toolbar">
        <div className="tools" role="group" aria-label="Tool">
          {/*
            Icon-only, so the name has to reach anyone not reading the picture:
            `aria-label` carries it, and `title` carries it plus the shortcut
            for anyone who hovers.
          */}
          <button
            className={tool === HAND && shapeEditing === null ? "icon active" : "icon"}
            disabled={recording !== null}
            onClick={() => setTool(HAND)}
            aria-label="Hand"
            aria-pressed={tool === HAND && shapeEditing === null}
            title={`Hand (${chord("hand")}) · pan, select and transform · shift-drag for a rubber band, add cmd to reach across layers · cmd-click to add or remove one object`}
          >
            <ToolIcon tool={HAND} />
          </button>
          {/*
            The tool that draws a *region* rather than an object (spec.md 8.2,
            M14). Not in the backend's palette: it describes vector-creation
            tools, and the select tool makes no object at all — what it makes
            is the footprint the brush and the operators then apply to.
          */}
          <button
            className={tool === SELECT ? "icon active" : "icon"}
            disabled={recording !== null}
            onClick={() => setTool(SELECT)}
            aria-label="Select"
            aria-pressed={tool === SELECT}
            title={`Select (${chord("select")}) · drag a region of the map · cmd-A selects the view, cmd-shift-A the whole map, cmd-D clears`}
          >
            <ToolIcon tool={SELECT} />
          </button>
          {/*
            The eraser (spec.md 8.1, M28). Not in the backend's palette: it
            makes no object, it takes them away — from every frame, or with
            Shift from this one, by removing the geometry it covers.
          */}
          <button
            className={tool === ERASE ? "icon active" : "icon"}
            disabled={recording !== null}
            onClick={() => setTool(ERASE)}
            aria-label="Erase"
            aria-pressed={tool === ERASE}
            title={`Erase (${chord("erase")}) · drag to erase geometry in the active layer · hold Shift to erase from this frame only`}
          >
            <ToolIcon tool={ERASE} />
          </button>
          {/*
            The measurement tools (spec.md 10, M8). Not in the backend's
            palette either, and for the plainest reason of the three: a
            measurement is not an object at all. It adds nothing to the field
            and reaches no exported file — it is drawn on the map to read a
            number off it.
          */}
          {/*
            The macro tools (spec.md 8.7, M16). Neither is in the backend's
            palette: the capture tool makes a *file*, not an object, and the
            insert tool makes a macro object, which has a library to choose
            from rather than a bar of options to describe.
          */}
          <button
            className={tool === INSERT ? "icon active" : "icon"}
            disabled={recording !== null}
            onClick={() => setTool(INSERT)}
            aria-label="Insert macro"
            aria-pressed={tool === INSERT}
            title={`Insert macro (${chord("insert")}) · put a captured run of frames back on the map`}
          >
            <ToolIcon tool={INSERT} />
          </button>
          {palette.map((entry) => (
            <button
              key={entry.tool}
              className={tool === entry.tool ? "icon active" : "icon"}
              disabled={recording !== null}
              onClick={() => setTool(entry.tool)}
              aria-label={entry.label}
              aria-pressed={tool === entry.tool}
              title={`${entry.label} (${chord(entry.tool)})`}
            >
              <ToolIcon tool={entry.tool} />
            </button>
          ))}
        </div>

        {/*
          The select tool's bar. Bespoke rather than schema-driven, and
          deliberately: the schema describes an *object's* properties, and a
          region has no object for one to live on. Three modes and a way to
          clear, which is the whole of what a region can be told.
        */}
        {tool === SELECT && (
          <div className="tool-options" role="group" aria-label="Select options">
            <label>
              Shape
              <ToolSelect
                value={regionMode}
                onChange={(event) => {
                  setRegionMode(event.target.value as RegionMode);
                  releaseFocus(event);
                }}
                title="Rectangle and lasso are drawn corner to corner and freehand; a circle is dragged out from its centre"
              >
                <option value="rect">Rectangle</option>
                <option value="circle">Circle</option>
                <option value="lasso">Lasso</option>
              </ToolSelect>
            </label>
            <button
              disabled={region === null}
              onClick={() => {
                setRegion(null);
                requestOverlay();
              }}
              title="Clear the selected region (cmd-D)"
            >
              Deselect
            </button>
          </div>
        )}

        {/*
          The capture bar (spec.md 8.7, M16). Before a capture it draws the
          region and says what will be recorded; during one it is the only way
          out, because every document write is refused while it runs.
        */}
        {(tool === CAPTURE || recording !== null) && (
          <div className="tool-options" role="group" aria-label="Capture options">
            {recording === null ? (
              <>
                <label>
                  Shape
                  <ToolSelect
                    value={regionMode}
                    onChange={(event) => {
                  setRegionMode(event.target.value as RegionMode);
                  releaseFocus(event);
                }}
                    title="The select tool's own gestures: rectangle and lasso are drawn corner to corner and freehand, a circle from its centre"
                  >
                    <option value="rect">Rectangle</option>
                    <option value="circle">Circle</option>
                    <option value="lasso">Lasso</option>
                  </ToolSelect>
                </label>
                <label title="Static writes every frame as if the region never moved, so a region dragged to follow a system yields that system standing still. Record movement keeps each frame's displacement from the first.">
                  <input
                    type="checkbox"
                    checked={recordMovement}
                    onChange={(event) => setRecordMovement(event.target.checked)}
                  />
                  Record movement
                </label>
                <button
                  disabled={region === null || busy}
                  onClick={() => {
                    if (region === null) return;
                    void api
                      .startCapture(regionShape(region), stepRef.current, recordMovement, activeKind)
                      .then((mode) => setRecording(mode.active ? mode : null))
                      .catch((err: unknown) => setError(String(err)));
                  }}
                  title="Start recording. Until it is finished or cancelled, every edit is refused."
                >
                  Start capture
                </button>

              </>
            ) : (
              <>
                {recording.phase === "recording" ? (
                  <span className="accent">
                    Recording from step {recording.first_step} · {recording.keys.length} keys
                    {recording.record_movement ? " · movement" : " · static"}
                  </span>
                ) : (
                  <span className="accent">
                    Preview · steps {recording.first_step}–{recording.last_step}
                  </span>
                )}
                {recording.phase === "recording" && (
                  <button
                    onClick={() => {
                      // The run ends on the frame the playhead is on: a
                      // macro is as long as the user recorded, not as long
                      // as the timeline (spec.md 8.7).
                      void api
                        .previewCapture(stepRef.current)
                        .then(setCapture)
                        .catch((err: unknown) => setError(String(err)));
                    }}
                    title="End the recording on this frame, bake the frames and show the macro alone on the map, looping. Nothing is written."
                  >
                    Finish recording
                  </button>
                )}
                {recording.phase === "previewing" && (
                  <button
                    onClick={() => {
                      setCaptureName(null);
                      void api
                        .editCapture()
                        .then(setCapture)
                        .catch((err: unknown) => setError(String(err)));
                    }}
                    title="Back to recording, keys intact"
                  >
                    Edit macro
                  </button>
                )}
                {recording.phase === "previewing" &&
                  (captureName === null ? (
                  <button onClick={() => setCaptureName(`Macro ${library?.entries.length ?? 0}`)}>
                    Save macro…
                  </button>
                ) : (
                  <>
                    <input
                      autoFocus
                      value={captureName}
                      onChange={(event) => setCaptureName(event.target.value)}
                      onKeyDown={(event) => {
                        if (event.key === "Enter") event.currentTarget.blur();
                      }}
                      aria-label="Macro name"
                      placeholder="Name"
                    />
                    <button
                      disabled={captureName.trim().length === 0}
                      onClick={() => {
                        const name = captureName.trim();
                        setCaptureName(null);
                        void api
                          .finishCapture(name, recording.last_step)
                          .then((held) => {
                            setRecording(null);
                            setLibrary(held);
                            setMacroId(held.entries[0]?.id ?? null);
                            setRegion(null);
                            requestOverlay();
                          })
                          .catch((err: unknown) => setError(String(err)));
                      }}
                    >
                      Save macro
                    </button>
                  </>
                ))}
                <button
                  onClick={() => {
                    setCaptureName(null);
                    void api
                      .cancelCapture()
                      .then(() => {
                        setRecording(null);
                        requestOverlay();
                      })
                      .catch((err: unknown) => setError(String(err)));
                  }}
                  title="Abandon the capture. Nothing is written."
                >
                  Cancel
                </button>
              </>
            )}
          </div>
        )}

        {/*
          The insert bar: the library, and nothing else. A macro carries its
          own frames and its own size, so there is nothing to set — only which
          one, and where, and the map answers the second question.
        */}
        {/*
          The eraser's bar (spec.md 8.1, M29): the brush's options, because
          the eraser is a brush that takes away. Hand-written, since the
          eraser makes no object for a schema to describe.
        */}
        {tool === ERASE && recording === null && (
          <div className="tool-options" role="group" aria-label="Eraser options">
            <label>
              Size
              <NumberField
                min={eraser.unit === "px" ? 1 : units.distanceFromKm(1)}
                max={eraser.unit === "px" ? 2000 : units.distanceFromKm(20000)}
                step={eraser.unit === "px" ? 5 : 50}
                value={eraser.unit === "px" ? eraser.size : units.distanceFromKm(eraser.size)}
                format={(v) => String(Math.round(v * 100) / 100)}
                onCommit={(size) => setEraser({ ...eraser, size: eraser.unit === "px" ? size : units.distanceToKm(size) })}
              />
              <ToolSelect
                value={eraser.unit}
                onChange={(e) => {
                  setEraser({ ...eraser, unit: e.target.value === "px" ? "px" : "km" });
                  releaseFocus(e);
                }}
                title="px cuts a stamp on the map — the same size on screen at any latitude; a ground distance cuts one on the ground. The choice is made where the stroke begins and holds for the whole stroke."
              >
                <option value="km">{units.distanceUnit}</option>
                <option value="px">px</option>
              </ToolSelect>
            </label>
            <label>
              Brush shape
              <ToolSelect
                value={eraser.shape}
                onChange={(e) => {
                  setEraser({ ...eraser, shape: e.target.value === "square" ? "square" : "circle" });
                  releaseFocus(e);
                }}
              >
                <option value="circle">circle</option>
                <option value="square">square</option>
              </ToolSelect>
            </label>
            <label>
              Feather
              <input
                type="range"
                min={0}
                max={1}
                step={0.05}
                value={eraser.feather}
                onChange={(e) => setEraser({ ...eraser, feather: Number(e.target.value) })}
              />
            </label>
          </div>
        )}
        {tool === INSERT && recording === null && (
          <div className="tool-options" role="group" aria-label="Insert options">
            <label>
              Macro
              <ToolSelect
                value={macroId ?? ""}
                disabled={(library?.entries.length ?? 0) === 0}
                onChange={(event) => {
                  setMacroId(event.target.value);
                  releaseFocus(event);
                }}
              >
                {library?.entries.map((entry) => (
                  <option key={entry.id} value={entry.id}>
                    {entry.name} · {entry.frames} frames · {entry.span_hours} h
                    {entry.moves ? " · moves" : ""}
                  </option>
                ))}
              </ToolSelect>
            </label>
            <button onClick={readLibrary} title="Re-read the macro library from disk">
              Refresh
            </button>
          </div>
        )}

        {/*
          The measurement bar. Bespoke like the select tool's, and for the same
          reason: the schema describes an *object's* properties, and a
          measurement has no object. What a measurement can be told is which
          kind it is, how big a ring set is, and when to go away.
        */}
        {tool === MEASURE && (
          <div className="tool-options" role="group" aria-label="Measure options">
            <label>
              Measure
              <ToolSelect
                value={measureKind}
                onChange={(event) => {
                  endMeasuring();
                  setMeasureKind(event.target.value as MeasurementKind);
                  releaseFocus(event);
                }}
                title="Dividers measure a chain leg by leg; a passage draws both ways of sailing between two points; range rings are geodesic circles about a centre"
              >
                {(Object.keys(MEASURE_LABELS) as MeasurementKind[]).map((kind) => (
                  <option key={kind} value={kind}>
                    {MEASURE_LABELS[kind]}
                  </option>
                ))}
              </ToolSelect>
            </label>
            {measureKind === "rings" && (
              <>
                <label>
                  Interval ({units.distanceUnit})
                  <NumberField
                    value={units.distanceFromKm(ringIntervalKm)}
                    min={units.distanceFromKm(0.1)}
                    step={10}
                    onCommit={(value: number) => {
                      const km = units.distanceToKm(value);
                      setRingIntervalKm(km);
                      if (activeRings !== null) {
                        void api
                          .setMeasurementRings(activeRings, km, ringCount)
                          .then(tookMeasurements)
                          .catch(() => undefined);
                      }
                    }}
                    title={`Spacing between rings, in ${units.distanceUnit}`}
                  />
                </label>
                <label>
                  Rings
                  <NumberField
                    value={ringCount}
                    min={1}
                    max={50}
                    step={1}
                    onCommit={(value: number) => {
                      const count = Math.round(value);
                      setRingCount(count);
                      if (activeRings !== null) {
                        void api
                          .setMeasurementRings(activeRings, ringIntervalKm, count)
                          .then(tookMeasurements)
                          .catch(() => undefined);
                      }
                    }}
                    title="How many rings"
                  />
                </label>
              </>
            )}
            <span className="muted">
              {pointsNeeded(measureKind) === 1
                ? activeRings === null
                  ? "Click to place."
                  : "Editing the last set placed. Click to place another."
                : openChain.current !== null
                  ? "Click to add a leg; Enter or Escape to finish."
                  : `Click ${pointsNeeded(measureKind)} points.`}
              {" Alt-click a point to remove its measurement."}
            </span>
            <button
              disabled={!measurements.some((m) => m.kind === measureKind)}
              onClick={() => {
                endMeasuring();
                if (measureKind === "rings") setActiveRings(null);
                void api
                  .clearMeasurements(measureKind)
                  .then(tookMeasurements)
                  .catch(() => undefined);
              }}
              title={`Clear every ${MEASURE_LABELS[measureKind].toLowerCase()} measurement`}
            >
              Clear {MEASURE_LABELS[measureKind].toLowerCase()}
            </button>
            <button
              disabled={measurements.length === 0}
              onClick={() => {
                endMeasuring();
                setActiveRings(null);
                void api.clearMeasurements(null).then(tookMeasurements).catch(() => undefined);
              }}
              title="Clear every measurement on the map"
            >
              Clear all
            </button>
          </div>
        )}

        {schema &&
          tool !== MEASURE &&
          tool !== CAPTURE &&
          tool !== INSERT &&
          recording === null && (
          <ToolOptions
            schema={schema}
            state={toolState}
            onChange={setToolState}
            convention={project.direction_convention}
            camera={cameraRef.current}
            picking={toolPick}
            onPick={(pick) => {
              setToolPick(pick);
              if (pick) setEyedropper(false);
            }}
            sampling={eyedropper}
            onSample={(on) => {
              setEyedropper(on);
              if (on) setToolPick(null);
            }}
          />
        )}

      </div>

      {/*
        The view controls, in the title bar (M25, D68): what the map is
        showing rather than what is being painted. Rendered from here through
        a portal so the tool, the glyph style and the graticule stay the
        map's state; only where the buttons sit has changed.
      */}
      {viewSlot !== null &&
        createPortal(
          <div className="view-controls" role="group" aria-label="View">
        <button
          className={tool === CAPTURE ? "icon active" : "icon"}
          onClick={() => setTool(CAPTURE)}
          aria-label="Capture"
          aria-pressed={tool === CAPTURE}
          title={`Capture (${chord("capture")}) · record a region of the field over a run of frames into the macro library`}
        >
          <ToolIcon tool={CAPTURE} />
        </button>
        <button
          className={tool === MEASURE ? "icon active" : "icon"}
          onClick={() => setTool(MEASURE)}
          aria-label="Measure"
          aria-pressed={tool === MEASURE}
          title={`Measure (${chord("measure")}) · dividers, a passage's two paths, or range rings · click to place, drag a point to move it, Enter or Escape to finish a chain`}
        >
          <ToolIcon tool={MEASURE} />
        </button>
        <span className="divider" />
        <div className="history" role="group" aria-label="History">
          <button
            className="icon"
            disabled={!project.can_undo || busy}
            onClick={() => void api.undo().then(onProjectChanged)}
            title="Undo (Cmd+Z)"
            aria-label="Undo"
          >
            <IconSvg icon={UNDO_ICON} />
          </button>
          <button
            className="icon"
            disabled={!project.can_redo || busy}
            onClick={() => void api.redo().then(onProjectChanged)}
            title="Redo (Cmd+Shift+Z)"
            aria-label="Redo"
          >
            <IconSvg icon={REDO_ICON} />
          </button>
        </div>
        <span className="divider" />

        <label title="Direction glyphs: wind as barbs, currents as arrows. The colour ramp carries the speed either way.">
          <input
            type="checkbox"
            checked={showGlyphs}
            onChange={(e) => setShowGlyphs(e.target.checked)}
          />
          Glyphs
        </label>
        <label>
          Projection
          <ProjectionPicker value={settings?.projection ?? "equirectangular"} disabled={settings === null}
            onChange={id => api.setProjection(id).then(onSettings)} />
        </label>
        <label>
          <input
            type="checkbox"
            checked={showGraticule}
            onChange={(e) => setShowGraticule(e.target.checked)}
          />
          Graticule
        </label>
        <label title="Auto scale: run the colour ramp from the slowest to the fastest speed in view, across every layer and object. A view setting: it changes nothing stored or exported.">
          <input
            type="checkbox"
            checked={autoScale}
            disabled={settings === null}
            onChange={(e) => {
              void api.setAutoScale(e.target.checked).then(onSettings);
            }}
          />
          Auto scale
        </label>
        <label title="The colour legend over the map: the ramp each kind of field is painted with, and the speeds at its ends. A view setting: it changes nothing stored or exported.">
          <input
            type="checkbox"
            checked={showLegend}
            onChange={(e) => setShowLegend(e.target.checked)}
          />
          Legend
        </label>
        <label title={chartStatus?.directory
          ? `Electronic charts from ${chartStatus.directory}${chartStatus.cells ? ` (${chartStatus.cells} cells)` : ""}. Drawn under everything; not a layer, and not part of the project.`
          : "Electronic charts (S-57). Choose the chart directory in Settings first."}>
          <input
            type="checkbox"
            checked={showCharts}
            disabled={!chartStatus?.directory || chartStatus.cells === 0}
            onChange={(e) => setShowCharts(e.target.checked)}
          />
          Charts
        </label>
        <label title="OpenStreetMap tiles in place of the built-in basemap. Tiles are fetched from openstreetmap.org while this is on, and kept on disk; everything else in the application stays offline.">
          <input
            type="checkbox"
            checked={showOsm}
            onChange={(e) => setShowOsm(e.target.checked)}
          />
          OpenStreetMap
        </label>
        <label title="The cursor readout over the map: the position under the pointer and the field there. A view setting: it changes nothing stored or exported.">
          <input
            type="checkbox"
            checked={showReadout}
            onChange={(e) => setShowReadout(e.target.checked)}
          />
          Readout
        </label>
          </div>,
          viewSlot,
        )}

      {showLegend && kindsShown.length > 0 && (
        <div className="map-legend">
          {kindsShown.map((kind) => (
            <div key={kind} className="legend-row">
              <div className="legend-kind">
                <span>{KIND_LABELS[kind]}</span>
                <span className="muted">{kind === "wind" ? "barbs" : "arrows"}</span>
              </div>
              {/*
                The bar is the gradient the map is actually painted with
                (M42), not a fixed one: a legend that showed a different run
                of colours from the pixels beside it would be worse than no
                legend at all.
              */}
              <div
                className="legend-bar"
                style={{
                  background: `linear-gradient(to right, ${rampStops(
                    stopsOf(gradientCatalogue, project[`${kind}_gradient`]),
                  ).join(", ")})`,
                }}
              />
              <div className="legend-labels">
                <span>{ramps[kind].minKnots}</span>
                {ramps[kind].auto && (
                  <span
                    className="legend-auto"
                    title="Auto scale: the ramp spans the speeds in view"
                  >
                    auto
                  </span>
                )}
                <span>{ramps[kind].maxKnots} {units.speedUnit}</span>
              </div>
            </div>
          ))}
        </div>
      )}

      {showReadout && (
        <MapReadout store={readoutStore.current} convention={project.direction_convention} />
      )}
    </div>
  );
}

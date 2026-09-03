import { useCallback, useEffect, useRef, useState } from "react";

import { api } from "../ipc";
import type { BrushDirectionMode } from "../generated/BrushDirectionMode";
import type { BrushShape } from "../generated/BrushShape";
import type { ProjectSummary } from "../generated/ProjectSummary";
import type { ObjectOutline } from "../generated/ObjectOutline";
import type { StampSpace } from "../generated/StampSpace";
import type { PositionPick } from "../picking";
import type { SelectionTransform } from "../generated/SelectionTransform";
import type { TransformPreview } from "../generated/TransformPreview";
import type { TransformKind } from "../generated/TransformKind";
import { displayDirection, knotsFromMps, mpsFromKnots } from "../project/format";
import {
  type Camera,
  type Viewport,
  clampCamera,
  minPxPerDeg,
  normalizeLon,
  project as toScreen,
  unproject,
  zoomAbout,
} from "./camera";
import {
  addFootprint,
  buildStrokePath,
  footprintRadii,
  kmFromPixels,
  pixelsFromKm,
} from "./footprint";
import { destination, initialBearing } from "./geo";
import { glyphGeometry, glyphLayout, latticeUnderStroke } from "./glyph";
import { RAMP_STOPS, rampCss } from "./ramp";
import { parseBasemap } from "./format";
import { marqueeBounds } from "./marquee";
import {
  previewHasLanded,
  SETTLE_TIMEOUT_MS,
  type HeldPreview,
  type Settling,
  type StrokePreview,
} from "./preview";
import { MapRenderer, type RenderState } from "./renderer";
import { TileCache } from "./tiles";

interface Readout {
  lon: number;
  lat: number;
  /** Speed in knots, the only unit shown (`ve_core::units`). */
  speedKnots: number;
  /** Direction, already converted to the project's convention. */
  directionDeg: number;
}

/**
 * Speed at the top of the colour ramp, in knots.
 *
 * Currents run an order of magnitude slower than wind, so sharing one scale
 * would leave every current project rendered in the bottom of the ramp.
 */
function rampMaxKnotsFor(fieldKind: string): number {
  return fieldKind === "current" ? 6 : 60;
}

/** Formats a latitude or longitude with a hemisphere suffix. */
function formatDegrees(value: number, positive: string, negative: string): string {
  const suffix = value >= 0 ? positive : negative;
  return `${Math.abs(value).toFixed(2)}° ${suffix}`;
}

/** What a pointer drag does. */
type Tool = "hand" | "brush";

/** Hit radius of a transform handle, in CSS pixels. */
const HANDLE_RADIUS_CSS = 6;

/**
 * Ink for preview glyphs.
 *
 * The renderer's `GLYPH` colour, as CSS: the preview's barbs and the map's are
 * the same mark and should not read as two different things.
 */
const GLYPH_INK = "rgba(240, 247, 255, 0.9)";

/**
 * Floor under a preview's opacity.
 *
 * The field fades calm out entirely so the basemap stays readable, but a
 * preview the user cannot see is not a preview.
 */
const PREVIEW_MIN_ALPHA = 0.28;

/**
 * Most glyphs one stroke preview will draw.
 *
 * A stroke around the world at a fine lattice would ask for far more than is
 * worth drawing every pointer move; the preview is a proxy, not the field.
 */
const MAX_PREVIEW_GLYPHS = 600;

export default function MapView({
  project,
  step,
  selection,
  activeLayer,
  picking,
  onPicked,
  onProjectChanged,
  onStepChange,
  onSelect,
}: {
  project: ProjectSummary;
  step: number;
  selection: number[];
  activeLayer: number | null;
  /** A position property waiting for a map click, armed by the inspector. */
  picking: PositionPick | null;
  onPicked: () => void;
  onProjectChanged: (project: ProjectSummary) => void;
  onStepChange: (step: number) => void;
  onSelect: (objects: number[]) => void;
}) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const rendererRef = useRef<MapRenderer | null>(null);
  const tilesRef = useRef<TileCache | null>(null);
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
  const dragging = useRef<{ x: number; y: number } | null>(null);
  /** Where the pointer went down, to tell a click from a pan. */
  const pressOrigin = useRef<{ x: number; y: number } | null>(null);
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
  /** Points of the stroke in progress, as [lon, lat]. */
  const strokeRef = useRef<Array<[number, number]> | null>(null);
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
  const cursorRef = useRef<{ x: number; y: number } | null>(null);
  /**
   * Display options mirrored into refs.
   *
   * `draw` runs from an animation frame or a timer, so it must read the current
   * values rather than whatever its closure captured. Setting React state and
   * drawing in the same tick would otherwise render the previous options.
   */
  const glyphStyleRef = useRef<"arrow" | "barb">("barb");
  const showGlyphsRef = useRef(true);
  const showGraticuleRef = useRef(true);
  const stepRef = useRef(0);
  const loggedDrawError = useRef(false);

  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [glyphStyle, setGlyphStyle] = useState<"arrow" | "barb">("barb");
  const [showGlyphs, setShowGlyphs] = useState(true);
  const [showGraticule, setShowGraticule] = useState(true);
  const [readout, setReadout] = useState<Readout | null>(null);
  const [pending, setPending] = useState(0);
  const [tool, setTool] = useState<Tool>("hand");
  const [brushShape, setBrushShape] = useState<BrushShape>("circle");
  /**
   * The size the user typed, in whichever unit is selected.
   *
   * Pixels are an input convenience only: they resolve to kilometres against
   * the map scale at the moment a stroke is committed and are never revisited,
   * so zooming afterwards cannot resize an existing object (spec.md 3.5).
   */
  const [brushSize, setBrushSize] = useState(600);
  const [brushSizeUnit, setBrushSizeUnit] = useState<"km" | "px">("km");
  const [brushSpeedKnots, setBrushSpeedKnots] = useState(30);
  const [brushDirectionMode, setBrushDirectionMode] =
    useState<BrushDirectionMode>("constant");
  const [brushDirection, setBrushDirection] = useState(270);
  /** Where `toward_point` aims, as [lon, lat]. */
  const [brushTarget, setBrushTarget] = useState<[number, number]>([0, 0]);
  /** While set, the next click on the map places the target instead of painting. */
  const [pickingTarget, setPickingTarget] = useState(false);
  const [brushFeather, setBrushFeather] = useState(0.25);
  const [busy, setBusy] = useState(false);
  /**
   * Where the selection's handles go.
   *
   * One object or many: the backend reports the pivot, which is the object's
   * anchor for one and the collective centroid for a group (spec.md 8.2).
   */
  const [committedTransform, setTransform] = useState<SelectionTransform | null>(null);
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
  /** Set while the aim point is being dragged around the map. */
  const targetDrag = useRef(false);
  /** The rubber band, in device pixels, while one is being dragged. */
  const marquee = useRef<{
    from: { x: number; y: number };
    to: { x: number; y: number };
    crossLayer: boolean;
  } | null>(null);

  const rampMaxKnots = rampMaxKnotsFor(project.field_kind);
  // The renderer works in stored units; the ramp is chosen in displayed ones.
  const rampMax = mpsFromKnots(rampMaxKnots);
  // Barbs are a wind convention and are hidden for current projects (spec.md 5.3).
  const barbsAvailable = project.field_kind === "wind";
  const lastStep = Math.max(0, project.step_count - 1);

  /**
   * Which space the brush's stamp is a shape in (spec.md 3.5).
   *
   * A size in pixels is asking for a shape on the *map*, and a ground disc is
   * an ellipse there — twice as wide as tall at 60 degrees. So px paints a
   * projected stamp, which is a circle on screen at any latitude and any zoom;
   * km paints a geodesic one, which is a circle on the ground.
   */
  const brushSpace: StampSpace = brushSizeUnit === "px" ? "projected" : "geodesic";

  /**
   * The brush's stored size in kilometres, for a stamp at `lat`.
   *
   * A size already in kilometres is the answer. One in pixels converts against
   * the map scale, which for a geodesic stamp depends on where it lands — hence
   * the latitude — and for a projected one does not.
   */
  const brushSizeKmAt = useCallback(
    (lat: number) =>
      brushSizeUnit === "km"
        ? brushSize
        : kmFromPixels(cameraRef.current, lat, brushSize, brushSpace),
    [brushSize, brushSizeUnit, brushSpace],
  );

  /**
   * The brush's direction as it is stored.
   *
   * The user enters a direction in the project's convention; storage is always
   * azimuth-toward (spec.md 3.3). One conversion, read by both the preview and
   * the commit, so the arrows on screen cannot disagree with what gets painted.
   */
  const brushAzimuthToward =
    project.direction_convention === "from"
      ? (brushDirection + 180) % 360
      : brushDirection;

  /** Whether the current aim mode needs a target to mean anything. */
  const brushAims = brushDirectionMode !== "constant";

  /** The azimuth-toward a stamp at this position will carry (spec.md 6.2). */
  const brushAzimuthAt = useCallback(
    (lon: number, lat: number) => {
      if (!brushAims) return brushAzimuthToward;
      const toward = initialBearing(
        { lon, lat },
        { lon: brushTarget[0], lat: brushTarget[1] },
      );
      // Away is the reciprocal at the cell, which is the outward tangent to the
      // same great circle — not the bearing measured at the target, which is a
      // different angle once the meridians have converged.
      return brushDirectionMode === "away_from_point" ? (toward + 180) % 360 : toward;
    },
    [brushAims, brushAzimuthToward, brushDirectionMode, brushTarget],
  );

  const draw = useCallback(() => {
    const renderer = rendererRef.current;
    if (!renderer) return;

    const state: RenderState = {
      camera: cameraRef.current,
      view: viewRef.current,
      // Revision then step: an edit changes the address, so a cached tile can
      // never show a field that no longer exists.
      frame: `${projectRef.current.revision}/${stepRef.current}`,
      glyphStyle: glyphStyleRef.current,
      showGlyphs: showGlyphsRef.current,
      showGraticule: showGraticuleRef.current,
      rampMax,
      stale: (tilesRef.current?.stats().pending ?? 0) > 0,
      pixelRatio: window.devicePixelRatio || 1,
    };

    try {
      renderer.render(state);
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

    // Retire the previews whose field has arrived. Pending tiles mean the map
    // is still showing the previous revision (spec.md 5.4), so the preview has
    // to stay until nothing is outstanding.
    if (settling.current.length > 0) {
      const now = performance.now();
      const held = settling.current.filter(
        (entry) =>
          !previewHasLanded(entry, projectRef.current.revision, stats?.pending ?? 0, now),
      );
      settling.current = held;
    }

    const settled = settlingDrag.current;
    if (
      settled &&
      previewHasLanded(settled, projectRef.current.revision, stats?.pending ?? 0, performance.now())
    ) {
      settlingDrag.current = null;
    }

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
        renderer = new MapRenderer(gl, basemap, tiles);
        tiles.onChange = () => requestDraw();
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
      rendererRef.current = null;
      tilesRef.current = null;
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

  // Single-key tool shortcuts, as in every other paint application.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.metaKey || event.ctrlKey || event.altKey) return;
      const target = event.target as HTMLElement | null;
      // Never steal a keystroke from a field the user is typing in.
      if (target && /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName)) return;

      if (event.key === "v" || event.key === "V") setTool("hand");
      if (event.key === "b" || event.key === "B") setTool("brush");
      if (event.key === "Escape") setTool("hand");
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // The selection's handles, so the map shows what the panels are pointing at
  // and where a drag would act.
  const selectionKey = selection.join(",");
  useEffect(() => {
    if (selection.length === 0) {
      setTransform(null);
      return;
    }
    let cancelled = false;
    api
      .selectionTransform(selection, step)
      .then((value) => {
        if (!cancelled) setTransform(value);
      })
      .catch(() => setTransform(null));
    return () => {
      cancelled = true;
    };
    // `selectionKey` rather than `selection`: a new array of the same ids is a
    // new value every render and would refetch forever.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project.revision, selectionKey, step]);

  useEffect(() => {
    drawOverlay();
  }, [committedTransform]);

  useEffect(() => {
    projectRef.current = project;
    requestDraw();
  }, [project, requestDraw]);

  // Leaving the brush disarms target picking: the panel that shows it is armed
  // goes away with the tool, and coming back to a brush that swallows the next
  // click would be a small mystery.
  useEffect(() => {
    if (tool !== "brush") setPickingTarget(false);
  }, [tool]);

  // A project change can shorten the timeline or forbid barbs.
  useEffect(() => {
    if (step > lastStep) onStepChange(lastStep);
    if (!barbsAvailable) setGlyphStyle("arrow");
  }, [barbsAvailable, lastStep, onStepChange, step]);

  // Mirror display state into the refs `draw` reads, then redraw.
  useEffect(() => {
    glyphStyleRef.current = glyphStyle;
    showGlyphsRef.current = showGlyphs;
    showGraticuleRef.current = showGraticule;
    stepRef.current = step;
    requestDraw();
  }, [requestDraw, step, glyphStyle, showGlyphs, showGraticule]);

  /**
   * Whether a screen position is close enough to grab the aim point.
   *
   * The same reach as a transform handle, so the two feel alike.
   */
  const nearTarget = useCallback(
    (point: { x: number; y: number }) => {
      const at = toScreen(cameraRef.current, viewRef.current, {
        lon: brushTarget[0],
        lat: brushTarget[1],
      });
      const reach = HANDLE_RADIUS_CSS * 2 * (window.devicePixelRatio || 1);
      return Math.hypot(point.x - at.x, point.y - at.y) <= reach;
    },
    [brushTarget],
  );

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
      const { lengthPx } = glyphLayout(glyphStyle, camera.pxPerDeg, dpr);
      const width = Math.max(1, 1.8 * dpr);

      const strokes = new Path2D();
      const fills = new Path2D();

      for (const [lon, lat] of at) {
        const point = toScreen(camera, view, { lon, lat });
        const geometry = glyphGeometry(
          glyphStyle,
          azimuthAt(lon, lat),
          knots,
          lengthPx,
          lat,
          width,
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
      context.strokeStyle = GLYPH_INK;
      context.fillStyle = GLYPH_INK;
      context.lineWidth = width;
      context.lineCap = "round";
      context.lineJoin = "round";
      context.stroke(strokes);
      context.fill(fills);
      context.restore();
    },
    [glyphStyle],
  );

  /**
   * The field a stroke will paint: its swept region in the speed colour, with
   * direction glyphs over it.
   *
   * Glyphs sit on the map's own globe-anchored lattice, so what the preview
   * shows is what the field will show once the stroke is committed -- same
   * positions, same mark, same direction (spec.md 6.1).
   */
  const drawStrokePreview = useCallback(
    (context: CanvasRenderingContext2D, preview: StrokePreview, dpr: number) => {
      const camera = cameraRef.current;
      const view = viewRef.current;

      // Every stamp is its own subpath, so filling once merges the overlapping
      // footprints into a single silhouette instead of drawing a chain of
      // outlines on top of each other.
      const swept = new Path2D();
      buildStrokePath(
        swept,
        camera,
        view,
        preview.points,
        preview.radiusKm,
        preview.shape,
        preview.space,
      );
      context.fillStyle = preview.paint;
      context.fill(swept);

      if (!showGlyphs) return;
      const { stepDeg } = glyphLayout(glyphStyle, camera.pxPerDeg, dpr);
      const covered = latticeUnderStroke(
        preview.points,
        preview.radiusKm,
        stepDeg,
        MAX_PREVIEW_GLYPHS,
        preview.shape,
        preview.space,
      );
      // A brush narrower than the lattice can cover no point at all. The stroke
      // still has a direction, and a preview showing none of it is worse than
      // one glyph off the lattice, so the head of the stroke stands in.
      drawGlyphs(
        context,
        covered.length > 0
          ? covered
          : [preview.points[preview.points.length - 1]!],
        preview.knots,
        preview.azimuthAt,
        dpr,
      );
    },
    [drawGlyphs, glyphStyle, showGlyphs],
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
      const camera = cameraRef.current;
      const view = viewRef.current;

      const silhouette = new Path2D();
      for (const outline of outlines) {
        if (outline.kind === "swept") {
          for (const chain of outline.chains) {
            buildStrokePath(
              silhouette,
              camera,
              view,
              chain,
              outline.radius_km / 2,
              outline.square ? "square" : "circle",
              outline.space,
            );
          }
        } else {
          const [first, ...rest] = outline.points;
          if (first === undefined) continue;
          const start = toScreen(camera, view, { lon: first[0], lat: first[1] });
          silhouette.moveTo(start.x, start.y);
          for (const [lon, lat] of rest) {
            const point = toScreen(camera, view, { lon, lat });
            silhouette.lineTo(point.x, point.y);
          }
          silhouette.closePath();
        }
      }

      context.save();
      context.fillStyle = "rgba(255, 214, 102, 0.18)";
      context.fill(silhouette);
      context.strokeStyle = "rgba(255, 214, 102, 0.75)";
      context.lineWidth = Math.max(1, dpr);
      context.stroke(silhouette);
      context.restore();
    },
    [],
  );

  /**
   * Tool feedback, drawn on a 2D canvas over the WebGL one.
   *
   * Separate from the renderer because it changes on every pointer move and has
   * nothing to do with the field: mixing it into the GL pass would mean
   * redrawing the whole map to move a cursor outline.
   */
  const drawOverlay = useCallback(() => {
    const canvas = overlayRef.current;
    const context = canvas?.getContext("2d");
    if (!canvas || !context) return;

    const view = viewRef.current;
    context.clearRect(0, 0, view.width, view.height);

    const camera = cameraRef.current;

    // While a drag is in flight the handles follow it rather than the document,
    // which is deliberately not being written until the pointer comes up.
    const live = dragPreview.current ?? settlingDrag.current;
    if (live) drawDragOutlines(context, live.outlines, window.devicePixelRatio || 1);
    const transform = live?.handles ?? committedTransform;

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

    const dpr = window.devicePixelRatio || 1;
    const cursor = cursorRef.current;

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
    for (const settled of settling.current) drawStrokePreview(context, settled, dpr);

    if (tool !== "brush") return;

    const stroke = strokeRef.current;

    // Where the target is, and where the next click would put it.
    if (brushAims) {
      const aim =
        pickingTarget && cursor
          ? unproject(camera, view, cursor)
          : { lon: brushTarget[0], lat: brushTarget[1] };
      const at = toScreen(camera, view, aim);
      const arm = 9 * dpr;
      const armed = pickingTarget || targetDrag.current;
      context.strokeStyle = armed
        ? "rgba(255, 214, 120, 0.95)"
        : "rgba(255, 168, 96, 0.9)";
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
      if (targetDrag.current || (!pickingTarget && cursor && nearTarget(cursor))) {
        context.fillStyle = "rgba(255, 168, 96, 0.22)";
        context.fill();
      }
      context.stroke();
    }

    // A pixel-valued size resolves against the latitude the commit will use —
    // the stroke's first point — so what is previewed is what gets painted,
    // rather than a footprint that changes as the drag moves north.
    const sizeLat = stroke?.[0]?.[1] ?? (cursor ? unproject(camera, view, cursor).lat : 0);
    const radiusKm = brushSizeKmAt(sizeLat) / 2;

    // The colour the field will take once this stroke lands, from the same ramp
    // the map paints with. A floor under the alpha keeps a calm stroke visible:
    // the field fades calm out entirely, but a preview the user cannot see is
    // not a preview.
    const paint = rampCss(mpsFromKnots(brushSpeedKnots), rampMax, PREVIEW_MIN_ALPHA);

    if (stroke && stroke.length > 0) {
      drawStrokePreview(
        context,
        {
          points: stroke,
          radiusKm,
          shape: brushShape,
          space: brushSpace,
          paint,
          knots: brushSpeedKnots,
          azimuthAt: brushAzimuthAt,
        },
        dpr,
      );
    }

    // Only the brush tip is outlined: it is the thing being aimed, and an
    // outline per stamp is noise.
    if (cursor && !pickingTarget) {
      const geo = unproject(camera, view, cursor);
      const tip = new Path2D();
      addFootprint(tip, camera, view, geo.lon, geo.lat, radiusKm, brushShape, brushSpace);
      if (!stroke) {
        context.fillStyle = paint;
        context.fill(tip);
        // One glyph at the tip, so the aim is readable before the drag starts
        // rather than only after something has been painted.
        if (showGlyphs) {
          drawGlyphs(
            context,
            [[geo.lon, geo.lat]],
            brushSpeedKnots,
            brushAzimuthAt,
            dpr,
          );
        }
      }
      context.strokeStyle = "rgba(160, 232, 255, 0.95)";
      context.lineWidth = Math.max(1, dpr);
      context.stroke(tip);
    }
  }, [
    brushAims,
    brushAzimuthAt,
    drawDragOutlines,
    nearTarget,
    brushShape,
    brushSizeKmAt,
    brushSpace,
    brushSpeedKnots,
    brushTarget,
    drawGlyphs,
    drawStrokePreview,
    glyphStyle,
    pickingTarget,
    rampMax,
    showGlyphs,
    tool,
    picking,
    committedTransform,
  ]);

  useEffect(() => {
    drawOverlayRef.current = drawOverlay;
    drawOverlay();
  }, [drawOverlay]);

  /** Screen positions of the handles, or null when nothing is selected. */
  const handlePositions = useCallback(() => {
    if (!committedTransform) return null;
    const camera = cameraRef.current;
    const view = viewRef.current;
    const anchor = { lon: committedTransform.lon, lat: committedTransform.lat };
    return {
      pivot: anchor,
      centre: toScreen(camera, view, anchor),
      rotate: toScreen(
        camera,
        view,
        destination(anchor, committedTransform.rotation_deg, committedTransform.radius_m),
      ),
      scale: toScreen(
        camera,
        view,
        destination(anchor, committedTransform.rotation_deg + 90, committedTransform.radius_m),
      ),
    };
  }, [committedTransform]);

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
      void api
        .beginTransform(selection, step, kind, geo.lon, geo.lat)
        .catch((err: unknown) => {
          handleDrag.current = null;
          void api.frontendLog("error", `transform failed to start: ${String(err)}`);
        });
    },
    [selection, step],
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
    (geo: { lon: number; lat: number }) => {
      void api
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
          drawOverlayRef.current();
          void api.frontendLog("error", `drag failed: ${String(err)}`);
        });
    },
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

  const onPointerDown = (event: React.PointerEvent<HTMLCanvasElement>) => {
    event.currentTarget.setPointerCapture(event.pointerId);
    const point = toDevice(event);

    // A pick armed in the inspector takes the click ahead of every tool: the
    // user asked for this one place, and painting or panning instead would
    // both lose the click and do something they did not ask for.
    if (picking !== null) {
      const geo = unproject(cameraRef.current, viewRef.current, point);
      onPicked();
      void api
        .setObjectProperty(picking.object, picking.property, {
          kind: "position",
          lon: geo.lon,
          lat: geo.lat,
        })
        .then(onProjectChanged)
        .catch((err: unknown) =>
          void api.frontendLog("error", `placing ${picking.property} failed: ${String(err)}`),
        );
      return;
    }

    if (tool === "brush") {
      const geo = unproject(cameraRef.current, viewRef.current, point);
      // While picking, the click places the aim point rather than painting —
      // one click, one target, and the mode ends itself so the next stroke is
      // an ordinary one.
      if (pickingTarget) {
        setBrushTarget([geo.lon, geo.lat]);
        setPickingTarget(false);
        return;
      }
      // The aim point is a handle, and a handle takes precedence over what is
      // under it — the same rule the transform handles follow (spec.md 8.1).
      // Without it the marker could be placed but never adjusted except by
      // arming the picker again or typing coordinates.
      if (brushAims && nearTarget(point)) {
        targetDrag.current = true;
        return;
      }
      strokeRef.current = [[geo.lon, geo.lat]];
      drawOverlay();
      return;
    }

    const geo = unproject(cameraRef.current, viewRef.current, point);

    // A handle takes precedence over everything else under the pointer.
    const handles = handlePositions();
    if (handles && committedTransform) {
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
            ? committedTransform.count === 1
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

    // Dragging a selected object moves it; dragging empty map pans (spec.md
    // 8.1). Which of the two this is takes a hit test, and a hit test is a
    // round trip — so panning starts immediately and converts to a move if the
    // answer comes back before the pointer has actually gone anywhere. Waiting
    // for the answer instead would put IPC latency in front of every pan.
    if (tool === "hand" && selection.length > 0) {
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

  const onPointerMove = (event: React.PointerEvent<HTMLCanvasElement>) => {
    const point = toDevice(event);
    cursorRef.current = point;

    if (targetDrag.current) {
      const geo = unproject(cameraRef.current, viewRef.current, point);
      // The overlay redraws off the state change: every glyph in the preview is
      // aimed at this point, so they all swing round as it moves.
      setBrushTarget([geo.lon, geo.lat]);
      return;
    }

    if (handleDrag.current) {
      previewDrag(unproject(cameraRef.current, viewRef.current, point));
      return;
    }

    if (marquee.current) {
      marquee.current.to = point;
      drawOverlay();
      return;
    }

    const stroke = strokeRef.current;
    if (stroke) {
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
        const last = stroke[stroke.length - 1];
        const moved =
          last === undefined ||
          Math.hypot(
            normalizeLon(geo.lon - last[0]) * cameraRef.current.pxPerDeg,
            (geo.lat - last[1]) * cameraRef.current.pxPerDeg,
          ) > spacing;
        if (moved) stroke.push([geo.lon, geo.lat]);
      }
      drawOverlay();
      return;
    }

    // The brush's outline follows the cursor, and so does a pick's crosshair.
    if (tool === "brush" || picking !== null) drawOverlay();

    if (dragging.current) {
      const dx = point.x - dragging.current.x;
      const dy = point.y - dragging.current.y;
      dragging.current = point;
      const camera = cameraRef.current;
      cameraRef.current = clampCamera(
        {
          ...camera,
          // Longitude is never clamped, so dragging past the dateline just
          // keeps going and wraps.
          centerLon: camera.centerLon - dx / camera.pxPerDeg,
          centerLat: camera.centerLat + dy / camera.pxPerDeg,
        },
        viewRef.current,
      );
      requestDraw();
    }

    const geo = unproject(cameraRef.current, viewRef.current, point);
    void api
      .sampleField(geo.lon, geo.lat, step)
      .then((sample) =>
        setReadout({
          lon: geo.lon,
          lat: geo.lat,
          speedKnots: knotsFromMps(sample.speed_mps),
          directionDeg: displayDirection(
            project.direction_convention,
            sample.azimuth_toward_deg,
          ),
        }),
      )
      .catch(() => setReadout(null));
  };

  const commitStroke = useCallback(
    async (points: Array<[number, number]>) => {
      // A pixel size becomes kilometres here and nowhere else: at the latitude
      // the stroke started, against the camera as it is now (spec.md 3.5). The
      // document only ever holds the kilometres.
      const sizeKm = brushSizeKmAt(points[0]?.[1] ?? 0);

      // Keep previewing this stroke until its field is drawn, with the values
      // it froze rather than whatever the toolbar says by then (spec.md 6.1).
      const settled: HeldPreview = {
        points,
        radiusKm: sizeKm / 2,
        shape: brushShape,
        space: brushSpace,
        paint: rampCss(mpsFromKnots(brushSpeedKnots), rampMax, PREVIEW_MIN_ALPHA),
        knots: brushSpeedKnots,
        azimuthAt: brushAzimuthAt,
        revision: null,
        at: performance.now(),
      };
      settling.current = [...settling.current, settled];

      setBusy(true);
      try {
        const summary = await api.addBrushStroke({
          points,
          size_km: sizeKm,
          speed_mps: mpsFromKnots(brushSpeedKnots),
          direction_toward_deg: brushAzimuthToward,
          feather: brushFeather,
          shape: brushShape,
          space: brushSpace,
          direction_mode: brushDirectionMode,
          // A target is meaningless in constant mode, and sending one anyway
          // would stop two otherwise identical strokes merging.
          ...(brushAims ? { target: brushTarget } : {}),
          // A new object joins the layer that was selected when it was created
          // (spec.md 6.1). Omitted rather than null when there is none: the
          // field is optional, and the backend reads its absence as "the top".
          ...(activeLayer !== null ? { layer: activeLayer } : {}),
        });
        settled.revision = summary.revision;
        onProjectChanged(summary);
        // The revision change re-addresses every tile, but if they were all
        // cached nothing else would schedule the draw that retires the preview.
        requestDraw();
        // ...and if the new tiles never arrive at all, the backstop does.
        window.setTimeout(requestDraw, SETTLE_TIMEOUT_MS + 100);
      } catch (err) {
        // Nothing was painted, so nothing should keep looking painted.
        settling.current = settling.current.filter((entry) => entry !== settled);
        drawOverlayRef.current();
        void api.frontendLog("error", `stroke failed: ${String(err)}`);
      } finally {
        setBusy(false);
      }
    },
    [
      activeLayer,
      brushAzimuthAt,
      brushAzimuthToward,
      brushDirectionMode,
      brushFeather,
      brushShape,
      brushSizeKmAt,
      brushSpace,
      brushSpeedKnots,
      brushTarget,
      onProjectChanged,
      rampMax,
      requestDraw,
    ],
  );

  const endDrag = (event: React.PointerEvent<HTMLCanvasElement>) => {
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    // Finish an aim-point drag. Nothing to commit: the target is a tool option,
    // not document state, until a stroke freezes it (spec.md 6.1).
    if (targetDrag.current) {
      targetDrag.current = false;
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
      const release = unproject(cameraRef.current, viewRef.current, toDevice(event));
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
          commitDrag(release);
          void api.endGesture();
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
      const point = toDevice(event);
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

    const stroke = strokeRef.current;
    strokeRef.current = null;
    // The commit picks the preview up synchronously, so the overlay redraw
    // below never sees a moment with neither the stroke nor its field.
    if (stroke && stroke.length > 0) void commitStroke(stroke);
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

  /** Reads the next frame back and writes it beside the logs. */
  const capture = useCallback(async (name: string) => {
    const renderer = rendererRef.current;
    if (!renderer) {
      void api.frontendLog("warn", "capture requested before the renderer was ready");
      return;
    }
    // Wait for the tiles to settle: capturing mid-load photographs a
    // half-drawn map and looks like a rendering bug.
    for (let attempt = 0; attempt < 40; attempt++) {
      if ((tilesRef.current?.stats().pending ?? 0) === 0) break;
      await new Promise((resolve) => window.setTimeout(resolve, 100));
    }

    const pendingCapture = renderer.captureNextFrame();
    requestDraw();
    const image = await pendingCapture;
    if (!image) return;

    const scratch = document.createElement("canvas");
    scratch.width = image.width;
    scratch.height = image.height;
    scratch.getContext("2d")?.putImageData(image, 0, 0);
    const blob = await new Promise<Blob | null>((resolve) =>
      scratch.toBlob(resolve, "image/png"),
    );
    if (!blob) return;

    const dataUrl = await new Promise<string>((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(String(reader.result));
      reader.onerror = () => reject(reader.error ?? new Error("read failed"));
      reader.readAsDataURL(blob);
    });
    const base64 = dataUrl.slice(dataUrl.indexOf(",") + 1);
    const path = await api.saveDebugCapture(name, base64);
    void api.frontendLog("info", `capture written to ${path}`);
  }, [requestDraw]);

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
      style: "arrow" | "barb";
      probes: Array<{ lon: number; lat: number }>;
    }> = [
      {
        name: "world-barbs",
        camera: { centerLon: 0, centerLat: 0, pxPerDeg: 8 },
        style: "barb",
        probes: [],
      },
      {
        name: "zoom-arrows",
        camera: { centerLon: -40, centerLat: 35, pxPerDeg: 60 },
        style: "arrow",
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
        style: "arrow",
        probes: [],
      },
      {
        name: "dateline",
        camera: { centerLon: 180, centerLat: 20, pxPerDeg: 24 },
        style: "arrow",
        probes: [
          { lon: 179, lat: 20 },
          { lon: -179, lat: 20 },
        ],
      },
    ];

    for (const scenario of scenarios) {
      cameraRef.current = clampCamera(scenario.camera, viewRef.current);
      // Set the refs directly: state updates are async and would not reach the
      // draw that follows on this tick.
      glyphStyleRef.current = scenario.style;
      showGlyphsRef.current = true;
      setGlyphStyle(scenario.style);
      requestDraw();

      for (const probe of scenario.probes) {
        const sample = await api.sampleField(probe.lon, probe.lat, 0);
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
          glyphStyle: "barb", showGlyphs: true,
          showGraticule: true, rampMax, stale: false,
          pixelRatio: window.devicePixelRatio || 1,
        });
      }

      const started = performance.now();
      for (let i = 0; i < frames; i++) {
        renderer.render({
          camera: { ...base, centerLon: base.centerLon + i * 0.25 },
          view,
          frame: `${projectRef.current.revision}/0`,
          glyphStyle: "barb", showGlyphs: true,
          showGraticule: true, rampMax, stale: false,
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
    requestDraw,
  ]);

  const zoomPercent = Math.round(
    (cameraRef.current.pxPerDeg / minPxPerDeg(viewRef.current)) * 100,
  );

  return (
    <div className="map">
      <canvas
        ref={canvasRef}
        className={
          picking !== null || tool === "brush" ? "map-canvas painting" : "map-canvas"
        }
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={endDrag}
        onPointerCancel={endDrag}
        onWheel={onWheel}
      />

      <canvas ref={overlayRef} className="map-overlay" />

      {error !== null && <div className="map-error">{error}</div>}
      {error === null && !ready && <div className="map-status">Loading basemap…</div>}

      <div className="map-toolbar">
        <div className="tools" role="group" aria-label="Tool">
          <button
            className={tool === "hand" ? "active" : ""}
            onClick={() => setTool("hand")}
            title="Pan, select and transform (V) · shift-drag for a rubber band, add cmd to reach across layers · cmd-click to add or remove one object"
          >
            Hand
          </button>
          <button
            className={tool === "brush" ? "active" : ""}
            onClick={() => setTool("brush")}
            title="Paint vectors (B)"
          >
            Brush
          </button>
        </div>

        {tool === "brush" && (
          <div className="brush-options">
            <label>
              Shape
              <select
                value={brushShape}
                onChange={(e) => setBrushShape(e.target.value as BrushShape)}
                title="The stamp swept along the stroke"
              >
                <option value="circle">Circle</option>
                <option value="square">Square</option>
              </select>
            </label>
            <label>
              Size
              <input
                type="number"
                min={1}
                max={brushSizeUnit === "km" ? 20000 : 2000}
                step={brushSizeUnit === "km" ? 50 : 5}
                value={brushSize}
                onChange={(e) => setBrushSize(Math.max(1, Number(e.target.value) || 1))}
              />
              <select
                value={brushSizeUnit}
                onChange={(e) => {
                  // Carry the size across rather than reinterpreting the
                  // number: switching units should not resize the brush.
                  const next = e.target.value === "px" ? "px" : "km";
                  const lat = cameraRef.current.centerLat;
                  // The unit also chooses the space, so the conversion is
                  // between two different stamps. Carrying the north-south
                  // extent across is what keeps the brush the same height on
                  // screen through the switch.
                  const space: StampSpace = next === "px" ? "projected" : "geodesic";
                  setBrushSize((current) =>
                    next === brushSizeUnit
                      ? current
                      : Math.max(
                          1,
                          Math.round(
                            next === "px"
                              ? pixelsFromKm(cameraRef.current, lat, current, space)
                              : kmFromPixels(cameraRef.current, lat, current, brushSpace),
                          ),
                        ),
                  );
                  setBrushSizeUnit(next);
                }}
                title="A size in pixels paints a stamp that is a circle on the map — the same size on screen at any latitude. It resolves to kilometres when the stroke is painted and never changes afterwards."
              >
                <option value="km">km</option>
                <option value="px">px</option>
              </select>
            </label>
            <label>
              Speed
              <input
                type="number"
                min={0}
                max={200}
                step={1}
                value={brushSpeedKnots}
                onChange={(e) =>
                  setBrushSpeedKnots(Math.max(0, Number(e.target.value) || 0))
                }
              />
              kt
            </label>
            <label>
              Aim
              <select
                value={brushDirectionMode}
                onChange={(e) => {
                  setBrushDirectionMode(e.target.value as BrushDirectionMode);
                  setPickingTarget(false);
                }}
                title="A fixed bearing, or every vector aimed at one point — or straight away from it"
              >
                <option value="constant">Fixed bearing</option>
                <option value="toward_point">Toward a point</option>
                <option value="away_from_point">Away from a point</option>
              </select>
            </label>
            {!brushAims ? (
              <label>
                Dir
                <input
                  type="number"
                  min={0}
                  max={360}
                  step={5}
                  value={brushDirection}
                  onChange={(e) => setBrushDirection(Number(e.target.value) || 0)}
                />
                ° ({project.direction_convention})
              </label>
            ) : (
              <div className="brush-target">
                <label>
                  Target
                  <input
                    type="number"
                    step="any"
                    value={brushTarget[0]}
                    title="Longitude"
                    onChange={(e) =>
                      setBrushTarget(([, lat]) => [Number(e.target.value) || 0, lat])
                    }
                  />
                </label>
                <input
                  type="number"
                  step="any"
                  value={brushTarget[1]}
                  title="Latitude"
                  onChange={(e) =>
                    setBrushTarget(([lon]) => [
                      lon,
                      Math.min(90, Math.max(-90, Number(e.target.value) || 0)),
                    ])
                  }
                />
                <button
                  className={pickingTarget ? "active" : ""}
                  onClick={() => setPickingTarget((on) => !on)}
                  title="Click the map to place the point every vector aims at. Once placed, drag the marker to move it."
                >
                  {pickingTarget ? "Click the map…" : "Pick on map"}
                </button>
              </div>
            )}
            <label>
              Feather
              <input
                type="range"
                min={0}
                max={1}
                step={0.05}
                value={brushFeather}
                onChange={(e) => setBrushFeather(Number(e.target.value))}
              />
            </label>
          </div>
        )}

        <div className="history" role="group" aria-label="History">
          <button
            disabled={!project.can_undo || busy}
            onClick={() => void api.undo().then(onProjectChanged)}
            title="Undo (Cmd+Z)"
          >
            Undo
          </button>
          <button
            disabled={!project.can_redo || busy}
            onClick={() => void api.redo().then(onProjectChanged)}
            title="Redo (Cmd+Shift+Z)"
          >
            Redo
          </button>
        </div>

        <label>
          Glyphs
          <select
            value={showGlyphs ? glyphStyle : "off"}
            onChange={(e) => {
              const value = e.target.value;
              if (value === "off") setShowGlyphs(false);
              else {
                setShowGlyphs(true);
                setGlyphStyle(value === "barb" ? "barb" : "arrow");
              }
            }}
          >
            {barbsAvailable && <option value="barb">Wind barbs</option>}
            <option value="arrow">Arrows</option>
            <option value="off">Off</option>
          </select>
        </label>
        <label>
          <input
            type="checkbox"
            checked={showGraticule}
            onChange={(e) => setShowGraticule(e.target.checked)}
          />
          Graticule
        </label>
        <label className="step">
          Step {step} / {lastStep} (+{step * project.step_hours} h)
          <input
            type="range"
            min={0}
            max={lastStep}
            value={step}
            onChange={(e) => onStepChange(Number(e.target.value))}
          />
        </label>
        {pending > 0 && <span className="activity">rendering {pending}…</span>}
      </div>

      <div className="map-legend">
        <div
          className="legend-bar"
          style={{ background: `linear-gradient(to right, ${RAMP_STOPS.join(", ")})` }}
        />
        <div className="legend-labels">
          <span>0</span>
          <span>{rampMaxKnots} kt</span>
        </div>
      </div>

      <div className="map-readout">
        {readout ? (
          <>
            <span>{formatDegrees(readout.lat, "N", "S")}</span>
            <span>{formatDegrees(normalizeLon(readout.lon), "E", "W")}</span>
            <span className="accent">{readout.speedKnots.toFixed(1)} kt</span>
            <span>
              {Math.round(readout.directionDeg)}° ({project.direction_convention})
            </span>
            <span className="muted">{zoomPercent}%</span>
          </>
        ) : (
          <span className="muted">move the cursor over the map</span>
        )}
      </div>
    </div>
  );
}

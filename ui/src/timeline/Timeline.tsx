/**
 * The timeline dock (spec.md 9).
 *
 * Three things in one panel: a transport with a ruler and a readiness strip
 * (9.1, 9.4, 9.5); a tree of layers, objects and property tracks with range
 * bars and keyframe diamonds (9.2, 9.3); and the two timeline-wide settings —
 * the step count, gated behind its confirmation (4.1), and the start time.
 *
 * Every rule that is not drawing lives in `playback.ts`, and every edit goes
 * through the backend's keyframe commands, so what this file owns is layout
 * and pointer handling. Drags — a key, a range end, a box — preview locally and
 * write once on release: a write per pointer report bumps the revision and
 * re-renders the map (decision D29), and a keyframe drag must not do that.
 */

import { listen } from "@tauri-apps/api/event";
import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from "react";

import type { AppSettings } from "../generated/AppSettings";
import type { CaptureMode } from "../generated/CaptureMode";
import type { DocumentTree } from "../generated/DocumentTree";
import { actionFor, chordOf } from "../settings/bindings";
import type { InterpolationView } from "../generated/InterpolationView";
import type { ObjectTracks } from "../generated/ObjectTracks";
import type { ProjectSummary } from "../generated/ProjectSummary";
import type { PropertyValue } from "../generated/PropertyValue";
import type { RenderProgress } from "../generated/RenderProgress";
import type { ShrinkImpact } from "../generated/ShrinkImpact";
import type { TileAddress } from "../generated/TileAddress";
import type { TrackSamples } from "../generated/TrackSamples";
import NumberField from "../NumberField";
import { reportError, setHint } from "../hint";
import type { FieldKindName } from "../kind";
import { api } from "../ipc";
import { IconSvg, LOOP_ICON } from "../map/ToolIcon";
import { MAX_STEPS } from "../project/format";
import {
  type Extent,
  extentOf,
  formatValue,
  type PlotSeries,
  plotSeries,
  pointsOf,
  polyline,
} from "./graph";
import { markKind, runBetween } from "./frames";
import {
  classify,
  draggedStep,
  forecastLabel,
  freshMemory,
  interpolatedSteps,
  labelEvery,
  type ReadinessMemory,
  stepAt,
  steppedBy,
  type StepState,
  nextStep,
  tick,
  utcLabel,
} from "./playback";

/** Width of the labels column, in CSS pixels. Sticky, so it never scrolls. */
const LABELS_PX = 200;
/** The narrowest a step may be drawn, in CSS pixels. */
const MIN_STEP_PX = 6;
/** The widest. Past this the ruler stops stretching to fill the dock. */
const MAX_STEP_PX = 48;
/** Default playback rate (spec.md 9.4). */
const DEFAULT_RATE = 8;
/** Height of an expanded value graph, in CSS pixels. */
const GRAPH_PX = 46;

/** A selected keyframe's identity. */
function keyId(object: number, property: string, step: number): string {
  return `${object}|${property}|${step}`;
}

/** One key of the box or drag selection. */
interface KeyRef {
  object: number;
  property: string;
  step: number;
}

function parseKey(id: string): KeyRef {
  const [object, property, step] = id.split("|");
  return { object: Number(object), property: property ?? "", step: Number(step) };
}

/** A track's identity, for the graphs that are open and the samples fetched. */
function trackId(object: number, property: string): string {
  return `${object}|${property}`;
}

/**
 * Whether a property has a magnitude to graph.
 *
 * A boolean and a choice do not: they hold rather than blend (spec.md 4.5), so
 * a line through them would say nothing the diamonds do not.
 */
function graphable(base: PropertyValue): boolean {
  return base.kind === "number" || base.kind === "angle" || base.kind === "position";
}

/** Names for the easings, in the order the backend offers them. */
function easingName(interp: InterpolationView): string {
  switch (interp.kind) {
    case "step":
      return "Hold";
    case "linear":
      return "Linear";
    case "ease_in":
      return "Ease in";
    case "ease_out":
      return "Ease out";
    case "ease_in_out":
      return "Ease in–out";
    case "bezier":
      return "Custom curve";
  }
}

/** A start time as it is typed: a UTC date and hour (M29). */
interface StartDraft {
  year: number;
  month: number;
  day: number;
  hour: number;
}

/** A draft from a start time, or from now rounded to the nearest hour. */
export function startDraftFrom(startUnixS: number | null): StartDraft {
  const at =
    startUnixS !== null ? new Date(startUnixS * 1000) : nearestHour(new Date());
  return {
    year: at.getUTCFullYear(),
    month: at.getUTCMonth() + 1,
    day: at.getUTCDate(),
    hour: at.getUTCHours(),
  };
}

/** The clock rounded to the nearest whole hour, UTC. */
export function nearestHour(now: Date): Date {
  const rounded = new Date(now.getTime());
  rounded.setUTCMinutes(0, 0, 0);
  if (now.getUTCMinutes() >= 30) rounded.setUTCHours(rounded.getUTCHours() + 1);
  return rounded;
}

/** Seconds since the epoch for a draft, or null for a date that does not exist. */
export function unixOf(draft: StartDraft): number | null {
  const ms = Date.UTC(draft.year, draft.month - 1, draft.day, draft.hour);
  const back = new Date(ms);
  if (back.getUTCMonth() + 1 !== draft.month || back.getUTCDate() !== draft.day) return null;
  return Math.round(ms / 1000);
}

/**
 * What the motion switch adds, named for the track (M29): the object's
 * travel is a velocity, its turn a rotational vector, its growth a scale one.
 */
function motionWord(label: string): string {
  const lower = label.toLowerCase();
  if (lower.startsWith("pos")) return "velocity";
  if (lower.startsWith("rot")) return "rotational";
  return "scale";
}

export default function Timeline({
  project,
  step,
  onStepChange,
  selection,
  onSelect,
  viewport,
  warm,
  autoKey,
  onAutoKey,
  onChanged,
  onFramesSelected,
  onKeysSelected,
  settings,
  capture,
  hidden = false,
  shownKind,
  onCapture,
}: {
  project: ProjectSummary;
  step: number;
  onStepChange: (step: number) => void;
  selection: number[];
  onSelect: (objects: number[]) => void;
  /** The map's visible tiles, for render-ahead and readiness (spec.md 9.5). */
  viewport: TileAddress[];
  /**
   * Fetches a step's tiles onto the GPU and says whether they are all there.
   * Playback advances into a step only when the backend has it rendered *and*
   * the map has it resident (spec.md 9.4).
   */
  warm: (step: number) => boolean;
  autoKey: boolean;
  onAutoKey: (on: boolean) => void;
  onChanged: (project: ProjectSummary) => void;
  /**
   * Whether a GRIB frame is selected here, so the app's own copy and paste
   * stand down (spec.md 4.8, M20).
   */
  onFramesSelected: (active: boolean) => void;
  /**
   * Whether keyframes are selected here, so the app's own `Delete` stands
   * down (spec.md 9.3, M23).
   */
  onKeysSelected: (active: boolean) => void;
  /**
   * A capture command answered from here — a key removed from the
   * *selection position* row (D72) — handed back to the map, which owns
   * the capture state.
   */
  onCapture: (mode: CaptureMode) => void;
  /** The application's bindings table (spec.md 8.6, M15). */
  settings: AppSettings | null;
  /**
   * The macro capture in progress, if any (spec.md 8.7, M16).
   *
   * While it runs the timeline is a ruler and nothing else: every edit is
   * refused by the backend, so rather than let a drag fire and fail, the rows
   * and the transport are greyed out and dead to the pointer. The ruler stays
   * live — scrubbing is how the frames get visited — and marks the ones that
   * have been.
   */
  capture: CaptureMode | null;
  /** Put away: not drawn, but mounted, so playback goes on (M27). */
  hidden?: boolean;
  /** The kind of field the map shows, whose tiles readiness is about (M29). */
  shownKind: FieldKindName;
}) {
  const capturing = capture !== null && capture.active;
  const recordingPhase = capturing && capture.phase === "recording";
  const previewing = capturing && capture.phase === "previewing";
  const last = Math.max(0, project.step_count - 1);
  const steps = last + 1;
  /**
   * The first step the playhead may stand on: the capture's first step
   * while one runs — the steps before it are out of the run and dimmed
   * (spec.md 8.7, M26) — and step 0 otherwise.
   */
  const firstStep = capturing ? Math.min(capture.first_step, last) : 0;
  /** The last step playback reaches: the preview's run, or the timeline's. */
  const playLast = previewing ? Math.min(capture.last_step, last) : last;
  /**
   * Where the run ends, for the ruler's dimming: the frame the playhead is
   * on while recording — the macro will end there when recording ends — and
   * the fixed last step once the preview has baked it (spec.md 8.7).
   */
  const runEnd = recordingPhase ? step : previewing ? playLast : last;
  const clampStep = useCallback(
    (target: number) => Math.max(firstStep, Math.min(last, target)),
    [firstStep, last],
  );
  /** The capture's key the user has selected on its row, to delete. */
  const [selectedCaptureKey, setSelectedCaptureKey] = useState<number | null>(null);
  useEffect(() => {
    if (!recordingPhase) setSelectedCaptureKey(null);
  }, [recordingPhase]);

  // --- Layout ---
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const [dockWidth, setDockWidth] = useState(1200);
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    // A put-away timeline measures zero; the last real width stands.
    const observer = new ResizeObserver(() => {
      if (el.clientWidth > 0) setDockWidth(el.clientWidth);
    });
    observer.observe(el);
    setDockWidth(el.clientWidth);
    return () => observer.disconnect();
  }, []);
  // Stretch the ruler to fill the dock, within reason; scroll beyond it.
  const pxPerStep = Math.max(
    MIN_STEP_PX,
    Math.min(MAX_STEP_PX, Math.floor((dockWidth - LABELS_PX - 16) / steps)),
  );
  const gridWidth = steps * pxPerStep;
  const every = labelEvery(pxPerStep);

  // --- Readiness ---
  const memory = useRef<ReadinessMemory>(freshMemory());
  const [states, setStates] = useState<StepState[]>([]);
  const [progress, setProgress] = useState<number[]>([]);
  const statesRef = useRef<StepState[]>([]);
  statesRef.current = states;
  const viewportKey = viewport.map((t) => `${t.z}/${t.x}/${t.y}`).join(",");

  const refreshReadiness = useCallback(() => {
    if (viewport.length === 0) return;
    void api
      .frameReadiness(viewport, shownKind)
      .then((report) => {
        const next = classify(report, memory.current);
        setStates(next.states);
        setProgress(next.progress);
      })
      .catch(() => undefined);
    // `viewportKey` stands in for the array, which is a new value every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [viewportKey]);

  // A new revision, viewport or playhead re-queues the pool and re-asks.
  useEffect(() => {
    if (viewport.length === 0) return;
    void api.renderAhead(step, viewport, shownKind).catch(() => undefined);
    refreshReadiness();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project.revision, step, viewportKey, refreshReadiness]);

  // The pool says when a tile lands. Coalesced: a viewport is a hundred tiles
  // and a probe per tile would be a hundred probes a second.
  useEffect(() => {
    let timer: number | null = null;
    const pending = listen<RenderProgress>("render://progress", () => {
      if (timer !== null) return;
      timer = window.setTimeout(() => {
        timer = null;
        refreshReadiness();
      }, 120);
    });
    return () => {
      if (timer !== null) window.clearTimeout(timer);
      void pending.then((unlisten) => unlisten());
    };
  }, [refreshReadiness]);

  // --- Playback (spec.md 9.4) ---
  const [playing, setPlaying] = useState(false);
  const [loop, setLoop] = useState(false);
  const [rate, setRate] = useState(DEFAULT_RATE);
  const [buffering, setBuffering] = useState(false);
  /** The start time being typed, until Set or Cancel (M29). */
  const [startDraft, setStartDraft] = useState<StartDraft | null>(null);
  const stepRef = useRef(step);
  stepRef.current = step;
  const loopRef = useRef(loop);
  loopRef.current = loop;
  const rateRef = useRef(rate);
  rateRef.current = rate;
  const warmRef = useRef(warm);
  warmRef.current = warm;
  const previewRef = useRef(previewing);
  previewRef.current = previewing;
  const firstRef = useRef(firstStep);
  firstRef.current = firstStep;
  const playLastRef = useRef(playLast);
  playLastRef.current = playLast;
  const allReadyRef = useRef<StepState[]>([]);
  if (allReadyRef.current.length !== steps) {
    allReadyRef.current = Array.from({ length: steps }, () => "solid" as StepState);
  }
  // Entering the preview starts it playing from its first frame; leaving
  // stops. The playhead is never left before the run (spec.md 8.7, M26).
  useEffect(() => {
    if (previewing) {
      onStepChange(firstStep);
      setPlaying(true);
    } else {
      setPlaying(false);
    }
    // Only the phase change starts or stops it, not every re-render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [previewing]);
  useEffect(() => {
    if (capturing && step < firstStep) onStepChange(firstStep);
  }, [capturing, firstStep, onStepChange, step]);

  useEffect(() => {
    if (!playing) {
      setBuffering(false);
      return;
    }
    let frame = 0;
    let lastAdvance = performance.now();
    // A macro preview plays its run round and round, whatever the loop
    // switch says, and its readiness is the map's alone: the strip above
    // is the document's, and the preview is not the document (D71).
    const looping = () => loopRef.current || previewRef.current;
    const gate = () =>
      previewRef.current ? (allReadyRef.current as readonly StepState[]) : statesRef.current;
    const loopFrame = (now: number) => {
      // Keep the map two steps ahead of the playhead, so a step's tiles are on
      // the GPU before its turn comes and playback never waits on a fetch it
      // could have started earlier.
      const next = nextStep(stepRef.current, playLastRef.current, looping(), firstRef.current);
      if (next !== null && warmRef.current(next)) {
        const after = nextStep(next, playLastRef.current, looping(), firstRef.current);
        if (after !== null) warmRef.current(after);
      }
      const result = tick(
        stepRef.current,
        playLastRef.current,
        looping(),
        rateRef.current,
        now - lastAdvance,
        gate(),
        (target) => warmRef.current(target),
        firstRef.current,
      );
      if (result.finished) {
        setPlaying(false);
        return;
      }
      setBuffering(result.buffering);
      if (result.advanced) {
        lastAdvance = now;
        onStepChange(result.step);
      }
      frame = requestAnimationFrame(loopFrame);
    };
    frame = requestAnimationFrame(loopFrame);
    return () => cancelAnimationFrame(frame);
  }, [playing, last, onStepChange]);

  const stop = () => {
    setPlaying(false);
    onStepChange(0);
  };

  // --- Tree and tracks ---
  const [tree, setTree] = useState<DocumentTree | null>(null);
  const [expanded, setExpanded] = useState<Set<number>>(new Set());
  const [tracks, setTracks] = useState<Map<number, ObjectTracks>>(new Map());
  // Errors go to the status bar's hint area (M25), not a line of their own.
  const setError = reportError;
  /** Tracks whose value graph is expanded, by `trackId`. */
  const [graphs, setGraphs] = useState<Set<string>>(new Set());
  const [samples, setSamples] = useState<Map<string, TrackSamples>>(new Map());

  useEffect(() => {
    api
      .documentTree(step)
      .then(setTree)
      .catch((err: unknown) => setError(String(err)));
  }, [project.revision, step]);

  const expandedKey = [...expanded].sort((a, b) => a - b).join(",");
  useEffect(() => {
    const ids = [...expanded];
    if (ids.length === 0) {
      setTracks(new Map());
      return;
    }
    let live = true;
    void Promise.all(ids.map((id) => api.objectTracks(id, step).catch(() => null))).then(
      (found) => {
        if (!live) return;
        const next = new Map<number, ObjectTracks>();
        found.forEach((entry, index) => {
          if (entry) next.set(ids[index]!, entry);
        });
        setTracks(next);
      },
    );
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [expandedKey, project.revision, step]);

  // Samples for the open graphs. They describe the whole timeline, so they
  // depend on the revision and not on the step being viewed — scrubbing must
  // not re-fetch a hundred values per open graph.
  const graphKey = [...graphs].sort().join(",");
  useEffect(() => {
    const ids = [...graphs];
    if (ids.length === 0) {
      setSamples(new Map());
      return;
    }
    let live = true;
    void Promise.all(
      ids.map((id) => {
        const [object, property] = id.split("|");
        return api.trackSamples(Number(object), property ?? "").catch(() => null);
      }),
    ).then((found) => {
      if (!live) return;
      const next = new Map<string, TrackSamples>();
      found.forEach((entry, index) => {
        if (entry) next.set(ids[index]!, entry);
      });
      setSamples(next);
    });
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [graphKey, project.revision]);

  // Converted and scaled once per fetch rather than once per frame: a scrub
  // re-renders this panel at frame rate and the numbers do not change with the
  // playhead.
  const plots = useMemo(() => {
    const out = new Map<string, { series: PlotSeries[]; extent: Extent }>();
    for (const [id, entry] of samples) {
      if (entry.series.length === 0) continue;
      const series = entry.series.map((one) => plotSeries(one, project.direction_convention));
      out.set(id, { series, extent: extentOf(series) });
    }
    return out;
  }, [samples, project.direction_convention]);

  /**
   * Shows or hides an object's property tracks.
   *
   * Collapsing closes the graphs under it too: an open graph is re-sampled on
   * every revision, and one nobody can see should not be.
   */
  const toggleObject = (id: number) => {
    const closing = expanded.has(id);
    setExpanded((current) => {
      const next = new Set(current);
      if (closing) next.delete(id);
      else next.add(id);
      return next;
    });
    if (closing) {
      setGraphs((open) => new Set([...open].filter((graph) => !graph.startsWith(`${id}|`))));
    }
  };

  const toggleGraph = (object: number, property: string) => {
    setGraphs((current) => {
      const next = new Set(current);
      const id = trackId(object, property);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const run = (action: Promise<ProjectSummary>) => {
    setError(null);
    action.then(onChanged).catch((err: unknown) => setError(String(err)));
  };

  // --- Key selection and editing (spec.md 9.3) ---
  const [selectedKeys, setSelectedKeys] = useState<Set<string>>(new Set());
  /*
    Selected frames of an imported layer (spec.md 4.8, M20). One layer at a
    time: a run copied from two files would have to paste into two, and the
    paste goes back to the layer it came from.
  */
  const [frameSel, setFrameSel] = useState<{
    layer: number;
    anchor: number;
    steps: Set<number>;
  } | null>(null);
  /*
    A `Cmd`-drag from one object's position or rotation row to another's makes
    the first follow the second (spec.md 9.3, M13). Held here while the drag
    is in flight so the rows can say which of them would take the drop.
  */
  /**
   * A link armed by a *click* on the link button (M29): the next click on
   * another object's row of the same property completes it. The drag still
   * works; the click is for anyone who did not know to drag, which the
   * button gave no sign of.
   */
  const [linkArm, setLinkArm] = useState<{ object: number; property: string } | null>(null);
  useEffect(() => {
    if (linkArm === null) return;
    setHint("Click another object's row of the same property to follow it; click the link again to cancel.");
    return () => setHint(null);
  }, [linkArm]);
  const [linkDrag, setLinkDrag] = useState<{
    object: number;
    property: string;
  } | null>(null);
  /** A key drag in progress: which keys, and how far, in steps. */
  const [keyDrag, setKeyDrag] = useState<{ ids: string[]; delta: number } | null>(null);
  const keyDragRef = useRef<{ ids: string[]; startX: number; delta: number } | null>(null);
  /** A range-end drag in progress. */
  const [rangeDrag, setRangeDrag] = useState<{
    object: number;
    end: "start" | "end";
    step: number;
  } | null>(null);
  const rangeDragRef = useRef<{ object: number; end: "start" | "end"; other: number } | null>(null);
  /** A box selection in progress, in grid pixels. */
  const [box, setBox] = useState<{ x0: number; y0: number; x1: number; y1: number } | null>(null);
  const boxRef = useRef<{ x0: number; y0: number } | null>(null);
  /** The easing menu, for the key it was opened on. */
  const [menu, setMenu] = useState<{
    key: KeyRef;
    options: InterpolationView[];
    x: number;
    y: number;
  } | null>(null);

  const gridX = (event: { clientX: number }) => {
    const el = scrollRef.current;
    if (!el) return 0;
    const rect = el.getBoundingClientRect();
    return event.clientX - rect.left - LABELS_PX + el.scrollLeft;
  };

  const linkTo = useCallback(
    (follower: number, property: string, primary: number | null) => {
      void api
        .setFollow(follower, property, primary, stepRef.current)
        .then(onChanged)
        .catch((err: unknown) => setError(String(err)));
    },
    [onChanged],
  );

  /** Click a mark to select it; shift-click extends the run from the anchor. */
  const selectFrame = useCallback((layer: number, step: number, extend: boolean) => {
    setSelectedKeys(new Set());
    setFrameSel((current) => {
      if (extend && current && current.layer === layer) {
        return {
          layer,
          anchor: current.anchor,
          steps: new Set(runBetween(current.anchor, step)),
        };
      }
      return { layer, anchor: step, steps: new Set([step]) };
    });
  }, []);

  const copyFrames = useCallback(() => {
    if (!frameSel || frameSel.steps.size === 0) return;
    void api
      .copyGribFrames(frameSel.layer, [...frameSel.steps].sort((a, b) => a - b))
      .catch((err: unknown) => setError(String(err)));
  }, [frameSel]);

  const pasteFrames = useCallback(() => {
    void api
      .pasteGribFrames(stepRef.current)
      .then(onChanged)
      .catch((err: unknown) => setError(String(err)));
  }, [onChanged]);

  const deleteFrames = useCallback(() => {
    if (!frameSel || frameSel.steps.size === 0) return;
    const { layer, steps } = frameSel;
    setFrameSel(null);
    void api
      .deleteGribFrames(layer, [...steps].sort((a, b) => a - b))
      .then(onChanged)
      .catch((err: unknown) => setError(String(err)));
  }, [frameSel, onChanged]);

  const deleteSelected = useCallback(() => {
    const keys = [...selectedKeys].map(parseKey);
    if (keys.length === 0) return;
    setSelectedKeys(new Set());
    // One after another rather than in parallel, so the history entries land
    // in a stable order.
    void keys
      .reduce(
        (chain, key) =>
          chain.then(() => api.removeKeyframe(key.object, key.property, key.step)),
        Promise.resolve(project),
      )
      .then(onChanged)
      .catch((err: unknown) => setError(String(err)));
  }, [onChanged, project, selectedKeys]);

  // Keys: space plays and stops, the arrows step, delete removes, escape
  // clears. Never from a field.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      if (target && /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName)) return;
      // Copy and paste of imported frames, before the modifier guard below
      // sends every other combination to the app's own handler. A selected
      // mark is what makes this a frame gesture rather than an object one
      // (M20); App skips its object copy while one is selected.
      // During a capture only the arrows do anything here: the frames are
      // visited by scrubbing, and every other key is an edit the backend would
      // refuse (spec.md 8.7).
      if (
        recordingPhase &&
        (event.key === "Delete" || event.key === "Backspace") &&
        selectedCaptureKey !== null
      ) {
        event.preventDefault();
        const at = selectedCaptureKey;
        setSelectedCaptureKey(null);
        void api.unplaceCapture(at).then(onCapture).catch((err: unknown) => setError(String(err)));
        return;
      }
      if (capturing && !(event.key === "ArrowLeft" || event.key === "ArrowRight")) return;
      if ((event.metaKey || event.ctrlKey) && !event.altKey) {
        const key = event.key.toLowerCase();
        if (key === "c" && frameSel && frameSel.steps.size > 0) {
          event.preventDefault();
          copyFrames();
        } else if (key === "v" && frameSel) {
          event.preventDefault();
          pasteFrames();
        }
        return;
      }
      if (event.metaKey || event.ctrlKey || event.altKey) return;
      // Every binding comes from one table (spec.md 8.6, M15), so the map's
      // shifted arrows and the timeline's bare ones cannot collide and either
      // can be rebound without touching this file.
      const chord = chordOf(event);
      const bound = chord !== null && settings ? actionFor(settings, chord) : null;
      if (bound?.action === "play_pause") {
        event.preventDefault();
        setPlaying((on) => !on);
      } else if (bound?.action === "step_back" || bound?.action === "step_forward") {
        // One step per press, along the ruler. Playback stops: the steps are
        // for looking at a particular time, and a playhead that carried on
        // moving would take the step away again.
        event.preventDefault();
        setPlaying(false);
        const to = clampStep(
          steppedBy(stepRef.current, bound.action === "step_forward" ? 1 : -1, last),
        );
        if (to !== stepRef.current) onStepChange(to);
      } else if (
        (event.key === "Delete" || event.key === "Backspace") &&
        frameSel &&
        frameSel.steps.size > 0
      ) {
        // On a pasted frame this restores the file's own message; on the
        // file's own it hides that message (spec.md 4.8, M20).
        event.preventDefault();
        deleteFrames();
      } else if ((event.key === "Delete" || event.key === "Backspace") && selectedKeys.size > 0) {
        event.preventDefault();
        deleteSelected();
      } else if (event.key === "Escape" && (selectedKeys.size > 0 || menu || frameSel)) {
        setSelectedKeys(new Set());
        setFrameSel(null);
        setMenu(null);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [
    capturing,
    clampStep,
    onCapture,
    recordingPhase,
    selectedCaptureKey,
    copyFrames,
    deleteFrames,
    deleteSelected,
    frameSel,
    last,
    menu,
    onStepChange,
    pasteFrames,
    selectedKeys.size,
    settings,
  ]);

  // The app's own copy and paste stand down while a frame is selected, so one
  // key press cannot both copy an object and copy a frame (M20).
  useEffect(() => {
    onFramesSelected(frameSel !== null && frameSel.steps.size > 0);
  }, [frameSel, onFramesSelected]);
  useEffect(() => {
    onKeysSelected(selectedKeys.size > 0);
  }, [onKeysSelected, selectedKeys]);

  // --- Pointer handling over the grid ---
  const onGridPointerMove = (event: React.PointerEvent) => {
    const drag = keyDragRef.current;
    if (drag) {
      const delta = Math.round((event.clientX - drag.startX) / pxPerStep);
      if (delta !== drag.delta) {
        drag.delta = delta;
        setKeyDrag({ ids: drag.ids, delta });
      }
      return;
    }
    const range = rangeDragRef.current;
    if (range) {
      const at = stepAt(gridX(event), pxPerStep, last);
      setRangeDrag({ object: range.object, end: range.end, step: at });
      return;
    }
    const start = boxRef.current;
    if (start) {
      const el = scrollRef.current;
      const rect = el?.getBoundingClientRect();
      const y = rect ? event.clientY - rect.top + (el?.scrollTop ?? 0) : 0;
      setBox({ x0: start.x0, y0: start.y0, x1: gridX(event), y1: y });
    }
  };

  const onGridPointerUp = (event: React.PointerEvent) => {
    (event.currentTarget as HTMLElement).releasePointerCapture?.(event.pointerId);
    const drag = keyDragRef.current;
    if (drag) {
      keyDragRef.current = null;
      setKeyDrag(null);
      if (drag.delta !== 0) {
        // Written once, on release, in a stable order — and moved from the far
        // end first when going right, so a key never lands on a selected
        // neighbour that has not moved yet.
        const moves = drag.ids
          .map(parseKey)
          .sort((a, b) => (drag.delta > 0 ? b.step - a.step : a.step - b.step));
        const gesture = `keys:${Date.now()}`;
        void moves
          .reduce(
            (chain, key) =>
              chain.then(() =>
                api.moveKeyframe(
                  key.object,
                  key.property,
                  key.step,
                  draggedStep(key.step, drag.delta * pxPerStep, pxPerStep, last),
                  gesture,
                ),
              ),
            Promise.resolve(project),
          )
          .then((next) => {
            setSelectedKeys(
              new Set(
                moves.map((key) =>
                  keyId(
                    key.object,
                    key.property,
                    draggedStep(key.step, drag.delta * pxPerStep, pxPerStep, last),
                  ),
                ),
              ),
            );
            onChanged(next);
          })
          .catch((err: unknown) => setError(String(err)))
          .finally(() => void api.endGesture());
      }
      return;
    }
    const range = rangeDragRef.current;
    if (range) {
      rangeDragRef.current = null;
      const at = rangeDrag?.step;
      setRangeDrag(null);
      if (at !== undefined) {
        const [start, end] = range.end === "start" ? [at, range.other] : [range.other, at];
        run(api.setActiveRange(range.object, Math.min(start, end), Math.max(start, end)));
      }
      return;
    }
    if (boxRef.current && box) {
      boxRef.current = null;
      const chosen = new Set<string>();
      const x0 = Math.min(box.x0, box.x1);
      const x1 = Math.max(box.x0, box.x1);
      const y0 = Math.min(box.y0, box.y1);
      const y1 = Math.max(box.y0, box.y1);
      for (const [object, entry] of tracks) {
        entry.tracks.forEach((track, row) => {
          const top = rowTop(object, row);
          if (top === null || top + 11 < y0 || top - 11 > y1) return;
          for (const key of track.keys) {
            const x = (key.step + 0.5) * pxPerStep;
            if (x >= x0 && x <= x1) chosen.add(keyId(object, track.property, key.step));
          }
        });
      }
      setSelectedKeys(chosen);
      setBox(null);
    }
  };

  /**
   * The vertical position of a track row in the scroll area, from the DOM.
   * Rows have data attributes for exactly this; measuring beats re-deriving
   * the tree's layout in code.
   */
  const rowTop = (object: number, row: number): number | null => {
    const el = scrollRef.current?.querySelector<HTMLElement>(
      `[data-track="${object}:${row}"]`,
    );
    if (!el || !scrollRef.current) return null;
    const rect = el.getBoundingClientRect();
    const host = scrollRef.current.getBoundingClientRect();
    return rect.top - host.top + scrollRef.current.scrollTop + rect.height / 2;
  };

  const beginKeyDrag = (event: React.PointerEvent, id: string) => {
    event.stopPropagation();
    (event.currentTarget as HTMLElement).setPointerCapture?.(event.pointerId);
    const toggle = event.metaKey || event.ctrlKey || event.shiftKey;
    let ids: Set<string>;
    if (toggle) {
      ids = new Set(selectedKeys);
      if (ids.has(id)) ids.delete(id);
      else ids.add(id);
    } else if (selectedKeys.has(id)) {
      ids = selectedKeys;
    } else {
      ids = new Set([id]);
    }
    setSelectedKeys(ids);
    keyDragRef.current = { ids: [...ids], startX: event.clientX, delta: 0 };
  };

  const openMenu = (event: React.MouseEvent, key: KeyRef, options: InterpolationView[]) => {
    event.preventDefault();
    event.stopPropagation();
    setMenu({ key, options, x: event.clientX, y: event.clientY });
  };

  // --- Timeline-wide settings ---
  const [shrink, setShrink] = useState<{ to: number; impact: ShrinkImpact } | null>(null);
  const changeStepCount = (to: number) => {
    if (!Number.isFinite(to) || to < 1 || to > MAX_STEPS || to === project.step_count) return;
    if (to > project.step_count) {
      run(api.setStepCount(to));
      return;
    }
    // Shrinking deletes; the confirmation states exactly what (spec.md 4.1).
    void api
      .stepCountImpact(to)
      .then((impact) => setShrink({ to, impact }))
      .catch((err: unknown) => setError(String(err)));
  };


  // --- Render ---
  const tickLabel = (s: number) =>
    utcLabel(s, project.step_hours, project.start_unix_s) ?? forecastLabel(s, project.step_hours);

  return (
    <div className={capturing ? "timeline tl-capturing" : "timeline"} hidden={hidden}>
      <div className="tl-transport">
        <button onClick={() => setPlaying((on) => !on)} title="Play / pause (Space)">
          {playing ? "❚❚" : "▶"}
        </button>
        <button onClick={stop} title="Stop and return to the start">
          ■
        </button>
        <button
          className={loop ? "active tl-loop" : "tl-loop"}
          onClick={() => setLoop((on) => !on)}
          title="Loop"
          aria-label="Loop"
          aria-pressed={loop}
        >
          <IconSvg icon={LOOP_ICON} size={22} />
        </button>
        <label title="Steps per second (spec.md 9.4)">
          <NumberField
            min={0.5}
            max={60}
            step={0.5}
            value={rate}
            onCommit={setRate}
          />
          steps/s
        </label>
        <button
          className={autoKey ? "active autokey" : "autokey"}
          onClick={() => onAutoKey(!autoKey)}
          title="Auto-key: editing a property keys it at the current step (spec.md 9.3)"
        >
          ◆ Auto-key
        </button>
        <span className="tl-position">
          Step {step} / {last} · {forecastLabel(step, project.step_hours)}
          {/* The frame's UTC time, when step 0 has one (M29). */}
          {utcLabel(step, project.step_hours, project.start_unix_s) !== null && (
            <span className="tl-when"> · {utcLabel(step, project.step_hours, project.start_unix_s)}</span>
          )}
          {buffering && <span className="tl-buffering"> · buffering…</span>}
        </span>
        {startDraft !== null && (
          <span className="tl-start-editor" role="group" aria-label="Start time (UTC)">
            <label>
              Y
              <NumberField
                min={1900}
                max={2999}
                value={startDraft.year}
                onCommit={(year) => setStartDraft({ ...startDraft, year })}
              />
            </label>
            <label>
              M
              <NumberField
                min={1}
                max={12}
                value={startDraft.month}
                onCommit={(month) => setStartDraft({ ...startDraft, month })}
              />
            </label>
            <label>
              D
              <NumberField
                min={1}
                max={31}
                value={startDraft.day}
                onCommit={(day) => setStartDraft({ ...startDraft, day })}
              />
            </label>
            <label>
              h
              <NumberField
                min={0}
                max={23}
                value={startDraft.hour}
                onCommit={(hour) => setStartDraft({ ...startDraft, hour })}
              />
            </label>
            <span className="muted">UTC</span>
            <button
              onClick={() => {
                const unix = unixOf(startDraft);
                setStartDraft(null);
                if (unix !== null) run(api.setStartTime(unix));
              }}
            >
              Set
            </button>
            <button onClick={() => setStartDraft(null)}>Cancel</button>
          </span>
        )}
        <span className="spacer" />
        {/*
          No start-time field (D69): a project has no clock of its own until a
          GRIB file gives it one, and the export asks for the one it needs.
          The ruler labels forecast hours until then.
        */}
        <label title="Number of time steps. Reducing it deletes keyframes past the end, after a confirmation (spec.md 4.1)">
          Steps
          <NumberField
            min={1}
            max={MAX_STEPS}
            value={project.step_count}
            // On blur, not on every keystroke: shrinking asks for a
            // confirmation that names what it will delete (spec.md 4.1), and
            // typing 12 must not ask it on the way past 1.
            commitWhileTyping={false}
            onCommit={changeStepCount}
          />
        </label>
      </div>

      <div
        className="tl-scroll"
        ref={scrollRef}
        onPointerMove={onGridPointerMove}
        onPointerUp={onGridPointerUp}
        onPointerCancel={onGridPointerUp}
        onClick={() => setMenu(null)}
      >
        {/* The ruler: ticks, labels, readiness, and the playhead. */}
        <div className="tl-row tl-ruler">
          <div className="tl-labels tl-ruler-label">
            {recordingPhase ? (
              <span className="tl-recording">● Recording</span>
            ) : previewing ? (
              <span className="tl-preview-label">Macro preview</span>
            ) : (
              <>
                Time
                {/*
                  A start time, by choice (M29): step 0 as a UTC date and
                  hour, so the ruler and the step readout can say when a
                  frame is. Optional, removable, and read by nothing but the
                  labels and the export dialog's default (spec.md 9.1).
                */}
                {project.start_unix_s === null ? (
                  <button
                    className="tl-start"
                    onClick={() => setStartDraft(startDraftFrom(null))}
                    title="Give step 0 a UTC date and time. Labels then show when each frame is, and the export starts there by default."
                  >
                    Add start time
                  </button>
                ) : (
                  <button
                    className="tl-start"
                    onClick={() => run(api.setStartTime(null))}
                    title="Remove the start time. Nothing else changes."
                    aria-label="Remove the start time"
                  >
                    ×
                  </button>
                )}
              </>
            )}
          </div>
          <div
            className="tl-grid"
            style={{ width: gridWidth }}
            onPointerDown={(event) => {
              setPlaying(false);
              onStepChange(clampStep(stepAt(gridX(event), pxPerStep, last)));
              const scrub = (move: PointerEvent) =>
                onStepChange(clampStep(stepAt(gridX(move), pxPerStep, last)));
              const done = () => {
                window.removeEventListener("pointermove", scrub);
                window.removeEventListener("pointerup", done);
              };
              window.addEventListener("pointermove", scrub);
              window.addEventListener("pointerup", done);
            }}
          >
            {Array.from({ length: steps }, (_, s) => (
              <div
                key={s}
                className={[
                  "tl-tick",
                  states[s] ?? "empty",
                  // A frame the capture has placed its region at, and the one
                  // it began at: what the user has done and where it started.
                  capturing && capture.keys.includes(s) ? "tl-visited" : "",
                  capturing && capture.first_step === s ? "tl-capture-origin" : "",
                  capturing && (s < firstStep || s > runEnd) ? "tl-outside" : "",
                ].join(" ")}
                style={{ left: s * pxPerStep, width: pxPerStep }}
                title={
                  capturing
                    ? `${tickLabel(s)} · ${
                        s < firstStep
                          ? "before the capture"
                          : s > runEnd
                            ? "after the frame the recording ends on"
                            : capture.keys.includes(s)
                            ? "region keyed here"
                            : "region between its keys"
                      }`
                    : `${tickLabel(s)} · ${states[s] ?? "not rendered"}`
                }
              >
                {s % every === 0 && <span className="tl-tick-label">{tickLabel(s)}</span>}
                <span
                  className="tl-ready"
                  style={{ width: `${Math.round((progress[s] ?? 0) * 100)}%` }}
                />
              </div>
            ))}
            <div className="tl-playhead" style={{ left: (step + 0.5) * pxPerStep }} />
          </div>
        </div>

        {/*
          The capture's own track while it records (D72, M26): a temporary
          row, *selection position*, with a diamond at every step that holds
          a key and a dot at every step between keys, which the position
          interpolates through. Click a diamond and press Delete, or
          Alt-click it, to remove the key. The first step's key stays.
        */}
        {recordingPhase && (
          <div className="tl-row tl-track tl-capture-track">
            <div className="tl-labels tl-track-label">
              <span className="tl-track-name" title="Where the recorded region sits at each step">
                selection position
              </span>
            </div>
            <div className="tl-grid" style={{ width: gridWidth }}>
              {Array.from({ length: steps }, (_, s) => {
                if (s < firstStep) return null;
                const keyed = capture.keys.includes(s);
                const lastKey = capture.keys[capture.keys.length - 1] ?? firstStep;
                if (!keyed) {
                  return s < lastKey ? (
                    <span key={s} className="tl-dot" style={{ left: (s + 0.5) * pxPerStep }} />
                  ) : null;
                }
                return (
                  <span
                    key={s}
                    className={`tl-key${selectedCaptureKey === s ? " selected" : ""}`}
                    style={{ left: (s + 0.5) * pxPerStep }}
                    title={
                      s === capture.first_step
                        ? "The first key; it stays"
                        : "Click and press Delete, or Alt-click, to remove this key"
                    }
                    onPointerDown={(event) => {
                      event.stopPropagation();
                      if (s === capture.first_step) return;
                      if (event.altKey) {
                        void api
                          .unplaceCapture(s)
                          .then(onCapture)
                          .catch((err: unknown) => setError(String(err)));
                        return;
                      }
                      setSelectedCaptureKey((current) => (current === s ? null : s));
                    }}
                  />
                );
              })}
            </div>
          </div>
        )}

        {/*
          The tree, in the layer panel's order (M29): top of the stack first,
          and within a layer the topmost object first, which is the reverse
          of the document's bottom-first order both here and there.
        */}
        {[...(tree?.layers ?? [])].reverse().map((layer) => (
          <div key={layer.id} className="tl-layer">
            <div className="tl-row tl-layer-row">
              <div className="tl-labels">{layer.name}</div>
              <div className="tl-grid" style={{ width: gridWidth }}>
                {/*
                  Which steps an imported field has a message for (spec.md 4.8).
                  A step it says nothing about shows no field at all, and
                  without this the user is left to work out from a field that
                  comes and goes which times the file actually covers.
                */}
                {layer.grib?.steps.map((frame, s) => {
                  // A step worth marking is one that has something to say:
                  // the file's own message, a frame pasted onto it, or a
                  // message the user hid. A plain gap is left blank.
                  const kind = markKind(frame);
                  if (kind === "none") return null;
                  const chosen =
                    frameSel?.layer === layer.id && frameSel.steps.has(s);
                  const what =
                    kind === "pasted"
                      ? `shows the message from ${tickLabel(frame.source ?? 0)}`
                      : kind === "hidden"
                        ? "its message is hidden"
                        : `a message at ${tickLabel(s)}`;
                  return (
                    <span
                      key={s}
                      className={`tl-grib${kind === "pasted" ? " pasted" : ""}${
                        kind === "hidden" ? " hidden" : ""
                      }${chosen ? " selected" : ""}`}
                      style={{ left: s * pxPerStep, width: Math.max(2, pxPerStep - 1) }}
                      title={`${layer.name}: ${what}`}
                      onPointerDown={(event) => {
                        event.stopPropagation();
                        selectFrame(layer.id, s, event.shiftKey);
                      }}
                    />
                  );
                })}
              </div>
            </div>
            {[...layer.objects].reverse().map((object) => {
              const open = expanded.has(object.id);
              const entry = tracks.get(object.id);
              const drag = rangeDrag?.object === object.id ? rangeDrag : null;
              const start = drag?.end === "start" ? drag.step : object.start_step;
              const end = drag?.end === "end" ? drag.step : object.end_step;
              const lo = Math.min(start, end);
              const hi = Math.max(start, end);
              return (
                <div key={object.id} className="tl-object">
                  <div
                    className={`tl-row tl-object-row${selection.includes(object.id) ? " selected" : ""}`}
                  >
                    <div className="tl-labels">
                      <button
                        className="tl-disclose"
                        onClick={() => toggleObject(object.id)}
                        title={open ? "Hide properties" : "Show properties"}
                      >
                        {open ? "▾" : "▸"}
                      </button>
                      <span
                        className="tl-object-name"
                        onClick={() => onSelect([object.id])}
                        title={object.tool_label}
                      >
                        {object.name}
                      </span>
                    </div>
                    <div
                      className="tl-grid"
                      style={{ width: gridWidth }}
                      onPointerDown={(event) => {
                        // Empty grid: begin a box selection.
                        const el = scrollRef.current;
                        const rect = el?.getBoundingClientRect();
                        const y = rect ? event.clientY - rect.top + (el?.scrollTop ?? 0) : 0;
                        boxRef.current = { x0: gridX(event), y0: y };
                        setBox({ x0: gridX(event), y0: y, x1: gridX(event), y1: y });
                        (event.currentTarget as HTMLElement).setPointerCapture?.(event.pointerId);
                      }}
                    >
                      {/* The lifetime bar, draggable at either end (spec.md 9.2). */}
                      <div
                        className="tl-range"
                        style={{ left: lo * pxPerStep, width: (hi - lo + 1) * pxPerStep }}
                        title={`Active steps ${lo}–${hi}`}
                      >
                        <span
                          className="tl-range-end"
                          onPointerDown={(event) => {
                            event.stopPropagation();
                            (event.currentTarget as HTMLElement).setPointerCapture?.(
                              event.pointerId,
                            );
                            rangeDragRef.current = {
                              object: object.id,
                              end: "start",
                              other: object.end_step,
                            };
                            setRangeDrag({ object: object.id, end: "start", step: object.start_step });
                          }}
                        />
                        <span
                          className="tl-range-end right"
                          onPointerDown={(event) => {
                            event.stopPropagation();
                            (event.currentTarget as HTMLElement).setPointerCapture?.(
                              event.pointerId,
                            );
                            rangeDragRef.current = {
                              object: object.id,
                              end: "end",
                              other: object.start_step,
                            };
                            setRangeDrag({ object: object.id, end: "end", step: object.end_step });
                          }}
                        />
                      </div>
                    </div>
                  </div>

                  {/* Property tracks (spec.md 9.3). */}
                  {open &&
                    entry?.tracks.map((track, row) => {
                      const graphId = trackId(object.id, track.property);
                      const graphOpen = graphs.has(graphId);
                      const plot = plots.get(graphId) ?? null;
                      // The keys as drawn: a drag moves the selected ones, and
                      // the dots between them have to move with them.
                      const drawn = track.keys.map((key) => ({
                        step:
                          keyDrag && selectedKeys.has(keyId(object.id, track.property, key.step))
                            ? draggedStep(key.step, keyDrag.delta * pxPerStep, pxPerStep, last)
                            : key.step,
                        hold: key.interp.kind === "step",
                      }));
                      return (
                        <Fragment key={track.property}>
                        <div
                          className={`tl-row tl-track${track.interpolated_here ? " interpolated" : ""}${
                            (linkDrag ?? linkArm) &&
                            (linkDrag ?? linkArm)?.property === track.property &&
                            (linkDrag ?? linkArm)?.object !== object.id
                              ? " link-target"
                              : ""
                          }${track.follows !== null ? " following" : ""}`}
                          data-track={`${object.id}:${row}`}
                          onClick={() => {
                            // An armed link lands on the same property of
                            // another object, as a dragged one does (M29).
                            if (
                              linkArm &&
                              linkArm.property === track.property &&
                              linkArm.object !== object.id
                            ) {
                              linkTo(linkArm.object, linkArm.property, object.id);
                              setLinkArm(null);
                            }
                          }}
                          onPointerUp={() => {
                            // A drop on the *same* property of another object
                            // is the only one that means anything: a link is
                            // between two of the same kind of value.
                            if (
                              linkDrag &&
                              linkDrag.property === track.property &&
                              linkDrag.object !== object.id
                            ) {
                              linkTo(linkDrag.object, linkDrag.property, object.id);
                            }
                            setLinkDrag(null);
                          }}
                        >
                          <div className="tl-labels tl-track-label">
                            {graphable(track.base) ? (
                              <button
                                className="tl-disclose tl-graph-toggle"
                                onClick={() => toggleGraph(object.id, track.property)}
                                title={
                                  graphOpen
                                    ? "Hide the value graph"
                                    : "Show this property's value at every step"
                                }
                              >
                                {graphOpen ? "▾" : "▸"}
                              </button>
                            ) : (
                              <span className="tl-graph-toggle" />
                            )}
                            <span className="tl-track-name">{track.label}</span>
                            {/*
                              Motion: while this is on, the object's own
                              movement along this track is added to the vector
                              it paints (spec.md 9.3, M13). One per track and
                              not one per object, so a system that spins and
                              travels can put in the spin alone.
                            */}
                            {track.motion_available && (
                              <button
                                className={`tl-motion${track.motion ? " on" : ""}`}
                                title={
                                  track.motion
                                    ? `Remove ${motionWord(track.label)} vectors from the vector data.`
                                    : `Add ${motionWord(track.label)} vectors to the vector data.`
                                }
                                onClick={() =>
                                  run(api.setMotion(object.id, track.property, !track.motion))
                                }
                              >
                                ⇢
                              </button>
                            )}
                            {/*
                              Following: the glyph says which object, and
                              clicking it unlinks — which holds the value
                              where it stands rather than snapping back to the
                              dormant keys underneath (spec.md 9.3, D42).
                            */}
                            {track.can_follow &&
                              (track.follows !== null ? (
                                <button
                                  className="tl-follow on"
                                  title={`Following ${track.follows_name ?? "another object"} — click to unlink`}
                                  onClick={() => linkTo(object.id, track.property, null)}
                                >
                                  ⛓
                                </button>
                              ) : (
                                <button
                                  className={
                                    linkArm?.object === object.id && linkArm.property === track.property
                                      ? "tl-follow armed"
                                      : "tl-follow"
                                  }
                                  title={`Click, then click another object's ${track.label.toLowerCase()} row to follow it — or drag onto that row`}
                                  onClick={() =>
                                    setLinkArm((current) =>
                                      current?.object === object.id && current.property === track.property
                                        ? null
                                        : { object: object.id, property: track.property },
                                    )
                                  }
                                  onPointerDown={(event) => {
                                    event.preventDefault();
                                    setLinkDrag({
                                      object: object.id,
                                      property: track.property,
                                    });
                                    const done = () => {
                                      setLinkDrag(null);
                                      window.removeEventListener("pointerup", done);
                                    };
                                    window.addEventListener("pointerup", done);
                                  }}
                                >
                                  ⛓
                                </button>
                              ))}
                            {track.interpolated_here && (
                              <span className="muted" title="Interpolated between keys at this step">
                                ~
                              </span>
                            )}
                            <button
                              className={`tl-key-here${track.keyed_here ? " on" : ""}`}
                              title={
                                track.keyed_here
                                  ? "Remove the key at this step"
                                  : "Key this property at this step"
                              }
                              onClick={() =>
                                run(
                                  track.keyed_here
                                    ? api.removeKeyframe(object.id, track.property, step)
                                    : api.setKeyframe(object.id, track.property, step),
                                )
                              }
                            >
                              ◆
                            </button>
                          </div>
                          <div
                            className="tl-grid"
                            style={{ width: gridWidth }}
                            onPointerDown={(event) => {
                              const el = scrollRef.current;
                              const rect = el?.getBoundingClientRect();
                              const y = rect ? event.clientY - rect.top + (el?.scrollTop ?? 0) : 0;
                              boxRef.current = { x0: gridX(event), y0: y };
                              setBox({ x0: gridX(event), y0: y, x1: gridX(event), y1: y });
                              (event.currentTarget as HTMLElement).setPointerCapture?.(event.pointerId);
                            }}
                          >
                            {/* One dot per interpolated step, so a blended
                                segment reads as animated and a held one does
                                not (spec.md 9.3). */}
                            {interpolatedSteps(drawn).map((tween) => (
                              <span
                                key={`tween-${tween}`}
                                className="tl-tween"
                                style={{ left: (tween + 0.5) * pxPerStep }}
                              />
                            ))}
                            {track.keys.map((key) => {
                              const id = keyId(object.id, track.property, key.step);
                              const selected = selectedKeys.has(id);
                              const shown =
                                keyDrag && selected
                                  ? draggedStep(key.step, keyDrag.delta * pxPerStep, pxPerStep, last)
                                  : key.step;
                              return (
                                <span
                                  key={key.step}
                                  className={`tl-key${selected ? " selected" : ""}${key.interp.kind === "step" ? " hold" : ""}`}
                                  style={{ left: (shown + 0.5) * pxPerStep }}
                                  title={`${track.label} at step ${key.step} · ${easingName(key.interp)}`}
                                  onPointerDown={(event) => beginKeyDrag(event, id)}
                                  onContextMenu={(event) =>
                                    openMenu(
                                      event,
                                      { object: object.id, property: track.property, step: key.step },
                                      track.interpolations,
                                    )
                                  }
                                />
                              );
                            })}
                          </div>
                        </div>

                        {/* The value graph, below its track (spec.md 9.3). */}
                        {graphOpen && (
                          <div className="tl-row tl-graph-row">
                            <div className="tl-labels tl-graph-label">
                              {plot ? (
                                <>
                                  <span className="tl-graph-now-value">
                                    {plot.series.map((one, index) => (
                                      <span
                                        key={one.label || "value"}
                                        className={`tl-graph-series s${index}`}
                                      >
                                        {one.label && `${one.label} `}
                                        {one.values[step] === undefined
                                          ? "—"
                                          : formatValue(one.unit, one.values[step])}
                                      </span>
                                    ))}
                                  </span>
                                  <span className="muted">
                                    {formatValue(plot.series[0]?.unit ?? "none", plot.extent.min)}
                                    {" – "}
                                    {formatValue(plot.series[0]?.unit ?? "none", plot.extent.max)}
                                  </span>
                                </>
                              ) : (
                                <span className="muted">Sampling…</span>
                              )}
                            </div>
                            <div className="tl-grid" style={{ width: gridWidth }}>
                              {plot && (
                                <svg
                                  className="tl-graph"
                                  width={gridWidth}
                                  height={GRAPH_PX}
                                  viewBox={`0 0 ${gridWidth} ${GRAPH_PX}`}
                                  preserveAspectRatio="none"
                                >
                                  <line
                                    className="tl-graph-playhead"
                                    x1={(step + 0.5) * pxPerStep}
                                    x2={(step + 0.5) * pxPerStep}
                                    y1={0}
                                    y2={GRAPH_PX}
                                  />
                                  {plot.series.map((one, index) => {
                                    const points = pointsOf(
                                      one.values,
                                      pxPerStep,
                                      GRAPH_PX,
                                      plot.extent,
                                    );
                                    const here = points[step];
                                    return (
                                      <g key={one.label || "value"} className={`s${index}`}>
                                        <polyline
                                          className="tl-graph-line"
                                          points={polyline(points)}
                                        />
                                        {track.keys.map((key) => {
                                          const at = points[key.step];
                                          return at ? (
                                            <circle
                                              key={key.step}
                                              className="tl-graph-key"
                                              cx={at.x}
                                              cy={at.y}
                                              r={2.5}
                                            />
                                          ) : null;
                                        })}
                                        {here && (
                                          <circle
                                            className="tl-graph-at"
                                            cx={here.x}
                                            cy={here.y}
                                            r={3}
                                          />
                                        )}
                                      </g>
                                    );
                                  })}
                                </svg>
                              )}
                            </div>
                          </div>
                        )}
                        </Fragment>
                      );
                    })}
                </div>
              );
            })}
          </div>
        ))}

        {box && (
          <div
            className="tl-box"
            style={{
              left: LABELS_PX + Math.min(box.x0, box.x1),
              top: Math.min(box.y0, box.y1),
              width: Math.abs(box.x1 - box.x0),
              height: Math.abs(box.y1 - box.y0),
            }}
          />
        )}
      </div>



      {menu && (
        <div
          className="tl-menu"
          style={{ left: menu.x, top: menu.y }}
          onClick={(event) => event.stopPropagation()}
        >
          <div className="tl-menu-title">Ease the segment leaving step {menu.key.step}</div>
          {menu.options.map((option) => (
            <button
              key={option.kind}
              onClick={() => {
                setMenu(null);
                run(api.setInterpolation(menu.key.object, menu.key.property, menu.key.step, option));
              }}
            >
              {easingName(option)}
            </button>
          ))}
        </div>
      )}

      {shrink && (
        <div className="modal-backdrop" onClick={() => setShrink(null)}>
          <div className="modal modal-narrow" onClick={(event) => event.stopPropagation()}>
            <h2>Reduce to {shrink.to} steps?</h2>
            <p>
              Reducing to {shrink.to} steps will delete <strong>{shrink.impact.keyframes}</strong>{" "}
              keyframe{shrink.impact.keyframes === 1 ? "" : "s"} and shorten{" "}
              <strong>{shrink.impact.clamped_ranges}</strong> lifetime
              {shrink.impact.clamped_ranges === 1 ? "" : "s"} across{" "}
              <strong>{shrink.impact.objects.length}</strong> object
              {shrink.impact.objects.length === 1 ? "" : "s"}.
            </p>
            {shrink.impact.objects.length > 0 && (
              <ul className="tl-impact">
                {shrink.impact.objects.map((name, index) => (
                  <li key={`${name}-${index}`}>{name}</li>
                ))}
              </ul>
            )}
            <p className="muted">This can be undone until the project is saved.</p>
            <div className="modal-actions">
              <button onClick={() => setShrink(null)}>Cancel</button>
              <button
                className="danger"
                onClick={() => {
                  const to = shrink.to;
                  setShrink(null);
                  run(api.setStepCount(to));
                  if (step > to - 1) onStepChange(to - 1);
                }}
              >
                Reduce to {shrink.to} steps
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

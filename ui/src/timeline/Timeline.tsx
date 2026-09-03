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
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import type { DocumentTree } from "../generated/DocumentTree";
import type { InterpolationView } from "../generated/InterpolationView";
import type { ObjectTracks } from "../generated/ObjectTracks";
import type { ProjectSummary } from "../generated/ProjectSummary";
import type { RenderProgress } from "../generated/RenderProgress";
import type { ShrinkImpact } from "../generated/ShrinkImpact";
import type { TileAddress } from "../generated/TileAddress";
import { api } from "../ipc";
import {
  classify,
  draggedStep,
  forecastLabel,
  freshMemory,
  labelEvery,
  type ReadinessMemory,
  stepAt,
  type StepState,
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

export default function Timeline({
  project,
  step,
  onStepChange,
  selection,
  onSelect,
  viewport,
  autoKey,
  onAutoKey,
  onChanged,
}: {
  project: ProjectSummary;
  step: number;
  onStepChange: (step: number) => void;
  selection: number[];
  onSelect: (objects: number[]) => void;
  /** The map's visible tiles, for render-ahead and readiness (spec.md 9.5). */
  viewport: TileAddress[];
  autoKey: boolean;
  onAutoKey: (on: boolean) => void;
  onChanged: (project: ProjectSummary) => void;
}) {
  const last = Math.max(0, project.step_count - 1);
  const steps = last + 1;

  // --- Layout ---
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const [dockWidth, setDockWidth] = useState(1200);
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const observer = new ResizeObserver(() => setDockWidth(el.clientWidth));
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
      .frameReadiness(viewport)
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
    void api.renderAhead(step, viewport).catch(() => undefined);
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
  const stepRef = useRef(step);
  stepRef.current = step;
  const loopRef = useRef(loop);
  loopRef.current = loop;
  const rateRef = useRef(rate);
  rateRef.current = rate;

  useEffect(() => {
    if (!playing) {
      setBuffering(false);
      return;
    }
    let frame = 0;
    let lastAdvance = performance.now();
    const loopFrame = (now: number) => {
      const result = tick(
        stepRef.current,
        last,
        loopRef.current,
        rateRef.current,
        now - lastAdvance,
        statesRef.current,
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
  const [error, setError] = useState<string | null>(null);

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

  const run = (action: Promise<ProjectSummary>) => {
    setError(null);
    action.then(onChanged).catch((err: unknown) => setError(String(err)));
  };

  // --- Key selection and editing (spec.md 9.3) ---
  const [selectedKeys, setSelectedKeys] = useState<Set<string>>(new Set());
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

  // Keys: space plays, delete removes, escape clears. Never from a field.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      if (target && /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName)) return;
      if (event.metaKey || event.ctrlKey || event.altKey) return;
      if (event.key === " ") {
        event.preventDefault();
        setPlaying((on) => !on);
      } else if ((event.key === "Delete" || event.key === "Backspace") && selectedKeys.size > 0) {
        event.preventDefault();
        deleteSelected();
      } else if (event.key === "Escape" && (selectedKeys.size > 0 || menu)) {
        setSelectedKeys(new Set());
        setMenu(null);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [deleteSelected, menu, selectedKeys.size]);

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
    if (!Number.isFinite(to) || to < 1 || to > 240 || to === project.step_count) return;
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

  const startValue = useMemo(() => {
    if (project.start_unix_s === null) return "";
    return new Date(project.start_unix_s * 1000).toISOString().slice(0, 16);
  }, [project.start_unix_s]);

  // --- Render ---
  const tickLabel = (s: number) =>
    utcLabel(s, project.step_hours, project.start_unix_s) ?? forecastLabel(s, project.step_hours);

  return (
    <div className="timeline">
      <div className="tl-transport">
        <button onClick={() => setPlaying((on) => !on)} title="Play / pause (Space)">
          {playing ? "❚❚" : "▶"}
        </button>
        <button onClick={stop} title="Stop and return to the start">
          ■
        </button>
        <button
          className={loop ? "active" : ""}
          onClick={() => setLoop((on) => !on)}
          title="Loop"
        >
          ⟳
        </button>
        <label title="Steps per second (spec.md 9.4)">
          <input
            type="number"
            min={0.5}
            max={60}
            step={0.5}
            value={rate}
            onChange={(e) => setRate(Math.max(0.5, Number(e.target.value) || DEFAULT_RATE))}
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
          Step {step} / {last} · {tickLabel(step)}
          {buffering && <span className="tl-buffering"> · buffering…</span>}
        </span>
        <span className="spacer" />
        <label title="When step 0 is, in UTC (spec.md 9.1)">
          Start
          <input
            type="datetime-local"
            value={startValue}
            onChange={(e) => {
              const raw = e.target.value;
              if (raw === "") {
                run(api.setStartTime(null));
                return;
              }
              // The input has no zone; it is read as UTC, because the ruler is.
              const at = Date.parse(`${raw}:00Z`);
              if (Number.isFinite(at)) run(api.setStartTime(Math.round(at / 1000)));
            }}
          />
          <button onClick={() => run(api.setStartTime(null))} title="Clear the start time">
            ×
          </button>
        </label>
        <label title="Number of time steps. Reducing it deletes keyframes past the end, after a confirmation (spec.md 4.1)">
          Steps
          <input
            type="number"
            min={1}
            max={240}
            value={project.step_count}
            onChange={(e) => changeStepCount(Number(e.target.value))}
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
          <div className="tl-labels tl-ruler-label">Time</div>
          <div
            className="tl-grid"
            style={{ width: gridWidth }}
            onPointerDown={(event) => {
              setPlaying(false);
              onStepChange(stepAt(gridX(event), pxPerStep, last));
              const scrub = (move: PointerEvent) =>
                onStepChange(stepAt(gridX(move), pxPerStep, last));
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
                className={`tl-tick ${states[s] ?? "empty"}`}
                style={{ left: s * pxPerStep, width: pxPerStep }}
                title={`${tickLabel(s)} · ${states[s] ?? "not rendered"}`}
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

        {/* The tree. */}
        {tree?.layers.map((layer) => (
          <div key={layer.id} className="tl-layer">
            <div className="tl-row tl-layer-row">
              <div className="tl-labels">{layer.name}</div>
              <div className="tl-grid" style={{ width: gridWidth }} />
            </div>
            {layer.objects.map((object) => {
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
                        onClick={() =>
                          setExpanded((current) => {
                            const next = new Set(current);
                            if (next.has(object.id)) next.delete(object.id);
                            else next.add(object.id);
                            return next;
                          })
                        }
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
                    entry?.tracks.map((track, row) => (
                      <div
                        key={track.property}
                        className={`tl-row tl-track${track.interpolated_here ? " interpolated" : ""}`}
                        data-track={`${object.id}:${row}`}
                      >
                        <div className="tl-labels tl-track-label">
                          <span>{track.label}</span>
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
                    ))}
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

      {error && <div className="tl-error">{error}</div>}

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

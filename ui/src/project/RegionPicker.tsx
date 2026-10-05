import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";

import NumberField from "../NumberField";
import { useT } from "../i18n";
import { api } from "../ipc";
import { parseBasemap, type BasemapLod } from "../map/format";
import type { RegionRequest } from "../generated/RegionRequest";
import {
  ANTARCTIC,
  ARCTIC,
  dragRegion,
  followCentre,
  lonToPickerX,
  moveRegion,
  normalizeLon,
  pickerToLonLat,
  regionContains,
  regionFromView,
  regionSpan,
  snapEdges,
  type PickPoint,
} from "./regionPick";

/**
 * The coast the picker draws: the basemap's coarsest level, fetched once for
 * every picker. A picker with no backend (a test) draws an empty ocean.
 */
let coast: Promise<BasemapLod | null> | null = null;
function loadCoast(): Promise<BasemapLod | null> {
  coast ??= api
    .basemap()
    .then((bytes) => parseBasemap(bytes).lods[0] ?? null)
    .catch(() => null);
  return coast;
}

type Drag =
  | { kind: "draw"; anchor: PickPoint }
  | { kind: "move"; start: PickPoint; region: RegionRequest }
  | { kind: "pan"; x: number; centre: number };

/**
 * Chooses the part of the earth a regional project covers (spec.md 4.2,
 * decision R9): a small equirectangular map to drag a box on, and the four
 * edges as numbers, which are the source of truth — the map only writes them.
 *
 * Every change leaves as `snapEdges` makes it, so the fields show the lattice
 * Rust will create; Rust snaps again on create and is the authority.
 */
export default function RegionPicker({
  value,
  resolution,
  onChange,
  viewBounds,
}: {
  value: RegionRequest;
  /** The project's resolution, `"0.25"` and so on, which the edges snap to. */
  resolution: string;
  onChange: (region: RegionRequest) => void;
  /** The open map's view, for *Use current view*; absent on the start screen. */
  viewBounds?: (() => [number, number, number, number] | null) | undefined;
}) {
  const t = useT();
  const res = Number(resolution);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [lod, setLod] = useState<BasemapLod | null>(null);
  // Unwrapped: a canvas turned past 180° keeps counting, so a drag's
  // direction stays readable from its longitudes.
  const [centre, setCentre] = useState(() => value.west + regionSpan(value) / 2);
  const [width, setWidth] = useState(0);
  const drag = useRef<Drag | null>(null);
  const centreRef = useRef(centre);
  centreRef.current = centre;
  const valueRef = useRef(value);
  valueRef.current = value;
  const last = useRef<{ x: number; y: number } | null>(null);
  const follow = useRef<number | null>(null);

  const toCell = (p: PickPoint): PickPoint => ({
    lon: Math.round(p.lon / res) * res,
    lat: Math.round(p.lat / res) * res,
  });
  const send = (region: RegionRequest) => onChange(snapEdges(region, res));

  useEffect(() => {
    let live = true;
    void loadCoast().then((loaded) => {
      if (live) setLod(loaded);
    });
    return () => {
      live = false;
    };
  }, []);

  // The canvas is as wide as the form, and half as tall.
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => setWidth(canvas.clientWidth));
    observer.observe(canvas);
    setWidth(canvas.clientWidth);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || width <= 0) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const height = canvas.clientHeight || width / 2;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(width * dpr);
    canvas.height = Math.round(height * dpr);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    const style = getComputedStyle(canvas);
    const colour = (name: string, fallback: string) => style.getPropertyValue(name).trim() || fallback;

    ctx.fillStyle = colour("--inset", "#25323c");
    ctx.fillRect(0, 0, width, height);

    // A graticule every 30°, for reading the box without the fields.
    ctx.strokeStyle = colour("--border-subtle", "#3a5460");
    ctx.lineWidth = 1;
    ctx.beginPath();
    for (let lon = -180; lon < 180; lon += 30) {
      const x = Math.round(lonToPickerX(lon, centre, width)) + 0.5;
      ctx.moveTo(x, 0);
      ctx.lineTo(x, height);
    }
    for (let lat = -60; lat <= 60; lat += 30) {
      const y = Math.round(((90 - lat) / 180) * height) + 0.5;
      ctx.moveTo(0, y);
      ctx.lineTo(width, y);
    }
    ctx.stroke();

    if (lod) {
      const v = lod.lineVertices;
      const idx = lod.lineIndices;
      ctx.strokeStyle = colour("--muted", "#bfdec4");
      ctx.globalAlpha = 0.7;
      ctx.beginPath();
      for (let k = 0; k + 1 < idx.length; k += 2) {
        const a = idx[k]! * 2;
        const b = idx[k + 1]! * 2;
        const xa = lonToPickerX(v[a]!, centre, width);
        const xb = lonToPickerX(v[b]!, centre, width);
        // A segment that wraps round the canvas's seam is not drawn across it.
        if (Math.abs(xa - xb) > width / 2) continue;
        ctx.moveTo(xa, ((90 - v[a + 1]!) / 180) * height);
        ctx.lineTo(xb, ((90 - v[b + 1]!) / 180) * height);
      }
      ctx.stroke();
      ctx.globalAlpha = 1;
    }

    // The box: one rectangle, or two where it runs off one side and back on
    // the other.
    const top = ((90 - value.north) / 180) * height;
    const bottom = ((90 - value.south) / 180) * height;
    const rects: Array<[number, number]> = [];
    if (value.full_circle) rects.push([0, width]);
    else {
      const x0 = lonToPickerX(value.west, centre, width);
      const spanPx = (regionSpan(value) / 360) * width;
      if (x0 + spanPx <= width) rects.push([x0, spanPx]);
      else rects.push([x0, width - x0], [0, x0 + spanPx - width]);
    }
    const accent = colour("--accent", "#87bba2");
    for (const [x, w] of rects) {
      ctx.fillStyle = accent;
      ctx.globalAlpha = 0.25;
      ctx.fillRect(x, top, w, bottom - top);
      ctx.globalAlpha = 1;
      ctx.strokeStyle = accent;
      ctx.lineWidth = 2;
      ctx.strokeRect(x + 1, top + 1, Math.max(0, w - 2), Math.max(0, bottom - top - 2));
    }
  }, [lod, value, centre, width]);

  const pointAt = (x: number, y: number, centreLon: number): PickPoint => {
    const canvas = canvasRef.current!;
    return pickerToLonLat(x, y, centreLon, canvas.clientWidth, canvas.clientHeight);
  };

  /** One drag report, from the pointer or from the timer that turns the canvas. */
  const dragTo = (x: number, y: number) => {
    const state = drag.current;
    const canvas = canvasRef.current;
    if (!state || !canvas) return;
    const w = canvas.clientWidth;
    if (state.kind === "pan") {
      setCentre(state.centre - ((x - state.x) / w) * 360);
      return;
    }
    const turned = followCentre(centreRef.current, x, w);
    if (turned !== centreRef.current) {
      centreRef.current = turned;
      setCentre(turned);
    }
    const here = pointAt(x, y, turned);
    if (state.kind === "draw") {
      // A pointer is a cell or so wide: the nearest node, not the next one out.
      const box = dragRegion(state.anchor, toCell(here));
      if (box.north - box.south < res || (!box.full_circle && regionSpan(box) < res)) return;
      send(box);
    } else {
      // Whole cells, so moving a snapped box leaves it the size it was.
      const step = (d: number) => Math.round(d / res) * res;
      onChange(snapEdges(moveRegion(state.region, step(here.lon - state.start.lon), step(here.lat - state.start.lat)), res));
    }
  };

  const local = (event: ReactPointerEvent) => {
    // The rectangle is the border box; the drawing starts inside the border.
    const canvas = canvasRef.current!;
    const rect = canvas.getBoundingClientRect();
    return { x: event.clientX - rect.left - canvas.clientLeft, y: event.clientY - rect.top - canvas.clientTop };
  };

  const stopFollowing = () => {
    if (follow.current !== null) window.clearInterval(follow.current);
    follow.current = null;
  };

  useEffect(() => stopFollowing, []);

  const fieldLabel = (label: string, field: "north" | "south" | "west" | "east", lat: boolean) => (
    <label>
      {label}
      <NumberField
        aria-label={label}
        value={value[field]}
        min={lat ? -90 : -180}
        max={lat ? 90 : 180}
        step={res}
        commitWhileTyping={false}
        onCommit={(v) => send({ ...valueRef.current, [field]: v })}
      />
    </label>
  );

  return (
    <div className="region-picker">
      <canvas
        ref={canvasRef}
        className="region-canvas"
        data-feature="new:region-picker"
        aria-label={t("Region on the map")}
        onPointerDown={(event) => {
          if (event.button !== 0) return;
          const { x, y } = local(event);
          try {
            canvasRef.current?.setPointerCapture(event.pointerId);
          } catch {
            // A synthetic pointer has nothing to capture.
          }
          const here = pointAt(x, y, centreRef.current);
          if (event.shiftKey) drag.current = { kind: "pan", x, centre: centreRef.current };
          else if (regionContains(valueRef.current, normalizeLon(here.lon), here.lat)) {
            drag.current = { kind: "move", start: here, region: valueRef.current };
          } else drag.current = { kind: "draw", anchor: toCell(here) };
          last.current = { x, y };
          stopFollowing();
          // Holding the pointer still in a margin keeps turning the canvas.
          follow.current = window.setInterval(() => {
            if (last.current) dragTo(last.current.x, last.current.y);
          }, 50);
        }}
        onPointerMove={(event) => {
          if (!drag.current) return;
          const { x, y } = local(event);
          last.current = { x, y };
          dragTo(x, y);
        }}
        onPointerUp={() => {
          drag.current = null;
          last.current = null;
          stopFollowing();
        }}
        onPointerCancel={() => {
          drag.current = null;
          last.current = null;
          stopFollowing();
        }}
      />
      <p className="muted region-hint">
        {t("Drag to draw the region, drag inside it to move it, Shift-drag to turn the map.")}
      </p>
      <div className="region-edges" data-feature="new:region-edges">
        {fieldLabel(t("North"), "north", true)}
        {fieldLabel(t("South"), "south", true)}
        {!value.full_circle && fieldLabel(t("West"), "west", false)}
        {!value.full_circle && fieldLabel(t("East"), "east", false)}
      </div>
      <div className="region-actions">
        <label className="region-check" data-feature="new:region-full-circle">
          <input
            type="checkbox"
            aria-label={t("Full circle")}
            checked={value.full_circle}
            onChange={(event) => {
              const current = valueRef.current;
              if (event.target.checked) send({ ...current, full_circle: true });
              else {
                // Leaving a full circle needs an east edge that is not the
                // west one again: a quarter of the way round.
                const east = regionSpan({ ...current, full_circle: false }) === 0
                  ? normalizeLon(current.west + 90)
                  : current.east;
                send({ ...current, east, full_circle: false });
              }
            }}
          />
          {t("Full circle")}
        </label>
        <button type="button" data-feature="new:region-arctic" onClick={() => send(ARCTIC)}>
          {t("Arctic")}
        </button>
        <button type="button" data-feature="new:region-antarctic" onClick={() => send(ANTARCTIC)}>
          {t("Antarctic")}
        </button>
        {viewBounds && (
          <button
            type="button"
            data-feature="new:region-view"
            onClick={() => {
              const bounds = viewBounds();
              if (bounds) {
                const region = regionFromView(bounds);
                setCentre(region.full_circle ? 0 : region.west + regionSpan(region) / 2);
                send(region);
              }
            }}
          >
            {t("Use current view")}
          </button>
        )}
      </div>
    </div>
  );
}

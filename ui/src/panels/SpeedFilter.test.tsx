// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { GribLayerInfo } from "../generated/GribLayerInfo";
import { mpsFromKnots } from "../project/format";
import SpeedFilter from "./SpeedFilter";

// React only records updates as acted upon when told it is in a test.
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/**
 * A band as the document hands it back: the backend stores f32 metres per
 * second, so what the panel reads is `Math.fround` of what it sent — 7 kt
 * comes back as 7.00000017 kt. A fixture built from the exact f64 would pass
 * comparisons the real app never does.
 */
const f32Mps = (knots: number) => Math.fround(mpsFromKnots(knots));

/** A loaded wind layer filtered to `[lowKt, highKt]`, with a 30 kt ceiling. */
function grib(lowKt: number, highKt: number): GribLayerInfo {
  return {
    path: "/forecast.grib2",
    field_kind: "wind",
    loaded: true,
    frame_count: 1,
    span_hours: 0,
    speed_min_mps: f32Mps(lowKt),
    speed_max_mps: f32Mps(highKt),
    speed_ceiling_mps: f32Mps(30),
    covered_steps: [true],
    steps: [],
  };
}

/** The pending write's controls: answer it from the test with a revision, or refuse it. */
type Write = { accept: (revision: number) => void; refuse: () => void };

let container: HTMLDivElement;
let root: Root;
let writes: Array<{ min: number | null; max: number | null }>;
let inflight: Write[];

const onChange = (min: number | null, max: number | null) => {
  writes.push({ min, max });
  return new Promise<number | null>((resolve) => {
    inflight.push({ accept: resolve, refuse: () => resolve(null) });
  });
};

/** Renders the panel's view of the document: its band, at the revision the tree was read. */
async function render(info: GribLayerInfo, treeRevision: number) {
  await act(async () =>
    root.render(<SpeedFilter grib={info} treeRevision={treeRevision} onChange={onChange} />),
  );
}

/** The two sliders, slowest end first. */
function sliders(): [HTMLInputElement, HTMLInputElement] {
  const [low, high] = Array.from(
    container.querySelectorAll<HTMLInputElement>('input[type="range"]'),
  );
  if (low === undefined || high === undefined) throw new Error("expected two sliders");
  return [low, high];
}

/** What the browser does when the thumb is dragged to `value`: sets it, fires `input`. */
async function dragTo(slider: HTMLInputElement, value: number) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  await act(async () => {
    setter?.call(slider, String(value));
    slider.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

async function release(slider: HTMLInputElement) {
  await act(async () => {
    slider.dispatchEvent(new Event("pointerup", { bubbles: true }));
  });
}

/** The `n`th band written to the document, read back in knots. */
function writtenKnots(n: number): [number, number] {
  const write = writes[n];
  if (write === undefined) throw new Error(`no write ${n}`);
  return [(write.min ?? NaN) / mpsFromKnots(1), (write.max ?? NaN) / mpsFromKnots(1)];
}

/** The backend answers the `n`th write: the document is now at `revision`. */
async function accept(n: number, revision: number) {
  const write = inflight[n];
  if (write === undefined) throw new Error(`no write ${n} in flight`);
  await act(async () => write.accept(revision));
}

async function refuse(n: number) {
  const write = inflight[n];
  if (write === undefined) throw new Error(`no write ${n} in flight`);
  await act(async () => write.refuse());
}

const shows = (slider: HTMLInputElement, knots: number) =>
  expect(Number(slider.value)).toBeCloseTo(knots, 6);

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  writes = [];
  inflight = [];
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

describe("the two thumbs", () => {
  it("the fast end stops at the slow end instead of pushing it", async () => {
    await render(grib(5, 10), 1);
    const [low, high] = sliders();
    await dragTo(high, 3);
    shows(low, 5);
    shows(high, 5);
  });

  it("the slow end stops at the fast end instead of pushing it", async () => {
    await render(grib(5, 10), 1);
    const [low, high] = sliders();
    await dragTo(low, 14);
    shows(high, 10);
    shows(low, 10);
  });

  it("a clamped release writes the band the thumbs show", async () => {
    await render(grib(5, 10), 1);
    const [, high] = sliders();
    await dragTo(high, 3);
    await release(high);
    expect(writes).toHaveLength(1);
    const [min, max] = writtenKnots(0);
    expect(min).toBeCloseTo(5, 6);
    expect(max).toBeCloseTo(5, 6);
  });

  it("shows the document's f32 band as whole knots", async () => {
    await render(grib(7, 23), 1);
    const [low, high] = sliders();
    expect(low.value).toBe("7");
    expect(high.value).toBe("23");
  });
});

describe("the release", () => {
  it("writes nothing while the thumb is down and once when it comes up", async () => {
    await render(grib(5, 10), 1);
    const [low] = sliders();
    await dragTo(low, 6);
    await dragTo(low, 7);
    expect(writes).toHaveLength(0);
    await release(low);
    expect(writes).toHaveLength(1);
    const [min, max] = writtenKnots(0);
    expect(min).toBeCloseTo(7, 6);
    expect(max).toBeCloseTo(10, 6);
  });

  it("holds the released value through both round trips", async () => {
    await render(grib(5, 10), 1);
    const [low] = sliders();
    await dragTo(low, 7);
    await release(low);
    // The write is in flight; the document still says 5.
    shows(low, 7);
    // The write returns: the document is at revision 2. The panel's tree is
    // still the one read at revision 1, so the prop still says 5.
    await accept(0, 2);
    shows(low, 7);
    // Something else re-renders the panel before the tree arrives.
    await render(grib(5, 10), 1);
    shows(low, 7);
    // The tree fetched at revision 2 arrives with the new band.
    await render(grib(7, 10), 2);
    shows(low, 7);
  });

  it("goes back to the document's band when the write is refused", async () => {
    await render(grib(5, 10), 1);
    const [low] = sliders();
    await dragTo(low, 7);
    await release(low);
    shows(low, 7);
    await refuse(0);
    shows(low, 5);
  });

  it("follows the document once the tree has caught up", async () => {
    await render(grib(5, 10), 1);
    const [low] = sliders();
    await dragTo(low, 7);
    await release(low);
    await accept(0, 2);
    await render(grib(7, 10), 2);
    // An undo: the document goes back to 5 at revision 3, and the thumb with it.
    await render(grib(5, 10), 3);
    shows(low, 5);
  });

  it("a drag that ends where it began writes nothing", async () => {
    await render(grib(5, 10), 1);
    const [low] = sliders();
    await dragTo(low, 8);
    await dragTo(low, 5);
    await release(low);
    expect(writes).toHaveLength(0);
    shows(low, 5);
  });

  it("an earlier write's tree does not put away a later release", async () => {
    await render(grib(5, 10), 1);
    const [low] = sliders();
    await dragTo(low, 7);
    await release(low);
    await accept(0, 2);
    // Before the tree for revision 2 lands, a second drag is released.
    await dragTo(low, 9);
    await release(low);
    expect(writes).toHaveLength(2);
    // The first write's tree arrives: not the second's, so the thumb holds.
    await render(grib(7, 10), 2);
    shows(low, 9);
    await accept(1, 3);
    shows(low, 9);
    await render(grib(9, 10), 3);
    shows(low, 9);
  });
});

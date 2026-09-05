// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { GribLayerInfo } from "../generated/GribLayerInfo";
import { mpsFromKnots } from "../project/format";
import SpeedFilter from "./SpeedFilter";

// React only records updates as acted upon when told it is in a test.
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/** A loaded wind layer filtered to `[lowKt, highKt]`, with a 30 kt ceiling. */
function grib(lowKt: number, highKt: number): GribLayerInfo {
  return {
    path: "/forecast.grib2",
    field_kind: "wind",
    loaded: true,
    frame_count: 1,
    span_hours: 0,
    speed_min_mps: mpsFromKnots(lowKt),
    speed_max_mps: mpsFromKnots(highKt),
    speed_ceiling_mps: mpsFromKnots(30),
    covered_steps: [true],
    steps: [],
  };
}

/** The pending write's controls: resolve or reject it from the test. */
type Write = { resolve: () => void; reject: () => void };

let container: HTMLDivElement;
let root: Root;
let writes: Array<{ min: number | null; max: number | null }>;
let inflight: Write[];

const onChange = (min: number | null, max: number | null) => {
  writes.push({ min, max });
  return new Promise<void>((resolve, reject) => {
    inflight.push({ resolve, reject: () => reject(new Error("refused")) });
  });
};

async function render(info: GribLayerInfo) {
  await act(async () => root.render(<SpeedFilter grib={info} onChange={onChange} />));
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

/** Settles the `n`th write, the way the backend's answer would. */
async function settle(n: number, how: keyof Write) {
  const write = inflight[n];
  if (write === undefined) throw new Error(`no write ${n} in flight`);
  await act(async () => write[how]());
}

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
    await render(grib(5, 10));
    const [low, high] = sliders();
    await dragTo(high, 3);
    expect(Number(low.value)).toBeCloseTo(5, 6);
    expect(Number(high.value)).toBeCloseTo(5, 6);
  });

  it("the slow end stops at the fast end instead of pushing it", async () => {
    await render(grib(5, 10));
    const [low, high] = sliders();
    await dragTo(low, 14);
    expect(Number(high.value)).toBeCloseTo(10, 6);
    expect(Number(low.value)).toBeCloseTo(10, 6);
  });

  it("a clamped release writes the band the thumbs show", async () => {
    await render(grib(5, 10));
    const [, high] = sliders();
    await dragTo(high, 3);
    await release(high);
    expect(writes).toHaveLength(1);
    const [min, max] = writtenKnots(0);
    expect(min).toBeCloseTo(5, 6);
    expect(max).toBeCloseTo(5, 6);
  });
});

describe("the release", () => {
  it("writes nothing while the thumb is down and once when it comes up", async () => {
    await render(grib(5, 10));
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

  it("holds the released value until the document has it", async () => {
    await render(grib(5, 10));
    const [low] = sliders();
    await dragTo(low, 7);
    await release(low);
    // The write is still in flight and the document still says 5.
    expect(Number(low.value)).toBeCloseTo(7, 6);
    // The summary comes back with the new band, then the write settles.
    await render(grib(7, 10));
    expect(Number(low.value)).toBeCloseTo(7, 6);
    await settle(0, "resolve");
    expect(Number(low.value)).toBeCloseTo(7, 6);
  });

  it("goes back to the document's band when the write is refused", async () => {
    await render(grib(5, 10));
    const [low] = sliders();
    await dragTo(low, 7);
    await release(low);
    expect(Number(low.value)).toBeCloseTo(7, 6);
    await settle(0, "reject");
    expect(Number(low.value)).toBeCloseTo(5, 6);
  });

  it("a drag that ends where it began writes nothing", async () => {
    await render(grib(5, 10));
    const [low] = sliders();
    await dragTo(low, 8);
    await dragTo(low, 5);
    await release(low);
    expect(writes).toHaveLength(0);
    expect(Number(low.value)).toBeCloseTo(5, 6);
  });

  it("a second release settling first does not put away the later one", async () => {
    await render(grib(5, 10));
    const [low] = sliders();
    await dragTo(low, 7);
    await release(low);
    await dragTo(low, 9);
    await release(low);
    expect(writes).toHaveLength(2);
    await settle(0, "resolve");
    expect(Number(low.value)).toBeCloseTo(9, 6);
  });
});


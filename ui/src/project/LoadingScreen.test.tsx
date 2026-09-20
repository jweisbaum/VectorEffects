// @vitest-environment happy-dom
/**
 * The loading page: nothing unless something is opening, then a real bar fed
 * by the backend's events.
 */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenProgress } from "../generated/OpenProgress";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const bus = vi.hoisted(() => ({
  handlers: new Map<string, (event: { payload: unknown }) => void>(),
  unlistened: [] as string[],
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: (name: string, handler: (event: { payload: unknown }) => void) => {
    bus.handlers.set(name, handler);
    return Promise.resolve(() => {
      bus.unlistened.push(name);
    });
  },
}));

const LoadingScreen = (await import("./LoadingScreen")).default;
const { beginOpening, finishOpening } = await import("./opening");

let container: HTMLDivElement;
let root: Root;

beforeEach(async () => {
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  await act(async () => root.render(<LoadingScreen />));
});

afterEach(async () => {
  await act(async () => finishOpening());
  await act(async () => root.unmount());
  container.remove();
  bus.handlers.clear();
});

const emit = (payload: OpenProgress) =>
  act(async () => bus.handlers.get("open://progress")?.({ payload }));

const bar = () => container.querySelector('[role="progressbar"]');

describe("the loading page", () => {
  it("is not there until something is opening", () => {
    expect(container.innerHTML).toBe("");
  });

  it("listens before anything opens, so the first report is not missed", () => {
    expect(bus.handlers.has("open://progress")).toBe(true);
  });

  it("names what is opening and moves its bar with the backend's reports", async () => {
    let end: (opened: boolean) => void = () => undefined;
    await act(async () => {
      end = beginOpening("atlantic.veproj");
    });
    expect(container.textContent).toContain("atlantic.veproj");
    expect(bar()?.getAttribute("aria-valuenow")).toBe("0");

    await emit({ label: "routing_test", done: 744, total: 1488, fraction: 0.525 });
    expect(bar()?.getAttribute("aria-valuenow")).toBe("50");
    expect(container.textContent).toContain("routing_test");
    expect(container.textContent).toContain("744");

    await act(async () => end(true));
    expect(container.textContent).toContain("Drawing the map");
    expect(bar()?.getAttribute("aria-valuenow")).toBe("97");

    await act(async () => finishOpening());
    expect(container.innerHTML).toBe("");
  });

  it("goes when the opening is refused", async () => {
    await act(async () => beginOpening("atlantic.veproj")(false));
    expect(container.innerHTML).toBe("");
  });
});

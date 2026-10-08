// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { HistoryProgress } from "./generated/HistoryProgress";
import { beginBusy } from "./busy";
import { msg } from "./i18n";

const events = vi.hoisted(() => new Map<string, (e: { payload: HistoryProgress }) => void>());
vi.mock("@tauri-apps/api/event", () => ({ listen: (name: string, callback: (e: { payload: HistoryProgress }) => void) => {
  events.set(name, callback); return Promise.resolve(() => events.delete(name));
} }));
const FetchProgressBar = (await import("./FetchProgressBar")).default;
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root;
let container: HTMLDivElement;
let finish: () => void;
beforeEach(async () => {
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
  finish = beginBusy("history");
  await act(async () => root.render(<FetchProgressBar busyLabel="history" event="history://progress" waiting={msg("Opening the archives")} />));
});
afterEach(async () => { await act(async () => { root.unmount(); finish(); }); container.remove(); events.clear(); });
const bar = () => container.querySelector('[role="progressbar"]');
async function report(progress: HistoryProgress) { await act(async () => events.get("history://progress")!({ payload: progress })); }

it("shows download work before a single completed hour, and waits for layer import before 100%", async () => {
  expect(bar()?.hasAttribute("aria-valuenow")).toBe(false);
  await report({ archive: "R2", done: 0, total: 48, work: { phase: "downloading", fraction: 0.35, downloaded_bytes: 12_500_000 } });
  expect(bar()?.getAttribute("aria-valuenow")).toBe("35");
  expect(container.textContent).toContain("Downloading history");
  expect(container.textContent).toContain("12.5 MB");
  await report({ archive: "R2", done: 48, total: 48, work: { phase: "importing", fraction: 0.97, downloaded_bytes: 50_000_000 } });
  expect(bar()?.getAttribute("aria-valuenow")).toBe("97");
  expect(container.textContent).toContain("Adding history layers");
});

it("preserves step progress for Open Data/NRT and clears stale events between imports", async () => {
  await report({ archive: "ERA5", done: 3, total: 4, work: null });
  expect(bar()?.getAttribute("aria-valuenow")).toBe("75");
  expect(container.textContent).toContain("ERA5 · 3/4");
  await act(async () => finish());
  expect(bar()).toBeNull();
  await act(async () => { finish = beginBusy("history"); });
  expect(bar()?.hasAttribute("aria-valuenow")).toBe(false);
});

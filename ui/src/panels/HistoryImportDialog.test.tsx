// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { HistoryArchives } from "../generated/HistoryArchives";
import type { ProjectSummary } from "../generated/ProjectSummary";
import { availableRange } from "./historyRange";

const fetchArchives = vi.hoisted(() => vi.fn());
vi.mock("../ipc", () => ({ api: { historyArchives: fetchArchives } }));
const HistoryImportDialog = (await import("./HistoryImportDialog")).default;
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const catalogue = (): HistoryArchives => ({ now: "2026-10-08T12:00Z", archives: [
  { field: "wind", label: "Wind source (R2)", description: "", first: "2000-01-01T00:00Z", last: "2026-05-01T23:00Z", unreachable: null },
  { field: "current", label: "Current source (R2)", description: "", first: "2001-02-03T06:00Z", last: "2025-04-05T12:00Z", unreachable: null },
] });
const seconds = (text: string) => Date.parse(text) / 1000;
let root: Root;
let container: HTMLDivElement;
const imported = vi.fn();

beforeEach(() => {
  fetchArchives.mockReset().mockResolvedValue(catalogue());
  imported.mockReset();
  container = document.createElement("div"); document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); });
async function open(start = "2020-01-01T00:00Z") {
  await act(async () => root.render(<HistoryImportDialog
    project={{ start_unix_s: seconds(start), step_hours: 1, step_count: 24 } as ProjectSummary}
    now={seconds("2026-10-08T00:00Z")} onImport={imported} onClose={() => {}} />));
}
const button = () => document.querySelector<HTMLButtonElement>('[data-feature="history-import:import"]')!;

it("blocks while availability is unknown, then enables a verified range", async () => {
  let resolve!: (value: HistoryArchives) => void;
  fetchArchives.mockReturnValue(new Promise<HistoryArchives>((r) => { resolve = r; }));
  await open();
  expect(button().disabled).toBe(true);
  expect(document.body.textContent).toContain("Checking available archive dates");
  await act(async () => { resolve(catalogue()); });
  expect(button().disabled).toBe(false);
  expect(document.querySelector('input[type="datetime-local"]')?.getAttribute("min")).toBe("2001-02-03T06:00");
  expect(document.querySelector('input[type="datetime-local"]')?.getAttribute("max")).toBe("2025-04-05T12:00");
  await act(async () => button().click());
  expect(imported).toHaveBeenCalledOnce();
});

it("names the limiting source and both exact dates, and cannot import outside them", async () => {
  await open("2025-04-06T00:00Z");
  expect(button().disabled).toBe(true);
  const alert = document.querySelector('[role="alert"]')?.textContent;
  expect(alert).toContain("Current source (R2)");
  expect(alert).toContain("2001-02-03 06:00");
  expect(alert).toContain("2025-04-05 12:00");
  await act(async () => button().click());
  expect(imported).not.toHaveBeenCalled();
  const current = document.querySelectorAll<HTMLInputElement>('[data-feature="history-import:archives"] input')[1]!;
  await act(async () => current.click());
  expect(button().disabled).toBe(false);
});

it("blocks a failed lookup instead of guessing dates", async () => {
  fetchArchives.mockRejectedValue(new Error("offline"));
  await open();
  expect(button().disabled).toBe(true);
  expect(document.querySelector('[role="alert"]')?.textContent).toContain("offline");
});

it("refreshes coverage for a new dialog/source and rejects null coverage", async () => {
  await open();
  expect(button().disabled).toBe(false);
  await act(async () => root.render(null));
  const changed = catalogue();
  changed.archives[0]!.first = null;
  changed.archives[0]!.last = null;
  changed.archives[0]!.label = "S3";
  changed.archives[0]!.unreachable = "403 Forbidden";
  fetchArchives.mockResolvedValue(changed);
  await open();
  expect(fetchArchives).toHaveBeenCalledTimes(2);
  expect(button().disabled).toBe(true);
  expect(document.body.textContent).toContain("S3");
  expect(document.body.textContent).toContain("403 Forbidden");
});

it("uses inclusive live boundaries and refuses an earlier start or later end", () => {
  const data = catalogue();
  const first = seconds(data.archives[1]!.first!);
  const last = seconds(data.archives[1]!.last!);
  const both = ["era5-wind", "globcurrent"];
  expect(availableRange(data, both, first, last).problem).toBeNull();
  expect(availableRange(data, both, first - 3600, last).problem).toContain("2001-02-03 06:00");
  expect(availableRange(data, both, first, last + 3600).problem).toContain("2025-04-05 12:00");
  data.archives[1]!.last = "2026-08-01T00:00Z";
  expect(availableRange(data, ["globcurrent"], first, last + 3600).problem).toBeNull();
});

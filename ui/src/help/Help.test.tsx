// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import Help from "./Help";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const events = vi.hoisted(() => ({ open: () => {}, off: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async (_name, callback) => {
  events.open = callback;
  return events.off;
}) }));

it("opens from the native menu, shows bundled help, and restores the editor on Escape", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<Help><button>Editor action</button></Help>));
    expect(host.querySelector('[role="dialog"]')).toBeNull();
    await act(async () => events.open());
    expect(host.querySelector('[role="dialog"]')?.textContent).toContain("Get started");
    expect(host.querySelector("img")?.getAttribute("src")).toBe("/help/workspace.png");
    expect(host.firstElementChild?.hasAttribute("inert")).toBe(true);
    const motion = [...host.querySelectorAll("nav button")].find(b => b.textContent === "Keyframes and motion");
    await act(async () => (motion as HTMLButtonElement).click());
    expect(host.querySelector("article")?.textContent).toContain("next position key");
    expect(host.querySelector("img")?.getAttribute("src")).toBe("/help/motion.png");
    await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })));
    expect(host.querySelector('[role="dialog"]')).toBeNull();
    expect(host.firstElementChild?.hasAttribute("inert")).toBe(false);
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
  expect(events.off).toHaveBeenCalledOnce();
});

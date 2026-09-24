// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import ProjectMenu from "./ProjectMenu";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
let host: HTMLDivElement;
let root: Root;
let actions: ReturnType<typeof vi.fn>[];
const trigger = () => host.querySelector<HTMLButtonElement>('[aria-haspopup="menu"]')!;
const items = () => [...host.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')];
const press = async (key: string) => act(async () => {
  document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
});

beforeEach(async () => {
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  actions = Array.from({ length: 5 }, () => vi.fn());
  await act(async () => root.render(<><ProjectMenu onNew={actions[0]!} onOpen={actions[1]!}
    onSave={actions[2]!} onSaveAs={actions[3]!} onClose={actions[4]!} /><button>Outside</button></>));
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });

it.each(["New…", "Open…", "Save", "Save As…", "Close"])("runs %s once and closes the menu", async label => {
  expect(items()).toHaveLength(0);
  await act(async () => trigger().click());
  expect(items().map(item => item.textContent)).toEqual(["New…", "Open…", "Save", "Save As…", "Close"]);
  const index = items().findIndex(item => item.textContent === label);
  await act(async () => items()[index]!.click());
  expect(actions.map(action => action.mock.calls.length)).toEqual(actions.map((_, i) => i === index ? 1 : 0));
  expect(items()).toHaveLength(0);
  expect(trigger().getAttribute("aria-expanded")).toBe("false");
  expect(document.activeElement).toBe(trigger());
});

it("navigates by arrows, Home and End, and Escape restores focus without selecting", async () => {
  trigger().focus(); await press("ArrowUp");
  expect(document.activeElement?.textContent).toBe("Close");
  await press("ArrowDown"); expect(document.activeElement?.textContent).toBe("New…");
  await press("End"); expect(document.activeElement?.textContent).toBe("Close");
  await press("Home"); expect(document.activeElement?.textContent).toBe("New…");
  await press("ArrowDown"); expect(document.activeElement?.textContent).toBe("Open…");
  await press("Escape");
  expect(items()).toHaveLength(0);
  expect(document.activeElement).toBe(trigger());
  expect(actions.every(action => action.mock.calls.length === 0)).toBe(true);
});

it("dismisses on outside clicks, outside focus, and Tab", async () => {
  await act(async () => trigger().click());
  await act(async () => document.body.dispatchEvent(new Event("pointerdown", { bubbles: true })));
  expect(items()).toHaveLength(0);
  await act(async () => trigger().click());
  await act(async () => host.querySelector<HTMLButtonElement>(":scope > button")!.focus());
  expect(items()).toHaveLength(0);
  await act(async () => trigger().click());
  await press("Tab");
  expect(items()).toHaveLength(0);
  expect(document.activeElement).toBe(trigger());
});

// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { IS_MAC } from "../chords";
import { setLanguage } from "../i18n";
import HelpMenu, { isSearchChord } from "./HelpMenu";
import { onReveal } from "./highlight";
import { onOpenHelp } from "./open";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
let host: HTMLDivElement;
let root: Root;
const input = () => host.querySelector<HTMLInputElement>('input[type="search"]');
const options = () => [...host.querySelectorAll<HTMLLIElement>('[role="option"]')];
const labels = () => options().map(o => o.querySelector(".help-search-label")?.textContent);
const type = async (text: string) => act(async () => {
  const field = input()!;
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(field, text);
  field.dispatchEvent(new Event("input", { bubbles: true }));
});
const key = async (target: EventTarget, init: KeyboardEventInit) => act(async () => {
  target.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
});
const settle = () => act(() => new Promise(resolve => setTimeout(resolve, 120)));

beforeEach(async () => {
  setLanguage("en");
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await act(async () => root.render(<HelpMenu />));
});
afterEach(async () => {
  await act(async () => root.unmount()); host.remove();
  document.querySelectorAll(".feature-flash, [data-feature]").forEach(e => e.remove());
  setLanguage("en");
  vi.restoreAllMocks();
});

it("reads Cmd-F on a Mac and Ctrl-F elsewhere, and nothing else", () => {
  const chord = (init: Partial<KeyboardEvent>) => ({ key: "f", metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...init });
  expect(isSearchChord(chord({ metaKey: true }), true)).toBe(true);
  expect(isSearchChord(chord({ ctrlKey: true }), true)).toBe(false);
  expect(isSearchChord(chord({ ctrlKey: true }), false)).toBe(true);
  expect(isSearchChord(chord({ metaKey: true, shiftKey: true }), true)).toBe(false);
  expect(isSearchChord(chord({}), true)).toBe(false);
  expect(isSearchChord(chord({ key: "F", metaKey: true }), true)).toBe(true);
});

it("opens on the chord with the search focused, and closes on Escape", async () => {
  expect(input()).toBeNull();
  await key(window, { key: "f", metaKey: true, ctrlKey: true });
  expect(input()).toBeNull();
  await key(window, { key: "f", metaKey: IS_MAC, ctrlKey: !IS_MAC });
  await settle();
  expect(input()).not.toBeNull();
  expect(document.activeElement).toBe(input());
  await key(input()!, { key: "Escape" });
  expect(input()).toBeNull();
});

it("lists features as the person types, in the language on screen", async () => {
  const picker = document.createElement("select");
  picker.dataset.feature = "shell:language";
  document.body.append(picker);
  await act(async () => host.querySelector("button")!.click());
  await type("langu");
  expect(labels()[0]).toBe("Language");
  await act(async () => setLanguage("de"));
  await type("sprache");
  expect(labels()[0]).toBe("Sprache");
  await type("zzzz-nothing");
  expect(host.querySelector(".help-search-empty")?.textContent).toContain("zzzz-nothing");
  // The reference is always offered, last.
  expect(labels().at(-1)).toBe("Hilfereferenz öffnen");
});

it("leaves out a feature that nothing on this screen can show", async () => {
  const { FEATURES } = await import("./features");
  const feature = { id: "shell:zqxnowhere", label: "Zqxnowhere" };
  FEATURES.unshift(feature);
  try {
    await act(async () => host.querySelector("button")!.click());
    await type("zqxnowhere");
    expect(labels()).not.toContain("Zqxnowhere");
    const element = document.createElement("button");
    element.dataset.feature = feature.id;
    document.body.append(element);
    await type("zqxnowher");
    expect(labels()).toContain("Zqxnowhere");
  } finally {
    FEATURES.splice(FEATURES.indexOf(feature), 1);
  }
});

it("reveals the chosen feature and flashes a rectangle around it", async () => {
  const target = document.createElement("select");
  target.dataset.feature = "shell:language";
  target.hidden = true;
  document.body.append(target);
  vi.spyOn(target, "getBoundingClientRect").mockImplementation(() =>
    (target.hidden ? new DOMRect(0, 0, 0, 0) : new DOMRect(40, 10, 90, 24)));
  const revealed: string[] = [];
  // Stands in for a component that owns the hiding state.
  const off = onReveal("panel:", step => { revealed.push(step); target.hidden = false; });
  const { FEATURES } = await import("./features");
  // A feature of the test's own, so the query has one best match.
  const feature = { id: "shell:language", label: "Zqxlanguage", reveal: ["panel:left"] };
  FEATURES.unshift(feature);
  try {
    await act(async () => host.querySelector("button")!.click());
    await type("zqxlang");
    await key(input()!, { key: "Enter" });
    await settle();
    expect(revealed).toEqual(["panel:left"]);
    const box = document.querySelector<HTMLElement>(".feature-flash");
    expect(box).not.toBeNull();
    expect([box!.style.left, box!.style.top, box!.style.width, box!.style.height]).toEqual(["36px", "6px", "98px", "32px"]);
    expect(input()).toBeNull();
  } finally {
    FEATURES.splice(FEATURES.indexOf(feature), 1);
    off();
  }
});

it("opens a help page, and the reference, from the results", async () => {
  const opened: (string | undefined)[] = [];
  const off = onOpenHelp(topic => opened.push(topic));
  await act(async () => host.querySelector("button")!.click());
  await type("feather");
  const page = options().findIndex(o => o.querySelector(".help-search-label")?.textContent === "Tool basics: size, feather, and edges");
  expect(page).toBeGreaterThanOrEqual(0);
  await act(async () => options()[page]!.click());
  await act(async () => host.querySelector("button")!.click());
  await act(async () => options().at(-1)!.click());
  expect(opened).toEqual(["tools", undefined]);
  off();
});

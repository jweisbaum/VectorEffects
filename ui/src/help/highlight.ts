/**
 * Showing a person where a feature is (spec.md 5.8): bring it on screen,
 * then flash a rectangle around it.
 *
 * Bringing it on screen belongs to whoever owns the state that hides it — the
 * map selects a tool, the shell opens a panel, Settings opens itself — so this
 * module only keeps a table of those handlers (`onReveal`) and runs a
 * feature's steps in order. Then it waits for the element to be laid out,
 * because the handler's state change renders on a later frame.
 */

import type { Feature } from "./features";

type Reveal = (step: string) => void | Promise<void>;

const handlers = new Map<string, Reveal>();

/**
 * Handles a reveal step: an exact step (`"panel:left"`) or every step with a
 * prefix (`"tool:"`). Returns the unregistration, for an effect's cleanup.
 */
export function onReveal(step: string, handler: Reveal): () => void {
  handlers.set(step, handler);
  return () => { if (handlers.get(step) === handler) handlers.delete(step); };
}

function handlerFor(step: string): Reveal | undefined {
  const exact = handlers.get(step);
  if (exact) return exact;
  const colon = step.indexOf(":");
  return colon >= 0 ? handlers.get(step.slice(0, colon + 1)) : undefined;
}

const frame = () => new Promise<void>(resolve => {
  // A frame, or a moment when frames are suspended (WKWebView occluded).
  let done = false;
  const finish = () => { if (!done) { done = true; resolve(); } };
  requestAnimationFrame(finish);
  setTimeout(finish, 50);
});

/** The element for a feature, if it is laid out and visible. */
export function elementFor(id: string): HTMLElement | null {
  for (const element of document.querySelectorAll<HTMLElement>(`[data-feature="${CSS.escape(id)}"]`)) {
    const rect = element.getBoundingClientRect();
    if (rect.width > 0 && rect.height > 0) return element;
  }
  return null;
}

/**
 * Reveals a feature and flashes it. Resolves false when it could not be
 * found on screen, so the caller can fall back to its help page.
 */
export async function locateFeature(feature: Feature): Promise<boolean> {
  for (const step of feature.reveal ?? []) {
    await handlerFor(step)?.(step);
    await frame();
  }
  for (let attempt = 0; attempt < 20; attempt += 1) {
    const element = elementFor(feature.id);
    if (element) {
      element.scrollIntoView({ block: "nearest", inline: "nearest" });
      flash(element);
      return true;
    }
    await frame();
  }
  return false;
}

let current: (() => void) | null = null;

/** How long the rectangle flashes for, in ms. Three pulses. */
export const FLASH_MS = 2400;

/**
 * Draws a flashing rectangle around an element, following it if it moves,
 * and removes it after `FLASH_MS` or on the next press anywhere.
 */
export function flash(element: HTMLElement): void {
  current?.();
  const box = document.createElement("div");
  box.className = "feature-flash";
  box.setAttribute("aria-hidden", "true");
  document.body.appendChild(box);
  let raf = 0;
  const place = () => {
    const rect = element.getBoundingClientRect();
    const pad = 4;
    box.style.left = `${rect.left - pad}px`;
    box.style.top = `${rect.top - pad}px`;
    box.style.width = `${rect.width + pad * 2}px`;
    box.style.height = `${rect.height + pad * 2}px`;
    raf = requestAnimationFrame(place);
  };
  place();
  const stop = () => {
    cancelAnimationFrame(raf);
    clearTimeout(timer);
    box.remove();
    window.removeEventListener("pointerdown", stop, true);
    if (current === stop) current = null;
  };
  const timer = setTimeout(stop, FLASH_MS);
  window.addEventListener("pointerdown", stop, true);
  current = stop;
}

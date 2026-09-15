/**
 * Handing the keyboard back to the map after a control is used (M46).
 *
 * A tool's options sit over the map, and a control that keeps focus after it
 * has been used keeps the keyboard too: the arrows that nudge a selection go
 * to the select instead, the tool shortcuts type into it, and on WebKit the
 * click that dismisses a native menu's popup is swallowed before it reaches
 * the canvas — which is why the first map click after changing a setting did
 * nothing.
 *
 * So a control that has finished its job gives focus up. Only the ones that
 * finish in one action: a menu, a checkbox, a button. A text field is still
 * being typed into and keeps it until it is left.
 */

import { flushSync } from "react-dom";

/** Commit a pending numeric edit before the same press reaches the tool. */
export function focusMapForGesture(canvas: HTMLElement): void {
  const active = document.activeElement;
  if (active instanceof HTMLElement && active !== canvas) {
    flushSync(() => active.blur());
  }
  canvas.focus({ preventScroll: true });
}

/** Finish single-action controls after their native pointer-up handling. */
export function finishToolControl(event: { target: EventTarget | null }): void {
  const target = event.target;
  if (!(target instanceof HTMLElement)) return;
  if (!target.matches('select, button, input[type="checkbox"], input[type="range"]')) return;
  releaseFocus({ currentTarget: target });
  // WebKit can restore focus as the native popup closes after change. Let it
  // finish, then relinquish only this control, never a newly focused input.
  window.setTimeout(() => {
    if (document.activeElement === target) target.blur();
  }, 0);
}

/** Drops focus from the control an event came from. */
export function releaseFocus(event: {
  currentTarget: EventTarget | null;
}): void {
  const target = event.currentTarget;
  if (
    target &&
    "blur" in target &&
    typeof (target as HTMLElement).blur === "function"
  ) {
    (target as HTMLElement).blur();
  }
}

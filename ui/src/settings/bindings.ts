/**
 * The one bindings table (spec.md 8.6, M15).
 *
 * Before this, the timeline wired `Space` and the arrows by hand and the map
 * wired the palette letters by hand, which meant a key could be bound twice
 * with nothing to say so and a rebind was not a thing the application could
 * have. Every handler now asks this module what a key press means.
 *
 * Pure, so the rules can be tested against a table rather than by pressing
 * keys at a browser.
 */

import { IS_MAC } from "../chords";
import type { AppSettings } from "../generated/AppSettings";
import type { Shortcut } from "../generated/Shortcut";
import type { ShortcutAction } from "../generated/ShortcutAction";

/** The chord a keyboard event describes, in the table's own spelling. */
export function chordOf(event: {
  key: string;
  shiftKey: boolean;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
}): string | null {
  // Modifiers are spelled in a fixed order — `accel+alt+shift+key` — the same
  // order the backend spells them, so the two tables compare equal.
  //
  // The command key was a *disqualifier* until M47: a chord carrying one
  // belonged to the window's own menu keys and nothing here. Deselect is
  // accel-D, which is what a hand reaches for, so the table can now hold one
  // — and a chord that is *only* the accel spelling is still not a plain
  // one, so nothing that was bound before now answers to a menu key.
  const key = event.key.toLowerCase();
  const accel = event.metaKey || event.ctrlKey;
  return `${accel ? "accel+" : ""}${event.altKey ? "alt+" : ""}${event.shiftKey ? "shift+" : ""}${key}`;
}

/** The chord a binding is set to. */
export function chordFor(binding: Shortcut): string {
  return `${binding.accel ? "accel+" : ""}${binding.alt ? "alt+" : ""}${binding.shift ? "shift+" : ""}${binding.key}`;
}

/** What a chord does, or nothing. */
export function actionFor(
  settings: AppSettings,
  chord: string,
): { action: ShortcutAction; tool: string } | null {
  const found = settings.shortcuts.find((binding) => chordFor(binding) === chord);
  return found ? { action: found.action, tool: found.tool } : null;
}

/** The binding for an action, if it has one. */
export function bindingFor(
  settings: AppSettings,
  action: ShortcutAction,
  tool = "",
): Shortcut | null {
  return (
    settings.shortcuts.find((binding) => binding.action === action && binding.tool === tool) ??
    null
  );
}

/**
 * A chord as it should read in a tooltip.
 *
 * The same spelling the backend uses, so a tooltip and the settings dialog
 * cannot describe one binding two ways.
 */
export function chordLabel(binding: Shortcut | null): string {
  if (!binding) return "unbound";
  const named: Record<string, string> = {
    " ": "Space",
    arrowleft: "←",
    arrowright: "→",
    arrowup: "↑",
    arrowdown: "↓",
  };
  const key = named[binding.key] ?? binding.key.toUpperCase();
  // The command key by the symbol this machine actually shows on it.
  const accel = binding.accel ? (IS_MAC ? "\u2318" : "Ctrl-") : "";
  return `${accel}${binding.alt ? "Alt-" : ""}${binding.shift ? "Shift-" : ""}${key}`;
}

/** A tool's shortcut, for its palette button. */
export function toolChord(settings: AppSettings, tool: string): string {
  return chordLabel(bindingFor(settings, "tool", tool));
}

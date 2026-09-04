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
  // A shortcut is a bare key or a shifted one. Anything with a command or
  // control modifier belongs to the application's own menu keys, and anything
  // with alt is left alone.
  if (event.metaKey || event.ctrlKey || event.altKey) return null;
  const key = event.key.toLowerCase();
  return event.shiftKey ? `shift+${key}` : key;
}

/** The chord a binding is set to. */
export function chordFor(binding: Shortcut): string {
  return binding.shift ? `shift+${binding.key}` : binding.key;
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
  return binding.shift ? `Shift-${key}` : key;
}

/** A tool's shortcut, for its palette button. */
export function toolChord(settings: AppSettings, tool: string): string {
  return chordLabel(bindingFor(settings, "tool", tool));
}

/**
 * Key chords the app reads itself, beside the rebindable shortcuts.
 *
 * Pure, so each is a table with a test rather than a branch in a key handler.
 */

/** Whether this machine's command key is `Cmd`, leaving `Ctrl` free for chords of its own. */
export const IS_MAC = /Mac|iPhone|iPad/.test(globalThis.navigator?.platform ?? "");

/** The modifier keys of a key event, which is all a chord reads. */
export interface Modifiers {
  shiftKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
}

/**
 * The still-paste chord (spec.md 8.5, M27): `Ctrl`-`Shift`-`V` on a Mac,
 * where `Ctrl` is free, and `Ctrl`-`Alt`-`Shift`-`V` where `Ctrl` is the
 * command key and `Ctrl`-`Shift`-`V` is already the absolute-timing paste.
 * Read once the key handler knows the key is `V` and a command key is down.
 */
export function stillPasteChord(event: Modifiers, mac: boolean = IS_MAC): boolean {
  if (!event.shiftKey) return false;
  return mac ? event.ctrlKey && !event.metaKey : event.altKey;
}

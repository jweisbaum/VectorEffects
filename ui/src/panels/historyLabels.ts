/**
 * The History panel's entries in the interface language (spec.md 5.6).
 *
 * `HistoryEntry.label` is built in English by the backend — `Command::label`
 * in `ve-core`, `label_for` in `ve-app`'s transform, and the few batches
 * `ve-app` labels itself — and stays English there, since it is also what a
 * log or the MCP service reads. This turns each one the backend can produce
 * into the language on screen: exact labels through a table, and the
 * `format!` ones through patterns whose templates are the catalogue keys.
 *
 * An object's or a file's name inside a label is document data and goes back
 * in as written. A label nothing here knows falls through unchanged, so a new
 * command reads in English until it is added — never blank.
 */

import { msg, t } from "../i18n";

/** Every label the backend writes with nothing varying in it. */
const EXACT: readonly string[] = [
  msg("Add layer"),
  msg("Delete layer"),
  msg("Rename layer"),
  msg("Filter layer speeds"),
  msg("Clear the speed filter"),
  msg("Layer holds wind"),
  msg("Layer holds current"),
  msg("Show layer"),
  msg("Hide layer"),
  msg("Lock layer"),
  msg("Unlock layer"),
  msg("Restore the file's frames"),
  msg("Change pasted frames"),
  msg("Paste 1 frame"),
  msg("Reorder layers"),
  msg("Paste 1 object"),
  msg("Rename object"),
  msg("Move object"),
  msg("Follow another object"),
  msg("Stop following"),
  msg("Add motion to the field"),
  msg("Take motion out of the field"),
  msg("Change which motion reaches the field"),
  msg("Change active range"),
  msg("Animate shape"),
  msg("Edit shape"),
  msg("Erase"),
  msg("Rename project"),
  msg("Change the colour scale"),
  msg("Change the colour gradient"),
  msg("Set start time"),
  msg("Change duration"),
  msg("Change measurements"),
  msg("Place the image"),
  msg("Delete object"),
  msg("Import history"),
  // Exact, and so tried before the `Import {name}` pattern would take it as
  // a file called "near-real-time data".
  msg("Import near-real-time data"),
  msg("Transform"),
  // `label_for` in transform.rs, for a single object.
  msg("Move"),
  msg("Rotate"),
  msg("Scale"),
  msg("Move anchor"),
];

const EXACT_SET = new Set(EXACT);

/**
 * The labels with something in them, most specific first: "Delete 3 objects"
 * has to be tried before "Delete {name}" would take it as an object called
 * "3 objects". The template's `{placeholders}` are named after the pattern's
 * groups, in order.
 */
const PATTERNS: readonly { pattern: RegExp; template: string; names: string[] }[] = [
  { pattern: /^Paste (\d+) frames$/, template: msg("Paste {count} frames"), names: ["count"] },
  { pattern: /^Paste (\d+) objects$/, template: msg("Paste {count} objects"), names: ["count"] },
  { pattern: /^Delete (\d+) objects$/, template: msg("Delete {count} objects"), names: ["count"] },
  { pattern: /^Move anchor (\d+) objects$/, template: msg("Move anchor {count} objects"), names: ["count"] },
  { pattern: /^Move (\d+) objects$/, template: msg("Move {count} objects"), names: ["count"] },
  { pattern: /^Rotate (\d+) objects$/, template: msg("Rotate {count} objects"), names: ["count"] },
  { pattern: /^Scale (\d+) objects$/, template: msg("Scale {count} objects"), names: ["count"] },
  // `Change {prop:?}`: the property's identifier, which is not a word to translate.
  { pattern: /^Change ([A-Z][A-Za-z0-9]*)$/, template: msg("Change {property}"), names: ["property"] },
  { pattern: /^Import (.+)$/, template: msg("Import {name}"), names: ["name"] },
  { pattern: /^Add (.+)$/, template: msg("Add {name}"), names: ["name"] },
  { pattern: /^Delete (.+)$/, template: msg("Delete {name}"), names: ["name"] },
];

/** A history label in the interface language; an unknown one as it came. */
export function translateHistoryLabel(label: string): string {
  if (EXACT_SET.has(label)) return t(label);
  for (const { pattern, template, names } of PATTERNS) {
    const match = pattern.exec(label);
    if (!match) continue;
    const params: Record<string, string> = {};
    names.forEach((name, index) => {
      params[name] = match[index + 1] ?? "";
    });
    return t(template, params);
  }
  return label;
}

/**
 * The interface language (spec.md 5.7).
 *
 * Every string a person reads is written in English at the place it is shown
 * and passed through `t`, which looks it up in the selected language's
 * catalogue. The English text *is* the key: the source stays readable, and a
 * string nobody translated still shows, in English, rather than as a key.
 *
 * A catalogue is split by area of the interface — `locales/<lang>/<area>.ts`,
 * each a default-exported record — and gathered here by glob, so adding an
 * area is adding a file. `coverage.test.ts` holds every catalogue to every
 * string the source passes to `t` or `msg`, to the schema's labels in
 * `rust-strings.json`, and flags JSX text and labelling attributes that skip
 * `t` altogether. A feature added in English only fails the suite.
 *
 * The language is a person's preference, saved in `AppSettings.language` by
 * `set_language`. A copy is kept in `localStorage` only so the first frame
 * after launch is already in the right language; the settings file is the
 * authority and overwrites it when it loads.
 */

import { useSyncExternalStore } from "react";
import { interpolate, type Params } from "./msg";

export { interpolate, msg, TranslatableError, type Params } from "./msg";

export type Language = "en" | "es" | "fr" | "de";

/** The languages offered, each named in itself: a reader looks for their own. */
export const LANGUAGES: readonly { id: Language; name: string }[] = [
  { id: "en", name: "English" },
  { id: "es", name: "Español" },
  { id: "fr", name: "Français" },
  { id: "de", name: "Deutsch" },
];

export type Catalogue = Record<string, string>;

const modules = import.meta.glob<{ default: Catalogue }>("./locales/*/*.ts", { eager: true });

/** Each language's areas, merged. Exported for the coverage test. */
export const CATALOGUES: Record<Exclude<Language, "en">, Catalogue> = { es: {}, fr: {}, de: {} };
/** Where each key came from, per language, so the test can name a conflict's files. */
export const SOURCES: Record<string, Record<string, string[]>> = { es: {}, fr: {}, de: {} };
for (const [path, module] of Object.entries(modules)) {
  const match = /\.\/locales\/(\w+)\/([\w-]+)\.ts$/.exec(path);
  if (!match || !(match[1]! in CATALOGUES)) continue;
  const language = match[1] as Exclude<Language, "en">;
  for (const [key, value] of Object.entries(module.default)) {
    CATALOGUES[language][key] ??= value;
    (SOURCES[language]![key] ??= []).push(match[2]!);
  }
}

const STORAGE_KEY = "ve.language";

function known(value: unknown): value is Language {
  return LANGUAGES.some(l => l.id === value);
}

function initial(): Language {
  try {
    const saved = globalThis.localStorage?.getItem(STORAGE_KEY);
    if (known(saved)) return saved;
  } catch {
    // Storage can be unavailable; English until the settings arrive.
  }
  return "en";
}

let current: Language = initial();
const listeners = new Set<() => void>();

function applyDocumentLanguage() {
  if (typeof document !== "undefined") document.documentElement.lang = current;
}
applyDocumentLanguage();

/** The selected language. */
export function language(): Language {
  return current;
}

/**
 * Switches the interface. The caller persists it (`api.setLanguage`); this
 * only changes what is shown, and is what the settings' arrival calls too.
 */
export function setLanguage(next: string): void {
  if (!known(next) || next === current) return;
  current = next;
  try {
    globalThis.localStorage?.setItem(STORAGE_KEY, next);
  } catch {
    // A convenience copy; the settings file is the authority.
  }
  applyDocumentLanguage();
  for (const listener of listeners) listener();
}

export function subscribeLanguage(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/**
 * The text in the selected language.
 *
 * `text` is the English, with `{name}` placeholders for anything that varies;
 * never build the key by concatenation, or no catalogue can hold it.
 */
export function t(text: string, params?: Params): string {
  const translated = current === "en" ? undefined : CATALOGUES[current][text];
  return interpolate(translated ?? text, params);
}

/**
 * Subscribes a component to the language and returns `t`. Every component
 * that shows translated text calls this, or it keeps the old language until
 * something else re-renders it.
 */
export function useT(): typeof t {
  useSyncExternalStore(subscribeLanguage, language, language);
  return t;
}

/** The selected language, re-rendering when it changes. */
export function useLanguage(): Language {
  return useSyncExternalStore(subscribeLanguage, language, language);
}

/** Locale-aware comparison for search: case- and accent-insensitive. */
export function fold(text: string): string {
  return text.normalize("NFD").replace(/\p{M}/gu, "").toLocaleLowerCase(current);
}

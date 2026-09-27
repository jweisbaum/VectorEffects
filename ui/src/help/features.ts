/**
 * What the Help search finds and points at (spec.md 5.8).
 *
 * A feature is a control a person might be looking for: a tool, a button, a
 * panel, a setting. Its element carries `data-feature="<id>"`, and choosing
 * it in the search flashes a rectangle around that element (`highlight.ts`).
 * One that is not on screen yet — a tool's option, a control in a closed
 * panel or the Settings dialog — names the `reveal` steps that bring it there,
 * handled by whichever component owns that state.
 *
 * The registry is split by area — `features/<area>.ts`, each default-exporting
 * a list — and gathered here, so each area keeps its own. `label`,
 * `description` and `keywords` are English and translated at search time, so
 * the search reads the language on screen; they are written with `msg` so the
 * coverage test holds every catalogue to them. `features.test.ts` checks that
 * every id is unique, every `data-feature` in the source is registered, and
 * every registered id is used by some element.
 */

import { fold, t } from "../i18n";

export interface Feature {
  /** Matches the element's `data-feature`. `area:name`, lower-case. */
  id: string;
  /** What the control is called, as the interface calls it. English, via `msg`. */
  label: string;
  /** One sentence on what it does. English, via `msg`. */
  description?: string;
  /** Other words a person might type for it. English, via `msg`. */
  keywords?: string[];
  /** The help page that explains it, by `HelpTopic.id`. */
  topic?: string;
  /**
   * Reveal steps, in order, before the element is looked for: e.g.
   * `["panel:left"]` or `["tool:brush"]`. Each is handled by a component that
   * called `onReveal` for it (or for its prefix, `"tool:"`).
   */
  reveal?: string[];
}

const modules = import.meta.glob<{ default: Feature[] }>("./features/*.ts", { eager: true });

export const FEATURES: Feature[] = Object.keys(modules).sort().flatMap(path => modules[path]!.default);

export interface FeatureMatch { feature: Feature; score: number }

/**
 * Features whose translated label, description or keywords hold every word of
 * the query, best first: the label beats the keywords beats the description,
 * and a word's start beats its middle.
 */
export function searchFeatures(query: string, features: Feature[] = FEATURES): FeatureMatch[] {
  const words = fold(query).trim().split(/\s+/).filter(Boolean);
  if (words.length === 0) return [];
  const matches: FeatureMatch[] = [];
  for (const feature of features) {
    const label = fold(t(feature.label));
    const keywords = fold((feature.keywords ?? []).map(k => t(k)).join(" "));
    const description = fold(feature.description ? t(feature.description) : "");
    let score = 0;
    let all = true;
    for (const word of words) {
      const at = (text: string) => {
        const index = text.indexOf(word);
        if (index < 0) return 0;
        return index === 0 ? 3 : /[\s/(-]/.test(text[index - 1]!) ? 2 : 1;
      };
      const best = Math.max(at(label) * 10, at(keywords) * 4, at(description));
      if (best === 0) { all = false; break; }
      score += best;
    }
    if (all) matches.push({ feature, score });
  }
  return matches.sort((a, b) => b.score - a.score || t(a.feature.label).localeCompare(t(b.feature.label)));
}

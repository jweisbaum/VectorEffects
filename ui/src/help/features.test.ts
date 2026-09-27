/**
 * The Help search's registry agrees with the interface (spec.md 5.8): every
 * control tagged `data-feature` is findable, every registered feature has an
 * element to flash, and every reveal step has a handler.
 */
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { FEATURES, searchFeatures } from "./features";
import { TOPICS } from "./topics";
import { setLanguage } from "../i18n";

const ROOT = new URL("..", import.meta.url).pathname;

function source(dir: string, out: string[] = [], tsxOnly = false): string[] {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) { if (name !== "generated" && name !== "locales") source(path, out, tsxOnly); }
    else if ((tsxOnly ? /\.tsx$/ : /\.tsx?$/).test(name) && !/\.test\.tsx?$/.test(name)) out.push(readFileSync(path, "utf8"));
  }
  return out;
}
const text = source(ROOT).join("\n");
const jsx = source(ROOT, [], true).join("\n");
const tagged = new Set([...jsx.matchAll(/data-feature=(?:"([^"$]+)"|\{"([^"$]+)"\})/g)].map(m => m[1] ?? m[2]!));
/** `data-feature={`tool:${…}`}`: a family of ids built from a prefix. */
const families = [...jsx.matchAll(/data-feature=\{`([^`$]*)\$\{/g)].map(m => m[1]!);
const reveals = [...text.matchAll(/onReveal\(\s*"([^"]+)"/g)].map(m => m[1]!);

describe("feature registry", () => {
  it("has unique ids", () => {
    const ids = FEATURES.map(f => f.id);
    expect(ids.filter((id, i) => ids.indexOf(id) !== i)).toEqual([]);
  });
  it("registers every tagged control", () => {
    const ids = new Set(FEATURES.map(f => f.id));
    expect([...tagged].filter(id => !ids.has(id))).toEqual([]);
  });
  it("has an element for every feature", () => {
    expect(FEATURES.map(f => f.id).filter(id => !tagged.has(id) && !families.some(p => id.startsWith(p)))).toEqual([]);
  });
  it("names help pages that exist", () => {
    const topics = new Set(TOPICS.map(t => t.id));
    expect(FEATURES.filter(f => f.topic && !topics.has(f.topic)).map(f => `${f.id} → ${f.topic}`)).toEqual([]);
  });
  it("has a handler for every reveal step", () => {
    const steps = FEATURES.flatMap(f => f.reveal ?? []);
    expect(steps.filter(step => !reveals.some(r => r === step || (r.endsWith(":") && step.startsWith(r))))).toEqual([]);
  });
  it("searches in the language on screen", () => {
    setLanguage("en");
    expect(searchFeatures("langu").map(m => m.feature.id)).toContain("settings:language");
    setLanguage("es");
    expect(searchFeatures("idioma").map(m => m.feature.id)).toContain("settings:language");
    setLanguage("de");
    expect(searchFeatures("sprache").map(m => m.feature.id)).toContain("settings:language");
    setLanguage("fr");
    expect(searchFeatures("langue").map(m => m.feature.id)).toContain("settings:language");
    for (const [language, query] of [["it", "lingua"], ["nl", "taal"], ["ja", "言語"], ["zh", "语言"], ["ar", "اللغة"]] as const) {
      setLanguage(language);
      expect(searchFeatures(query).map(m => m.feature.id), language).toContain("settings:language");
    }
    setLanguage("en");
  });
  it("ranks a label match above a description match", () => {
    const found = searchFeatures("help").map(m => m.feature.id);
    expect(found[0]).toBe("shell:help");
  });
});

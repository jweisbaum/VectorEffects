/**
 * Every string a person reads is translated into every language (spec.md 5.7).
 *
 * Three checks, each against the source rather than a list kept by hand:
 *
 * 1. Every literal passed to `t(...)` or `msg(...)` anywhere in `ui/src`, and
 *    every schema label in `rust-strings.json`, has an entry in each
 *    catalogue, with the same `{placeholders}`.
 * 2. No catalogue holds an entry nothing uses, and no two areas translate the
 *    same English differently.
 * 3. No JSX text, labelling attribute (`title`, `aria-label`, `placeholder`,
 *    `alt`, `label`) or `setHint`/`reportError` literal skips `t` — the check
 *    that makes a new feature written in English only fail the suite.
 *
 * A string that must stay as written — a brand, a file format, a unit symbol —
 * passes when every word in it is in `VERBATIM`. Anything else deliberate goes
 * on the line after an `i18n-ignore` comment, with the reason.
 */
import { describe, expect, it } from "vitest";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { parse } from "@babel/parser";
import { CATALOGUES, type Catalogue } from "./index";
import rustStrings from "./rust-strings.json";

const ROOT = new URL("..", import.meta.url).pathname;

/** Words that read the same in every language offered. */
const VERBATIM = new Set([
  "VectorEffects", "GRIB", "GRIB2", "grib", "grib2", "Zarr", "zarr", "UTC", "EPSG", "PROJ", "WKT", "MCP",
  "ERA5", "GlobCurrent", "OpenStreetMap", "OSM", "S-57", "ENC", "GIS", "KML", "KMZ", "GPX", "GeoJSON",
  "GeoTIFF", "TIFF", "PNG", "JPEG", "JSON", "Claude", "Codex", "Desktop", "Code", "UTM", "WGS84", "veproj",
  "vecap", "API", "URL", "HTTP", "Cmd", "Ctrl", "Alt", "Shift", "Esc", "Enter", "Tab", "kt", "km", "nm",
  "mph", "px", "hPa", "OK", "EOF",
]);

const LABEL_ATTRIBUTES = new Set(["title", "aria-label", "placeholder", "alt", "label", "aria-description"]);
const MESSAGE_CALLS = new Set(["setHint", "reportError"]);

function sources(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    const rel = relative(ROOT, path);
    if (statSync(path).isDirectory()) {
      if (["generated", "locales"].includes(name)) continue;
      sources(path, out);
    } else if (/\.tsx?$/.test(name) && !/\.test\.tsx?$/.test(name) && !/\.d\.ts$/.test(name)) {
      if (rel === "i18n/index.ts") continue;
      out.push(path);
    }
  }
  return out;
}

/** Whether a piece of text reads the same untranslated: every word of two letters or more is verbatim. */
function verbatim(text: string): boolean {
  return (text.match(/\p{L}+/gu) ?? []).every(word => word.length < 2 || VERBATIM.has(word));
}

type Node = { type: string; loc?: { start: { line: number } }; [key: string]: unknown };

function walk(node: unknown, visit: (node: Node) => void) {
  if (!node || typeof node !== "object") return;
  if (Array.isArray(node)) { for (const child of node) walk(child, visit); return; }
  const n = node as Node;
  if (typeof n.type === "string") visit(n);
  for (const key of Object.keys(n)) {
    if (key === "loc" || key === "start" || key === "end" || key === "extra" || key === "leadingComments"
      || key === "trailingComments" || key === "innerComments") continue;
    walk(n[key], visit);
  }
}

interface Scan { keys: Map<string, string>; problems: string[] }

function literal(node: Node | undefined): string | null {
  if (!node) return null;
  if (node.type === "StringLiteral") return node.value as string;
  if (node.type === "TemplateLiteral" && (node.expressions as unknown[]).length === 0) {
    return ((node.quasis as { value: { cooked: string } }[])[0]!.value.cooked);
  }
  return null;
}

function scan(): Scan {
  const keys = new Map<string, string>();
  const problems: string[] = [];
  for (const file of sources(ROOT)) {
    const text = readFileSync(file, "utf8");
    const lines = text.split("\n");
    const rel = relative(ROOT, file);
    const ignored = (line: number) => /i18n-ignore/.test(lines[line - 2] ?? "") || /i18n-ignore/.test(lines[line - 1] ?? "");
    const ast = parse(text, { sourceType: "module", plugins: ["typescript", "jsx"], errorRecovery: true });
    const flag = (node: Node, what: string) => {
      const line = node.loc?.start.line ?? 0;
      if (!ignored(line)) problems.push(`${rel}:${line} ${what}`);
    };
    walk(ast.program, node => {
      if (node.type === "CallExpression") {
        const callee = node.callee as Node;
        const name = callee.type === "Identifier" ? callee.name as string
          : callee.type === "MemberExpression" && (callee.property as Node).type === "Identifier"
            ? (callee.property as Node).name as string : null;
        const first = (node.arguments as Node[])[0];
        if (name === "t" || name === "msg") {
          const key = literal(first);
          if (key !== null) keys.set(key, `${rel}:${node.loc?.start.line}`);
          else if (first?.type === "TemplateLiteral") flag(node, `${name}() with an interpolated template: use {placeholders}`);
        } else if (name && MESSAGE_CALLS.has(name)) {
          const value = literal(first);
          if (value !== null && !verbatim(value)) flag(node, `${name}(${JSON.stringify(value)}) is not translated`);
        }
      } else if (node.type === "JSXText") {
        const value = (node.value as string).trim();
        if (value && !verbatim(value)) flag(node, `JSX text ${JSON.stringify(value)} is not translated`);
      } else if (node.type === "JSXAttribute") {
        const attr = node.name as Node;
        const attrName = attr.type === "JSXIdentifier" ? attr.name as string : "";
        if (!LABEL_ATTRIBUTES.has(attrName)) return;
        let value = node.value as Node | null;
        if (value?.type === "JSXExpressionContainer") value = value.expression as Node;
        const found = literal(value ?? undefined);
        if (found !== null && !verbatim(found)) flag(node, `${attrName}=${JSON.stringify(found)} is not translated`);
      } else if (node.type === "JSXExpressionContainer") {
        const found = literal(node.expression as Node);
        if (found !== null && found.trim() && !verbatim(found)) flag(node, `JSX string ${JSON.stringify(found)} is not translated`);
      }
    });
  }
  return { keys, problems };
}

const placeholders = (text: string) => [...text.matchAll(/\{(\w+)\}/g)].map(m => m[1]).sort();

const { keys, problems } = scan();
const required = new Set<string>([...keys.keys(), ...(rustStrings as string[])]);

describe("translations", () => {
  it("leave no interface text outside t()", () => {
    expect(problems).toEqual([]);
  });

  for (const [language, catalogue] of Object.entries(CATALOGUES) as [string, Catalogue][]) {
    it(`cover every string in ${language}`, () => {
      const missing = [...required].filter(key => !(key in catalogue))
        .map(key => `${key}  (${keys.get(key) ?? "rust-strings.json"})`);
      expect(missing).toEqual([]);
    });
    it(`keep the placeholders in ${language}`, () => {
      const wrong = Object.entries(catalogue)
        .filter(([key, value]) => placeholders(key).join() !== placeholders(value).join())
        .map(([key, value]) => `${key} → ${value}`);
      expect(wrong).toEqual([]);
    });
    it(`hold nothing unused in ${language}`, () => {
      expect(Object.keys(catalogue).filter(key => !required.has(key))).toEqual([]);
    });
    it(`translate each string one way in ${language}`, () => {
      const modules = import.meta.glob<{ default: Catalogue }>("./locales/*/*.ts", { eager: true });
      const seen = new Map<string, [string, string]>();
      const conflicts: string[] = [];
      for (const [path, module] of Object.entries(modules)) {
        if (!path.startsWith(`./locales/${language}/`)) continue;
        for (const [key, value] of Object.entries(module.default)) {
          const before = seen.get(key);
          if (before && before[1] !== value) conflicts.push(`${key}: ${before[0]} "${before[1]}" vs ${path} "${value}"`);
          else seen.set(key, [path, value]);
        }
      }
      expect(conflicts).toEqual([]);
    });
  }
});

import { existsSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { expect, it } from "vitest";
import { TOPICS, searchTopics } from "./topics";
import { LANGUAGES } from "../i18n";

it("keeps every reference image and cross-reference reachable offline", () => {
  const ids = new Set(TOPICS.map(topic => topic.id));
  expect(ids.size).toBe(TOPICS.length);
  const paths = new Set<string>();
  for (const topic of TOPICS) {
    for (const id of topic.related ?? []) expect(ids.has(id), `${topic.id} links to ${id}`).toBe(true);
    for (const image of [...topic.images ?? [], ...topic.sections?.flatMap(section => section.images ?? []) ?? []]) {
      expect(image.caption.length).toBeGreaterThan(0);
      expect(existsSync(fileURLToPath(new URL(`../../public/help/${image.path}`, import.meta.url))), image.path).toBe(true);
      paths.add(image.path);
    }
  }
  for (const path of readdirSync(fileURLToPath(new URL("../../public/help/reference", import.meta.url)), { recursive: true })) {
    const relative = String(path).replaceAll("\\", "/");
    if (relative.endsWith(".png")) expect(paths.has(`reference/${relative}`), relative).toBe(true);
  }
});
it("finds parameter descriptions and nested workflows", () => {
  expect(searchTopics("angle tangent").map(t => t.id)).toContain("circle");
  expect(searchTopics("Direction toward true").map(t => t.id)).toContain("animation");
  expect(searchTopics("shadow opacity").map(t => t.id)).toContain("settings");
  expect(searchTopics("no-such-tool-name")).toEqual([]);
});

it("translates the whole reference, page for page", async () => {
  const { topicsFor } = await import("./topics");
  const shape = (topics: typeof TOPICS) => topics.map(topic => ({
    id: topic.id, related: topic.related, steps: topic.steps?.length, parameters: topic.parameters?.length,
    paragraphs: topic.paragraphs.length, images: topic.images?.map(image => image.path),
    sections: topic.sections?.map(section => ({
      steps: section.steps?.length, parameters: section.parameters?.length, paragraphs: section.paragraphs?.length,
      images: section.images?.map(image => image.path),
    })),
  }));
  for (const { id: language } of LANGUAGES.filter(l => l.id !== "en")) {
    const translated = topicsFor(language);
    expect(translated, language).not.toBe(TOPICS);
    expect(shape(translated), language).toEqual(shape(TOPICS));
  }
});
it("searches the reference in the language on screen", async () => {
  const { topicsFor } = await import("./topics");
  expect(searchTopics("pincel", topicsFor("es")).map(t => t.id)).toContain("brush");
  expect(searchTopics("pinceau", topicsFor("fr")).map(t => t.id)).toContain("brush");
  expect(searchTopics("pinsel", topicsFor("de")).map(t => t.id)).toContain("brush");
});

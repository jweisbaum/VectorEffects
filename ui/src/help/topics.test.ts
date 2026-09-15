import { existsSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { expect, it } from "vitest";
import { TOPICS, searchTopics } from "./topics";

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
    if (String(path).endsWith(".png")) expect(paths.has(`reference/${path}`), String(path)).toBe(true);
  }
});
it("finds parameter descriptions and nested workflows", () => {
  expect(searchTopics("angle tangent").map(t => t.id)).toContain("circle");
  expect(searchTopics("Direction toward true").map(t => t.id)).toContain("animation");
  expect(searchTopics("shadow opacity").map(t => t.id)).toContain("settings");
  expect(searchTopics("no-such-tool-name")).toEqual([]);
});

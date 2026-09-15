/** Opens every help page in the real WebKit view and verifies bundled assets. */
import { TOPICS } from "../../ui/src/help/topics";
import { screenshot as capturePage } from "./help-fixture";
const tick = () => new Promise(resolve => setTimeout(resolve, 40));
export async function check(css: string) {
  // Exercise the packaged app's own Help instance; mounting another would
  // leave the original application's capture-phase keyboard listener alive.
  const host = document.body;
  for (let attempt = 0; !host.querySelector("button") && attempt < 100; attempt++) await tick();
  const editor = host.querySelector<HTMLButtonElement>("button")!;
  editor.focus();
  window.dispatchEvent(new KeyboardEvent("keydown", { key: "F1" })); await tick();
  const visited: string[] = [], images = new Set<string>();
  for (const topic of TOPICS) {
    (window as unknown as { helpCheckPage: string }).helpCheckPage = topic.id;
    const button = [...host.querySelectorAll<HTMLButtonElement>("nav button")].find(b => b.textContent === topic.title)!;
    if (!button) throw new Error(`Missing help topic: ${topic.title}`);
    button.click(); await tick();
    const article = host.querySelector<HTMLElement>("article")!;
    if (article.scrollWidth > article.clientWidth + 1) throw new Error(`Page overflows: ${topic.id}`);
    for (const img of article.querySelectorAll<HTMLImageElement>("img")) {
      img.scrollIntoView({ block: "center" });
      await Promise.race([img.decode(), new Promise((_, reject) => setTimeout(() => reject(new Error(`Image did not load: ${img.src}`)), 10_000))]);
      if (!img.naturalWidth) throw new Error(`Missing ${img.src}`);
      images.add(img.src);
    }
    visited.push(topic.id);
  }
  const circle = [...host.querySelectorAll<HTMLButtonElement>("nav button")].find(b => b.textContent === "Circle stamp")!;
  circle.click(); await tick();
  const screenshot = host.querySelector<HTMLButtonElement>(".help-screenshot")!;
  screenshot.scrollIntoView({ block: "center" }); await tick();
  screenshot.click(); await tick();
  if (screenshot.getAttribute("aria-expanded") !== "true") throw new Error("Screenshot did not enlarge");
  window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })); await tick();
  if (!host.querySelector('[role="dialog"]') || screenshot.getAttribute("aria-expanded") !== "false") throw new Error("Escape should reduce screenshot first");
  const bounds = host.querySelector<HTMLElement>('[role="dialog"]')!.getBoundingClientRect();
  const result = { pages: visited.length, images: images.size, width: bounds.width, height: bounds.height,
    viewport: [innerWidth, innerHeight], overflowing: bounds.right > innerWidth || bounds.bottom > innerHeight,
    screenshot: await capturePage(null, css) };
  window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })); await tick();
  if (host.querySelector('[role="dialog"]') || document.activeElement !== editor) throw new Error("Help did not restore editor focus");
  return result;
}

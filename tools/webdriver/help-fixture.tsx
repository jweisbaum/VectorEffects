/** Screenshot fixture using the real application components and native IPC. */
import { createRoot } from "react-dom/client";
import App from "../../ui/src/App";
import { reportError } from "../../ui/src/hint";

/** Exercise the real status bar with an error longer than the available space. */
export async function checkRetry() {
  let retried = false;
  reportError("Download failed: " + "archive connection timed out; ".repeat(30), null, () => { retried = true; });
  await new Promise(resolve => setTimeout(resolve, 100));
  const button = document.querySelector<HTMLButtonElement>(".status-retry");
  if (!button) throw new Error("Retry button is missing");
  const buttonRect = button.getBoundingClientRect();
  const panelRect = button.parentElement!.getBoundingClientRect();
  const visible = buttonRect.width > 50 && buttonRect.left >= panelRect.left && buttonRect.right <= panelRect.right + 1 && buttonRect.right <= innerWidth;
  button.click();
  await new Promise(resolve => setTimeout(resolve, 100));
  if (!visible || !retried || document.querySelector(".status-retry")) throw new Error("Retry is clipped or did not run once");
  return { visible, retried };
}

export function install(css: string) {
  const style = document.createElement("style");
  style.textContent = css;
  document.head.append(style);
  const host = document.createElement("div");
  host.id = "root";
  document.body.replaceChildren(host);
  createRoot(host).render(<App />);
}

/** Capture laid-out DOM, replacing canvases with their actual framebuffer. */
export async function screenshot(mapPng: string | null, css: string): Promise<string> {
  const source = document.body;
  const clone = source.cloneNode(true) as HTMLElement;
  const canvases = [...source.querySelectorAll("canvas")];
  const copies = [...clone.querySelectorAll("canvas")];
  for (let i = 0; i < canvases.length; i++) {
    const canvas = canvases[i]!;
    const copy = copies[i]!;
    const img = document.createElement("img");
    img.setAttribute("style", canvas.getAttribute("style") ?? "");
    img.className = canvas.className;
    img.width = canvas.width; img.height = canvas.height;
    const isMap = canvas.classList.contains("map-canvas");
    if (isMap && mapPng) img.src = mapPng;
    else if (canvas.classList.contains("map-overlay") && mapPng) { copy.remove(); continue; }
    else img.src = canvas.toDataURL("image/png");
    copy.replaceWith(img);
  }
  const inputs = [...source.querySelectorAll("input")];
  for (const [i, input] of [...clone.querySelectorAll("input")].entries()) {
    input.setAttribute("value", inputs[i]!.value);
    if (inputs[i]!.checked) input.setAttribute("checked", ""); else input.removeAttribute("checked");
  }
  const options = [...source.querySelectorAll("option")];
  for (const [i, option] of [...clone.querySelectorAll("option")].entries()) {
    if (options[i]!.selected) option.setAttribute("selected", ""); else option.removeAttribute("selected");
  }
  clone.querySelectorAll("script, link").forEach((element) => element.remove());
  // WebKit's SVG foreignObject needs an explicit stacking context for the
  // stage's z=0 map, otherwise it can paint beneath the body's background.
  const stage = clone.querySelector<HTMLElement>(".stage");
  if (stage) stage.style.isolation = "isolate";
  await Promise.all([...clone.querySelectorAll("img")].map(img => img.decode()));
  // An SVG foreignObject cannot fetch the app's bundled URLs. Inline loaded
  // help screenshots so the captured page includes the illustrations too.
  for (const img of clone.querySelectorAll<HTMLImageElement>("img")) {
    if (img.src.startsWith("data:")) continue;
    const raster = document.createElement("canvas");
    raster.width = img.naturalWidth; raster.height = img.naturalHeight;
    raster.getContext("2d")!.drawImage(img, 0, 0);
    img.src = raster.toDataURL("image/png");
  }
  const wrapper = document.createElement("html");
  wrapper.setAttribute("xmlns", "http://www.w3.org/1999/xhtml");
  const style = document.createElement("style");
  style.textContent = css;
  wrapper.append(style, clone);
  const width = innerWidth; const height = innerHeight;
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}"><foreignObject width="100%" height="100%">${new XMLSerializer().serializeToString(wrapper)}</foreignObject></svg>`;
  const image = new Image();
  await new Promise<void>((resolve, reject) => { image.onload = () => resolve(); image.onerror = () => reject(new Error("Screenshot SVG failed")); image.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`; });
  await image.decode();
  await new Promise(resolve => setTimeout(resolve, 100));
  const canvas = document.createElement("canvas"); canvas.width = width; canvas.height = height;
  canvas.getContext("2d")!.drawImage(image, 0, 0);
  return canvas.toDataURL("image/png");
}

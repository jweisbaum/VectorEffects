import { api } from "../../ui/src/ipc";
import { gestureKind } from "../../ui/src/map/tools";
import { install as installApp, screenshot } from "./help-fixture";
export { screenshot };

const calls: Parameters<typeof api.createObject>[0][] = [];
const sleep = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));

export function install(css: string) {
  const create = api.createObject;
  api.createObject = object => { calls.push(object); return create(object); };
  installApp(css);
}

async function until(test: () => boolean) {
  for (let i = 0; i < 100; i++) { if (test()) return; await sleep(50); }
  throw new Error("Interaction did not settle");
}

/** Real components and IPC in WebKit; synthetic pointer capture is a no-op. */
export async function check(css: string) {
  await until(() => !!document.querySelector('.tools [aria-label="Brush"]'));
  const canvas = document.querySelector<HTMLCanvasElement>(".map-canvas")!;
  canvas.setPointerCapture = () => {};
  canvas.releasePointerCapture = () => {};
  const palette = await api.toolPalette();
  const results = [];
  for (const schema of palette) {
    const button = [...document.querySelectorAll<HTMLButtonElement>(".tools button")].find(b => b.getAttribute("aria-label") === schema.label)!;
    button.click(); await sleep(80);
    if (schema.tool === "brush") {
      // Exact reported sequence: paint once, choose Square, then paint again.
      const bounds = canvas.getBoundingClientRect();
      const x = bounds.left + bounds.width * .55, y = bounds.top + bounds.height * .5;
      for (const [type, dx] of [["pointerdown",0],["pointermove",30],["pointerup",30]] as const) {
        canvas.dispatchEvent(new PointerEvent(type,{bubbles:true,cancelable:true,pointerId:1,pointerType:"mouse",button:0,
          buttons:type === "pointerup" ? 0 : 1,clientX:x+dx,clientY:y}));
        await sleep(20);
      }
      await until(() => calls.length === 1); await sleep(160);
    }
    // Prefer the user's exact Brush/Square case; other tools use their mode.
    const property = schema.options.find(o => o.property === "BrushShape")
      ?? schema.options.find(o => o.property === "FillMode")
      ?? schema.options.find(o => o.property === "CurveDirectionMode")
      ?? schema.options.find(o => o.variants.length > 1)
      ?? schema.options.find(o => o.property === "Feather");
    if (!property) throw new Error(`${schema.tool}: no option to exercise`);
    const before = calls.length;
    const chosen = property.variants.length > 1 ? {kind:"choice" as const, index:1} : {kind:"number" as const, value:0.25};
    if (chosen.kind === "choice") {
      const select = [...document.querySelectorAll<HTMLSelectElement>(".tool-options select")]
        .find(select => select.closest("label")?.textContent?.trim().startsWith(property.label));
      if (!select) throw new Error(`${schema.tool}: missing ${property.label}`);
      select.dispatchEvent(new PointerEvent("pointerdown", { bubbles:true, cancelable:true, button:0 }));
      await until(() => !!document.querySelector('[role="listbox"]'));
      (document.querySelectorAll<HTMLButtonElement>('[role="option"]')[1]!).click();
    } else {
      const input = [...document.querySelectorAll<HTMLInputElement>(".tool-options input")]
        .find(input => input.closest("label")?.textContent?.trim().startsWith(property.label));
      if (!input) throw new Error(`${schema.tool}: missing ${property.label}`);
      input.focus();
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "0.25");
      input.dispatchEvent(new Event("input", {bubbles:true}));
      input.dispatchEvent(new Event("change", {bubbles:true}));
    }
    await sleep(20);
    if (document.querySelector('[role="listbox"]')) throw new Error(`${schema.tool}: menu did not close`);
    const rect = canvas.getBoundingClientRect();
    const toolbar = document.querySelector(".map-toolbar")!.getBoundingClientRect();
    const x = rect.left + rect.width * .59, y = Math.max(toolbar.bottom + 45, rect.top + rect.height * .45);
    const pointer = (type: string, x: number, y: number) => canvas.dispatchEvent(new PointerEvent(type, {
      bubbles:true, cancelable:true, pointerId:1, pointerType:"mouse", isPrimary:true,
      button:0, buttons:type === "pointerup" ? 0 : 1, clientX:x, clientY:y,
    }));
    const values = Object.fromEntries(schema.options.map(o => [o.property, o.default]));
    values[property.property] = chosen;
    const gesture = gestureKind(schema, values);
    if (gesture === "path" || gesture === "ring") {
      for (const [dx,dy] of [[0,0],[35,25],[65,-10]]) {
        pointer("pointerdown",x+dx!,y+dy!); pointer("pointerup",x+dx!,y+dy!); await sleep(20);
      }
      canvas.dispatchEvent(new KeyboardEvent("keydown", {key:"Enter",bubbles:true,cancelable:true}));
    } else {
      pointer("pointerdown", x, y);
      for (let i = 1; i <= 5; i++) { pointer("pointermove", x+i*9, y+i*4); await sleep(15); }
      pointer("pointerup", x+45, y+20);
    }
    await until(() => calls.length > before);
    const request = calls[before]!;
    const option = request.options.find(o => o.property === property.property)?.value;
    if (calls.length !== before + 1 || request.tool !== schema.tool || JSON.stringify(option) !== JSON.stringify(chosen)) {
      throw new Error(`${schema.tool}: first gesture used wrong options: ${JSON.stringify(request)}`);
    }
    await sleep(160);
    results.push({tool:schema.tool, property:property.property, firstGesture:true});
  }
  // Circle sliders must fit the Properties panel without horizontal scrolling.
  const tree = await api.documentTree(0);
  const circle = tree.layers.flatMap(l => l.objects).find(o => o.tool === "circle")!;
  document.querySelector<HTMLButtonElement>(`.object[data-object-id="${circle.id}"] .name`)?.click();
  await sleep(150);
  const inspector = document.querySelector<HTMLElement>(".inspector")!;
  if (!inspector.querySelector(".centred-slider")) throw new Error("Circle inspector not selected");
  const overflow = inspector.scrollWidth - inspector.clientWidth;
  if (overflow > 1) throw new Error(`Circle inspector overflows by ${overflow}px`);
  const circleImage = await screenshot(null, css);
  // A short toolbar is centred over the map remaining between both docks.
  document.querySelector<HTMLButtonElement>('.tools [aria-label="Hand"]')!.click();
  await sleep(80);
  const map = document.querySelector<HTMLElement>(".map")!;
  const bar = document.querySelector<HTMLElement>(".map-toolbar")!;
  const style = getComputedStyle(map), bounds = map.getBoundingClientRect(), box = bar.getBoundingClientRect();
  const left = parseFloat(style.getPropertyValue("--dock-left")) || 0;
  const right = parseFloat(style.getPropertyValue("--dock-right")) || 0;
  const centreError = Math.abs((box.left+box.right)/2 - (bounds.left+bounds.width/2+(left-right)/2));
  if (centreError > 1 || bar.scrollWidth > bar.clientWidth+1) throw new Error("Toolbar centre/overflow failed");
  const ruler = document.querySelector<HTMLElement>(".tl-ruler .tl-grid")!;
  const rulerRect = ruler.getBoundingClientRect();
  const event = new PointerEvent("pointerdown", {bubbles:true,cancelable:true,button:0,clientX:rulerRect.left+50,clientY:rulerRect.top+10});
  ruler.dispatchEvent(event);
  window.dispatchEvent(new PointerEvent("pointermove", {clientX:rulerRect.left+120,clientY:rulerRect.top+10}));
  window.dispatchEvent(new PointerEvent("pointerup"));
  if (!event.defaultPrevented || getComputedStyle(ruler).getPropertyValue("-webkit-user-select") !== "none") throw new Error("Ruler allows text selection");
  return {tools:results, circleOverflow:overflow, toolbarCentreError:centreError, toolbarWidth:box.width, rulerPreventsSelection:true, circleImage};
}

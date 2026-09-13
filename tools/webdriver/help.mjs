/** Generate bundled help screenshots from the real app in isolated storage.
 * Build first: cargo build -p ve-app --release --features webdriver,tauri/custom-protocol
 */
import { spawn } from "node:child_process";
import { mkdtemp, mkdir, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { build } from "esbuild";
import { connect, portFromLine } from "./client.mjs";

const root = await mkdtemp(join(tmpdir(), "ve-help-"));
const output = resolve("ui/public/help");
await mkdir(output, { recursive: true });
const css = await readFile("ui/src/styles.css", "utf8");
const bundle = await build({
  entryPoints: ["tools/webdriver/help-fixture.tsx"], bundle: true, write: false,
  format: "iife", globalName: "HelpFixture", jsx: "automatic",
  define: { "import.meta.env.DEV": "true", "process.env.NODE_ENV": '"production"' },
  plugins: [{ name: "camera-fixture", setup(builder) {
    builder.onLoad({ filter: /ui\/src\/map\/MapView\.tsx$/ }, async args => {
      let contents = await readFile(args.path, "utf8");
      contents = contents.replace("hooks.__veCapture = capture;", `hooks.__veCapture = capture;
        window.__helpCamera = (camera) => { cameraRef.current = camera; requestDraw(); };`);
      return { contents, loader: "tsx", resolveDir: resolve("ui/src/map") };
    });
  } }],
});
const child = spawn(resolve("target/release/ve-app"), [], {
  env: { ...process.env, VE_AUTOMATION_ROOT: root }, stdio: ["ignore", "pipe", "pipe"],
});
try {
  const port = await new Promise((done, reject) => {
    const timeout = setTimeout(() => reject(new Error("No automation port after 5 minutes")), 300_000);
    createInterface({ input: child.stdout }).on("line", line => {
      const port = portFromLine(line);
      if (port !== null) { clearTimeout(timeout); done(port); }
    });
    child.once("exit", code => { clearTimeout(timeout); reject(new Error(`App exited: ${code}`)); });
    createInterface({ input: child.stderr }).on("line", line => { if (line.includes("ERROR") || line.includes("panicked")) process.stderr.write(`${line}\n`); });
  });
  const driver = connect(port);
  await driver.ready();
  await driver.call("/window/set-current", { label: "main" });
  const evaluate = (body, args = []) => driver.evaluate(`const done = arguments[arguments.length - 1]; ${body}`, args);
  const invoke = (command, args = {}) => evaluate(`window.__TAURI_INTERNALS__.invoke(arguments[0],arguments[1]).then(done,e=>done({error:"IPC",message:JSON.stringify(e)}));`, [command, args]);
  const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
  async function until(body, timeout = 120_000) {
    const end = Date.now() + timeout;
    while (Date.now() < end) { if (await evaluate(`done(${body});`)) return; await sleep(250); }
    throw new Error(`Timed out: ${body}`);
  }
  console.log(JSON.stringify(await evaluate("done({url:location.href,ready:document.readyState,body:document.body.innerText.slice(0,500),invoke:typeof window.__TAURI_INTERNALS__?.invoke});")));
  const info = await invoke("app_info");
  if (info.cache_dir !== join(root, "cache")) throw new Error("Automation storage is not isolated");
  await invoke("new_project", { request: { name: "North Atlantic study", field_kind: "wind", resolution: "1.0", step_hours: 1, step_count: 12 }, discardUnsaved: true });
  await invoke("set_projection", { projection: "mercator" });
  await invoke("add_brush_stroke", { stroke: { points: [[-42, 46], [-26, 51], [-5, 53]], size_km: 1500, speed_mps: 18, direction_toward_deg: 80, feather: 0.3, space: "mercator" } });
  let tree = await invoke("document_tree", { step: 0 });
  const layer = tree.layers[0].id;
  const object = tree.layers[0].objects[0].id;
  await invoke("rename_layer", { layer, name: "Wind systems" });
  await invoke("rename_object", { object, name: "Atlantic flow" });
  await invoke("set_keyframe", { object, property: "Speed", step: 0, value: { kind: "number", value: 18 } });
  await invoke("set_keyframe", { object, property: "Speed", step: 11, value: { kind: "number", value: 26 } });
  await invoke("add_brush_stroke", { stroke: { points: [[-8, 68]], size_km: 950, speed_mps: 12, direction_toward_deg: 140, feather: 0.15, space: "mercator" } });
  tree = await invoke("document_tree", { step: 0 });
  const polar = tree.layers[0].objects[1].id;
  await invoke("rename_object", { object: polar, name: "Northern winds" });
  await invoke("set_layer_speed_range", { layer, minMps: 0, maxMps: 35, gesture: null });
  await evaluate(`${bundle.outputFiles[0].text}\nwindow.__helpFixture = HelpFixture; HelpFixture.install(arguments[0]); done(true);`, [css]);
  await until("!!window.__helpCamera && !!document.querySelector('.timeline')");
  await evaluate(`window.__helpCamera({centerLon:-22,centerLat:54,pxPerDeg:10,projection:"mercator"}); done(true);`);
  async function click(selector) {
    await until(`!!document.querySelector(${JSON.stringify(selector)})`);
    await evaluate(`document.querySelector(arguments[0]).click(); done(true);`, [selector]);
    await sleep(250);
  }
  async function label(text) {
    await evaluate(`const b=[...document.querySelectorAll('button')].find(b=>b.textContent.trim()===arguments[0]); if(!b) throw new Error('No button '+arguments[0]); b.click(); done(true);`, [text]);
    await sleep(250);
  }
  async function shot(name) {
    await driver.call("/window/set-current", { label: "main" });
    const path = await driver.capture(`help-${name}`);
    const mapPng = path ? `data:image/png;base64,${(await readFile(path)).toString("base64")}` : null;
    const data = await evaluate("window.__helpFixture.screenshot(arguments[0],arguments[1]).then(done,e=>done({error:'Screenshot',message:String(e)}));", [mapPng, css]);
    await writeFile(join(output, `${name}.png`), Buffer.from(data.split(",")[1], "base64"));
    console.log(JSON.stringify({ screenshot: name, path: join(output, `${name}.png`) }));
  }
  await click(`[data-object-id="${object}"]`);
  await shot("workspace");
  await shot("layers");
  await click('.tl-object-row.selected [title="Show properties"]');
  await click('[aria-label="Add constant motion"]');
  await shot("motion");
  await label("Cancel");
  await click(`[aria-label="Animate shape of Atlantic flow"]`);
  await shot("shape");
  await click(`[aria-label="Animate shape of Atlantic flow"]`);
  await click('[aria-label="Brush"]');
  await evaluate(`const s=[...document.querySelectorAll('.tool-options select')].find(s=>[...s.options].some(o=>o.value==='px')); if(s){s.value='px';s.dispatchEvent(new Event('change',{bubbles:true}));} done(true);`);
  await sleep(250);
  await shot("pixel-tools");
  await click('[aria-label="Import history"]');
  await shot("history");
  await label("Cancel");
  await click('[aria-label="Settings"]');
  await shot("settings");
  await evaluate(`const b=[...document.querySelectorAll('.settings button')].find(b=>b.textContent.trim()==='Close'); b.click(); done(true);`);
  await label("Export GRIB…");
  await shot("export");
  await evaluate(`const b=[...document.querySelectorAll('.modal button')].find(b=>b.textContent.trim()==='Close'); b.click(); done(true);`);
  console.log(JSON.stringify({ retry: await evaluate("window.__helpFixture.checkRetry().then(done,e=>done({error:'RetryLayout',message:String(e)}));") }));
  console.log(JSON.stringify({ root, complete: true }));
} finally {
  child.kill("SIGTERM");
}

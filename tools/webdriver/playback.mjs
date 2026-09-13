/** Release-webview playback benchmark, using isolated storage.
 * Build: npm run tauri -- build --features webdriver --no-bundle
 * Run: node tools/webdriver/playback.mjs [seconds=60] [steps=24] [rates=8,12,24,30] [--report-only] [--project=path]
 * Imported-field fixtures: cargo run -p ve-app --example playback_rasters -- /tmp/ve-raster-fixtures 24
 */
import { spawn } from "node:child_process";
import { mkdtemp, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { connect, portFromLine } from "./client.mjs";

const seconds = Number(process.argv[2] ?? 60);
const steps = Number(process.argv[3] ?? 24);
const rates = (process.argv[4] ?? "8,12,24,30").split(",").map(Number);
const options = process.argv.slice(5);
const reportOnly = options.includes("--report-only");
const projectPath = options.find((arg) => arg.startsWith("--project="))?.slice("--project=".length);
const root = await mkdtemp(join(tmpdir(), "ve-playback-"));
const child = spawn(resolve("target/release/ve-app"), [], {
  env: { ...process.env, VE_AUTOMATION_ROOT: root }, stdio: ["ignore", "pipe", "pipe"],
});
const sleep = (ms) => new Promise((done) => setTimeout(done, ms));
let driver;
try {
  const port = await new Promise((done, reject) => {
    const timeout = setTimeout(() => reject(new Error("No automation port after 60 seconds")), 60_000);
    createInterface({ input: child.stdout }).on("line", (line) => {
      const port = portFromLine(line);
      if (port !== null) { clearTimeout(timeout); done(port); }
    });
    child.once("exit", (code) => { clearTimeout(timeout); reject(new Error(`App exited: ${code}`)); });
    createInterface({ input: child.stderr }).on("line", (line) => {
      if (line.includes("panicked") || line.includes("ERROR")) process.stderr.write(`${line}\n`);
    });
  });
  driver = connect(port);
  await driver.ready();
  await driver.call("/window/set-current", { label: "main" });
  const evaluate = (body, args = []) => driver.evaluate(`const done = arguments[arguments.length - 1]; ${body}`, args);
  const app = await evaluate(`window.__TAURI_INTERNALS__.invoke("app_info").then(done)`);
  if (app.cache_dir !== join(root, "cache")) throw new Error("The binary does not support isolated automation storage; rebuild it first.");
  await evaluate(`
    const invoke = window.__TAURI_INTERNALS__.invoke;
    window.__benchSetup = {state:"running"};
    const projectPath = arguments[0];
    (async () => {
      if (projectPath) {
        const project = await invoke("open_project", {path:projectPath,discardUnsaved:true});
        if (project.step_count !== ${steps}) throw new Error("Fixture step count does not match the benchmark");
      } else {
        await invoke("new_project", {request:{name:"Playback benchmark",field_kind:"wind",resolution:"0.25",step_hours:1,step_count:${steps}},discardUnsaved:true});
        for (let i=0;i<12;i++) {
          await invoke("add_brush_stroke", {stroke:{points:[[-150+(i%6)*60,-30+Math.floor(i/6)*60]],size_km:6500,speed_mps:10+i,direction_toward_deg:90,feather:0.25,shape:"circle",space:"geodesic",direction_mode:"constant"}});
          const tree = await invoke("document_tree", {step:0});
          const object = tree.layers.flatMap(layer => layer.objects).at(-1).id;
          await invoke("set_keyframe", {object,property:"Speed",step:0,value:{kind:"number",value:10+i}});
          await invoke("set_keyframe", {object,property:"Speed",step:${steps - 1},value:{kind:"number",value:30+i}});
        }
      }
      window.__benchSetup = {state:"done"};
    })().catch(error => {window.__benchSetup={state:"failed",message:String(error)}});
    done("started");
  `, [projectPath ? resolve(projectPath) : null]);
  async function until(body, timeoutMs = 180_000) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      const result = await evaluate(body);
      if (result?.state === "failed") throw new Error(result.message);
      if (result === true || result?.state === "done") return result;
      await sleep(250);
    }
    throw new Error(`Timed out waiting for: ${body}`);
  }
  await until("done(window.__benchSetup)");
  await evaluate("done(true); setTimeout(() => location.reload(), 50)");
  await sleep(1000);
  await until("done(!!document.querySelector('.timeline') && !!window.__vePlayback)");
  const environment = await evaluate(`done({userAgent:navigator.userAgent,viewport:[innerWidth,innerHeight],pixelRatio:devicePixelRatio,visibility:document.visibilityState})`);
  const preparationStarted = Date.now();
  await until(`done(document.querySelectorAll('.tl-tick.solid').length === ${steps} && !document.querySelector('.tl-position .tl-buffering'))`, 300_000);
  const preparationMs = Date.now() - preparationStarted;
  console.log(JSON.stringify({root, projectPath, steps, preparationMs, environment}));
  const results = [];
  for (const rate of rates) {
    // A dev rebuild can raise another app window while this one prepares.
    // WebKit throttles animation callbacks in an occluded window.
    await driver.call("/window/set-current", { label: "main" });
    await evaluate(`
      const transport = document.querySelector('.tl-transport');
      transport.querySelector('[title="Stop and return to the start"]').click();
      const loop = transport.querySelector('[aria-label="Loop"]');
      if (loop.getAttribute('aria-pressed') !== 'true') loop.click();
      const input = document.querySelector('.tl-transport input');
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value").set.call(input, String(arguments[0]));
      input.dispatchEvent(new Event("input", {bubbles:true}));
      input.dispatchEvent(new Event("change", {bubbles:true}));
      done(true);
    `, [rate]);
    await sleep(250);
    await evaluate(`
      window.__vePlayback.start();
      const play = document.querySelector('.tl-transport button[title="Play / pause (Space)"]');
      play.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true, view: window }));
      done(true);
    `);
    await until(`done(document.querySelector('.tl-transport button[title="Play / pause (Space)"]').textContent.includes('❚❚'))`);
    const visibility = [];
    for (let elapsed = 0; elapsed < seconds; elapsed += 10) {
      await sleep(Math.min(10, seconds - elapsed) * 1000);
      const state = await evaluate("done({visibility:document.visibilityState,focused:document.hasFocus()})");
      visibility.push(state);
      console.log(JSON.stringify({rate, seconds:Math.min(seconds, elapsed + 10), ...state}));
    }
    const result = await evaluate(`
      const result = window.__vePlayback.stop();
      document.querySelector('.tl-transport [title="Stop and return to the start"]').click();
      done(result);
    `);
    const { frames, ...summary } = result;
    const sequential = frames.every((frame, i) => i === 0 || Number(frame.frame.split("/").at(-1)) === (Number(frames[i-1].frame.split("/").at(-1)) + 1) % steps);
    // The first-to-last-frame rate alone can pass after a long trailing stall.
    // Include the entire measurement window in the acceptance check.
    const sustainedFps = frames.length * 1000 / summary.durationMs;
    const measured = { rate, ...summary, sustainedFps, sequential, frames: frames.length, visibility };
    results.push(measured);
    console.log(JSON.stringify(measured));
  }
  const path = join(root, "results.json");
  await writeFile(path, JSON.stringify({ projectPath, steps, preparationMs, environment, results }, null, 2));
  console.log(`Results: ${path}`);
  if (!reportOnly && results.some((result) => Math.abs(result.sustainedFps / result.rate - 1) > 0.05 || !result.sequential)) process.exitCode = 1;
} finally {
  const exited = new Promise((done) => { if (child.exitCode !== null || child.signalCode !== null) done(); else child.once("exit", done); });
  child.kill("SIGTERM");
  await exited;
  // Keep diagnostics, but not gigabytes of regenerable tiles from the fixture.
  await rm(join(root, "cache"), { recursive: true, force: true });
}

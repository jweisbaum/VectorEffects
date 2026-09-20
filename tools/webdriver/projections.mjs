/** Run all map projections in WebKit using an webdriver binary (cargo build -p ve-app --features webdriver).
 * node tools/webdriver/projections.mjs
 * Uses current frontend sources and isolated temporary app storage.
 */
import { createServer } from "node:http";
import { spawn } from "node:child_process";
import { mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { build } from "esbuild";
import { connect, portFromLine } from "./client.mjs";

const root = await mkdtemp(join(tmpdir(), "ve-projections-"));
const bundle = await build({ entryPoints: ["tools/webdriver/projections-fixture.ts"], bundle: true, write: false,
  format: "iife", globalName: "ProjectionCheck", define: { "import.meta.env.DEV": "false" } });
// Whole-world screenshots need only the coarsest LOD; keep the request below
// the automation endpoint's body limit.
const asset = await readFile("assets/basemap.bin");
const end = 32 + asset.readUInt32LE(16) * 8 + asset.readUInt32LE(20) * 4
  + asset.readUInt32LE(24) * 8 + asset.readUInt32LE(28) * 8;
const coarse = Buffer.from(asset.subarray(0, end)); coarse.writeUInt32LE(1, 8);
const basemap = coarse.toString("base64");
// A debug Tauri binary expects a page at devUrl. The fixture supplies its own
// renderer, so a blank local document is sufficient when Vite is not running.
const page = createServer((_request, response) => {
  response.writeHead(200, { "Content-Type": "text/html" });
  response.end("<!doctype html><html><head><title>Projection check</title></head><body></body></html>");
});
await new Promise((done, reject) => {
  page.once("error", error => error.code === "EADDRINUSE" ? done() : reject(error));
  page.listen(5173, "127.0.0.1", done);
});
const child = spawn(resolve(process.env.VE_APP_BINARY ?? "target/debug/ve-app"), [], {
  env: { ...process.env, VE_AUTOMATION_ROOT: root }, stdio: ["ignore", "pipe", "pipe"],
});
try {
  const port = await new Promise((done, reject) => {
    const timeout = setTimeout(() => reject(new Error("No automation port after 60 seconds")), 60_000);
    createInterface({ input: child.stdout }).on("line", line => {
      const port = portFromLine(line);
      if (port !== null) { clearTimeout(timeout); done(port); }
    });
    child.once("error", error => { clearTimeout(timeout); reject(error); });
    child.once("exit", code => { clearTimeout(timeout); reject(new Error(`App exited: ${code}`)); });
    createInterface({ input: child.stderr }).on("line", line => {
      if (line.includes("panicked") || line.includes("ERROR")) process.stderr.write(`${line}\n`);
    });
  });
  const driver = connect(port); await driver.ready();
  await new Promise(resolve => setTimeout(resolve, 500));
  await driver.evaluate(`const done = arguments[arguments.length - 1];
    ${bundle.outputFiles[0].text}
    window.__projectionCheck = null;
    const basemap = arguments[0];
    setTimeout(() => {
      ProjectionCheck.projectionsFixture(basemap).then(
        result => { window.__projectionCheck = result; },
        error => { window.__projectionCheck = { error: true, message: String(error) }; });
    }, 0);
    done({started: true});`, [basemap]);
  let result;
  const deadline = Date.now() + 600_000;
  do {
    await new Promise(resolve => setTimeout(resolve, 1000));
    result = await driver.evaluate("arguments[arguments.length - 1](window.__projectionCheck);");
  } while (!result && Date.now() < deadline);
  if (!result) throw new Error("Rendering check did not finish within ten minutes");
  if (result.error) throw new Error(result.message);
  if (result.metrics?.length !== result.count) throw new Error("Missing projection results");
  await writeFile(join(root, "projections.png"), Buffer.from(result.image.split(",")[1], "base64"));
  await writeFile(join(root, "results.json"), JSON.stringify(result.metrics, null, 2));
  console.log(JSON.stringify(result.metrics)); console.table(result.turning); console.log(root);
} finally { child.kill("SIGTERM"); if (page.listening) page.close(); }

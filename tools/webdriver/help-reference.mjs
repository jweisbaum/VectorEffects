/** Validate every help page, bundled screenshot, enlargement, and focus in WebKit. */
import { spawn } from "node:child_process";
import { mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { build } from "esbuild";
import { connect, portFromLine } from "./client.mjs";

const root = await mkdtemp(join(tmpdir(), "ve-help-reference-"));
const result = await build({ entryPoints: ["tools/webdriver/help-reference.tsx"], bundle: true, write: false,
  format: "iife", globalName: "HelpReference", jsx: "automatic", define: { "import.meta.env.DEV": "false", "process.env.NODE_ENV": '"production"' } });
const bundle = result.outputFiles[0].text;
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
    createInterface({ input: child.stderr }).on("line", line => {
      if (line.includes("panicked") || line.includes("ERROR")) process.stderr.write(`${line}\n`);
    });
  });
  const driver = connect(port);
  await driver.ready();
  const css = await readFile("ui/src/styles.css", "utf8");
  await driver.evaluate(`const done = arguments[arguments.length - 1];
    ${bundle}
    window.helpCheckResult = null;
    HelpReference.check(arguments[0]).then(result => window.helpCheckResult = result,
      e => window.helpCheckResult = {error:"HelpReference", message:String(e)});
    done("started");`, [css]);
  const deadline = Date.now() + 180_000;
  let report = null, previous = null;
  while (!report && Date.now() < deadline) {
    const state = await driver.evaluate(`const done = arguments[arguments.length - 1]; done({page:window.helpCheckPage,result:window.helpCheckResult});`);
    if (state.page !== previous) { previous = state.page; console.log(JSON.stringify({ page: state.page })); }
    report = state.result;
    if (!report) await new Promise(resolve => setTimeout(resolve, 250));
  }
  if (!report) throw new Error(`Help check timed out at ${previous}`);
  const { screenshot, ...metrics } = report;
  console.log(JSON.stringify(metrics));
  if (screenshot) {
    const path = join(root, "help-reference.png");
    await writeFile(path, Buffer.from(screenshot.split(",")[1], "base64"));
    console.log(path);
  }
  if (report.pages !== 33 || report.images < 36 || report.overflowing) throw new Error("Help reference check failed");
} finally {
  child.kill("SIGTERM");
}

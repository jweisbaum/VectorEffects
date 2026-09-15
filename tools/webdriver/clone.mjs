/** Verify clone transparency and draw cost using the real WebKit WebGL renderer.
 * Uses an existing release webdriver binary with isolated app storage. Bundles
 * current frontend sources, so no native rebuild is needed for a preview fix.
 * Run: node tools/webdriver/clone.mjs (optionally set VE_CLONE_BASELINE).
 */
import { spawn, execFileSync } from "node:child_process";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { build } from "esbuild";
import { connect, portFromLine } from "./client.mjs";

const root = await mkdtemp(join(tmpdir(), "ve-clone-"));
const baseline = process.env.VE_CLONE_BASELINE ?? "8a7a8eb15f9924930a821e297164f6260ceead83";
const bundles = {};
for (const version of ["before", "after"]) {
  const result = await build({
    entryPoints: ["tools/webdriver/clone-fixture.ts"], bundle: true, write: false,
    format: "iife", globalName: "CloneCheck", define: { "import.meta.env.DEV": "false" },
    plugins: version === "before" ? [{ name: "baseline", setup(builder) {
      builder.onLoad({ filter: /ui\/src\/map\/(renderer|shaders)\.ts$/ }, args => ({
        contents: execFileSync("git", ["show", `${baseline}:ui/src/map/${args.path.split("/").at(-1)}`], { encoding: "utf8" }),
        loader: "ts", resolveDir: resolve("ui/src/map"),
      }));
    } }] : [],
  });
  bundles[version] = result.outputFiles[0].text;
}
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
  const report = {};
  for (const version of ["before", "after"]) for (const scenario of ["wind", "calm", "empty"]) {
    const result = await driver.evaluate(`const done = arguments[arguments.length - 1];
      try { ${bundles[version]} done(CloneCheck.cloneFixture(arguments[0])); }
      catch (e) { done({error: "CloneCheck", message: String(e)}); }`, [scenario]);
    const { image, ...metrics } = result;
    if (metrics.glError !== 0) throw new Error(JSON.stringify(metrics));
    const name = `${version}-${scenario}`;
    await writeFile(join(root, `${name}.png`), Buffer.from(image.split(",")[1], "base64"));
    report[name] = metrics;
    console.log(JSON.stringify({ name, ...metrics }));
    if (version === "after") {
      for (const [i, sample] of metrics.samples.entries()) {
        const changed = sample.before.some((v, j) => Math.abs(v - sample.after[j]) > 2);
        if (changed !== (i === 0 && scenario !== "empty")) throw new Error(`Wrong transparency at ${name} site ${i}`);
      }
    }
  }
  await writeFile(join(root, "results.json"), JSON.stringify(report, null, 2));
  console.log(root);
} finally {
  child.kill("SIGTERM");
}

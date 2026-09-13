/** Compare glyph coverage and draw cost using the real WebKit WebGL renderer.
 * Uses an existing release webdriver binary with isolated app storage. Bundles
 * current frontend sources, so no native rebuild is needed for a glyph fix.
 * Run: node tools/webdriver/glyphs.mjs
 */
import { spawn, execFileSync } from "node:child_process";
import { mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { build } from "esbuild";
import { connect, portFromLine } from "./client.mjs";

const root = await mkdtemp(join(tmpdir(), "ve-glyphs-"));
const bundles = {};
for (const version of ["before", "after"]) {
  const result = await build({
    entryPoints: ["tools/webdriver/glyph-fixture.ts"], bundle: true, write: false,
    format: "iife", globalName: "GlyphCheck", define: { "import.meta.env.DEV": "false" },
    plugins: version === "before" ? [{ name: "baseline", setup(builder) {
      builder.onLoad({ filter: /ui\/src\/map\/(renderer|shaders)\.ts$/ }, args => ({
        contents: execFileSync("git", ["show", `HEAD:ui/src/map/${args.path.split("/").at(-1)}`], { encoding: "utf8" }),
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
    const timeout = setTimeout(() => reject(new Error("No automation port after 60 seconds")), 60_000);
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
  const css = await readFile("ui/src/styles.css", "utf8");
  const layout = await driver.evaluate(`const done = arguments[arguments.length - 1];
    ${bundles.after}
    GlyphCheck.glyphSettingsLayout(arguments[0]).then(done, e => done({error: "SettingsLayout", message: String(e)}));`, [css]);
  if (layout.controls !== 18 || layout.previews !== 2 || layout.overflowing || layout.stacked || layout.checkboxWidths.some(width => width > 30)) {
    throw new Error(`Settings controls do not fit: ${JSON.stringify(layout)}`);
  }
  console.log(JSON.stringify({ settingsLayout: layout }));
  for (const version of ["before", "after"]) for (const scenario of ["ring", "solid", ...(version === "after" ? ["styled"] : [])]) {
    const solid = scenario === "solid";
    const result = await driver.evaluate(`const done = arguments[arguments.length - 1];
      try { ${bundles[version]} done(GlyphCheck.glyphFixture(arguments[0], arguments[1])); }
      catch (e) { done({error: "GlyphCheck", message: String(e)}); }`, [solid, scenario === "styled"]);
    const name = `${version}-${scenario}`;
    const { image, ...metrics } = result;
    if (metrics.glError !== 0) throw new Error(`${name}: GL error ${metrics.glError}`);
    await writeFile(join(root, `${name}.png`), Buffer.from(image.split(",")[1], "base64"));
    report[name] = metrics;
    console.log(JSON.stringify({ name, ...metrics }));
  }
  await writeFile(join(root, "results.json"), JSON.stringify(report, null, 2));
  console.log(root);
} finally {
  child.kill("SIGTERM");
}

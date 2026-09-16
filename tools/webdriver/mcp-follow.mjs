/**
 * The M76 check for the MCP service: a client moves an object and the picture
 * shows it moved (spec.md 8.8).
 *
 * Everything else about the service is covered against a mock application in
 * `crates/ve-app/tests/mcp.rs`. What a mock cannot answer is whether the
 * *interface* follows: the service writes the document, emits
 * `document://changed`, and the map is supposed to redraw. So this runs the
 * real development build with its WebDriver endpoint, uses the driver only to
 * turn the service on and to take the pictures, and makes the edit itself over
 * HTTP the way any client would.
 *
 * Two things this is careful about, both of which have cost a day before.
 *
 * **Isolated storage.** Turning the service on is a settings write, and
 * `settings.json` is the person's own. The app is spawned with
 * `VE_AUTOMATION_ROOT` pointing at a fresh temporary directory (`paths.rs`,
 * and `help.mjs` does the same), and the run aborts unless `app_info` confirms
 * it landed there — otherwise this would leave their application listening on
 * a socket they never asked for.
 *
 * **A real port, chosen here.** `mcp_set` refuses port 0: a service whose port
 * moves every launch is no use to a client configuration, so "any free port"
 * is not a thing the setting can express. Node picks one by binding and
 * releasing it, and `bound_port` in the status is what confirms the app got
 * the same one.
 */
import { execFile, spawn } from "node:child_process";
import { createServer } from "node:net";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";

import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";

import { launch } from "./client.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const project = join(root, "assets/samples/cyclone.veproj");
const sleep = (ms) => new Promise((done) => setTimeout(done, ms));

/** A port nothing is using, released again before the app is told about it. */
async function freePort() {
  const server = createServer();
  await new Promise((done, fail) => {
    server.once("error", fail);
    server.listen(0, "127.0.0.1", done);
  });
  const { port } = server.address();
  await new Promise((done) => server.close(done));
  return port;
}

/**
 * The captures are written beside the app's logs, which are inside the
 * automation root and go when it does. Copied out first, so the pictures
 * outlive the run and can be looked at.
 */
async function keep(shots, path) {
  const bytes = await readFile(path);
  const copy = join(shots, basename(path));
  await writeFile(copy, bytes);
  return { path: copy, bytes };
}

/**
 * The compilation the Tauri CLI is about to do, done first.
 *
 * `launch` gives the application 180 s to announce its port and cannot tell
 * "still compiling" from "hung", so on a cold target directory the run fails
 * for the one reason that is not a fault — and leaves a detached cargo behind,
 * since the timeout rejects before there is a driver to stop. The flags are
 * the ones `tauri dev` passes (`cargo run --no-default-features --features
 * webdriver`), from the crate the CLI resolves, so this warms exactly the
 * artifacts it will use rather than a second set.
 */
async function prebuild() {
  const cargo = spawn(
    "cargo",
    ["build", "-p", "ve-app", "--no-default-features", "--features", "webdriver"],
    { cwd: root, stdio: ["ignore", "inherit", "inherit"] },
  );
  const [code] = await new Promise((done) => cargo.once("exit", (...a) => done(a)));
  if (code !== 0) throw new Error(`the application did not build (cargo exited ${code})`);
}

console.log("building the application, so the driver's wait for the port is not a race");
await prebuild();

const automation = await mkdtemp(join(tmpdir(), "ve-mcp-follow-"));
const shots = await mkdtemp(join(tmpdir(), "ve-mcp-shots-"));
const port = await freePort();
console.log(`automation root ${automation}`);
console.log(`chose port ${port}`);

let client = null;
const app = await launch({
  cwd: root,
  env: { VE_AUTOMATION_ROOT: automation },
  onLog: (line) => process.stderr.write(`${line}\n`),
});
try {
  await app.ready();
  // `help.mjs`'s assertion, for the same reason: if the app resolved its real
  // directories the run is about to edit the person's settings, and the only
  // safe thing to do is stop.
  const evaluate = (body, args = []) =>
    app.evaluate(`const done = arguments[arguments.length - 1]; ${body}`, args);
  const invoke = (command, args = {}) =>
    evaluate(
      `window.__TAURI_INTERNALS__.invoke(arguments[0],arguments[1]).then(done,e=>done({error:"IPC",message:JSON.stringify(e)}));`,
      [command, args],
    );
  const info = await invoke("app_info");
  if (info.cache_dir !== join(automation, "cache")) {
    throw new Error(
      `automation storage is not isolated: cache_dir is ${info.cache_dir}, not ${join(automation, "cache")}`,
    );
  }
  console.log(`storage is isolated: cache_dir ${info.cache_dir}`);

  // Through the application's own opening, never a bare `open_project`: the
  // command is only half of it and the interface would stay on the start
  // screen (M76).
  console.log(`opened ${await app.open(project)}`);

  const status = await invoke("mcp_set", { enabled: true, port });
  if (status.bind_error) throw new Error(`the listener did not bind: ${status.bind_error}`);
  if (status.bound_port !== port) {
    throw new Error(`asked for port ${port} and got ${JSON.stringify(status.bound_port)}`);
  }
  console.log(`service on, bound port ${status.bound_port}`);

  // The reference client, not a hand-rolled one. A first attempt posted the
  // JSON-RPC itself and fell over on the handshake: rmcp answers `initialize`
  // over SSE on a stream it holds open for the session, so reading the
  // response body to the end returns nothing and there is no JSON to parse.
  // The SDK is already a dependency (`tools/webdriver/mcp.mjs` uses its server
  // half), it is what a real client would use, and the point of this check is
  // to be a real client.
  client = new Client({ name: "mcp-follow", version: "0" });
  const transport = new StreamableHTTPClientTransport(
    new URL(`http://127.0.0.1:${status.bound_port}/mcp`),
    { requestInit: { headers: { authorization: `Bearer ${status.token}` } } },
  );
  await client.connect(transport);
  console.log(`connected: ${JSON.stringify(client.getServerVersion())}`);

  const before = await keep(shots, await app.capture("mcp-before"));
  console.log(`before ${before.path}`);

  const tree = await client.callTool({ name: "layers_list", arguments: { step: 0 } });
  const object = tree.structuredContent.layers.flatMap((l) => l.objects)[0];
  console.log(`moving object ${object.id} (${object.name}, ${object.tool})`);
  // `PropertyValue` is tagged: a position is `{kind:"position",lon,lat}` and
  // not a raw pair (`document.rs`, and the Task 4 test says so in as many
  // words). The cyclone's stroke sits around 40 W, 30 N under a default camera
  // showing the whole world; 60 E, 25 S is a move nobody could mistake for a
  // re-render.
  const moved = await client.callTool({
    name: "object_set",
    arguments: {
      object: object.id,
      step: 0,
      values: { Position: { kind: "position", lon: 60, lat: -25 } },
    },
  });
  if (moved.isError) throw new Error(`object_set was refused: ${JSON.stringify(moved.content)}`);

  // A capture settles by wall clock and gives up on a timeout, so one taken in
  // the same second as the write can show the frame before it and look exactly
  // like a failure that is not there. Two, spaced; the second is the one that
  // decides.
  await sleep(1500);
  const afterFirst = await keep(shots, await app.capture("mcp-after-1"));
  console.log(`after (first) ${afterFirst.path}`);
  await sleep(1500);
  const after = await keep(shots, await app.capture("mcp-after-2"));
  console.log(`after ${after.path}`);

  const same = before.bytes.equals(after.bytes);
  console.log(same ? "FAIL: the map did not change" : `OK: ${before.path} -> ${after.path}`);
  process.exitCode = same ? 1 : 0;
} finally {
  // Closed before the app, so the session is released rather than dropped
  // under the listener.
  await client?.close().catch(() => {});
  await app.close();
  await rm(automation, { recursive: true, force: true });
  // The promise this check makes is that the socket is gone with the app, so
  // it is worth one line to say so. By port, never by name: a pattern would
  // find the person's own application.
  const listeners = await promisify(execFile)("lsof", ["-nP", `-iTCP:${port}`])
    .then((r) => r.stdout.trim())
    .catch(() => "");
  console.log(listeners ? `STILL LISTENING on ${port}:\n${listeners}` : `port ${port} is free`);
}

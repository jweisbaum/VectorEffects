/**
 * A client for the app's automation endpoint (spec.md 14, M71).
 *
 * `tauri-plugin-webdriver-automation` serves a flat REST API on loopback —
 * `POST /element/find`, `/element/click`, `/script/execute-async` and so on.
 * It is *not* W3C WebDriver despite the name, so no off-the-shelf client
 * drives it; this is that client, shared by the MCP server and the CLI.
 *
 * **Screenshots do not come from the endpoint.** Its `/screenshot` serialises
 * the DOM into an SVG `foreignObject` and rasterises that, which does not
 * capture a canvas's contents — the map would come out blank, and the map is
 * the thing worth a picture. `capture()` asks the application to read its own
 * framebuffer back instead (`window.__veCapture`, dev builds only), which is
 * the mechanism the capture suite already uses and which waits for tiles to
 * settle first.
 */

import { spawn } from "node:child_process";
import { once } from "node:events";
import { createInterface } from "node:readline";

/** Where the plugin announces itself. The only place it does. */
const PORT_LINE = /^\[webdriver] listening on port (\d+)$/;

/**
 * The port from a line of the application's output, or null.
 *
 * Anchored, because the app's log is full of lines that merely mention ports
 * and one of them matching would point the driver at nothing.
 */
export function portFromLine(line) {
  const found = PORT_LINE.exec(line.trim());
  return found ? Number(found[1]) : null;
}

/** How long to wait for the application to announce its port. */
const START_TIMEOUT_MS = 180_000;

/**
 * Signals a child's whole process *group*, falling back to the child alone.
 *
 * The negative pid is the group, which is why `launch` detaches: `npm run
 * dev:webdriver` is a wrapper around the Tauri CLI, which starts Vite as its
 * `beforeDevCommand` and then cargo, so signalling the wrapper alone leaves
 * Vite holding port 5173 and the next run cannot start.
 */
function signalGroup(child, name) {
  try {
    process.kill(-child.pid, name);
  } catch {
    // Already gone, or never had a group of its own.
    try {
      child.kill(name);
    } catch {
      /* gone */
    }
  }
}

/**
 * Stops the whole tree a `launch` started, and waits for it to go.
 *
 * Signals the group even when the child itself has already exited: the group
 * outlives its leader, so `npm` having died says nothing about the Vite and
 * cargo it started. That is exactly the case on the `exited` branch below.
 */
async function stopTree(child) {
  signalGroup(child, "SIGTERM");
  if (child.exitCode === null && child.signalCode === null) {
    await Promise.race([once(child, "exit"), new Promise((r) => setTimeout(r, 5000).unref())]);
  }
  signalGroup(child, "SIGKILL");
}

/**
 * Starts the application with its automation endpoint and waits for the port.
 *
 * The endpoint binds `127.0.0.1:0` — a random port, announced on stdout and
 * nowhere else: no file, no environment variable, no fixed number. So the
 * driver has to own the process to know where to talk to it.
 *
 * `env` is merged over the caller's own environment. It exists for
 * `VE_AUTOMATION_ROOT` (`paths.rs`): a run that changes a setting — the MCP
 * service's, say — would otherwise write it into the person's real settings
 * file and leave their own application carrying it on its next launch.
 *
 * **A failure here stops the tree before it rethrows.** The group is detached
 * and no `Driver` is returned on the losing branches, so a caller that put its
 * cleanup in a `finally` after `await launch(...)` has nothing left to stop
 * with: the whole tree — npm, Vite, cargo, the application — is orphaned, and
 * the next run dies on port 5173. That happened, and the tree had to be found
 * and signalled by hand.
 *
 * `timeoutMs` is how long the port line is waited for. It is a parameter so
 * the failure path can be exercised without a cold build.
 */
export async function launch({
  cwd = process.cwd(),
  onLog,
  env = {},
  timeoutMs = START_TIMEOUT_MS,
} = {}) {
  // Its own process group, so it can be killed as a tree. `npm run
  // dev:webdriver` is a wrapper around the Tauri CLI, which starts Vite as
  // its `beforeDevCommand` and then cargo: signalling only the wrapper leaves
  // Vite holding port 5173 and the next run cannot start.
  const child = spawn("npm", ["run", "dev:webdriver"], {
    cwd,
    stdio: ["ignore", "pipe", "pipe"],
    detached: true,
    env: { ...process.env, FORCE_COLOR: "0", ...env },
  });

  let port = null;
  const waiters = [];
  // The last few lines, so a failure to start can say what went wrong rather
  // than only that it did.
  const recent = [];
  const note = (line) => {
    onLog?.(line);
    recent.push(line);
    if (recent.length > 12) recent.shift();
  };
  const lines = createInterface({ input: child.stdout });
  lines.on("line", (line) => {
    note(line);
    const found = portFromLine(line);
    if (found !== null && port === null) {
      port = found;
      for (const resolve of waiters.splice(0)) resolve(found);
    }
  });
  createInterface({ input: child.stderr }).on("line", note);

  const exited = once(child, "exit").then(([code]) => {
    const tail = recent.filter((line) => line.trim()).slice(-6).join("\n  ");
    throw new Error(
      `the application exited with code ${code} before it was ready:\n  ${tail}`,
    );
  });
  const announced = new Promise((resolve) => {
    if (port !== null) resolve(port);
    else waiters.push(resolve);
  });
  const timeout = new Promise((_, reject) =>
    setTimeout(
      () => reject(new Error(`no automation port announced within ${timeoutMs} ms`)),
      timeoutMs,
    ).unref(),
  );

  try {
    await Promise.race([announced, exited, timeout]);
  } catch (failure) {
    await stopTree(child);
    throw failure;
  }
  return new Driver(port, child);
}

/** Connects to an endpoint that is already listening. */
export function connect(port) {
  return new Driver(Number(port), null);
}

/** The application's automation endpoint. */
export class Driver {
  constructor(port, child) {
    this.port = port;
    this.child = child;
  }

  /** One call to the endpoint. Every route is a POST with a JSON body. */
  async call(route, body = {}) {
    const response = await fetch(`http://127.0.0.1:${this.port}${route}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    });
    const text = await response.text();
    let value;
    try {
      value = text ? JSON.parse(text) : null;
    } catch {
      throw new Error(`${route} answered with something that is not JSON: ${text.slice(0, 200)}`);
    }
    if (!response.ok) {
      throw new Error(`${route} failed (${response.status}): ${JSON.stringify(value)}`);
    }
    return value;
  }

  /**
   * Runs a script and returns what it resolves with.
   *
   * Two unwrappings, both of which are easy to get wrong and silent when you
   * do. A result comes back as `{"value": …}`, so a caller that returned the
   * body would hand back the envelope; and a script that *failed* reports it
   * inside that envelope as `{error, message}` rather than in the HTTP status,
   * so a caller that only checked the status would read an error as an answer.
   *
   * **A script hands its answer to the callback passed as its last
   * argument**, the ordinary executeAsync convention: the endpoint wraps the
   * script, appends `__done` to the arguments and applies it. Calling
   * `window.__WEBDRIVER__.resolve` directly does not work — the id there is a
   * uuid the caller never sees, and a wrong one makes the plugin panic inside
   * `.expect("no pending script with that id")` *while holding* its pending
   * table's mutex, poisoning it so every later call panics too. One bad script
   * bricks the endpoint for the life of the process.
   *
   * The same brick follows a script that runs longer than the endpoint's 30 s
   * timeout, since the entry is removed on timeout and the late resolve then
   * finds nothing. So long work is started and polled for, never awaited
   * inside the webview. See `capture`.
   */
  async evaluate(script, args = []) {
    const body = await this.call("/script/execute-async", { script, args });
    const value = body && typeof body === "object" && "value" in body ? body.value : body;
    if (value && typeof value === "object" && typeof value.error === "string") {
      throw new Error(`script failed: ${value.error} — ${value.message ?? ""}`);
    }
    return value;
  }

  /**
   * Asks the application for a picture of itself, and says where it landed.
   *
   * The hook is absent in a built bundle, so this says so plainly rather than
   * waiting out the timeout.
   */
  async capture(name = "driver") {
    await this.ready();
    await this.awaitCapture();
    const safe = String(name).replace(/[^A-Za-z0-9-]/g, "") || "driver";

    // Started, not awaited: the capture waits for tiles to settle and then
    // encodes a megapixel PNG, which can outlast the endpoint's 30 s script
    // timeout — and a script that resolves after its timeout bricks the
    // endpoint (see `evaluate`). This returns at once and leaves the result on
    // the window for the poll below.
    const started = await this.evaluate(
      `var done = arguments[arguments.length - 1];
       if (!window.__veCapture) {
         done({error:"NoCaptureHook",message:"__veCapture is absent; this is not a dev build",stacktrace:""});
       } else {
         window.__veShot = {state:"running"};
         window.__veCapture(${JSON.stringify(safe)}).then(function(path){
           window.__veShot = {state:"done", path:path};
         }, function(e){
           window.__veShot = {state:"failed", message:String(e)};
         });
         done("started");
       }`,
    );
    if (started !== "started") throw new Error(`capture did not start: ${JSON.stringify(started)}`);

    const deadline = Date.now() + 120_000;
    for (;;) {
      const shot = await this.evaluate(
        `var done = arguments[arguments.length - 1]; done(window.__veShot || null);`,
      );
      if (shot && shot.state === "done") return shot.path ?? null;
      if (shot && shot.state === "failed") throw new Error(`capture failed: ${shot.message}`);
      if (Date.now() > deadline) throw new Error("the capture did not finish within 120 s");
      await new Promise((resolve) => setTimeout(resolve, 250));
    }
  }

  /**
   * Waits for the capture hook to appear.
   *
   * It is installed by the map's own effect, so it exists only once a project
   * is open *and* React has mounted the map — which is a moment after
   * `open_project` returns, not the same instant. Polled rather than assumed,
   * because the failure without it is "this is not a dev build", which sends
   * the reader looking in entirely the wrong place.
   */
  async awaitCapture({ timeoutMs = 30_000 } = {}) {
    await this.awaitHook("__veCapture", "capturing", {
      timeoutMs,
      why:
        "a picture is read back from the map's renderer, so a project has to be open",
    });
  }

  /**
   * Waits for one of the application's dev-only hooks to appear.
   *
   * They are installed by React effects, so they exist a moment after the
   * component that owns them mounts rather than the instant a command
   * returns. Polled rather than assumed, because the failure without it reads
   * as "this is not a dev build", which sends the reader looking in entirely
   * the wrong place.
   */
  async awaitHook(name, doing, { timeoutMs = 30_000, why = "" } = {}) {
    const deadline = Date.now() + timeoutMs;
    for (;;) {
      const present = await this.evaluate(
        `var done = arguments[arguments.length - 1]; done(typeof window.${name} === "function");`,
      );
      if (present === true) return;
      if (Date.now() > deadline) {
        throw new Error(
          `no hook for ${doing} appeared (window.${name})` +
            `${why ? `: ${why}` : ""} — and it exists in dev builds only`,
        );
      }
      await new Promise((resolve) => setTimeout(resolve, 250));
    }
  }

  /**
   * Waits until the application has a webview to talk to.
   *
   * The endpoint announces its port as soon as the server binds, which is
   * before the window exists — every call in that gap answers `no window`.
   */
  async ready({ timeoutMs = 60_000 } = {}) {
    const deadline = Date.now() + timeoutMs;
    for (;;) {
      try {
        const handles = await this.call("/window/handles");
        if (Array.isArray(handles) && handles.length > 0) return handles;
      } catch {
        // The server is up but not answering yet; try again below.
      }
      if (Date.now() > deadline) throw new Error("the application opened no window");
      await new Promise((resolve) => setTimeout(resolve, 200));
    }
  }

  /**
   * Opens a project, so there is a map to photograph.
   *
   * Through the application's own opening (`window.__veOpen`), not by
   * invoking `open_project` directly. The command is only half of opening:
   * the other half is the frontend's state, and invoking it alone moved the
   * backend while the interface stayed on the start screen, holding a project
   * it would not show (M76). Unsaved changes are discarded without asking,
   * which is what a driver wants and why the hook is dev-only.
   */
  async open(path) {
    await this.ready();
    await this.awaitHook("__veOpen", "opening");
    return this.evaluate(
      `var done = arguments[arguments.length - 1];
       window.__veOpen(${JSON.stringify(path)}).then(function(name){
         done(name || true);
       }, function(e){
         done({error:"OpenFailed",message:String(e),stacktrace:""});
       });`,
    );
  }

  /** Finds one element, returning the endpoint's handle for it. */
  find(selector) {
    return this.call("/element/find", { using: "css selector", value: selector });
  }

  /**
   * Stops the application, if this driver started it — the whole tree of it.
   *
   * `connect` makes a driver with no child of its own: that application
   * belongs to whoever started it and is not this one's to stop.
   */
  async close() {
    if (!this.child) return;
    await stopTree(this.child);
  }
}

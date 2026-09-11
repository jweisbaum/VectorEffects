/**
 * An MCP server over the application's automation endpoint (M71).
 *
 * Exposes the real running application as tools: find and click elements,
 * type, run a script, and take a picture of the map. The picture is the point
 * — see `client.mjs` for why it does not come from the endpoint's own
 * `/screenshot`.
 *
 * The application is started on the first tool call and held until the server
 * stops, because the endpoint's port is announced on stdout and nowhere else:
 * whoever wants to talk to it has to own the process.
 */

import { readFile } from "node:fs/promises";

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";

import { connect, launch } from "./client.mjs";

const root = new URL("../..", import.meta.url).pathname;

let driver = null;
let starting = null;

/** The application, started if it is not already running. */
async function app() {
  if (driver) return driver;
  if (process.env.VE_DRIVER_PORT) {
    driver = connect(process.env.VE_DRIVER_PORT);
    return driver;
  }
  // One start even if several tools are called at once: a second `npm run
  // dev:webdriver` would build the same crate into the same target directory
  // and block on the lock.
  starting ??= launch({ cwd: root }).then((started) => {
    driver = started;
    starting = null;
    return started;
  });
  return starting;
}

const server = new McpServer({ name: "ve-driver", version: "0.1.0" });

const text = (value) => ({
  content: [{ type: "text", text: typeof value === "string" ? value : JSON.stringify(value, null, 2) }],
});

server.registerTool(
  "screenshot",
  {
    title: "Screenshot the map",
    description:
      "Read the application's own framebuffer back as a PNG, once its tiles have settled. " +
      "This captures the WebGL map, which the endpoint's own screenshot cannot.",
    inputSchema: { name: z.string().optional().describe("basename for the file beside the logs") },
  },
  async ({ name }) => {
    const path = await (await app()).capture(name ?? "driver");
    if (path === null) {
      throw new Error(
        "nothing was captured: the picture is read back from the renderer's framebuffer, " +
          "and there is no renderer until a project is open — use the `open` tool first",
      );
    }
    if (typeof path !== "string") {
      throw new Error(`capture did not return a path: ${JSON.stringify(path)}`);
    }
    const png = await readFile(path);
    return {
      content: [
        { type: "image", data: png.toString("base64"), mimeType: "image/png" },
        { type: "text", text: path },
      ],
    };
  },
);

server.registerTool(
  "open",
  {
    title: "Open a project",
    description:
      "Open a .veproj so there is a map to photograph. A cold start is the start screen, " +
      "and a screenshot reads the renderer's framebuffer, which does not exist until then. " +
      "assets/samples holds cyclone, gyre and trade-winds.",
    inputSchema: { path: z.string().describe("absolute path, or relative to the repository") },
  },
  async ({ path }) => {
    const absolute = path.startsWith("/") ? path : new URL(path, `file://${root}`).pathname;
    return text(await (await app()).open(absolute));
  },
);

server.registerTool(
  "evaluate",
  {
    title: "Run a script in the webview",
    description:
      "Run JavaScript in the running application and return what it resolves with. " +
      'The script must hand its answer back: window.__WEBDRIVER__.resolve("__CALLBACK_ID__", value).',
    inputSchema: { script: z.string() },
  },
  async ({ script }) => text(await (await app()).evaluate(script)),
);

server.registerTool(
  "click",
  {
    title: "Click an element",
    description: "Find one element by CSS selector and click it.",
    inputSchema: { selector: z.string() },
  },
  async ({ selector }) => {
    const driving = await app();
    const element = await driving.find(selector);
    return text(await driving.call("/element/click", element));
  },
);

server.registerTool(
  "type",
  {
    title: "Type into an element",
    description: "Find one element by CSS selector and send it keystrokes.",
    inputSchema: { selector: z.string(), keys: z.string() },
  },
  async ({ selector, keys }) => {
    const driving = await app();
    const element = await driving.find(selector);
    return text(await driving.call("/element/send-keys", { ...element, text: keys }));
  },
);

server.registerTool(
  "text",
  {
    title: "Read an element's text",
    description: "Find one element by CSS selector and return its text.",
    inputSchema: { selector: z.string() },
  },
  async ({ selector }) => {
    const driving = await app();
    const element = await driving.find(selector);
    return text(await driving.call("/element/text", element));
  },
);

server.registerTool(
  "stop",
  { title: "Stop the application", description: "Close the application this server started." },
  async () => {
    if (!driver) return text("nothing was running");
    await driver.close();
    driver = null;
    return text("stopped");
  },
);

const shutdown = async () => {
  await driver?.close();
  process.exit(0);
};
process.on("SIGINT", shutdown);
process.on("SIGTERM", shutdown);

await server.connect(new StdioServerTransport());

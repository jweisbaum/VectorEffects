#!/usr/bin/env node
// The VectorEffects extension for Claude Desktop (spec.md 8.8).
//
// A desktop extension is a local stdio server, and the VectorEffects MCP
// service is HTTP on loopback, so this stands between them: every JSON-RPC
// message Claude Desktop writes to stdin is posted to the service, and every
// answer is written to stdout. It has no dependencies, because Claude Desktop
// runs it with its own Node and nothing else is installed.
//
// The port and the token are read from the application's settings file for
// every request. The bundle therefore holds no secret, and a rotated token or
// a changed port needs no reinstall.
//
// The application comes and goes while Claude Desktop stays open, so nothing
// here assumes it is there: a session the application has forgotten (it was
// restarted) is opened again without the client noticing, and when the
// application is not running at all the extension still starts, offers one
// tool that says so, and swaps in the real tools when it appears.
"use strict";

const fs = require("node:fs");
const http = require("node:http");
const readline = require("node:readline");

const SETTINGS = process.env.VE_SETTINGS;
const OFF =
  "VectorEffects is not running, or its MCP service is off. Open VectorEffects, then Settings > MCP service > Enable, and try again.";
// What this is for, said even while the application is away. A client asks
// for the instructions once, at `initialize`, and Claude Desktop is usually
// started first: without this the whole session is told only that something
// is off, and a request for a GRIB is answered with a script.
const WHAT =
  "VectorEffects is an application on this computer that makes global wind and ocean-current fields — real past weather downloaded, or invented weather drawn — and exports them as GRIB2 or Zarr. Use its tools whenever the user names VectorEffects or wants such a field or file, rather than writing code to make one.";
const STATUS_TOOL = {
  name: "vectoreffects_status",
  description: `${WHAT} It is not reachable right now, so its tools are not listed. Ask the user to open VectorEffects (Settings > MCP service > Enable), then call this to connect; the real tools then appear, vectoreffects_guide first among them.`,
  inputSchema: { type: "object", properties: {} },
};

let hello = null; // the client's `initialize`, replayed to open a new session
let session = null; // the service's Mcp-Session-Id, once there is one
let connecting = null; // one reconnection at a time, shared by whoever needs it
let wasOffline = false;

const write = (message) => process.stdout.write(JSON.stringify(message) + "\n");

/** Where the service is, or null while it is off. */
function service() {
  try {
    const mcp = JSON.parse(fs.readFileSync(SETTINGS, "utf8")).app.mcp;
    return mcp && mcp.enabled && mcp.token ? mcp : null;
  } catch (_) {
    return null;
  }
}

/**
 * Posts one message. Resolves with the HTTP status and the JSON-RPC messages
 * that came back, whether as one JSON body or as a stream of events; rejects
 * when the service cannot be reached.
 */
function post(message, useSession) {
  return new Promise((resolve, reject) => {
    const mcp = service();
    if (!mcp) return reject(new Error(OFF));
    const body = Buffer.from(JSON.stringify(message));
    const headers = {
      "content-type": "application/json",
      accept: "application/json, text/event-stream",
      authorization: `Bearer ${mcp.token}`,
      "content-length": body.length,
    };
    if (useSession && session) headers["mcp-session-id"] = session;
    const request = http.request(
      { host: "127.0.0.1", port: mcp.port, path: "/mcp", method: "POST", headers },
      (response) => {
        const opened = response.headers["mcp-session-id"];
        const streamed = String(response.headers["content-type"] || "").includes("text/event-stream");
        const messages = [];
        let buffer = "";
        response.setEncoding("utf8");
        response.on("data", (chunk) => {
          buffer += chunk;
          if (!streamed) return;
          // Events end at a blank line; a progress notification has to reach
          // the client while the call it belongs to is still running.
          let end;
          while ((end = buffer.search(/\r?\n\r?\n/)) >= 0) {
            const event = buffer.slice(0, end);
            buffer = buffer.slice(end).replace(/^\r?\n\r?\n/, "");
            const data = event
              .split(/\r?\n/)
              .filter((line) => line.startsWith("data:"))
              .map((line) => line.slice(5).trimStart())
              .join("\n");
            if (!data) continue;
            try {
              const parsed = JSON.parse(data);
              if (message.id !== undefined && parsed.id === message.id) messages.push(parsed);
              else write(parsed);
            } catch (_) {
              // Not JSON: a keep-alive or a comment. Nothing to pass on.
            }
          }
        });
        response.on("end", () => {
          if (!streamed && buffer.trim()) {
            try {
              messages.push(JSON.parse(buffer));
            } catch (_) {
              // An error page rather than JSON-RPC; the status says enough.
            }
          }
          resolve({ status: response.statusCode, opened, messages });
        });
        response.on("error", reject);
      },
    );
    request.on("error", () => reject(new Error(OFF)));
    request.end(body);
  });
}

/** Opens a session by replaying the client's own `initialize`. */
function connect() {
  if (connecting) return connecting;
  connecting = (async () => {
    session = null;
    const opening = await post({ ...hello, id: "vectoreffects-bridge-hello" }, false);
    if (opening.status >= 400 || !opening.opened) throw new Error(`VectorEffects refused the connection (${opening.status}).`);
    session = opening.opened;
    await post({ jsonrpc: "2.0", method: "notifications/initialized" }, true);
    if (wasOffline) {
      wasOffline = false;
      write({ jsonrpc: "2.0", method: "notifications/tools/list_changed" });
    }
  })().finally(() => {
    connecting = null;
  });
  return connecting;
}

/** What the client gets when the application is not there. */
function offline(message) {
  wasOffline = true;
  if (message.id === undefined || message.id === null) return;
  if (message.method === "tools/list") return write({ jsonrpc: "2.0", id: message.id, result: { tools: [STATUS_TOOL] } });
  if (message.method === "tools/call")
    return write({ jsonrpc: "2.0", id: message.id, result: { content: [{ type: "text", text: OFF }], isError: true } });
  if (message.method === "ping") return write({ jsonrpc: "2.0", id: message.id, result: {} });
  write({ jsonrpc: "2.0", id: message.id, error: { code: -32000, message: OFF } });
}

async function forward(message) {
  try {
    if (message.method === "initialize") {
      hello = message;
      session = null;
      try {
        const opening = await post(message, false);
        if (opening.status < 400 && opening.opened && opening.messages.length) {
          session = opening.opened;
          return opening.messages.forEach(write);
        }
      } catch (_) {
        // Not there yet. Start anyway: see `offline`.
      }
      wasOffline = true;
      return write({
        jsonrpc: "2.0",
        id: message.id,
        result: {
          protocolVersion: (message.params && message.params.protocolVersion) || "2025-06-18",
          capabilities: { tools: { listChanged: true } },
          serverInfo: { name: "VectorEffects", version: "0" },
          instructions: `${WHAT} ${OFF}`,
        },
      });
    }
    // The client's own `initialized` went to whichever session it followed,
    // or to none; `connect` sends one for every session it opens.
    if (message.method === "notifications/initialized" && !session) return;
    // The stand-in tool is this bridge's own, never the application's: it
    // connects, which is all it is for, and says how that went.
    if (message.method === "tools/call" && message.params && message.params.name === STATUS_TOOL.name) {
      await connect();
      return write({ jsonrpc: "2.0", id: message.id, result: { content: [{ type: "text", text: "VectorEffects is connected; its tools are listed now." }] } });
    }
    if (!session) await connect();
    let answer = await post(message, true);
    if (answer.status === 404) {
      // The application was restarted and has forgotten the session.
      await connect();
      answer = await post(message, true);
    }
    if (answer.status >= 400 && message.id !== undefined && !answer.messages.length) {
      return write({ jsonrpc: "2.0", id: message.id, error: { code: -32000, message: `VectorEffects refused the request (${answer.status}).` } });
    }
    answer.messages.forEach(write);
  } catch (error) {
    // stderr is Claude Desktop's log for the extension; stdout is the protocol.
    process.stderr.write(`vectoreffects: ${message.method}: ${error && error.message}\n`);
    offline(message);
  }
}

readline.createInterface({ input: process.stdin }).on("line", (line) => {
  if (!line.trim()) return;
  let message;
  try {
    message = JSON.parse(line);
  } catch (_) {
    return;
  }
  void forward(message);
});

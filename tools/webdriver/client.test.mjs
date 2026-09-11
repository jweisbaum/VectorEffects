/**
 * The driver's parsing, which is where a wrong answer is silent.
 *
 * Run with `npm run tools:test`. Node's own runner, so the repository gains no
 * dependency for four assertions.
 */
import assert from "node:assert/strict";
import { test } from "node:test";

import { portFromLine } from "./client.mjs";

test("reads the port from the line the plugin announces", () => {
  assert.equal(portFromLine("[webdriver] listening on port 49557"), 49557);
  // The Tauri CLI colours and indents its own output; the plugin's line is a
  // bare println, so it arrives clean.
  assert.equal(portFromLine("  [webdriver] listening on port 1  "), 1);
});

test("ignores every other line that mentions a port", () => {
  // The app's log is full of these, and matching one points the driver at
  // nothing — anchored on purpose.
  for (const line of [
    "Local:   http://localhost:5173/",
    "note: [webdriver] listening on port 123 (from an old run)",
    "[webdriver] listening on port",
    "listening on port 49557",
    "[webdriver] listening on port abc",
    "",
  ]) {
    assert.equal(portFromLine(line), null, line);
  }
});

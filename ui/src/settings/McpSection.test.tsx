// @vitest-environment happy-dom
/**
 * The MCP section shows the URL and a ready client configuration only while
 * the service is on, and every change goes through the command (spec 8.8).
 */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { McpStatus } from "../generated/McpStatus";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const held = vi.hoisted(() => {
  const off: McpStatus = { enabled: false, port: 47391, token: "", bound_port: null, bind_error: null, sessions: 0, last_tool: null };
  const on: McpStatus = { ...off, enabled: true, token: "tok_abc", bound_port: 47391 };
  return {
    status: off,
    on,
    mcpStatus: vi.fn(async () => held.status),
    setMcp: vi.fn(async (enabled: boolean, port: number) => {
      held.status = enabled ? { ...held.on, port } : { ...held.on, enabled: false, token: "", bound_port: null, port };
      return held.status;
    }),
    rotateMcpToken: vi.fn(async () => {
      held.status = { ...held.status, token: "tok_new" };
      return held.status;
    }),
  };
});

vi.mock("../ipc", () => ({
  api: { mcpStatus: held.mcpStatus, setMcp: held.setMcp, rotateMcpToken: held.rotateMcpToken },
  IpcError: class extends Error {},
}));

import McpSection from "./McpSection";

let root: Root;
let host: HTMLDivElement;

beforeEach(() => {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  held.mcpStatus.mockClear();
  held.setMcp.mockClear();
  held.rotateMcpToken.mockClear();
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  held.status = { enabled: false, port: 47391, token: "", bound_port: null, bind_error: null, sessions: 0, last_tool: null };
});

async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

describe("McpSection", () => {
  it("shows no URL while off and one with the token once on", async () => {
    act(() => root.render(<McpSection onError={() => {}} />));
    await flush();
    expect(host.textContent).not.toContain("http://127.0.0.1");
    const toggle = host.querySelector<HTMLInputElement>('input[type="checkbox"]');
    await act(async () => {
      toggle?.click();
    });
    await flush();
    expect(held.setMcp).toHaveBeenCalledWith(true, 47391);
    expect(host.textContent).toContain("http://127.0.0.1:47391/mcp");
    const snippet = host.querySelector("pre")?.textContent ?? "";
    expect(snippet).toContain("Bearer tok_abc");
    expect(snippet).toContain("claude mcp add");
  });

  it("fetches its status once, regardless of a new onError identity on re-render", async () => {
    act(() => root.render(<McpSection onError={() => {}} />));
    await flush();
    expect(held.mcpStatus).toHaveBeenCalledTimes(1);
    // A parent re-render (SettingsDialog holds several unrelated useStates)
    // passes a *new* `onError` function each time; that alone must not
    // re-trigger the fetch.
    act(() => root.render(<McpSection onError={() => {}} />));
    await flush();
    expect(held.mcpStatus).toHaveBeenCalledTimes(1);
  });

  it("rotates the token through the command", async () => {
    held.status = held.on;
    act(() => root.render(<McpSection onError={() => {}} />));
    await flush();
    const rotate = Array.from(host.querySelectorAll("button")).find((b) => b.textContent?.includes("Rotate"));
    await act(async () => {
      rotate?.click();
    });
    await flush();
    expect(held.rotateMcpToken).toHaveBeenCalled();
    expect(host.textContent).toContain("tok_new");
  });
});

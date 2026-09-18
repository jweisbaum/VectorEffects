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
  const off: McpStatus = { enabled: false, port: 47391, token: "", bound_port: null, bind_error: null, sessions: 0, last_tool: null, clients: ["claude_code", "codex", "claude_desktop"] };
  const on: McpStatus = { ...off, enabled: true, token: "tok_abc", bound_port: 47391 };
  // What the command answers: where it wrote the client's skill, if it has one.
  const registered = (client: string): { skill: string | null } => ({
    skill:
      client === "claude_code"
        ? "/home/someone/.claude/skills/vectoreffects/SKILL.md"
        : client === "claude_desktop"
          ? "/home/someone/settings/VectorEffects-skill.zip"
          : null,
  });
  return {
    status: off,
    on,
    registered,
    mcpStatus: vi.fn(async () => held.status),
    setMcp: vi.fn(async (enabled: boolean, port: number) => {
      held.status = enabled ? { ...held.on, port } : { ...held.on, enabled: false, token: "", bound_port: null, port };
      return held.status;
    }),
    rotateMcpToken: vi.fn(async () => {
      held.status = { ...held.status, token: "tok_new" };
      return held.status;
    }),
    registerMcpClient: vi.fn(async (client: string): Promise<{ skill: string | null }> => held.registered(client)),
  };
});

vi.mock("../ipc", () => ({
  api: {
    mcpStatus: held.mcpStatus,
    setMcp: held.setMcp,
    rotateMcpToken: held.rotateMcpToken,
    registerMcpClient: held.registerMcpClient,
  },
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
  held.registerMcpClient.mockReset();
  held.registerMcpClient.mockImplementation(async (client: string) => held.registered(client));
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  held.status = { enabled: false, port: 47391, token: "", bound_port: null, bind_error: null, sessions: 0, last_tool: null, clients: ["claude_code", "codex", "claude_desktop"] };
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

  const button = (words: string) =>
    Array.from(host.querySelectorAll("button")).find((b) => b.textContent === words);

  it("offers no client button while the service is off", async () => {
    act(() => root.render(<McpSection onError={() => {}} />));
    await flush();
    expect(button("Add to Claude Code")).toBeUndefined();
    expect(button("Add to Codex")).toBeUndefined();
  });

  it("adds itself to a client through the command, and asks for an update once the token has moved on", async () => {
    held.status = held.on;
    act(() => root.render(<McpSection onError={() => {}} />));
    await flush();
    await act(async () => {
      button("Add to Claude Code")?.click();
    });
    await flush();
    expect(held.registerMcpClient).toHaveBeenCalledWith("claude_code");
    expect(button("Added to Claude Code")).toBeDefined();
    // The other client was not touched and still says so.
    expect(button("Add to Codex")).toBeDefined();
    expect(host.textContent).toContain("restarted");
    // It was given the skill too, and says where; Codex has none to be given.
    expect(host.textContent).toContain("/home/someone/.claude/skills/vectoreffects/SKILL.md");

    // Claude Code now holds a token the listener no longer answers.
    const rotate = Array.from(host.querySelectorAll("button")).find((b) => b.textContent?.includes("Rotate"));
    await act(async () => {
      rotate?.click();
    });
    await flush();
    expect(button("Update in Claude Code")).toBeDefined();
    expect(host.textContent).not.toContain("restarted");
  });

  it("opens the extension in Claude Desktop, never claims it was installed, and never asks for an update", async () => {
    held.status = held.on;
    act(() => root.render(<McpSection onError={() => {}} />));
    await flush();
    await act(async () => {
      button("Add to Claude Desktop")?.click();
    });
    await flush();
    expect(held.registerMcpClient).toHaveBeenCalledWith("claude_desktop");
    expect(button("Opened in Claude Desktop")).toBeDefined();
    expect(host.textContent).toContain("confirm it there");
    expect(host.textContent).not.toContain("picks it up when it is restarted");
    // The skill cannot ride in the extension: the person is told where it is.
    expect(host.textContent).toContain("VectorEffects-skill.zip");
    expect(host.textContent).toContain("upload");

    // The extension reads the token as it goes: a rotation stales nothing.
    const rotate = Array.from(host.querySelectorAll("button")).find((b) => b.textContent?.includes("Rotate"));
    await act(async () => {
      rotate?.click();
    });
    await flush();
    expect(button("Opened in Claude Desktop")).toBeDefined();
    expect(button("Update in Claude Desktop")).toBeUndefined();
  });

  it("offers only the clients this platform has", async () => {
    held.status = { ...held.on, clients: ["claude_code", "codex"] };
    act(() => root.render(<McpSection onError={() => {}} />));
    await flush();
    expect(button("Add to Codex")).toBeDefined();
    expect(button("Add to Claude Desktop")).toBeUndefined();
  });

  it("reports a client that could not be written and does not claim it was", async () => {
    held.status = held.on;
    const refusal = new Error("the `claude` command was not found");
    held.registerMcpClient.mockImplementation(async () => {
      throw refusal;
    });
    const onError = vi.fn();
    act(() => root.render(<McpSection onError={onError} />));
    await flush();
    await act(async () => {
      button("Add to Codex")?.click();
    });
    await flush();
    expect(held.registerMcpClient).toHaveBeenCalledWith("codex");
    expect(onError).toHaveBeenCalledWith(refusal);
    expect(button("Add to Codex")).toBeDefined();
    expect(button("Added to Codex")).toBeUndefined();
  });
});
